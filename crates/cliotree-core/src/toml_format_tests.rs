use super::*;

fn read(text: &str) -> Node {
    parse(text).unwrap().node
}

fn at<'a>(node: &'a Node, path: &str) -> &'a Node {
    node.get(&path.parse().unwrap()).unwrap()
}

const CARGO: &str = r#"# The package
[package]
# Its name
name = "demo"   # short
version = "0.1.0"
released = 2025-10-17
built = 2025-10-17T09:30:00
signed = 2025-10-17T09:30:00+10:00
"odd key" = 1.5
tags = [
  # first
  "a",
  "b", # second
  # after
]

[[bin]]
name = "one"

# The second binary
[[bin]]
name = "two"
test = false
"#;

#[test]
fn toml_is_read_with_its_types_and_comments() {
    let node = read(CARGO);
    assert_eq!(at(&node, "package").comment(), Some("The package"));
    let name = at(&node, "package.name");
    assert_eq!(name.value(), &Value::Str("demo".into()));
    assert_eq!(name.comment(), Some("Its name\nshort"));
    assert!(matches!(
        at(&node, "package.released").value(),
        Value::Date(_)
    ));
    assert!(matches!(
        at(&node, "package.built").value(),
        Value::DateTime(DateTimeValue::Naive(_))
    ));
    assert!(matches!(
        at(&node, "package.signed").value(),
        Value::DateTime(DateTimeValue::Zoned(_))
    ));
    assert_eq!(
        at(&node, "package[\"odd key\"]").value(),
        &Value::Float(1.5)
    );
    assert_eq!(at(&node, "package.tags[0]").comment(), Some("first"));
    assert_eq!(at(&node, "package.tags[1]").comment(), Some("second"));
    assert_eq!(
        at(&node, "package.tags").comment_at(CommentPlace::End),
        Some("after")
    );
    assert_eq!(at(&node, "bin[1]").comment(), Some("The second binary"));
    assert_eq!(at(&node, "bin[1].test").value(), &Value::Bool(false));
}

#[test]
fn a_time_of_day_is_text_with_a_warning_and_mistakes_are_placed() {
    let parsed = parse("alarm = 07:30:00\n").unwrap();
    assert_eq!(
        at(&parsed.node, "alarm").value(),
        &Value::Str("07:30:00".into())
    );
    assert!(parsed.warnings[0].message.contains("time of day"));
    let e = parse("a = 1\nb = \n").unwrap_err();
    assert_eq!(e.line, 2, "{e}");
    assert!(parse("x = nan").unwrap_err().message.contains("finite"));
}

#[test]
fn written_back_it_reads_the_same_with_its_comments() {
    let node = read(CARGO);
    let text = to_string(&node).unwrap();
    assert!(
        text.starts_with("# The package\n[package]\n# Its name\n# short\nname = \"demo\"\n"),
        "{text}"
    );
    assert!(text.contains("released = 2025-10-17\n"), "{text}");
    assert!(text.contains("\"odd key\" = 1.5\n"), "{text}");
    assert!(
        text.contains("tags = [\n  # first\n  \"a\",\n  # second\n  \"b\",\n  # after\n]\n"),
        "{text}"
    );
    assert!(
        text.contains("\n# The second binary\n[[bin]]\nname = \"two\"\ntest = false\n"),
        "{text}"
    );
    let again = read(&text);
    assert_eq!(
        crate::json::to_string(&again, false).unwrap(),
        crate::json::to_string(&node, false).unwrap()
    );
    assert_eq!(
        at(&again, "package.name").comment(),
        Some("Its name\nshort")
    );
}

#[test]
fn objects_in_plain_arrays_are_inline_tables_and_nothing_is_left_out() {
    let node = crate::json::parse(
        r#"{"points": [{"x": 1, "y": 2}, {}], "mixed": [1, {"a": "b"}], "empty": {}, "gone": null, "deep": {"a": {"b": {"c": 1}}}}"#,
    )
    .unwrap()
    .node;
    let text = to_string(&node).unwrap();
    assert!(text.contains("mixed = [1, { a = \"b\" }]"), "{text}");
    assert!(text.contains("[deep.a.b]\nc = 1"), "{text}");
    assert!(!text.contains("gone"), "{text}");
    let again = read(&text);
    assert_eq!(
        crate::json::to_string(&again, false).unwrap(),
        r#"{"mixed":[1,{"a":"b"}],"points":[{"x":1,"y":2},{}],"empty":{},"deep":{"a":{"b":{"c":1}}}}"#
    );
    let list = crate::json::parse("[1]").unwrap().node;
    assert!(to_string(&list).unwrap_err().message.contains("object"));
}
