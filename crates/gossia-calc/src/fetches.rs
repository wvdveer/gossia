//! Stopping and running again for HTTP reads in the wasm version (DESIGN.md,
//! Web port, 0.3.0). The worker cannot wait for the host, so a call whose
//! answer it does not have yet asks for it and stops what is running; once
//! the host has answered, that runs again from the start and finds the
//! answer here.
//!
//! - **Formulas and the expressions of controls** find their answers by the
//!   request, as the pass's read cache does, for every round of one pass: a
//!   formula that makes three calls one after another runs four times. The
//!   answers are forgotten when a pass asks for nothing more.
//! - **Procedures and the Immediate window** keep a log of their calls in the
//!   order they were made: when the run is made again, each call made before
//!   gets its logged answer, so a write is sent once. A different call at a
//!   place already logged (the run took another path) fails, and nothing
//!   more is sent.

use gossia_i18n::tr;
use std::collections::HashMap;

use crate::http::{FetchAnswer, FetchRequest};

/// Where an answer goes once it comes.
#[derive(Debug, Clone)]
enum Slot {
    Read(String),
    Log(String),
}

/// The answers the host has given, and the requests asked of it.
#[derive(Debug, Default)]
pub struct Fetches {
    next_id: u64,
    /// The answers to reads, by request, for the rounds of one pass.
    reads: HashMap<String, FetchAnswer>,
    /// The calls of a procedure or the Immediate window, in order, with
    /// their answers.
    log: Vec<(String, FetchAnswer)>,
    /// How many calls of the log the run going on has made.
    at: usize,
    /// Asked and not yet answered, by id.
    asked: HashMap<u64, Slot>,
    /// Asked during the run going on, not yet given to the host.
    new: Vec<FetchRequest>,
}

/// The key of a request: everything it is sent with.
fn key(request: &FetchRequest) -> String {
    format!(
        "{} {} {:?} {:?} {} {}",
        request.method,
        request.url,
        request.headers,
        request.body,
        request.timeout.as_millis(),
        request.max_bytes
    )
}

impl Fetches {
    /// The answer to `request`, if the host has given it. Otherwise it is
    /// asked for (once) and `None` says that what runs has to stop.
    /// `in_order` for a procedure or the Immediate window.
    pub fn answer(&mut self, mut request: FetchRequest, in_order: bool) -> Option<FetchAnswer> {
        let key = key(&request);
        if in_order {
            if let Some((logged, answer)) = self.log.get(self.at) {
                if *logged != key {
                    return Some(Err(tr!(
                        "this run called {method} {request} where it called something else before it \
                         stopped to wait for an answer, so nothing more is sent",
                        method = request.method,
                        request = request.shown_url()
                    )));
                }
                self.at += 1;
                return Some(request.checked(answer.clone()));
            }
            let pending = self.asked.values().any(|s| matches!(s, Slot::Log(_)));
            if !pending {
                self.ask(&mut request, Slot::Log(key));
            }
            return None;
        }
        if let Some(answer) = self.reads.get(&key) {
            return Some(request.checked(answer.clone()));
        }
        let pending = self
            .asked
            .values()
            .any(|s| matches!(s, Slot::Read(k) if *k == key));
        if !pending {
            self.ask(&mut request, Slot::Read(key));
        }
        None
    }

    fn ask(&mut self, request: &mut FetchRequest, slot: Slot) {
        self.next_id += 1;
        request.id = self.next_id;
        self.asked.insert(request.id, slot);
        self.new.push(request.clone());
    }

    /// The requests asked since the last call, for the host.
    pub fn take_new(&mut self) -> Vec<FetchRequest> {
        std::mem::take(&mut self.new)
    }

    /// Whether requests are waiting for an answer.
    pub fn waiting(&self) -> bool {
        !self.asked.is_empty()
    }

    /// The host's answer to a request. One that nobody waits for any more is
    /// dropped.
    pub fn answered(&mut self, id: u64, answer: FetchAnswer) {
        match self.asked.remove(&id) {
            Some(Slot::Read(key)) => {
                self.reads.insert(key, answer);
            }
            Some(Slot::Log(key)) => self.log.push((key, answer)),
            None => {}
        }
    }

    /// A procedure or Immediate expression starts, or starts again.
    pub fn begin_run(&mut self) {
        self.at = 0;
    }

    /// It finished: the next one starts with an empty log.
    pub fn end_run(&mut self) {
        self.log.clear();
        self.at = 0;
        self.asked.retain(|_, s| matches!(s, Slot::Read(_)));
    }

    /// A pass finished. If it asked for nothing, the answers are forgotten,
    /// so the next pass reads afresh.
    pub fn end_pass(&mut self) {
        if !self.asked.values().any(|s| matches!(s, Slot::Read(_))) {
            self.reads.clear();
        }
    }
}

#[cfg(test)]
#[path = "fetches_tests.rs"]
mod fetches_tests;
