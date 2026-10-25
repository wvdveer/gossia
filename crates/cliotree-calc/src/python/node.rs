//! The object formulas use to move through the data: `Orders.items[2].price`.
//!
//! A proxy stands for a node that is an array or an object. Moving through it
//! (`.name`, `[key]`, `[index]`, iteration, `len`) reads the child and records
//! that the formula looked at it, shallowly: the formula depends on the child's
//! structure, and on what is inside it only if it goes on to read that. A child
//! that is a plain value comes back as a plain Python value; one that is an
//! array or an object comes back as another proxy. So `Orders.items[2].price`
//! depends on that price, not on the whole array, and a formula on one item can
//! read another without being circular with the array it is in.
//!
//! A proxy made during an evaluation keeps the node it read, so that moving on
//! from it needs no second read. It is only trusted during the evaluation that
//! made it (see `Shared::epoch`).

use std::cell::RefCell;
use std::rc::Rc;

use cliotree_core::{Node, Path, Segment, Value};
use rustpython::vm::builtins::{PyInt, PyList, PySlice, PyStr, PyStrRef};
use rustpython::vm::function::{FuncArgs, PySetterValue};
use rustpython::vm::protocol::{PyMapping, PyMappingMethods};
use rustpython::vm::types::{AsMapping, Callable, GetAttr, Iterable, Representable, SetAttr};
use rustpython::vm::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    pyclass, pymodule,
};

use super::convert;
use super::shared::Shared;
use crate::engine::{Depth, Read, Write, WriteOp};

/// The module `_cliotree`, which the prelude imports `Node` from.
#[pymodule]
pub(super) mod _cliotree {
    use super::*;

    /// An array or an object in the data, as a formula sees it.
    #[pyattr]
    #[pyclass(module = "_cliotree", name = "Node")]
    #[derive(Debug, PyPayload)]
    pub struct NodeProxy {
        pub(in crate::python) shared: Rc<Shared>,
        pub(in crate::python) sheet: Rc<str>,
        pub(in crate::python) path: Path,
        /// The node, and the evaluation it was read in.
        pub(in crate::python) cached: RefCell<Option<(u64, Node)>>,
        /// Whether to remember the node. The proxies that live as long as the
        /// engine do not: a node kept here would be shared with the data, and
        /// the next edit of the data would have to copy it first.
        pub(in crate::python) remember: bool,
    }

    /// The `__import__` of the workbook's modules: it serves the workbook's
    /// own modules and the standard library modules formulas can use, and
    /// refuses everything else with `ImportError`.
    #[pyattr]
    #[pyclass(module = "_cliotree", name = "Importer")]
    #[derive(Debug, PyPayload)]
    pub struct Importer {
        pub(in crate::python) shared: Rc<Shared>,
    }

    #[pyclass(with(Callable))]
    impl Importer {}

