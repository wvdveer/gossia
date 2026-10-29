//! Moving values between the data tree and Python, and Python errors into
//! error kinds (DESIGN.md, "Result checking", "Scalar types", and the table in
//! "Semantics: Python first, Excel second").

use std::rc::Rc;

use chrono::{
    DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike,
};
use cliotree_core::{DateTimeValue, ErrorKind, ErrorValue, Node, Path, Value};
use indexmap::IndexMap;
use rustpython::vm::builtins::{
    PyBaseExceptionRef, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple,
};
use rustpython::vm::{AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine};

use super::node::{self, _cliotree::NodeProxy};
use super::shared::Shared;
use crate::engine::{Depth, Failure, Read};

// ------------------------------------------------------- data to Python

/// A node as a formula sees it: a plain value for a scalar, a proxy for an
/// array or an object, an `ErrorValue` for a node that holds an error.
pub(super) fn node_to_py(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    sheet: &Rc<str>,
    path: Path,
    node: &Node,
) -> PyResult {
    Ok(match node.value() {
        Value::Null => vm.ctx.none(),
        Value::Bool(b) => vm.ctx.new_bool(*b).into(),
        Value::Int(i) => vm.ctx.new_int(*i).into(),
        Value::Float(x) => vm.ctx.new_float(*x).into(),
        Value::Str(s) => vm.ctx.new_str(s.as_str()).into(),
        Value::Date(d) => date_to_py(shared, vm, *d)?,
        Value::DateTime(dt) => datetime_to_py(shared, vm, dt)?,
        Value::Array(_) | Value::Object(_) => node::proxy(shared, vm, sheet, path, node),
        Value::Error(e) => error_to_py(shared, vm, sheet, e)?,
    })
}

/// A node, and everything in it, as plain Python lists, dicts and values.
pub(super) fn node_to_plain(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    sheet: &Rc<str>,
    path: &Path,
    node: &Node,
) -> PyResult {
    match node.value() {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                out.push(node_to_plain(shared, vm, sheet, &path.index(i), item)?);
            }
            Ok(vm.ctx.new_list(out).into())
        }
        Value::Object(fields) => {
            let dict = vm.ctx.new_dict();
            for (key, item) in fields {
                let value = node_to_plain(shared, vm, sheet, &path.key(key.as_str()), item)?;
                dict.set_item(key.as_str(), value, vm)?;
            }
            Ok(dict.into())
        }
        _ => node_to_py(shared, vm, sheet, path.clone(), node),
    }
}

pub(super) fn error_to_py(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    sheet: &str,
    error: &ErrorValue,
) -> PyResult {
    let origin = match &error.origin {
        Some(path) => crate::Loc::new(sheet, path.clone()).to_string(),
        None => sheet.to_owned(),
    };
    let classes = shared.classes();
    classes.error_value.call(
        (
            error.kind.code(),
            error.message.as_str(),
            origin,
            vm.ctx.none(),
        ),
        vm,
    )
}

fn date_to_py(shared: &Rc<Shared>, vm: &VirtualMachine, d: NaiveDate) -> PyResult {
    shared
        .classes()
        .date
        .call((d.year(), d.month(), d.day()), vm)
}

fn datetime_to_py(shared: &Rc<Shared>, vm: &VirtualMachine, dt: &DateTimeValue) -> PyResult {
    let classes = shared.classes();
    let naive = match dt {
        DateTimeValue::Naive(n) => *n,
        DateTimeValue::Zoned(z) => z.naive_local(),
    };
    let micros = naive.nanosecond() / 1_000;
    let mut args: Vec<PyObjectRef> = [
        i64::from(naive.year()),
        i64::from(naive.month()),
        i64::from(naive.day()),
        i64::from(naive.hour()),
        i64::from(naive.minute()),
        i64::from(naive.second()),
        i64::from(micros),
    ]
    .into_iter()
    .map(|n| vm.ctx.new_int(n).into())
    .collect();
    if let DateTimeValue::Zoned(z) = dt {
        let offset = z.offset().local_minus_utc();
        let delta = classes.timedelta.call((0, offset), vm)?;
        args.push(classes.timezone.call((delta,), vm)?);
    }
    classes.datetime.call(args, vm)
}

