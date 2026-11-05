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
//!
//! The wasm version has no threads, so it has no watchdog: a formula that
//! overruns is ended by terminating the Web Worker it runs in (DESIGN.md, Web
//! port, 0.3.0). There `Watchdog` does nothing.

#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;
#[cfg(not(target_family = "wasm"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_family = "wasm"))]
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
#[cfg(not(target_family = "wasm"))]
use std::time::{Duration, Instant};

#[cfg(not(target_family = "wasm"))]
use rustpython::vm::signal::UserSignalSender;

/// How often a signal is sent again while a formula is still running past its
/// deadline.
#[cfg(not(target_family = "wasm"))]
const RESEND: Duration = Duration::from_millis(25);

#[cfg(not(target_family = "wasm"))]
enum Message {
    Start { id: u64, deadline: Instant },
    Stop,
}

#[cfg(target_family = "wasm")]
pub(super) use no_threads::Watchdog;

#[cfg(not(target_family = "wasm"))]
pub(super) struct Watchdog {
    messages: Sender<Message>,
    /// The evaluation that is allowed to be interrupted, or 0.
    armed: Arc<AtomicU64>,
    next: std::cell::Cell<u64>,
    /// When the allowance running now ends.
    deadline: std::cell::Cell<Option<Instant>>,
}

#[cfg(not(target_family = "wasm"))]
impl Watchdog {
    pub fn new(signals: UserSignalSender) -> Watchdog {
        let (messages, inbox) = mpsc::channel::<Message>();
        let armed = Arc::new(AtomicU64::new(0));
        let watched = Arc::clone(&armed);
        std::thread::Builder::new()
            .name("gossia-watchdog".to_owned())
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
            deadline: std::cell::Cell::new(None),
        }
    }

    /// Lets the interpreter be interrupted for `limit` from now.
    pub fn start(&self, limit: Duration) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.armed.store(id, Ordering::SeqCst);
        let deadline = Instant::now() + limit;
        self.deadline.set(Some(deadline));
        let _ = self.messages.send(Message::Start { id, deadline });
    }

    /// Ends the allowance, so that a signal still on its way does nothing.
    pub fn stop(&self) {
        self.deadline.set(None);
        self.armed.store(0, Ordering::SeqCst);
        let _ = self.messages.send(Message::Stop);
    }

    /// Stops the clock while the formula waits for a data source, which does
    /// not count against its limit. Gives what was left, for [`resume`].
    pub fn pause(&self) -> Option<Duration> {
        let left = self
            .deadline
            .get()
            .map(|d| d.saturating_duration_since(Instant::now()));
        if left.is_some() {
            self.stop();
        }
        left
    }

    /// Starts the clock again with what was left when it was paused.
    pub fn resume(&self, left: Option<Duration>) {
        if let Some(left) = left {
            self.start(left);
        }
    }
}

#[cfg(target_family = "wasm")]
mod no_threads {
    use std::time::Duration;

    use rustpython::vm::signal::UserSignalSender;

    /// The watchdog of the wasm version, which does nothing (see the module).
    pub struct Watchdog;

    impl Watchdog {
        pub fn new(_signals: UserSignalSender) -> Watchdog {
            Watchdog
        }

        pub fn start(&self, _limit: Duration) {}

        pub fn stop(&self) {}

        pub fn pause(&self) -> Option<Duration> {
            None
        }

        pub fn resume(&self, _left: Option<Duration>) {}
    }
}
