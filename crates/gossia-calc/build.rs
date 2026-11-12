//! Chooses the Python standard library modules frozen into the engine
//! (DESIGN.md, step 13g): those that the modules formulas can use reach by
//! importing, not the whole of RustPython's `Lib`.
//!
//! The roots are the modules of `python::MODULES`, the ones the prelude
//! imports, the `encodings` package, which the interpreter imports when it
//! starts and whose codecs are found by name, and the modules the
//! interpreter's own Rust code imports by name (`_strptime` for `strptime`). From each root the imports in
//! its file are followed: those made when it is imported always, and those
//! made inside a function or with `lazy` (Python 3.15) unless they lead to a
//! module for tools (a debugger, a test runner, the network). Nothing under `if __name__ == "__main__":` is
//! followed. Reading is by lines, not a parser, since the files are newer
//! Python than any parser here; it may find too much, which costs only size.
//!
//! The files chosen are copied under `OUT_DIR/pylib`, and `frozen_lib.rs`
//! freezes that folder with `py_freeze!`; `frozen_names.rs` lists the modules
//! for the tests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The modules formulas can use (`python::MODULES`), and those the prelude
/// and the engine import (`datetime`, `decimal`, `json`, `math`).
const ROOTS: &[&str] = &[
    "math",
    "datetime",
    "re",
    "json",
    "itertools",
    "decimal",
    "fractions",
    "collections",
    "functools",
    "string",
    "textwrap",
    "statistics",
    "encodings",
    // What the interpreter's Rust code imports by name, which a formula can
    // reach through the modules above (python::INTERNAL_MODULES).
    "locale",
    "_pycodecs",
    "typing",
    "warnings",
    "weakref",
    "copyreg",
    "codecs",
    "string.templatelib",
];

/// Modules the engine cannot have: `os` needs `posix` or `nt`, which give
/// Python the machine and are not built in, `stringprep` a Unicode database
/// left out, and the rest are written in Rust and left out. A module that imports one of them unconditionally
/// when it is imported cannot be imported either, so it is not frozen.
const UNAVAILABLE: &[&str] = &[
    "os",
    "posix",
    "nt",
    "importlib",
    "stringprep",
    // Modules written in Rust that the engine leaves out
    // (`rustpython_stdlib::formula_module_defs`).
    "zlib",
    "_tokenize",
    "_bz2",
    "_lzma",
    "_hashlib",
    "_pickle",
    "pyexpat",
    "_csv",
    "array",
];

/// Packages taken whole: `encodings` finds a codec by its name.
const WHOLE: &[&str] = &["encodings"];

