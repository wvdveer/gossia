//! *Print...* on a report tab's menu and in the File menu (DESIGN.md,
//! Reports, "Printing"): the report's PDF, as *Export as PDF...* writes it,
//! in a temporary file opened in the system's PDF program, where it is
//! printed with that program's print dialog and printer (owner, 2026-11-06:
//! many PDF programs, Chrome and Edge among them, give the system no print
//! action that could print a file without opening it).
//!
//! The dialog is the export's (`report_pdf`), so a report waits for its
//! calculation and its parameters as an export does. A host has no
//! printing: its PDF or HTML is saved through the host and printed from the
//! program that opens it.

use gossia_i18n::tr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::app::GossiaApp;
use crate::report_pdf::ReportFormat;

/// How a file is opened in the system's program for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outside {
    /// With the system's program.
    System,
    /// Only noted, for the tests.
    Record(Vec<PathBuf>),
}

/// How long the temporary PDFs are kept: long enough to print from.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

impl GossiaApp {
    /// *Print...*: shows the report, so that it is calculated, and opens the
    /// dialog.
    pub fn begin_print(&mut self, tab: &str) {
        if crate::host::current().is_some() {
            self.notice = Some(
                "Printing is not available here: export the report as PDF or HTML and \
                 print it from the program that opens it"
                    .to_owned(),
            );
            return;
        }
        self.begin_report_export(tab, ReportFormat::Pdf, true);
    }

    /// Opens a file in the system's program for it.
    pub(crate) fn open_outside(&mut self, path: &Path) -> Result<(), String> {
        self.last_printed = Some(path.to_path_buf());
        match &mut self.outside {
            Outside::Record(files) => {
                files.push(path.to_path_buf());
                Ok(())
            }
            Outside::System => open_with_system(path),
        }
    }
}

/// Where a report's PDF to print goes: `<report>.pdf`, so that the PDF
/// program shows the report's name, in a folder of its own under the
/// system's temporary folder. Folders older than a day are deleted first.
pub(crate) fn print_path(tab: &str) -> Result<PathBuf, String> {
    if cfg!(target_family = "wasm") {
        return Err(tr!("printing is not available here").to_owned());
    }
    let base = std::env::temp_dir().join("gossia-print");
    remove_old(&base, KEEP);
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let folder = base.join(format!("{stamp}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).map_err(|e| {
        tr!(
            "cannot make {folder}: {e}",
            folder = folder.display(),
            e = e
        )
    })?;
    Ok(folder.join(format!("{tab}.pdf")))
}

/// Deletes the folders in `base` last changed more than `keep` ago.
fn remove_old(base: &Path, keep: Duration) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Opens `path` with the program the system opens such files with:
/// Explorer on Windows (no console window, as `cmd /c start` would show),
/// `open` on macOS, `xdg-open` elsewhere. It is not waited for, beyond
/// collecting it when it ends.
fn open_with_system(path: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map_err(|e| {
            tr!(
                "cannot open {path} with {program}: {e}",
                path = path.display(),
                program = program,
                e = e
            )
        })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_print_goes_to_a_folder_of_its_own_named_after_the_report() {
        let a = print_path("Summary").unwrap();
        let b = print_path("Summary").unwrap();
        assert_eq!(a.file_name().unwrap(), "Summary.pdf");
        assert!(a.parent().unwrap().is_dir());
        assert!(a.starts_with(std::env::temp_dir().join("gossia-print")));
        // Two prints of the same report a moment apart do not meet.
        if a.parent() == b.parent() {
            std::thread::sleep(Duration::from_millis(2));
            assert_ne!(a.parent(), print_path("Summary").unwrap().parent());
        }
    }

    #[test]
    fn old_print_folders_are_deleted() {
        let base = std::env::temp_dir().join(format!("gossia-print-test-{}", std::process::id()));
        let folder = base.join("old");
        std::fs::create_dir_all(&folder).unwrap();
        remove_old(&base, Duration::from_secs(3600));
        assert!(folder.is_dir(), "a new folder is kept");
        std::thread::sleep(Duration::from_millis(20));
        remove_old(&base, Duration::from_millis(1));
        assert!(!folder.exists(), "an old one is deleted");
        let _ = std::fs::remove_dir_all(&base);
    }
}
