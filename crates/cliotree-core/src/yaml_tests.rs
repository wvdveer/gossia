use super::*;

fn read(text: &str) -> Node {
    parse(text).unwrap().node
}

fn at<'a>(node: &'a Node, path: &str) -> &'a Node {
    node.get(&path.parse().unwrap()).unwrap()
}

const COMPOSE: &str = r#"# A compose file

# The version
version: 3
services: # what runs
  web:
    image: "nginx:1.27"
    ports:
      # outside
      - 80
      - 443 # secure
    # end of web
  db:
    enabled: true
    ratio: 0.5
    nothing: ~
    also: null
    word: yes
    note: |
      two # lines
      of text
# the end
"#;

#[test]
fn yaml_is_read_with_core_types_and_comments() {
    let node = read(COMPOSE);
    assert_eq!(node.comment(), Some("A compose file"));
    assert_eq!(at(&node, "version").comment(), Some("The version"));
    assert_eq!(at(&node, "version").value(), &Value::Int(3));
    assert_eq!(at(&node, "services").comment(), Some("what runs"));
    assert_eq!(
        at(&node, "services.web.image").value(),
        &Value::Str("nginx:1.27".into())
    );
    assert_eq!(
        at(&node, "services.web.ports[0]").comment(),
        Some("outside")
    );
    assert_eq!(at(&node, "services.web.ports[1]").comment(), Some("secure"));
    assert_eq!(
        at(&node, "services.web").comment_at(CommentPlace::End),
        Some("end of web")
    );
    assert_eq!(at(&node, "services.db.enabled").value(), &Value::Bool(true));
    assert_eq!(at(&node, "services.db.ratio").value(), &Value::Float(0.5));
    assert_eq!(at(&node, "services.db.nothing").value(), &Value::Null);
    assert_eq!(at(&node, "services.db.also").value(), &Value::Null);
    // YAML 1.2: yes is only a word.
    assert_eq!(
        at(&node, "services.db.word").value(),
        &Value::Str("yes".into())
    );
    assert_eq!(
        at(&node, "services.db.note").value(),
        &Value::Str("two # lines\nof text\n".into())
    );
    assert_eq!(node.comment_at(CommentPlace::End), Some("the end"));
}

#[test]
fn yaml_written_reads_back_the_same() {
    let node = read(COMPOSE);
    let text = to_string(&node).unwrap();
    let again = read(&text);
    assert_eq!(again, node, "{text}");
    assert_eq!(to_string(&again).unwrap(), text);
}

#[test]
fn yaml_is_written_in_block_style() {
    let node = read("a: 1\nb: [x, {c: 2, d: [3]}]\ne: {}\nf: ''\ng: '10'\nh: \"x\\ny\"\n");
    assert_eq!(
        to_string(&node).unwrap(),
        "a: 1\nb:\n  - x\n  - c: 2\n    d:\n      - 3\ne: {}\nf: \"\"\ng: \"10\"\nh: |-\n  x\n  y\n"
    );
}

#[test]
fn text_that_reads_as_something_else_is_quoted() {
    for s in [
        "true", "No", "null", "~", "12", "-3", ".5", "1e3", "a: b", "#x", "- x", "x ", "key:",
        "*a", "",
    ] {
        let node = Node::new(Value::Object(
            [("k".to_owned(), Node::new(Value::Str(s.into())))].into(),
        ));
        let text = to_string(&node).unwrap();
        assert_eq!(
            at(&read(&text), "k").value(),
            &Value::Str(s.into()),
            "{s:?} as {text}"
        );
    }
}

#[test]
fn anchors_and_merge_keys_are_copied_out() {
    let parsed = parse(
        "base: &b\n  size: 1\n  colour: red\nitem:\n  <<: *b\n  colour: blue\nlist: &l [1, 2]\nagain: *l\n",
    )
    .unwrap();
    let node = parsed.node;
    assert_eq!(at(&node, "item.size").value(), &Value::Int(1));
    assert_eq!(at(&node, "item.colour").value(), &Value::Str("blue".into()));
    assert_eq!(at(&node, "again[1]").value(), &Value::Int(2));
    assert!(parsed.warnings.len() >= 2, "{:?}", parsed.warnings);
}

