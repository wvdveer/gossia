//! Conformance against the official JSON Schema Test Suite, vendored at the
//! revision pinned in `vendor.lock`. See DESIGN.md, "Testing and CI",
//! "Selecting JSON Schema Test Suite cases".
//!
//! - A test group whose schema uses only the allowed keywords (searched
//!   recursively) runs as pass or fail: the schema must lower, and every test
//!   case must come out as the suite expects.
//! - Any other group is a must-reject test: lowering must fail with an error
//!   that names one of the keywords outside the allowed list.
//! - A group that uses only allowed keywords but still cannot run is listed in
//!   `jsonschema-selection.toml` with a reason, so that nothing is skipped
//!   silently. The list is checked both ways: an entry for a group that now
//!   runs, or for one that no longer exists, fails the test.
//!
//! Two deliberate differences from the suite are accepted case by case rather
//! than listed: objects are closed, so a case that expects data with extra
//! properties to be valid passes when the only complaint is about undeclared
//! fields; and the entries in `skip_test` are single cases where the subset
//! differs on purpose.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use gossia_core::json;
use gossia_core::jsonschema::{LoadError, lower_str};
use gossia_core::validate::{IssueKind, validate};
use serde::Deserialize;

#[derive(Deserialize)]
struct Selection {
    version: u32,
    /// Keywords a group may use and still run as pass or fail.
    allowed: Vec<String>,
    /// Why a group is skipped, by short name.
    reasons: BTreeMap<String, String>,
    #[serde(default)]
    skip: Vec<SkipGroup>,
    #[serde(default)]
    skip_test: Vec<SkipTest>,
}

#[derive(Deserialize)]
struct SkipGroup {
    file: String,
    group: String,
    why: String,
}

#[derive(Deserialize)]
struct SkipTest {
    file: String,
    group: String,
    test: String,
    why: String,
}

#[derive(Deserialize)]
struct Group {
    description: String,
    schema: serde_json::Value,
    tests: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    description: String,
    data: serde_json::Value,
    valid: bool,
}

/// Keywords that never change what a schema accepts.
const ANNOTATIONS: &[&str] = &["title", "description", "$schema", "$defs"];

/// Stands for a `true` or `false` schema, which the subset does not have.
const BOOLEAN_SCHEMA: &str = "(boolean schema)";

fn suite_dir() -> PathBuf {
    let vendor = std::env::var_os("GOSSIA_VENDOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor"));
    let dir = vendor.join("json-schema-test-suite/tests/draft2020-12");
    assert!(
        dir.is_dir(),
        "the JSON Schema Test Suite is not at {}. Run `cargo xtask vendor fetch` first.",
        dir.display()
    );
    dir
}