// ------------------------------------------------------- Python to data

/// Why a Python value could not become data.
pub(super) enum Bad {
    /// A value that does not fit: the kind says which error it is.
    Value(ErrorKind, String),
    /// Python raised while the value was being read.
    Raised(PyBaseExceptionRef),
}

impl From<PyBaseExceptionRef> for Bad {
    fn from(e: PyBaseExceptionRef) -> Bad {
        Bad::Raised(e)
    }
}

/// A formula's result as data. Conversion is strict (DESIGN.md, "Scalar
/// types"): `list` and `tuple` become arrays, `dict` becomes an object, `None`
/// is no value, a proxy is read in full, and nothing is coerced.
pub(super) fn to_value(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    obj: &PyObject,
) -> Result<Value, Bad> {
    if vm.is_none(obj) {
        return Ok(Value::Null);
    }
    // `bool` is a kind of `int` in Python, and is not accepted for numbers.
    if obj.class().is(vm.ctx.types.bool_type) {
        return Ok(Value::Bool(obj.is(&vm.ctx.true_value)));
    }
    if let Some(int) = obj.downcast_ref::<PyInt>() {
        return match int.try_to_primitive::<i64>(vm) {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => Err(Bad::Value(
                ErrorKind::Value,
                "the integer does not fit in 64 bits".to_owned(),
            )),
        };
    }
    if let Some(float) = obj.downcast_ref::<PyFloat>() {
        let x = float.to_f64();
        return if x.is_finite() {
            Ok(Value::Float(x))
        } else {
            Err(Bad::Value(
                ErrorKind::Value,
                format!("{x} cannot be stored: NaN and infinity are not allowed"),
            ))
        };
    }
    if let Some(text) = obj.downcast_ref::<PyStr>() {
        return Ok(Value::Str(text.to_string_lossy().into_owned()));
    }
    if let Some(proxy) = obj.downcast_ref::<NodeProxy>() {
        let ctx = shared
            .ctx()
            .ok_or_else(|| Bad::Value(ErrorKind::Ref, "no formula is running".to_owned()))?;
        return match ctx.read(&proxy.sheet, &proxy.path, Depth::Deep) {
            Read::Node(node) => Ok(fresh(&node)),
            Read::Missing | Read::NoSheet => Err(Bad::Value(
                ErrorKind::Ref,
                format!(
                    "there is nothing at {}",
                    crate::Loc::new(&*proxy.sheet, proxy.path.clone())
                ),
            )),
        };
    }
    let classes = shared.classes();
    if obj.is_instance(&classes.error_value, vm)? {
        return Ok(Value::Error(error_from_py(vm, obj)?));
    }
    if let Some(list) = obj.downcast_ref::<PyList>() {
        let items = list.borrow_vec().to_vec();
        return array_of(shared, vm, &items);
    }
    if let Some(tuple) = obj.downcast_ref::<PyTuple>() {
        return array_of(shared, vm, tuple.as_slice());
    }
    if let Some(dict) = obj.downcast_ref::<PyDict>() {
        let mut fields = IndexMap::new();
        for (key, value) in dict.items_vec() {
            let Some(name) = key.downcast_ref::<PyStr>() else {
                return Err(Bad::Value(
                    ErrorKind::Type,
                    format!("keys of an object are strings, not {}", key.class().name()),
                ));
            };
            let inner = to_value(shared, vm, &value)?;
            if matches!(inner, Value::Null) {
                // `None` is no value: the field is left out.
                continue;
            }
            fields.insert(name.to_string_lossy().into_owned(), Node::new(inner));
        }
        return Ok(Value::Object(fields));
    }
    if obj.is_instance(&classes.datetime, vm)? {
        return datetime_from_py(vm, obj);
    }
    if obj.is_instance(&classes.date, vm)? {
        return date_from_py(vm, obj);
    }
    Err(Bad::Value(
        ErrorKind::Type,
        format!("a {} cannot be stored in a node", obj.class().name()),
    ))
}