    impl Callable for Importer {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            super::super::modules::import(&zelf.shared, &args, vm)
        }
    }

    /// `JSONPATH(query, node)`: see `jsonpath.rs`.
    #[pyfunction]
    fn jsonpath(query: PyStrRef, node: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        super::run_jsonpath(query.expect_str(), &node, vm)
    }

    /// `XMLPATH(query, node)`: see `xmlpath.rs`.
    #[pyfunction]
    fn xmlpath(query: PyStrRef, node: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        super::run_xmlpath(query.expect_str(), &node, vm)
    }

    #[pyclass(with(GetAttr, SetAttr, AsMapping, Iterable, Representable))]
    impl NodeProxy {
        /// `array.append(value)`: a button procedure adds an item at the end.
        #[pymethod]
        fn append(zelf: &Py<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            super::append(zelf, &value, vm)
        }

        /// `"array"` or `"object"`, for the functions that take arrays.
        #[pymethod]
        fn __kind__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<&'static str> {
            Ok(match super::resolve(zelf, vm)?.value() {
                Value::Array(_) => "array",
                _ => "object",
            })
        }

        /// The whole node as plain lists, dicts and values (`PLAIN(x)`).
        #[pymethod]
        fn __plain__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            super::plain(zelf, vm)
        }
    }

    impl GetAttr for NodeProxy {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let text = name.expect_str();
            if text.starts_with("__") {
                return zelf.as_object().generic_getattr(name, vm);
            }
            // While a procedure runs, `append` on an array is the method, not
            // the field of the items that it would otherwise be.
            if text == "append"
                && zelf.shared.writes.borrow().is_some()
                && matches!(
                    super::resolve(zelf, vm).map(|n| matches!(n.value(), Value::Array(_))),
                    Ok(true)
                )
            {
                return zelf.as_object().generic_getattr(name, vm);
            }
            super::attribute(zelf, text, vm)
        }
    }

    impl SetAttr for NodeProxy {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let segment = super::field_segment(zelf, name.expect_str(), vm)?;
            super::write(zelf, segment, value, vm)
        }
    }

    impl AsMapping for NodeProxy {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping: PyMapping<'_>, vm: &VirtualMachine| {
                    super::length(NodeProxy::mapping_downcast(mapping), vm)
                }),
                subscript: atomic_func!(
                    |mapping: PyMapping<'_>, key: &PyObject, vm: &VirtualMachine| {
                        super::subscript(NodeProxy::mapping_downcast(mapping), key, vm)
                    }
                ),
                ass_subscript: atomic_func!(
                    |mapping: PyMapping<'_>,
                     key: &PyObject,
                     value: Option<PyObjectRef>,
                     vm: &VirtualMachine| {
                        let zelf = NodeProxy::mapping_downcast(mapping);
                        let segment = super::key_segment(zelf, key, vm)?;
                        let value = match value {
                            Some(value) => PySetterValue::Assign(value),
                            None => PySetterValue::Delete,
                        };
                        super::write(zelf, segment, value, vm)
                    }
                ),
            };
            &AS_MAPPING
        }
    }

    impl Iterable for NodeProxy {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            super::iterate(&zelf, vm)
        }
    }

    impl Representable for NodeProxy {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<Node {}>",
                cliotree_core_loc(&zelf.sheet, &zelf.path)
            ))
        }
    }

    fn cliotree_core_loc(sheet: &str, path: &Path) -> String {
        crate::Loc::new(sheet, path.clone()).to_string()
    }
}

use _cliotree::NodeProxy;

/// A proxy for the node at `path`, which has been read already.
pub(super) fn proxy(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    sheet: &Rc<str>,
    path: Path,
    node: &Node,
) -> PyObjectRef {
    NodeProxy {
        shared: Rc::clone(shared),
        sheet: Rc::clone(sheet),
        path,
        cached: RefCell::new(Some((shared.epoch(), node.clone()))),
        remember: true,
    }
    .into_ref(&vm.ctx)
    .into()
}

/// A proxy for the node at `path` that reads it when first used. For the names
/// of tabs, which live as long as the engine does.
pub(super) fn lazy_proxy(
    shared: &Rc<Shared>,
    vm: &VirtualMachine,
    sheet: &Rc<str>,
    path: Path,
) -> PyObjectRef {
    NodeProxy {
        shared: Rc::clone(shared),
        sheet: Rc::clone(sheet),
        path,
        cached: RefCell::new(None),
        remember: false,
    }
    .into_ref(&vm.ctx)
    .into()
}

fn no_context(vm: &VirtualMachine) -> rustpython::vm::builtins::PyBaseExceptionRef {
    vm.new_runtime_error("a node can only be read while the formula that uses it is running")
}

/// The node a proxy stands for, read shallowly for the running evaluation.
fn resolve(zelf: &NodeProxy, vm: &VirtualMachine) -> PyResult<Node> {
    let epoch = zelf.shared.epoch();
    if let Some((at, node)) = &*zelf.cached.borrow()
        && *at == epoch
    {
        return Ok(node.clone());
    }
    let ctx = zelf.shared.ctx().ok_or_else(|| no_context(vm))?;
    match ctx.read(&zelf.sheet, &zelf.path, Depth::Shallow) {
        Read::Node(node) => {
            if zelf.remember {
                *zelf.cached.borrow_mut() = Some((epoch, node.clone()));
            }
            Ok(node)
        }
        Read::Missing | Read::NoSheet => {
            let loc = crate::Loc::new(&*zelf.sheet, zelf.path.clone());
            Err(vm.new_key_error(vm.ctx.new_str(format!("there is nothing at {loc}")).into()))
        }
    }
}

/// How a child that is not there is reported, which depends on how it was
/// asked for. All of them are `#REF!` when they reach the cell.
#[derive(Clone, Copy)]
enum Asked {
    Attribute,
    Key,
    Index,
}