/// Modules for tools, which no formula needs: an import of one inside a
/// function is not followed (one made at import time still is).
const TOOLS: &[&str] = &[
    "pdb",
    "pydoc",
    "pydoc_data",
    "doctest",
    "unittest",
    "_pyrepl",
    "asyncio",
    "argparse",
    "http",
    "email",
    "urllib",
    "ctypes",
    "subprocess",
    "socket",
    "ssl",
    "tarfile",
    "zipfile",
    "shutil",
    "tempfile",
    "logging",
    "code",
    "codeop",
    "bdb",
    "cmd",
    "webbrowser",
    "ftplib",
    "pickle",
    "tracemalloc",
    "platform",
    "sysconfig",
    "importlib",
    "runpy",
    "pkgutil",
    "zipimport",
    "glob",
    "gzip",
    "bz2",
    "lzma",
    "compression",
    "difflib",
    "pathlib",
    "tkinter",
    "turtle",
    "idlelib",
    "venv",
    "ensurepip",
    "multiprocessing",
    "concurrent",
    "threading",
    "selectors",
    "signal",
];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib = manifest.join("../../vendor/rustpython/Lib");
    let lib = lib.canonicalize().unwrap_or(lib);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", lib.display());

    // Leaving out what cannot be imported may leave out more.
    let mut dead: BTreeSet<String> = UNAVAILABLE.iter().map(|m| (*m).to_owned()).collect();
    let chosen = loop {
        let chosen = closure(&lib, &dead);
        let newly: Vec<String> = chosen
            .iter()
            .filter(|(_, (_, needed))| needed.iter().any(|n| dead.contains(n)))
            .map(|(m, _)| m.clone())
            .collect();
        if newly.is_empty() {
            break chosen;
        }
        dead.extend(newly);
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let pylib = out.join("pylib");
    let _ = std::fs::remove_dir_all(&pylib);
    let mut stamp: u64 = 0xcbf2_9ce4_8422_2325;
    for (module, (file, _)) in &chosen {
        let relative = file.strip_prefix(&lib).unwrap();
        let target = pylib.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let bytes = std::fs::read(file).unwrap();
        // FNV-1a over names and contents: in the generated file, so that a
        // changed module freezes again.
        for b in module.bytes().chain(bytes.iter().copied()) {
            stamp = (stamp ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        std::fs::write(&target, bytes).unwrap();
    }
    let dir = pylib.display().to_string().replace('\\', "/");
    std::fs::write(
        out.join("frozen_lib.rs"),
        format!(
            "// {} modules, contents {stamp:016x}\nrustpython_vm::py_freeze!(dir = \"{dir}\")\n",
            chosen.len()
        ),
    )
    .unwrap();
    let mut names = String::from("&[\n");
    for module in chosen.keys() {
        let _ = writeln!(names, "    {module:?},");
    }
    names.push(']');
    std::fs::write(out.join("frozen_names.rs"), names).unwrap();
    text_widths(&manifest, &out);
}

/// The ranges of characters whose widths `text_widths` records: Latin,
/// Greek, punctuation, super- and subscripts, currency signs, arrows and
/// the mathematical operators (the minus sign among them).
const WIDTH_RANGES: &[(u32, u32)] = &[
    (0x20, 0x24f),
    (0x370, 0x3ff),
    (0x2000, 0x20cf),
    (0x2190, 0x21ff),
    (0x2200, 0x22ff),
];

/// `text_widths.rs`: the advance width of each character of
/// `WIDTH_RANGES` in Inter, the charts' font, in thousandths of an em, for
/// Gossia pyplot's measuring of labels (DESIGN.md, Gossia pyplot). 0 is a
/// character the font lacks.
fn text_widths(manifest: &Path, out: &Path) {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::{FontRef, MetadataProvider};
    let path = manifest.join("../gossia-layout/assets/fonts/Inter.ttf");
    println!("cargo:rerun-if-changed={}", path.display());
    let data = std::fs::read(&path).unwrap();
    let font = FontRef::new(&data).unwrap();
    let em = f32::from(font.head().unwrap().units_per_em());
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let mut text = String::from(
        "&[
",
    );
    for &(first, last) in WIDTH_RANGES {
        let _ = write!(text, "    ({first:#x}, &[");
        for code in first..=last {
            let width = char::from_u32(code)
                .and_then(|c| charmap.map(c))
                .and_then(|glyph| metrics.advance_width(glyph))
                .map_or(0, |w| (w / em * 1000.0).round() as u32);
            let _ = write!(text, "{width},");
        }
        text.push_str(
            "]),
",
        );
    }
    text.push(']');
    std::fs::write(out.join("text_widths.rs"), text).unwrap();
}

/// The file of a module in `lib`, and whether it is a package.
fn file_of(lib: &Path, module: &str) -> Option<(PathBuf, bool)> {
    let base = module
        .split('.')
        .fold(lib.to_path_buf(), |p, part| p.join(part));
    let file = base.with_extension("py");
    if file.is_file() {
        return Some((file, false));
    }
    let init = base.join("__init__.py");
    init.is_file().then_some((init, true))
}

/// The modules chosen, by name, with their files and the modules each
/// imports unconditionally when it is imported. Nothing in `dead` is chosen.
fn closure(lib: &Path, dead: &BTreeSet<String>) -> BTreeMap<String, (PathBuf, BTreeSet<String>)> {
    let mut chosen = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut todo: VecDeque<String> = ROOTS.iter().map(|r| (*r).to_owned()).collect();
    for package in WHOLE {
        let dir = lib.join(package);
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".py")
                && stem != "__init__"
            {
                todo.push_back(format!("{package}.{stem}"));
            }
        }
    }
    while let Some(module) = todo.pop_front() {
        if !seen.insert(module.clone()) {
            continue;
        }
        let mut prefix = String::new();
        let in_dead = module.split('.').any(|part| {
            if !prefix.is_empty() {
                prefix.push('.');
            }
            prefix.push_str(part);
            dead.contains(&prefix)
        });
        if in_dead {
            continue;
        }
        let Some((file, package)) = file_of(lib, &module) else {
            continue;
        };
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let found = imports(&module, package, &text);
        let tool = |name: &String| TOOLS.contains(&name.split('.').next().unwrap_or_default());
        todo.extend(found.eager);
        todo.extend(found.lazy.into_iter().filter(|n| !tool(n)));
        chosen.insert(module, (file, found.needed));
    }
    chosen
}

