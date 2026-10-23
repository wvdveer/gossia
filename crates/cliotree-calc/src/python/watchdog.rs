//! The time limit on a formula (DESIGN.md, "Python engine", "Cancelling a
//! running formula").
//!
//! A thread waits for the deadline and then sends the interpreter a signal,
//! which the interpreter acts on between bytecode instructions. The formula
//! stops with an exception that is not an `Exception`, so nothing in the
//! formula can catch it, and the interpreter is fine to use afterwards.
//!
//! A single long call into the interpreter's own code, such as
//! `sum(range(10**9))`, does not look at signals and is not stopped by this. The
//! evaluation thread abandons such a formula instead (a later step).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rustpython::vm::signal::UserSignalSender;

/// How often a signal is sent again while a formula is still running past its
/// deadline.
const RESEND: Duration = Duration::from_millis(25);

enum Message {
    Start { id: u64, deadline: Instant },
    Stop,
}

pub(super) struct Watchdog {
    messages: Sender<Message>,
    /// The evaluation that is allowed to be interrupted, or 0.
    armed: Arc<AtomicU64>,
    next: std::cell::Cell<u64>,
}

impl Watchdog {
    pub fn new(signals: UserSignalSender) -> Watchdog {
        let (messages, inbox) = mpsc::channel::<Message>();
        let armed = Arc::new(AtomicU64::new(0));
        let watched = Arc::clone(&armed);
        std::thread::Builder::new()
            .name("cliotree-watchdog".to_owned())
            .spawn(move || {
                let mut current: Option<(u64, Instant)> = None;
                loop {
                    let received = match current {
                        Some((_, deadline)) => {
                            inbox.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        }
                        None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match received {
                        Ok(Message::Start { id, deadline }) => current = Some((id, deadline)),
                        Ok(Message::Stop) => current = None,
                        Err(RecvTimeoutError::Timeout) => {
                            if let Some((id, _)) = current {
                                // Ask again shortly: the flag that tells an
                                // interpreter to look for signals is shared by
                                // every interpreter in the process, and another
                                // one may look first and clear it.
                                current = Some((id, Instant::now() + RESEND));
                                let watched = Arc::clone(&watched);
                                let _ = signals.send(Box::new(move |vm| {
                                    // The formula may have finished while the
                                    // signal was on its way.
                                    if watched.load(Ordering::SeqCst) == id {
                                        Err(vm.new_exception_empty(
                                            vm.ctx.exceptions.keyboard_interrupt.to_owned(),
                                        ))
                                    } else {
                                        Ok(())
                                    }
                                }));
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("a thread can be started");
        Watchdog {
            messages,
            armed,
            next: std::cell::Cell::new(1),
        }
    }

    /// Lets the interpreter be interrupted for `limit` from now.
    pub fn start(&self, limit: Duration) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.armed.store(id, Ordering::SeqCst);
        let _ = self.messages.send(Message::Start {
            id,
            deadline: Instant::now() + limit,
        });
    }

    /// Ends the allowance, so that a signal still on its way does nothing.
    pub fn stop(&self) {
        self.armed.store(0, Ordering::SeqCst);
        let _ = self.messages.send(Message::Stop);
    }
}
