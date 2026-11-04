//! Reading the command line. Nothing here opens a file.

use std::path::PathBuf;
use std::time::Duration;

/// What `--help` prints, and what a wrong command line is answered with.
pub const USAGE: &str = "\
Usage:
  cliotree-cli report <workbook.clio> <Report> --out <file.pdf>
               [--param <name>=<value>]... [--trust-digest <sha256>]
               [--overwrite] [--timeout <seconds>]
  cliotree-cli report <workbook.clio> <Report> --list-params
               [--trust-digest <sha256>] [--timeout <seconds>]
  cliotree-cli --help | --version

Exit codes: 0 written, 1 wrong command line, 2 the workbook or report cannot
be opened, 3 the workbook is not trusted, 4 a parameter is wrong, 5 written
but some controls show error values, 6 the time ran out or the report reached
its limit of detail rows.";

/// How long a run may take unless `--timeout` says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Report(ReportArgs),
}

/// `cliotree-cli report`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportArgs {
    pub workbook: PathBuf,
    pub report: String,
    pub out: Option<PathBuf>,
    /// Each `--param name=value`, in order.
    pub params: Vec<(String, String)>,
    pub digest: Option<String>,
    pub overwrite: bool,
    pub timeout: Duration,
    pub list_params: bool,
}

/// Reads the command line, without the program name.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Err("give a command".to_owned());
    };
    match first.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "--version" | "-V" => Ok(Command::Version),
        "report" => parse_report(&args[1..]).map(Command::Report),
        other => Err(format!("there is no command {other}")),
    }
}

fn parse_report(args: &[String]) -> Result<ReportArgs, String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut params = Vec::new();
    let mut digest = None;
    let mut overwrite = false;
    let mut timeout = None;
    let mut list_params = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        // `--name=value` reads as `--name value`.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if arg.starts_with("--") => (n, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| format!("{name} needs {what}"))
        };
        match name {
            "--out" => set_once(&mut out, PathBuf::from(value("a file")?), name)?,
            "--param" => {
                let given = value("name=value")?;
                let (n, v) = given
                    .split_once('=')
                    .ok_or_else(|| format!("--param {given} is not name=value"))?;
                if n.trim().is_empty() {
                    return Err(format!("--param {given} has no name"));
                }
                params.push((n.trim().to_owned(), v.to_owned()));
            }
            "--trust-digest" => set_once(&mut digest, value("a digest")?, name)?,
            "--timeout" => {
                let text = value("a number of seconds")?;
                let seconds: u64 =
                    text.trim().parse().ok().filter(|s| *s > 0).ok_or_else(|| {
                        format!("--timeout {text} is not a whole number of seconds")
                    })?;
                set_once(&mut timeout, Duration::from_secs(seconds), name)?;
            }
            "--overwrite" if inline.is_none() => overwrite = true,
            "--list-params" if inline.is_none() => list_params = true,
            _ if name.starts_with("--") => return Err(format!("there is no option {name}")),
            _ => positional.push(arg.clone()),
        }
    }
    let [workbook, report] = positional.as_slice() else {
        return Err(if positional.len() < 2 {
            "report needs a workbook and the name of a report".to_owned()
        } else {
            format!("{} is one argument too many", positional[2])
        });
    };
    if list_params {
        if out.is_some() || !params.is_empty() || overwrite {
            return Err(
                "--list-params writes nothing, so it takes no --out, --param or --overwrite"
                    .to_owned(),
            );
        }
    } else if out.is_none() {
        return Err("report needs --out, the PDF to write".to_owned());
    }
    Ok(ReportArgs {
        workbook: PathBuf::from(workbook),
        report: report.clone(),
        out,
        params,
        digest,
        overwrite,
        timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
        list_params,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{name} is given twice"));
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Command, String> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args)
    }

    fn report(line: &str) -> ReportArgs {
        match parsed(line) {
            Ok(Command::Report(r)) => r,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn a_report_command_reads_its_options_in_any_order() {
        let r = report(
            "report book.clio Sales --param from=2026-10-01 --out s.pdf --overwrite \
             --param=units=Sales --param units=Finance --timeout 30 --trust-digest=ab12",
        );
        assert_eq!(r.workbook, PathBuf::from("book.clio"));
        assert_eq!(r.report, "Sales");
        assert_eq!(r.out, Some(PathBuf::from("s.pdf")));
        assert_eq!(
            r.params,
            [
                ("from".to_owned(), "2026-10-01".to_owned()),
                ("units".to_owned(), "Sales".to_owned()),
                ("units".to_owned(), "Finance".to_owned()),
            ]
        );
        assert!(r.overwrite);
        assert_eq!(r.timeout, Duration::from_secs(30));
        assert_eq!(r.digest.as_deref(), Some("ab12"));
        assert!(!r.list_params);
    }

    #[test]
    fn the_timeout_is_ten_minutes_unless_given() {
        assert_eq!(
            report("report b.clio R --out r.pdf").timeout,
            Duration::from_secs(600)
        );
    }

    #[test]
    fn list_params_needs_no_output_and_takes_none() {
        assert!(report("report b.clio R --list-params").list_params);
        assert!(parsed("report b.clio R --list-params --out r.pdf").is_err());
    }

    #[test]
    fn a_wrong_command_line_says_what_is_wrong() {
        let error = |line| parsed(line).unwrap_err();
        assert_eq!(error(""), "give a command");
        assert_eq!(error("print b.clio"), "there is no command print");
        assert_eq!(
            error("report b.clio"),
            "report needs a workbook and the name of a report"
        );
        assert_eq!(
            error("report b.clio R"),
            "report needs --out, the PDF to write"
        );
        assert_eq!(error("report b.clio R --out"), "--out needs a file");
        assert_eq!(
            error("report b.clio R --out a --out b"),
            "--out is given twice"
        );
        assert_eq!(
            error("report b.clio R --out a --colour"),
            "there is no option --colour"
        );
        assert_eq!(
            error("report b.clio R --out a --param x"),
            "--param x is not name=value"
        );
        assert_eq!(
            error("report b.clio R --out a --timeout soon"),
            "--timeout soon is not a whole number of seconds"
        );
        assert_eq!(
            error("report b.clio R extra --out a"),
            "extra is one argument too many"
        );
    }

    #[test]
    fn help_and_version_are_commands() {
        assert_eq!(parsed("--help"), Ok(Command::Help));
        assert_eq!(parsed("--version"), Ok(Command::Version));
    }
}