/// The child of `zelf` at `segment`, as a Python value.
fn child(zelf: &NodeProxy, segment: Segment, asked: Asked, vm: &VirtualMachine) -> PyResult {
    let parent = resolve(zelf, vm)?;
    let path = zelf.path.child(segment.clone());
    let ctx = zelf.shared.ctx().ok_or_else(|| no_context(vm))?;
    ctx.record(&zelf.sheet, &path, Depth::Shallow);
    match parent.child(&segment) {
        Some(node) => convert::node_to_py(&zelf.shared, vm, &zelf.sheet, path, node),
        None if matches!(segment, Segment::Key(_)) && ctx.absent_ok(&zelf.sheet, &path) => {
            // An optional field that is not there reads as `None`.
            Ok(vm.ctx.none())
        }
        None => {
            let loc = crate::Loc::new(&*zelf.sheet, path);
            let message = format!("there is nothing at {loc}");
            Err(match asked {
                Asked::Attribute => vm.new_attribute_error(message),
                Asked::Key => vm.new_key_error(vm.ctx.new_str(message).into()),
                Asked::Index => vm.new_index_error(message),
            })
        }
    }
}

// ------------------------------------------------------------- writing
//
// Only a button procedure may write. It runs with a list of writes open in the
// shared state; each assignment, deletion or `append` adds to it, and nothing
// is applied until the procedure has returned (DESIGN.md, "Button
// procedures"). What a procedure reads is the data as it was when it started,
// not what it has written so far.

fn refuse_write(vm: &VirtualMachine) -> rustpython::vm::builtins::PyBaseExceptionRef {
    vm.new_type_error("a formula cannot change data; only a button procedure can")
}

/// The field of an object that `proxy.name = value` is about.
fn field_segment(zelf: &Py<NodeProxy>, name: &str, vm: &VirtualMachine) -> PyResult<Segment> {
    if zelf.shared.writes.borrow().is_none() {
        return Err(refuse_write(vm));
    }
    match resolve(zelf, vm)?.value() {
        Value::Object(_) => Ok(Segment::Key(name.to_owned())),
        other => Err(vm.new_attribute_error(format!(
            "a {} has no fields to set; use an index or append",
            other.kind_name()
        ))),
    }
}

/// The child that `proxy[key] = value` or `del proxy[key]` is about.
fn key_segment(zelf: &Py<NodeProxy>, key: &PyObject, vm: &VirtualMachine) -> PyResult<Segment> {
    if zelf.shared.writes.borrow().is_none() {
        return Err(refuse_write(vm));
    }
    let node = resolve(zelf, vm)?;
    match node.value() {
        Value::Object(_) => match key.downcast_ref::<PyStr>() {
            Some(text) => Ok(Segment::Key(text.expect_str().to_owned())),
            None => Err(vm.new_type_error(format!(
                "keys of an object are strings, not {}",
                key.class().name()
            ))),
        },
        Value::Array(items) => {
            let Some(int) = key.downcast_ref::<PyInt>() else {
                return Err(vm.new_type_error(format!(
                    "indexes of an array are integers, not {}",
                    key.class().name()
                )));
            };
            let wanted: i64 = int.try_to_primitive(vm)?;
            let len = items.len() as i64;
            let index = if wanted < 0 { wanted + len } else { wanted };
            if index < 0 || index >= len {
                let loc = crate::Loc::new(&*zelf.sheet, zelf.path.clone());
                return Err(vm.new_index_error(format!(
                    "{wanted} is outside {loc}, which has {len} items; use append to add one"
                )));
            }
            Ok(Segment::Index(index as usize))
        }
        _ => Err(vm.new_type_error("this node has no items")),
    }
}

/// Reads a Python value to write, as data.
fn data_of(zelf: &NodeProxy, value: &PyObject, vm: &VirtualMachine) -> PyResult<Value> {
    match convert::to_value(&zelf.shared, vm, value) {
        Ok(value) => Ok(value),
        Err(convert::Bad::Value(_, message)) => Err(vm.new_value_error(message)),
        Err(convert::Bad::Raised(exc)) => Err(exc),
    }
}

/// Puts a write in the list the procedure is making.
fn write(
    zelf: &Py<NodeProxy>,
    segment: Segment,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let op = match value {
        PySetterValue::Assign(value) => WriteOp::Set(data_of(zelf, &value, vm)?),
        PySetterValue::Delete => WriteOp::Remove,
    };
    push_write(zelf, zelf.path.child(segment), op, vm)
}

fn push_write(zelf: &NodeProxy, path: Path, op: WriteOp, vm: &VirtualMachine) -> PyResult<()> {
    match zelf.shared.writes.borrow_mut().as_mut() {
        Some(writes) => {
            writes.push(Write {
                loc: crate::Loc::new(&*zelf.sheet, path),
                op,
            });
            Ok(())
        }
        None => Err(refuse_write(vm)),
    }
}