fn array_of(shared: &Rc<Shared>, vm: &VirtualMachine, items: &[PyObjectRef]) -> Result<Value, Bad> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let value = to_value(shared, vm, item)?;
        if matches!(value, Value::Null) {
            return Err(Bad::Value(
                ErrorKind::Type,
                "None cannot be an item of an array".to_owned(),
            ));
        }
        out.push(Node::new(value));
    }
    Ok(Value::Array(out))
}

/// A copy with new node ids, so that a result that contains part of the data
/// does not share node identity with it.
fn fresh(node: &Node) -> Value {
    match node.value() {
        Value::Array(items) => Value::Array(items.iter().map(|n| Node::new(fresh(n))).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, n)| (k.clone(), Node::new(fresh(n))))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn int_attr(vm: &VirtualMachine, obj: &PyObject, name: &'static str) -> PyResult<i64> {
    obj.get_attr(name, vm)?.try_into_value::<i64>(vm)
}

fn date_from_py(vm: &VirtualMachine, obj: &PyObject) -> Result<Value, Bad> {
    let (y, m, d) = (
        int_attr(vm, obj, "year")?,
        int_attr(vm, obj, "month")?,
        int_attr(vm, obj, "day")?,
    );
    NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)
        .map(Value::Date)
        .ok_or_else(|| Bad::Value(ErrorKind::Value, format!("{y}-{m}-{d} is not a date")))
}

fn datetime_from_py(vm: &VirtualMachine, obj: &PyObject) -> Result<Value, Bad> {
    let part = |name| int_attr(vm, obj, name);
    let date = NaiveDate::from_ymd_opt(
        part("year")? as i32,
        part("month")? as u32,
        part("day")? as u32,
    );
    let time = NaiveTime::from_hms_micro_opt(
        part("hour")? as u32,
        part("minute")? as u32,
        part("second")? as u32,
        part("microsecond")? as u32,
    );
    let (Some(date), Some(time)) = (date, time) else {
        return Err(Bad::Value(
            ErrorKind::Value,
            "that is not a date and time".to_owned(),
        ));
    };
    let naive = NaiveDateTime::new(date, time);
    let tz = obj.get_attr("tzinfo", vm)?;
    if vm.is_none(&tz) {
        return Ok(Value::DateTime(DateTimeValue::Naive(naive)));
    }
    let delta = vm.call_method(obj, "utcoffset", ())?;
    let seconds = vm
        .call_method(&delta, "total_seconds", ())?
        .try_into_value::<f64>(vm)?;
    let zoned: Option<DateTime<FixedOffset>> = FixedOffset::east_opt(seconds as i32)
        .and_then(|offset| offset.from_local_datetime(&naive).single());
    zoned
        .map(|z| Value::DateTime(DateTimeValue::Zoned(z)))
        .ok_or_else(|| {
            Bad::Value(
                ErrorKind::Value,
                "that time zone offset is not valid".to_owned(),
            )
        })
}

/// The error value inside an `ErrorValue` Python object.
fn error_from_py(vm: &VirtualMachine, obj: &PyObject) -> PyResult<ErrorValue> {
    let code = obj.get_attr("kind", vm)?.try_into_value::<String>(vm)?;
    let message = obj.get_attr("message", vm)?.try_into_value::<String>(vm)?;
    let path = obj.get_attr("path", vm)?.try_into_value::<String>(vm)?;
    Ok(ErrorValue {
        kind: kind_of_code(&code),
        message,
        origin: origin_of(&path),
    })
}

