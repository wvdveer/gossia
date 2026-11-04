//! ClioTree from the command line (DESIGN.md, "Batch export from the
//! command line"): `cliotree-cli report` opens a workbook, runs one of its
//! reports with the parameters given and writes it as a PDF, without a
//! window, so that a script or a scheduled job can wait for it and read its
//! exit code. `cliotree-cli disconnect` runs Disconnect from Data Sources and
//! writes the result to a copy.
//!
//! The workbook is opened as the application opens it, with this user's
//! preferences, trust record and data sources, and is never saved. The
//! report starts afresh: its results cache and its remembered parameter
//! values are not used.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cliotree_calc::python::{self, PythonSyntax};
use cliotree_calc::{
    Calc, Decision, DisconnectError, Evaluator, Meta, Session, SessionRunner, Setting,
};
use cliotree_user::file::{
    Limits, new_workbook_id, read_zip, save_atomic, saved_digest, write_zip,
};
use cliotree_user::prefs::{Config, User};

pub mod args;
pub mod params;

use args::{Command, DisconnectArgs, ReportArgs};

/// The exit codes (DESIGN.md, "Batch export from the command line").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Written.
    Written = 0,
    /// The command line is wrong.
    Usage = 1,
    /// The workbook does not open, or has no report of that name.
    Workbook = 2,
    /// The workbook is not trusted.
    Untrusted = 3,
    /// A parameter is wrong.
    Param = 4,
    /// Written, but some controls show error values.
    Errors = 5,
    /// The time ran out, or the report reached the limit of detail rows.
    Timeout = 6,
}

/// A failure: its exit code and what to say on stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub exit: Exit,
    pub message: String,
}

impl Failure {
    pub fn new(exit: Exit, message: impl Into<String>) -> Failure {
        Failure {
            exit,
            message: message.into(),
        }
    }
}

/// Runs the command line `args` (without the program name), writing to
/// `out` and `err`, and gives the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let command = match args::parse(args) {
        Ok(command) => command,
        Err(message) => {
            let _ = writeln!(err, "cliotree-cli: {message}");
            let _ = writeln!(err, "{}", args::USAGE);
            return Exit::Usage as i32;
        }
    };
    let result = match command {
        Command::Help => {
            let _ = writeln!(out, "{}", args::USAGE);
            Ok(Exit::Written)
        }
        Command::Version => {
            let _ = writeln!(out, "cliotree-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(Exit::Written)
        }
        Command::Report(report) => run_report(&report, out, err),
        Command::Disconnect(args) => run_disconnect(&args, out, err),
    };
    match result {
        Ok(exit) => exit as i32,
        Err(failure) => {
            let _ = writeln!(err, "cliotree-cli: {}", failure.message);
            failure.exit as i32
        }
    }
}

/// A workbook opened to run, with the evaluation thread, and when the run
/// has to be done by.
pub struct Opened {
    pub session: Session,
    pub deadline: Instant,
    /// What the file holds besides the workbook, written back to a copy.
    pub meta: Meta,
    /// This user's limits on reading a `.clio` file.
    pub limits: Limits,
}

impl Opened {
    /// Waits until everything is calculated, or the time is up.
    pub fn wait(&mut self) -> Result<(), Failure> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if self.session.wait_idle(left) {
            Ok(())
        } else {
            Err(Failure::new(
                Exit::Timeout,
                "the time ran out before the workbook was calculated; nothing was written",
            ))
        }
    }

    pub fn calc(&self) -> &Calc {
        self.session.calc()
    }
}