fn append(zelf: &Py<NodeProxy>, value: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
    if zelf.shared.writes.borrow().is_none() {
        return Err(refuse_write(vm));
    }
    if !matches!(resolve(zelf, vm)?.value(), Value::Array(_)) {
        return Err(vm.new_attribute_error("only an array has append"));
    }
    let item = data_of(zelf, value, vm)?;
    push_write(zelf, zelf.path.clone(), WriteOp::Append(item), vm)
}

/// How many children a node has.
fn count(node: &Node) -> usize {
    match node.value() {
        Value::Array(items) => items.len(),
        Value::Object(fields) => fields.len(),
        _ => 0,
    }
}

fn length(zelf: &Py<NodeProxy>, vm: &VirtualMachine) -> PyResult<usize> {
    Ok(count(&resolve(zelf, vm)?))
}

/// `proxy.name`: a field of an object, or, on an array, that field of every
/// item (`Orders.items.price` is the list of prices).
fn attribute(zelf: &Py<NodeProxy>, name: &str, vm: &VirtualMachine) -> PyResult {
    let node = resolve(zelf, vm)?;
    match node.value() {
        Value::Object(_) => child(zelf, Segment::Key(name.to_owned()), Asked::Attribute, vm),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for index in 0..items.len() {
                let item = child(zelf, Segment::Index(index), Asked::Index, vm)?;
                out.push(item_attribute(zelf, &item, name, vm)?);
            }
            Ok(vm.ctx.new_list(out).into())
        }
        _ => Err(vm.new_attribute_error(format!("a {} has no fields", node.value().kind_name()))),
    }
}

/// `item.name` for one item of an array being projected. An item that is not
/// an object has no such field.
fn item_attribute(
    zelf: &Py<NodeProxy>,
    item: &PyObjectRef,
    name: &str,
    vm: &VirtualMachine,
) -> PyResult {
    match item.downcast_ref::<NodeProxy>() {
        Some(inner) => attribute(inner, name, vm),
        None => {
            let loc = crate::Loc::new(&*zelf.sheet, zelf.path.clone());
            Err(vm.new_attribute_error(format!("an item of {loc} has no field {name}")))
        }
    }
}

/// `proxy[key]`.
fn subscript(zelf: &Py<NodeProxy>, key: &PyObject, vm: &VirtualMachine) -> PyResult {
    let node = resolve(zelf, vm)?;
    match node.value() {
        Value::Object(_) => match key.downcast_ref::<PyStr>() {
            Some(text) => child(
                zelf,
                Segment::Key(text.expect_str().to_owned()),
                Asked::Key,
                vm,
            ),
            None => Err(vm.new_type_error(format!(
                "keys of an object are strings, not {}",
                key.class().name()
            ))),
        },
        Value::Array(items) => {
            if let Some(slice) = key.downcast_ref::<PySlice>() {
                let all = children(zelf, items.len(), vm)?;
                let list: PyObjectRef = vm.ctx.new_list(all).into();
                let _ = slice;
                return list.get_item(key, vm);
            }
            let Some(int) = key.downcast_ref::<PyInt>() else {
                return Err(vm.new_type_error(format!(
                    "indexes of an array are integers, not {}",
                    key.class().name()
                )));
            };
            let wanted: i64 = int.try_to_primitive(vm)?;
            let len = items.len() as i64;
            let index = if wanted < 0 { wanted + len } else { wanted };
            if index < 0 {
                let loc = crate::Loc::new(&*zelf.sheet, zelf.path.clone());
                return Err(vm.new_index_error(format!("{wanted} is before the start of {loc}")));
            }
            child(zelf, Segment::Index(index as usize), Asked::Index, vm)
        }
        _ => Err(vm.new_type_error("this node has no items")),
    }
}

/// Every child of an array, as Python values.
fn children(zelf: &NodeProxy, len: usize, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
    (0..len)
        .map(|index| child(zelf, Segment::Index(index), Asked::Index, vm))
        .collect()
}