/// The path part of `Orders.items[2].price`, which names the tab first.
fn origin_of(text: &str) -> Option<Path> {
    let rest = match text.find(['.', '[']) {
        Some(at) if text[at..].starts_with('.') => &text[at + 1..],
        Some(at) => &text[at..],
        None => return Some(Path::root()),
    };
    rest.parse().ok()
}

pub(super) fn kind_of_code(code: &str) -> ErrorKind {
    const ALL: [ErrorKind; 12] = [
        ErrorKind::Type,
        ErrorKind::Ref,
        ErrorKind::Name,
        ErrorKind::Value,
        ErrorKind::DivZero,
        ErrorKind::Circular,
        ErrorKind::Syntax,
        ErrorKind::PyErr,
        ErrorKind::Timeout,
        ErrorKind::Source,
        ErrorKind::Untrusted,
        ErrorKind::Na,
    ];
    ALL.into_iter()
        .find(|k| k.code() == code)
        .unwrap_or(ErrorKind::PyErr)
}

// ------------------------------------------------------ errors from Python

/// What a Python exception means for the node (DESIGN.md, the table in
/// "Semantics: Python first, Excel second").
pub(super) fn failure(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    exc: &PyBaseExceptionRef,
) -> Failure {
    let classes = shared.classes();
    let mut traceback = String::new();
    let _ = vm.write_exception(&mut traceback, exc);
    let object = exc.as_object();

    if object.is_instance(&classes.propagated, vm).unwrap_or(false)
        && let Ok(error) = object.get_attr("error", vm)
        && let Ok(error) = error_from_py(vm, &error)
    {
        return Failure {
            error,
            traceback: Some(traceback),
        };
    }
    if object
        .is_instance(&classes.not_available, vm)
        .unwrap_or(false)
    {
        return Failure {
            error: ErrorValue {
                kind: ErrorKind::Na,
                message: "not available".to_owned(),
                origin: None,
            },
            traceback: Some(traceback),
        };
    }
    if object
        .is_instance(&classes.source_error, vm)
        .unwrap_or(false)
    {
        return Failure {
            error: ErrorValue {
                kind: ErrorKind::Source,
                message: object
                    .str(vm)
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                origin: None,
            },
            traceback: Some(traceback),
        };
    }
    if object.is_instance(&classes.timeout, vm).unwrap_or(false) {
        return Failure {
            error: ErrorValue {
                kind: ErrorKind::Timeout,
                message: "the formula ran out of time".to_owned(),
                origin: None,
            },
            traceback: Some(traceback),
        };
    }
    let kinds = &vm.ctx.exceptions;
    let is = |class: &'static rustpython::vm::Py<rustpython::vm::builtins::PyType>| {
        exc.fast_isinstance(class)
    };
    let kind = if is(kinds.keyboard_interrupt) {
        // How the watchdog stops a formula that is out of time.
        return Failure {
            error: ErrorValue {
                kind: ErrorKind::Timeout,
                message: "the formula ran out of time".to_owned(),
                origin: None,
            },
            traceback: Some(traceback),
        };
    } else if is(kinds.zero_division_error) {
        ErrorKind::DivZero
    } else if is(kinds.key_error) || is(kinds.index_error) || is(kinds.attribute_error) {
        ErrorKind::Ref
    } else if is(kinds.type_error) {
        ErrorKind::Type
    } else if is(kinds.value_error) {
        ErrorKind::Value
    } else if is(kinds.name_error) {
        ErrorKind::Name
    } else if is(kinds.syntax_error) {
        ErrorKind::Syntax
    } else {
        ErrorKind::PyErr
    };
    let class = exc.class().name().to_string();
    let detail = object
        .str(vm)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let message = if detail.is_empty() {
        class
    } else {
        format!("{class}: {detail}")
    };
    Failure {
        error: ErrorValue {
            kind,
            message,
            origin: None,
        },
        traceback: Some(traceback),
    }
}
