//! State shared between the engine and the objects it hands to Python.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use rustpython::vm::PyObjectRef;

use crate::engine::Context;

/// Python classes the Rust side needs to make or to recognise.
pub(super) struct Classes {
    pub error_value: PyObjectRef,
    pub propagated: PyObjectRef,
    pub timeout: PyObjectRef,
    pub date: PyObjectRef,
    pub datetime: PyObjectRef,
    pub timezone: PyObjectRef,
    pub timedelta: PyObjectRef,
}

/// What the proxies reach through: the context of the formula that is running,
/// and the classes. One per engine.
pub(super) struct Shared {
    ctx: RefCell<Option<Rc<dyn Context>>>,
    /// Counts evaluations, so that a proxy that outlives its evaluation does
    /// not use a node it read during an earlier one.
    epoch: Cell<u64>,
    classes: RefCell<Option<Rc<Classes>>>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("epoch", &self.epoch.get())
            .finish_non_exhaustive()
    }
}

impl Shared {
    pub fn new() -> Rc<Shared> {
        Rc::new(Shared {
            ctx: RefCell::new(None),
            epoch: Cell::new(0),
            classes: RefCell::new(None),
        })
    }

    /// Starts an evaluation.
    pub fn begin(&self, ctx: Rc<dyn Context>) {
        self.epoch.set(self.epoch.get() + 1);
        *self.ctx.borrow_mut() = Some(ctx);
    }

    /// Ends it. Proxies that are used afterwards find no context.
    pub fn end(&self) {
        *self.ctx.borrow_mut() = None;
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.get()
    }

    pub fn ctx(&self) -> Option<Rc<dyn Context>> {
        self.ctx.borrow().clone()
    }

    pub fn set_classes(&self, classes: Classes) {
        *self.classes.borrow_mut() = Some(Rc::new(classes));
    }

    pub fn classes(&self) -> Rc<Classes> {
        self.classes
            .borrow()
            .clone()
            .expect("the engine sets the classes before any formula runs")
    }
}