/// What a file imports.
struct Imports {
    /// When it is imported.
    eager: BTreeSet<String>,
    /// Inside functions, or with `lazy`.
    lazy: BTreeSet<String>,
    /// When it is imported, outside any `try` or `if`: the modules it
    /// cannot do without.
    needed: BTreeSet<String>,
}

/// The modules a file imports. Each dotted name brings its parent
/// packages, and `from a import b` may import the submodule `a.b`.
fn imports(module: &str, package: bool, text: &str) -> Imports {
    let here = if package {
        module.to_owned()
    } else {
        module.rsplit_once('.').map_or("", |(p, _)| p).to_owned()
    };
    let (mut eager, mut lazy, mut needed) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    // The open blocks: their indent and whether they are a function or the
    // `__main__` block.
    let mut stack: Vec<(usize, Block)> = Vec::new();
    let mut in_string: Option<&str> = None;
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        if let Some(quote) = in_string {
            if raw.contains(quote) {
                in_string = None;
            }
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        for quote in ["\"\"\"", "'''"] {
            if line.matches(quote).count() % 2 == 1 {
                in_string = Some(quote);
            }
        }
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let is_def = line.starts_with("def ") || line.starts_with("async def ");
        if is_def {
            stack.push((indent, Block::Function));
            continue;
        }
        if is_main(line) {
            stack.push((indent, Block::Main));
            continue;
        }
        if line.ends_with(':') {
            stack.push((indent, Block::Other));
            continue;
        }
        if stack.iter().any(|(_, b)| *b == Block::Main) {
            continue;
        }
        // `lazy import x` loads x when it is first used, as an import
        // inside a function does.
        let (line, deferred) = match line.strip_prefix("lazy ") {
            Some(rest) => (rest.trim_start(), true),
            None => (line, false),
        };
        let mut statement = line.to_owned();
        if statement.contains("import (") && !statement.contains(')') {
            for more in lines.by_ref() {
                statement.push(' ');
                statement.push_str(more.trim());
                if more.contains(')') {
                    break;
                }
            }
        }
        let names = imported(&here, &statement);
        if !deferred
            && stack.is_empty()
            && let Some(target) = names.first()
        {
            let parts: Vec<&str> = target.split('.').collect();
            for n in 1..=parts.len() {
                needed.insert(parts[..n].join("."));
            }
        }
        let into = if deferred || stack.iter().any(|(_, b)| *b == Block::Function) {
            &mut lazy
        } else {
            &mut eager
        };
        for name in names {
            let parts: Vec<&str> = name.split('.').collect();
            for n in 1..=parts.len() {
                into.insert(parts[..n].join("."));
            }
        }
    }
    Imports {
        eager,
        lazy,
        needed,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Function,
    Main,
    Other,
}

fn is_main(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.starts_with("if__name__==\"__main__\"") || compact.starts_with("if__name__=='__main__'")
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c == '_' || c == '.' || c.is_alphanumeric())
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

/// The modules an `import` or `from ... import` statement names.
fn imported(here: &str, statement: &str) -> Vec<String> {
    let statement = statement.split('#').next().unwrap_or_default().trim();
    let statement = statement.trim_end_matches(';');
    if let Some(rest) = statement.strip_prefix("import ") {
        return rest
            .split(',')
            .filter_map(|part| part.split(" as ").next())
            .map(str::trim)
            .filter(|n| is_name(n))
            .map(str::to_owned)
            .collect();
    }
    let Some(rest) = statement.strip_prefix("from ") else {
        return Vec::new();
    };
    let Some((source, names)) = rest.split_once(" import ") else {
        return Vec::new();
    };
    let source = source.trim();
    let dots = source.len() - source.trim_start_matches('.').len();
    let tail = &source[dots..];
    let target = if dots == 0 {
        tail.to_owned()
    } else {
        let mut base = here.to_owned();
        for _ in 1..dots {
            base = base.rsplit_once('.').map_or("", |(p, _)| p).to_owned();
        }
        match (base.is_empty(), tail.is_empty()) {
            (_, true) => base,
            (true, false) => tail.to_owned(),
            (false, false) => format!("{base}.{tail}"),
        }
    };
    if !is_name(&target) {
        return Vec::new();
    }
    let mut out = vec![target.clone()];
    for name in names.split([',', '(', ')', ' ', '\\']) {
        let name = name.trim();
        if name.is_empty() || name == "*" || name == "as" || !is_name(name) || name.contains('.') {
            continue;
        }
        out.push(format!("{target}.{name}"));
    }
    out
}
