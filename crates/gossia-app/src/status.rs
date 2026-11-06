//! The words of the status bar (DESIGN.md, "Evaluation and scheduling",
//! "Status bar").

use gossia_calc::Status;
use gossia_i18n::{counted, tr};

/// `Ready`, or `Calculating... N pending`, or `Running <button>...` while a
/// button procedure waits or runs, or that evaluation is suspended.
pub fn evaluation_text(status: &Status) -> String {
    if status.suspended {
        return tr!(
            "Evaluation suspended: {abandoned} abandoned threads still running",
            abandoned = status.abandoned
        );
    }
    let mut text = if let Some(button) = &status.procedure {
        tr!("Running {button}...", button = button)
    } else if status.running || status.pending > 0 {
        tr!("Calculating... {pending} pending", pending = status.pending)
    } else {
        tr!("Ready").to_owned()
    };
    if status.abandoned > 0 {
        text.push_str("   ");
        text.push_str(&tr!("{n} abandoned", n = status.abandoned));
    }
    text
}

/// `2 errors, 1 invalid`, or that there are none.
pub fn problems_text(errors: usize, invalid: usize) -> String {
    let errors_text = |e: usize| counted(e, tr!("1 error"), tr!("{n} errors"));
    match (errors, invalid) {
        (0, 0) => tr!("No errors").to_owned(),
        (e, 0) => errors_text(e),
        (0, i) => tr!("{n} invalid", n = i),
        (e, i) => tr!("{errors}, {n} invalid", errors = errors_text(e), n = i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(pending: usize, running: bool, abandoned: usize, suspended: bool) -> Status {
        Status {
            pending,
            running,
            abandoned,
            suspended,
            procedure: None,
        }
    }

    #[test]
    fn the_status_bar_says_what_evaluation_is_doing() {
        assert_eq!(evaluation_text(&status(0, false, 0, false)), "Ready");
        assert_eq!(
            evaluation_text(&status(3, true, 0, false)),
            "Calculating... 3 pending"
        );
        assert_eq!(
            evaluation_text(&status(2, false, 0, false)),
            "Calculating... 2 pending"
        );
        assert_eq!(
            evaluation_text(&status(0, false, 2, false)),
            "Ready   2 abandoned"
        );
        assert_eq!(
            evaluation_text(&status(5, false, 20, true)),
            "Evaluation suspended: 20 abandoned threads still running"
        );
    }

    #[test]
    fn the_status_bar_names_the_button_that_is_running() {
        let mut running = status(0, true, 0, false);
        running.procedure = Some("Add line".to_owned());
        assert_eq!(evaluation_text(&running), "Running Add line...");
    }

    #[test]
    fn problems_are_counted() {
        assert_eq!(problems_text(0, 0), "No errors");
        assert_eq!(problems_text(1, 0), "1 error");
        assert_eq!(problems_text(2, 0), "2 errors");
        assert_eq!(problems_text(0, 3), "3 invalid");
        assert_eq!(problems_text(1, 1), "1 error, 1 invalid");
    }
}