/// Opens `path` as the application would, with this user's configuration,
/// and starts calculating it. It must be trusted by this user's trust
/// record, or its trust digest must be `digest`.
pub fn open(
    path: &Path,
    digest: Option<&str>,
    timeout: Duration,
    err: &mut dyn Write,
) -> Result<Opened, Failure> {
    let deadline = Instant::now() + timeout;
    let (user, warnings) = User::load(Config::standard());
    for warning in warnings {
        let _ = writeln!(err, "cliotree-cli: {warning}");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Failure::new(Exit::Workbook, format!("{}: {e}", path.display())))?;
    let package = read_zip(&bytes, &user.prefs.limits)
        .map_err(|e| Failure::new(Exit::Workbook, format!("{}: {e}", path.display())))?;
    let opened = Calc::from_package(Arc::new(PythonSyntax), &package)
        .map_err(|e| Failure::new(Exit::Workbook, format!("{}: {e}", path.display())))?;
    for warning in &opened.warnings {
        let _ = writeln!(err, "cliotree-cli: {warning}");
    }
    let mut calc = opened.calc;
    let content = calc.fingerprint();
    let trusted = content.items.is_empty()
        || user.trust.decide(&opened.workbook_id, &content) == Decision::Trusted
        || digest.is_some_and(|d| d.trim().eq_ignore_ascii_case(&content.hash));
    if !trusted {
        let why = if digest.is_some() {
            "the trust digest given is not this workbook's"
        } else {
            "it is not in your trust record; trust it in ClioTree, or give its trust digest \
             with --trust-digest"
        };
        return Err(Failure::new(
            Exit::Untrusted,
            format!(
                "{} is not trusted, so nothing in it was run: {why}",
                path.display()
            ),
        ));
    }
    for (setting, asked, ceiling) in calc.settings().held_down(&user.prefs.ceilings) {
        let _ = writeln!(
            err,
            "cliotree-cli: the workbook asks for a {} of {asked}; your ceiling of {ceiling} is used",
            setting.label().to_lowercase()
        );
    }
    let effective = calc.settings().effective(&user.prefs.ceilings);
    let evaluator = Evaluator::new(
        python::factory(user.python_settings(&effective)),
        effective.eval_settings(),
    );
    calc.set_report_limits(
        effective.get(Setting::MaxDetailRows) as usize,
        effective.get(Setting::MaxChartSvg) as usize,
    );
    calc.recalc_all();
    Ok(Opened {
        session: Session::new(calc, evaluator),
        deadline,
        meta: Meta {
            workbook_id: opened.workbook_id,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            views: opened.views,
            workbook_view: opened.workbook_view,
            unknown: opened.unknown,
        },
        limits: user.prefs.limits,
    })
}