#[test]
fn core_tags_are_read_and_others_dropped() {
    let node = read("a: !!str 12\nb: !!float 3\nc: ! 7\n");
    assert_eq!(at(&node, "a").value(), &Value::Str("12".into()));
    assert_eq!(at(&node, "b").value(), &Value::Float(3.0));
    assert_eq!(at(&node, "c").value(), &Value::Str("7".into()));
    // Any other tag is dropped with a warning, and the value read without
    // it (owner, 2026-11-01).
    let parsed = parse(
        "a: 1\nb: !thing x\nc: !Ref 12\nd: !GetAtt [Web, Arn]\ne: !Sub\n  x: 1\nf: !!binary aGk=\n",
    )
    .unwrap();
    let node = &parsed.node;
    assert_eq!(at(node, "b").value(), &Value::Str("x".into()));
    assert_eq!(at(node, "c").value(), &Value::Int(12));
    assert_eq!(at(node, "d[1]").value(), &Value::Str("Arn".into()));
    assert_eq!(at(node, "e.x").value(), &Value::Int(1));
    assert_eq!(at(node, "f").value(), &Value::Str("aGk=".into()));
    let messages: Vec<String> = parsed
        .warnings
        .iter()
        .map(|w| format!("{}: {}", w.path, w.message))
        .collect();
    assert_eq!(messages.len(), 5, "{messages:?}");
    assert!(
        messages[0].starts_with("b: the tag !thing on line 2"),
        "{messages:?}"
    );
    assert!(messages[4].contains("!!binary"), "{messages:?}");
    assert!(!to_string(node).unwrap().contains('!'));
    assert!(parse("a: .inf\n").is_err());
}

#[test]
fn empty_lines_between_entries_are_kept() {
    let text = "\
a: 1

# about b
b: 2
c:
  - x

  - z
d:

  e: 3
  f: 4
";
    let node = read(text);
    assert!(!at(&node, "a").blank_before());
    assert!(at(&node, "b").blank_before());
    assert!(!at(&node, "c").blank_before());
    assert!(at(&node, "c[1]").blank_before());
    // Not before the first entry of a table or list.
    assert!(!at(&node, "d.e").blank_before());
    let written = to_string(&node).unwrap();
    assert_eq!(
        written,
        "a: 1\n\n# about b\nb: 2\nc:\n  - x\n\n  - z\nd:\n  e: 3\n  f: 4\n"
    );
    assert_eq!(read(&written), node);
}

#[test]
fn several_documents_are_a_list() {
    let text = "# one\na: 1\n---\nb: 2\n";
    assert_eq!(document_count(text), 2);
    assert_eq!(document_count("a: 1\n"), 1);
    let node = parse_documents(text).unwrap().node;
    assert_eq!(at(&node, "[1].b").value(), &Value::Int(2));
    let written = documents_to_string(&node).unwrap();
    assert_eq!(written, "---\n# one\na: 1\n---\nb: 2\n");
    assert_eq!(parse_documents(&written).unwrap().node, node);
    assert_eq!(
        parse_documents("").unwrap().node.value(),
        &Value::Array(Vec::new())
    );
}

#[test]
fn bad_yaml_says_where() {
    let error = parse("a: 1\n b: 2\n").unwrap_err();
    assert_eq!(error.line, 2, "{error:?}");
    assert!(parse("a: 1\na: 2\n").is_err(), "a key twice");
    assert!(parse("[a]: 1\n").is_err(), "a key that is a list");
}

#[test]
fn a_comment_above_the_first_entry_of_an_item_keeps_its_place() {
    let mut item = indexmap::IndexMap::new();
    let mut first = Node::new(Value::Int(1));
    first.set_comment(Some("first".into()));
    item.insert("k".to_owned(), first);
    item.insert("j".to_owned(), Node::new(Value::Int(2)));
    let node = Node::new(Value::Array(vec![Node::new(Value::Object(item))]));
    let text = to_string(&node).unwrap();
    assert_eq!(text, "-\n  # first\n  k: 1\n  j: 2\n");
    assert_eq!(read(&text), node);
}

proptest::proptest! {
    #[test]
    fn any_text_reads_back_as_written(s in r"\PC{0,12}|[ -~\n\t]{0,12}") {
        let node = Node::new(Value::Object(
            [(s.clone(), Node::new(Value::Str(s.clone())))].into(),
        ));
        let text = to_string(&node).unwrap();
        let back = parse(&text).map_err(|e| proptest::test_runner::TestCaseError::fail(format!("{e:?} in {text:?}")))?;
        proptest::prop_assert_eq!(back.node, node, "{:?}", text);
    }
}