/// Collects the keywords used by a schema and its subschemas. Property names,
/// `enum` values, and `const` values are data, not keywords, and are skipped.
fn collect(schema: &serde_json::Value, out: &mut BTreeSet<String>) {
    use serde_json::Value as J;
    let J::Object(map) = schema else {
        if schema.is_boolean() {
            out.insert(BOOLEAN_SCHEMA.to_string());
        }
        return;
    };
    for (key, value) in map {
        out.insert(key.clone());
        // Some allowed keywords are used in ways the subset excludes. These
        // count as outside the list, under a name that is the keyword and the
        // use: `type (list)`. Lowering must refuse them naming the keyword.
        match (key.as_str(), value) {
            ("type", J::Array(_)) => {
                out.insert("type (list)".to_string());
            }
            ("type", J::String(name)) if name == "null" => {
                out.insert("type (null)".to_string());
            }
            ("enum", J::Array(values)) if values.iter().any(|v| !v.is_string()) => {
                out.insert("enum (non-string)".to_string());
            }
            ("$schema", J::String(uri))
                if uri != "https://json-schema.org/draft/2020-12/schema" =>
            {
                out.insert("$schema (other)".to_string());
            }
            ("additionalProperties", J::Bool(true)) => {
                out.insert("additionalProperties (true)".to_string());
            }
            _ => {}
        }
        match (key.as_str(), value) {
            (
                "properties" | "$defs" | "definitions" | "patternProperties" | "dependentSchemas",
                J::Object(children),
            ) => children.values().for_each(|c| collect(c, out)),
            (
                "items"
                | "additionalProperties"
                | "not"
                | "if"
                | "then"
                | "else"
                | "contains"
                | "propertyNames"
                | "unevaluatedProperties"
                | "unevaluatedItems",
                child,
            ) => collect(child, out),
            ("allOf" | "anyOf" | "oneOf" | "prefixItems", J::Array(children)) => {
                children.iter().for_each(|c| collect(c, out))
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Stats {
    run_groups: usize,
    run_cases: usize,
    closed_object_cases: usize,
    reject_groups: usize,
    skipped_groups: usize,
    skipped_cases: usize,
}

#[test]
fn json_schema_test_suite_selection() {
    let selection: Selection =
        toml::from_str(include_str!("jsonschema-selection.toml")).expect("selection file parses");
    assert_eq!(selection.version, 1);
    let allowed: BTreeSet<&str> = selection
        .allowed
        .iter()
        .map(String::as_str)
        .chain(ANNOTATIONS.iter().copied())
        .collect();

    let mut failures: Vec<String> = Vec::new();
    let mut stats = Stats::default();
    let mut used_groups: HashSet<(String, String)> = HashSet::new();
    let mut used_cases: HashSet<(String, String, String)> = HashSet::new();
    let skip_groups: BTreeMap<(&str, &str), &str> = selection
        .skip
        .iter()
        .map(|s| ((s.file.as_str(), s.group.as_str()), s.why.as_str()))
        .collect();
    let skip_cases: BTreeMap<(&str, &str, &str), &str> = selection
        .skip_test
        .iter()
        .map(|s| {
            (
                (s.file.as_str(), s.group.as_str(), s.test.as_str()),
                s.why.as_str(),
            )
        })
        .collect();

    let mut files: Vec<PathBuf> = std::fs::read_dir(suite_dir())
        .expect("suite directory is readable")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    assert!(
        files.len() > 30,
        "expected the suite's files, found {}",
        files.len()
    );

    for path in &files {
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(path).expect("suite file is readable");
        let groups: Vec<Group> =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut seen_in_file = HashSet::new();
        for group in &groups {
            assert!(
                seen_in_file.insert(group.description.clone()),
                "{file}: two groups are called {:?}, so the selection file cannot tell them apart",
                group.description
            );
            let schema_text = serde_json::to_string(&group.schema).unwrap();
            let mut keywords = BTreeSet::new();
            collect(&group.schema, &mut keywords);
            let outside: BTreeSet<&str> = keywords
                .iter()
                .map(String::as_str)
                .filter(|k| !allowed.contains(k))
                .collect();
            let lowered = lower_str(&schema_text);
            let id = format!("{file} / {}", group.description);

            if !outside.is_empty() {
                stats.reject_groups += 1;
                match lowered {
                    Ok(_) => failures.push(format!(
                        "{id}: accepted a schema that uses {outside:?}, which must be refused"
                    )),
                    Err(LoadError::NotJson(e)) => {
                        failures.push(format!("{id}: not read as JSON: {e}"))
                    }
                    Err(LoadError::Schema(e)) => {
                        let named = |k: &&str| k.split(" (").next() == Some(e.keyword.as_str());
                        // A keyword of a type on a schema with no `type` is
                        // refused by a rule of its own, wherever it is, so the
                        // keyword outside may be further in.
                        let typeless = e.message.contains("with no `type`");
                        if !outside.iter().any(named)
                            && !outside.contains(BOOLEAN_SCHEMA)
                            && !typeless
                        {
                            failures.push(format!(
                                "{id}: refused, but the error names `{}` and not one of {outside:?}: {e}",
                                e.keyword
                            ));
                        }
                    }
                }
                continue;
            }

            let key = (file.as_str(), group.description.as_str());
            let spec = match (lowered, skip_groups.get(&key)) {
                (Ok(spec), None) => spec,
                (Ok(_), Some(_)) => {
                    failures.push(format!("{id}: now lowers, so remove it from the skip list"));
                    continue;
                }
                (Err(e), Some(why)) => {
                    assert!(
                        selection.reasons.contains_key(*why),
                        "{id}: unknown reason {why:?}"
                    );
                    used_groups.insert((file.clone(), group.description.clone()));
                    stats.skipped_groups += 1;
                    stats.skipped_cases += group.tests.len();
                    let _ = e;
                    continue;
                }
                (Err(e), None) => {
                    failures.push(format!(
                        "NEW {{ file = {file:?}, group = {:?}, why = \"\" }} -- {e}",
                        group.description
                    ));
                    continue;
                }
            };

            stats.run_groups += 1;
            for case in &group.tests {
                let case_key = (
                    file.as_str(),
                    group.description.as_str(),
                    case.description.as_str(),
                );
                if let Some(why) = skip_cases.get(&case_key) {
                    assert!(
                        selection.reasons.contains_key(*why),
                        "{id}: unknown reason {why:?}"
                    );
                    used_cases.insert((
                        file.clone(),
                        group.description.clone(),
                        case.description.clone(),
                    ));
                    stats.skipped_cases += 1;
                    continue;
                }
                stats.run_cases += 1;
                let data = json::parse(&serde_json::to_string(&case.data).unwrap())
                    .unwrap_or_else(|e| panic!("{id}: data is not read: {e}"))
                    .node;
                let issues = validate(&spec, &data);
                let only_undeclared = issues.iter().all(|i| i.kind == IssueKind::UndeclaredField);
                match (case.valid, issues.is_empty()) {
                    (true, true) | (false, false) => {}
                    (true, false) if only_undeclared => stats.closed_object_cases += 1,
                    (true, false) => failures.push(format!(
                        "NEWCASE {{ file = {file:?}, group = {:?}, test = {:?}, why = \"\" }} -- expected valid, got {:?}",
                        group.description,
                        case.description,
                        issues.iter().map(|i| format!("{}: {}", i.path, i.message)).collect::<Vec<_>>()
                    )),
                    (false, true) => failures.push(format!(
                        "NEWCASE {{ file = {file:?}, group = {:?}, test = {:?}, why = \"\" }} -- expected invalid, accepted",
                        group.description, case.description
                    )),
                }
            }
        }
    }

    for entry in &selection.skip {
        if !used_groups.contains(&(entry.file.clone(), entry.group.clone())) {
            failures.push(format!(
                "skip entry {} / {} matches no group that uses only allowed keywords; remove it",
                entry.file, entry.group
            ));
        }
    }
    for entry in &selection.skip_test {
        if !used_cases.contains(&(entry.file.clone(), entry.group.clone(), entry.test.clone())) {
            failures.push(format!(
                "skip_test entry {} / {} / {} matches no case; remove it",
                entry.file, entry.group, entry.test
            ));
        }
    }
    let used_reasons: BTreeSet<&str> = selection
        .skip
        .iter()
        .map(|s| s.why.as_str())
        .chain(selection.skip_test.iter().map(|s| s.why.as_str()))
        .collect();
    for reason in selection.reasons.keys() {
        if !used_reasons.contains(reason.as_str()) {
            failures.push(format!(
                "reason {reason:?} is not used by any entry; remove it"
            ));
        }
    }

    println!(
        "JSON Schema Test Suite: {} groups ({} cases) run as pass or fail, {} of those cases only because          objects are closed; {} groups checked as must-reject; {} groups ({} cases) skipped with a reason",
        stats.run_groups,
        stats.run_cases,
        stats.closed_object_cases,
        stats.reject_groups,
        stats.skipped_groups,
        stats.skipped_cases
    );
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
