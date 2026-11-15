//! What a dropped engine leaves behind. RustPython frees objects by
//! counting references, and its collector frees the cycles between them
//! only when it can see every reference; a class with `__slots__` was never
//! freed, nor its module, because its slots' descriptors hid the class they
//! hold (vendor-patches/rustpython.patch). Each engine kept 6 to 7 MB.

use rustpython::vm::scope::Scope;

/// Imports `functools` (whose `partial` has `__slots__`) and `decimal`,
/// lets go of them and of every module they imported, collects, and says
/// which of their namespaces are left.
const LEFT: &str = "
import gc, sys
before = set(sys.modules)
import functools, decimal
del functools, decimal
for name in set(sys.modules) - before:
    del sys.modules[name]
del name
gc.collect()
left = sorted(o['__name__'] for o in gc.get_objects()
              if type(o) is dict and o.get('__name__') in ('functools', 'decimal'))
";

/// An interpreter with the engine's standard library, and nothing that
/// holds on to its modules as an engine does.
fn bare() -> rustpython::Interpreter {
    let mut settings = rustpython::vm::Settings::default();
    settings.isolated = true;
    settings.import_site = false;
    settings.install_signal_handlers = false;
    let builder = rustpython::Interpreter::builder(settings);
    let native = rustpython_stdlib::formula_module_defs(&builder.ctx);
    builder
        .add_native_modules(&native)
        .add_frozen_modules(super::frozen_lib())
        .build()
}

#[test]
fn a_module_with_slotted_classes_is_freed_once_let_go() {
    let interpreter = bare();
    let left = interpreter.enter(|vm| {
        let names = vm.ctx.new_dict();
        let scope = Scope::with_builtins(None, names.clone(), vm);
        vm.run_string(scope, LEFT, "<left>").unwrap();
        let left: Vec<String> = names
            .get_item("left", vm)
            .unwrap()
            .try_to_value(vm)
            .unwrap();
        names.clear();
        left
    });
    assert!(left.is_empty(), "still alive: {left:?}");
}