/// `for x in proxy`: the items of an array, the keys of an object.
fn iterate(zelf: &Py<NodeProxy>, vm: &VirtualMachine) -> PyResult {
    let node = resolve(zelf, vm)?;
    let list: PyRef<PyList> = match node.value() {
        Value::Array(items) => vm.ctx.new_list(children(zelf, items.len(), vm)?),
        Value::Object(fields) => vm.ctx.new_list(
            fields
                .keys()
                .map(|k| vm.ctx.new_str(k.as_str()).into())
                .collect(),
        ),
        _ => return Err(vm.new_type_error("this node cannot be iterated")),
    };
    list.as_object().get_iter(vm).map(Into::into)
}

/// The whole node, read deeply, as plain Python values.
fn plain(zelf: &Py<NodeProxy>, vm: &VirtualMachine) -> PyResult {
    let ctx = zelf.shared.ctx().ok_or_else(|| no_context(vm))?;
    match ctx.read(&zelf.sheet, &zelf.path, Depth::Deep) {
        Read::Node(node) => {
            convert::node_to_plain(&zelf.shared, vm, &zelf.sheet, &zelf.path, &node)
        }
        Read::Missing | Read::NoSheet => Err(vm.new_key_error(
            vm.ctx
                .new_str(format!(
                    "there is nothing at {}",
                    crate::Loc::new(&*zelf.sheet, zelf.path.clone())
                ))
                .into(),
        )),
    }
}

/// `JSONPATH(query, node)`: the nodes of `node` that match the query, as a
/// list of the values and proxies a formula would get by walking there. The
/// whole of `node` is read, so the formula depends on all of it.
fn run_jsonpath(query: &str, base: &PyObject, vm: &VirtualMachine) -> PyResult {
    let proxy = path_base(
        base,
        vm,
        "JSONPATH takes a query and then a tab or a node, for example JSONPATH('$.items[*].price', Orders)",
    )?;
    let parsed = super::jsonpath::parse(query).map_err(|message| {
        vm.new_value_error(format!(
            "the JSONPath query {query:?} is not valid: {message}"
        ))
    })?;
    let root = read_deep(proxy, vm)?;
    let matches = super::jsonpath::select(&parsed, &root)
        .into_iter()
        .map(|(steps, node)| (steps, node.clone()))
        .collect();
    matches_to_py(proxy, matches, vm)
}

/// `XMLPATH(query, node)`: the nodes of `node` that match the XPath query, as a
/// list, like `JSONPATH`. The whole of `node` is read.
fn run_xmlpath(query: &str, base: &PyObject, vm: &VirtualMachine) -> PyResult {
    let proxy = path_base(
        base,
        vm,
        "XMLPATH takes a query and then a tab or a node, for example XMLPATH('line[qty > 1]/sku', Orders)",
    )?;
    let parsed = super::xmlpath::parse(query).map_err(|message| {
        vm.new_value_error(format!(
            "the XMLPATH query {query:?} is not valid: {message}"
        ))
    })?;
    let root = read_deep(proxy, vm)?;
    matches_to_py(proxy, super::xmlpath::select(&parsed, &root), vm)
}

/// The tab or node a path query is run on.
fn path_base<'a>(
    base: &'a PyObject,
    vm: &VirtualMachine,
    usage: &'static str,
) -> PyResult<&'a NodeProxy> {
    base.downcast_ref::<NodeProxy>()
        .map(|p| &**p)
        .ok_or_else(|| vm.new_type_error(usage))
}

/// Reads the node of `proxy` and everything below it.
fn read_deep(proxy: &NodeProxy, vm: &VirtualMachine) -> PyResult<cliotree_core::Node> {
    let ctx = proxy.shared.ctx().ok_or_else(|| no_context(vm))?;
    match ctx.read(&proxy.sheet, &proxy.path, Depth::Deep) {
        Read::Node(node) => Ok(node),
        Read::Missing | Read::NoSheet => Err(vm.new_key_error(
            vm.ctx
                .new_str(format!(
                    "there is nothing at {}",
                    crate::Loc::new(&*proxy.sheet, proxy.path.clone())
                ))
                .into(),
        )),
    }
}

/// The matches of a query as a list of the values and proxies a formula would
/// get by walking there.
fn matches_to_py(
    proxy: &NodeProxy,
    matches: Vec<(Vec<cliotree_core::Segment>, cliotree_core::Node)>,
    vm: &VirtualMachine,
) -> PyResult {
    let mut out = Vec::new();
    for (steps, node) in matches {
        let mut path = proxy.path.clone();
        for step in steps {
            path.push(step);
        }
        out.push(convert::node_to_py(
            &proxy.shared,
            vm,
            &proxy.sheet,
            path,
            &node,
        )?);
    }
    Ok(vm.ctx.new_list(out).into())
}
