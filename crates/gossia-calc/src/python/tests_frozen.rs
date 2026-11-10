//! The standard library frozen into the engine is only what formulas can
//! reach (build.rs, DESIGN.md step 13g). These tests are the check that
//! nothing they need was left out.

use super::{FROZEN, INTERNAL_MODULES, MODULES, PythonEngine};

/// An interpreter with the engine's standard library and nothing of the
/// engine's own: Python's import, not the formulas' one.
fn bare() -> rustpython::Interpreter {
    let mut settings = rustpython::vm::Settings::default();
    settings.isolated = true;
    settings.import_site = false;
    settings.install_signal_handlers = false;
    let builder = rustpython::Interpreter::builder(settings);
    let native = rustpython_stdlib::formula_module_defs(&builder.ctx);
    builder
        .add_native_modules(&native)
        .add_frozen_modules(super::frozen_lib())
        .build()
}

/// Every module frozen in can be imported: build.rs followed what each one
/// imports when it is imported. A few are for one system only.
#[test]
fn every_frozen_module_imports() {
    let interpreter = bare();
    // The codecs of Windows' code pages need Windows' own functions.
    let elsewhere = [
        "encodings._win_cp_codecs",
        "encodings.mbcs",
        "encodings.oem",
    ];
    let mut failed = Vec::new();
    for name in FROZEN {
        if elsewhere.contains(name) && !cfg!(windows) {
            continue;
        }
        let imported = interpreter.enter(|vm| {
            use rustpython::vm::AsObject;
            vm.import(*name, 0).map_err(|e| {
                format!(
                    "{}: {}",
                    e.as_object().class().name(),
                    e.as_object()
                        .str(vm)
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                )
            })
        });
        if let Err(why) = imported {
            failed.push(format!("{name} ({why})"));
        }
    }
    assert!(failed.is_empty(), "these do not import: {failed:?}");
    // The allowed modules and the internal ones import too, frozen or
    // written in Rust.
    for name in MODULES.iter().chain(INTERNAL_MODULES) {
        let imported = interpreter.enter(|vm| vm.import(*name, 0).is_ok());
        assert!(imported, "{name}");
    }
}

/// What formulas use from the allowed modules works, including what imports
/// more only when it is called (`strptime`, a locale's number format, a
/// codec by name).
#[test]
fn what_formulas_use_imports_what_it_needs() {
    let engine = PythonEngine::new();
    let uses = [
        "datetime.date(2026, 11, 6).strftime('%A %d %B %Y')",
        "datetime.datetime.fromisoformat('2026-11-06T14:30:00+01:00').astimezone(datetime.timezone.utc)",
        "str(datetime.timedelta(days=1.5)) + repr(datetime.timezone(datetime.timedelta(hours=2)))",
        "format(decimal.Decimal('1234.5678'), ',.2f')",
        "format(decimal.Decimal('1234.5'), 'n')",
        "decimal.Decimal(1) / decimal.Decimal(7)",
        "decimal.Decimal('1.25').quantize(decimal.Decimal('0.1'), rounding=decimal.ROUND_HALF_UP)",
        "fractions.Fraction('3.1415926').limit_denominator(100)",
        "fractions.Fraction(decimal.Decimal('0.25')) + fractions.Fraction(1, 3)",
        "json.dumps({'b': [1, 2.5, None], 'a': 'é'}, indent=2, sort_keys=True, ensure_ascii=False)",
        "json.loads('{\"a\": [1, 2.5, {\"b\": null}]}', parse_float=decimal.Decimal)",
        "re.sub(r'(?P<w>\\w+)', lambda m: m['w'].upper(), 'ab cd', flags=re.I)",
        "re.compile(r'\\d+  # digits', re.VERBOSE | re.MULTILINE).findall('1 22\\n333')",
        "re.escape('a.b') + re.fullmatch(r'[\\w.]+@\\w+', 'a.b@c').group(0)",
        "textwrap.fill('word ' * 30, width=20)",
        "textwrap.shorten('Hello world, this is long', width=12)",
        "statistics.mean([1, 2, 3, 4]) + statistics.median([3, 1, 2]) + statistics.mode([1, 1, 2])",
        "statistics.stdev([2, 4, 4, 4, 5, 5, 7, 9]) + statistics.pvariance([1, 2, 3, 4])",
        "statistics.quantiles(range(1, 101), n=4) + [statistics.fmean([1.5, 2.5])]",
        "statistics.correlation([1, 2, 3], [2, 4, 7]) + statistics.covariance([1, 2, 3], [1, 2, 4])",
        "statistics.linear_regression([1, 2, 3], [2, 4, 6]).slope",
        "statistics.NormalDist(100, 15).cdf(130) + statistics.NormalDist.from_samples([1, 2, 3]).stdev",
        "statistics.NormalDist(0, 1).samples(3, seed=7)",
        "statistics.median_grouped([1, 2, 2, 3]) + statistics.geometric_mean([1, 4]) + statistics.harmonic_mean([1, 2])",
        "textwrap.dedent('  a\\n  b') + textwrap.indent('a', '> ')",
        "collections.namedtuple('P', 'x y')(1, 2)._asdict()",
        "collections.Counter('abracadabra').most_common(2)",
        "collections.ChainMap({'a': 1}, {'b': 2})['b'] + len(collections.deque([1, 2], maxlen=1))",
        "functools.reduce(lambda a, b: a * b, range(1, 6))",
        "functools.lru_cache(maxsize=None)(abs)(-3) + functools.partial(int, base=2)('101')",
        "sorted([3, 1, 2], key=functools.cmp_to_key(lambda a, b: a - b))",
        "functools.singledispatch(lambda x: x)(1)",
        "string.Template('$a!').substitute(a=1) + string.Formatter().format('{0:>4}', 7)",
        "string.capwords('hello world') + string.ascii_letters[:3]",
        "list(itertools.accumulate([1, 2, 3])) + list(itertools.chain([1], [2]))",
        "math.prod([1, 2, 3]) + math.isqrt(17)",
        "'é'.encode('cp1252') + 'é'.encode('latin-1') + 'ü'.encode('iso8859_15')",
        "b'\\xff\\xfea\\x00'.decode('utf-16') + 'x'.encode('utf-8-sig').decode('utf-8-sig')",
        "'ab'.encode('ascii').decode('cp437') + 'é'.encode('mac_roman').decode('mac_roman')",
    ];
    // And a workbook module cannot import what the engine does not offer.
    assert!(!engine.can_import("os"));
    // `strptime` needs `os` (through `_strptime`), so it fails, and says so.
    let strptime = engine.probe(&["datetime.datetime.strptime('2026', '%Y')"]);
    assert!(strptime[0].contains("Error"), "{strptime:?}");
    let failed: Vec<String> = uses
        .iter()
        .zip(engine.probe(&uses))
        .filter(|(_, outcome)| outcome.contains("Error:"))
        .map(|(source, outcome)| format!("{source}: {outcome}"))
        .collect();
    assert!(
        failed.is_empty(),
        "{}",
        failed.join(
            "
"
        )
    );
}