/// `cliotree-cli report`.
fn run_report(
    args: &ReportArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<Exit, Failure> {
    let target = match &args.out {
        Some(target) => Some(output_path(target, "pdf", args.overwrite)?),
        None => None,
    };
    let mut opened = open(&args.workbook, args.digest.as_deref(), args.timeout, err)?;
    has_report(&opened, &args.workbook, &args.report)?;
    params::start(&mut opened, &args.report)?;
    if args.list_params {
        let _ = write!(out, "{}", params::listing(opened.calc(), &args.report));
        return Ok(Exit::Written);
    }
    params::choose(&mut opened, &args.report, &args.params)?;
    let run = cliotree_report::workbook::run(opened.calc(), &args.report)
        .map_err(|e| Failure::new(Exit::Workbook, e))?;
    if run.limited {
        return Err(Failure::new(
            Exit::Timeout,
            format!(
                "the report reached the limit of {} detail rows; nothing was written",
                opened.calc().detail_limit()
            ),
        ));
    }
    let target = target.expect("an output file unless --list-params");
    save_atomic(&target, &run.pdf)
        .map_err(|e| Failure::new(Exit::Workbook, format!("{}: {e}", target.display())))?;
    let _ = writeln!(out, "{}", target.display());
    if run.errors.is_empty() {
        return Ok(Exit::Written);
    }
    let _ = writeln!(
        err,
        "cliotree-cli: the report was written, but {} {} error values:",
        run.errors.len(),
        if run.errors.len() == 1 {
            "control shows"
        } else {
            "controls show"
        }
    );
    for error in &run.errors {
        let _ = writeln!(err, "  {error}");
    }
    Ok(Exit::Errors)
}

/// An error if the workbook has no report `report`.
fn has_report(opened: &Opened, workbook: &Path, report: &str) -> Result<(), Failure> {
    if opened.calc().report(report).is_some() {
        return Ok(());
    }
    Err(Failure::new(
        Exit::Workbook,
        format!(
            "{} has no report {report}{}",
            workbook.display(),
            list_reports(opened.calc())
        ),
    ))
}

/// `cliotree-cli disconnect` (DESIGN.md, "Disconnecting from the command
/// line"): the reads are replaced as the menu command replaces them, after
/// running `--report` with its parameters if given, and the result is saved
/// to a copy, which is not trusted: its trust digest is printed.
fn run_disconnect(
    args: &DisconnectArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<Exit, Failure> {
    let target = output_path(
        &args.out,
        "clio",
        args.overwrite || is_same_file(&args.out, &args.workbook),
    )?;
    if is_same_file(&target, &args.workbook) {
        return Err(Failure::new(
            Exit::Usage,
            format!(
                "{} is the workbook itself; --out must be another file",
                target.display()
            ),
        ));
    }
    let mut opened = open(&args.workbook, args.digest.as_deref(), args.timeout, err)?;
    if let Some(report) = &args.report {
        has_report(&opened, &args.workbook, report)?;
        params::start(&mut opened, report)?;
        params::choose(&mut opened, report, &args.params)?;
    }
    let timeout = || {
        Failure::new(
            Exit::Timeout,
            "the time ran out before the data sources had answered; nothing was written",
        )
    };
    let mut runner = SessionRunner {
        session: &mut opened.session,
        deadline: opened.deadline,
    };
    let found = cliotree_calc::find_disconnect(&mut runner).map_err(|e| match e {
        DisconnectError::Refused(why) => Failure::new(Exit::Workbook, why),
        DisconnectError::Timeout => timeout(),
    })?;
    if !cliotree_calc::apply_and_rerun(&mut runner, &found, args.report.as_deref()) {
        return Err(timeout());
    }
    opened.wait()?;
    let mut errors = Vec::new();
    if let Some(report) = &args.report {
        let run = cliotree_report::workbook::run(opened.calc(), report)
            .map_err(|e| Failure::new(Exit::Workbook, e))?;
        if run.limited {
            return Err(Failure::new(
                Exit::Timeout,
                format!(
                    "the report reached the limit of {} detail rows; nothing was written",
                    opened.calc().detail_limit()
                ),
            ));
        }
        errors = run.errors;
    }
    // A copy is another workbook, with an id of its own.
    let meta = Meta {
        workbook_id: new_workbook_id(),
        ..opened.meta.clone()
    };
    let write = |e: String| Failure::new(Exit::Workbook, format!("{}: {e}", target.display()));
    let package = opened
        .calc()
        .to_package(&meta)
        .map_err(|e| write(e.to_string()))?;
    let bytes = write_zip(&package).map_err(write)?;
    save_atomic(&target, &bytes).map_err(|e| write(e.to_string()))?;
    let digest = saved_digest(&target, &opened.limits).map_err(write)?;
    let _ = writeln!(out, "{}", target.display());
    let _ = writeln!(out, "{digest}");
    let _ = writeln!(
        err,
        "cliotree-cli: replaced {} {} by what {} now",
        found.replaced,
        if found.replaced == 1 {
            "read of a data source"
        } else {
            "reads of data sources"
        },
        if found.replaced == 1 {
            "it gives"
        } else {
            "they give"
        }
    );
    if found.problems.is_empty() && errors.is_empty() {
        return Ok(Exit::Written);
    }
    if !found.problems.is_empty() {
        let _ = writeln!(
            err,
            "cliotree-cli: the copy was written, but {} {} a data source:",
            found.problems.len(),
            if found.problems.len() == 1 {
                "use still needs"
            } else {
                "uses still need"
            }
        );
        for problem in &found.problems {
            let _ = writeln!(err, "  {problem}");
        }
    }
    if !errors.is_empty() {
        let _ = writeln!(
            err,
            "cliotree-cli: the copy was written, but {} of the report {} error values:",
            errors.len(),
            if errors.len() == 1 {
                "control shows"
            } else {
                "controls show"
            }
        );
        for error in &errors {
            let _ = writeln!(err, "  {error}");
        }
    }
    Ok(Exit::Errors)
}

/// Whether `a` and `b` are the same file, both being there.
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The file to write: given the extension `ext` if it has none, in a folder
/// that exists, and not a file that is there already unless `overwrite`.
fn output_path(given: &Path, ext: &str, overwrite: bool) -> Result<PathBuf, Failure> {
    let path = if given.extension().is_some() {
        given.to_path_buf()
    } else {
        given.with_extension(ext)
    };
    let path = std::path::absolute(&path).unwrap_or(path);
    if path.exists() && !overwrite {
        return Err(Failure::new(
            Exit::Usage,
            format!(
                "{} is there already; give --overwrite to replace it",
                path.display()
            ),
        ));
    }
    if let Some(folder) = path.parent()
        && !folder.is_dir()
    {
        return Err(Failure::new(
            Exit::Usage,
            format!("the folder {} does not exist", folder.display()),
        ));
    }
    Ok(path)
}

/// `: its reports are A, B` for a message, or nothing.
fn list_reports(calc: &Calc) -> String {
    let names: Vec<String> = calc
        .tab_names()
        .into_iter()
        .filter(|t| calc.report(t).is_some())
        .collect();
    if names.is_empty() {
        "; it has no reports".to_owned()
    } else {
        format!("; its reports are {}", names.join(", "))
    }
}
