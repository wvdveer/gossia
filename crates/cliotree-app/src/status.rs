//! The words of the status bar (DESIGN.md, "Evaluation and scheduling",
//! "Status bar").

use cliotree_calc::Status;

/// `Ready`, or `Calculating... N pending`, or `Running <button>...` while a
/// button procedure waits or runs, or that evaluation is suspended.
pub fn evaluation_text(status: &Status) -> String {
    if status.suspended {
        return format!(
            "Evaluation suspended: {} abandoned threads still running",
            status.abandoned
        );
    }
    let mut text = if let Some(button) = &status.procedure {
        format!("Running {button}...")
    } else if status.running || status.pending > 0 {
        format!("Calculating... {} pending", status.pending)
    } else {
        "Ready".to_owned()
    };
    if status.abandoned > 0 {
        text.push_str(&format!("   {} abandoned", status.abandoned));
    }
    text
}

/// `2 errors, 1 invalid`, or that there are none.
pub fn problems_text(errors: usize, invalid: usize) -> String {
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    match (errors, invalid) {
        (0, 0) => "No errors".to_owned(),
        (e, 0) => plural(e, "error", "errors"),
        (0, i) => format!("{} invalid", i),
        (e, i) => format!("{}, {} invalid", plural(e, "error", "errors"), i),
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
