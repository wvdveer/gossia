use super::*;

fn read(text: &str) -> Node {
    parse(text).unwrap().node
}

fn field<'a>(node: &'a Node, key: &str) -> &'a Node {
    &node.value().as_object().unwrap()[key]
}

#[test]
fn json5_reads_what_it_adds_to_json() {
    let node = read(
        "{unquoted: 'single \\'quoted\\'', hex: 0xFF, half: .5, whole: 5., plus: +3,\n\
         neg: -0x10, $dollar_1: true, \"quoted\": null, multi: 'one \\\n two', trailing: [1, 2,],}",
    );
    assert_eq!(
        field(&node, "unquoted").value(),
        &Value::Str("single 'quoted'".into())
    );
    assert_eq!(field(&node, "hex").value(), &Value::Int(255));
    assert_eq!(field(&node, "half").value(), &Value::Float(0.5));
    assert_eq!(field(&node, "whole").value(), &Value::Float(5.0));
    assert_eq!(field(&node, "plus").value(), &Value::Int(3));
    assert_eq!(field(&node, "neg").value(), &Value::Int(-16));
    assert_eq!(field(&node, "$dollar_1").value(), &Value::Bool(true));
    assert_eq!(field(&node, "quoted").value(), &Value::Null);
    assert_eq!(
        field(&node, "multi").value(),
        &Value::Str("one  two".into())
    );
    assert_eq!(
        field(&node, "trailing").value().as_array().unwrap().len(),
        2
    );
}

#[test]
fn infinity_nan_and_mistakes_are_refused_with_their_place() {
    let e = parse("{a: Infinity}").unwrap_err();
    assert!(e.message.contains("finite"), "{e}");
    let e = parse("[1,\n -NaN]").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("finite"), "{e}");
    let e = parse("{a: 1\n b: 2}").unwrap_err();
    assert_eq!((e.line, e.message.contains("`,`")), (2, true), "{e}");
    assert!(parse("[007]").unwrap_err().message.contains("leading zero"));
    assert!(parse("/* never ended").unwrap_err().message.contains("*/"));
    assert!(parse("1 2").is_err());
    assert!(parse("'a\nb'").is_err());
}

#[test]
fn comments_become_the_comments_of_the_nodes_they_stand_above() {
    let text = "// The settings\n\
                {\n\
                  // Who wrote it\n\
                  // and when\n\
                  \"author\": \"Ann\", // the owner\n\
                  /* A boxed\n   * comment */\n\
                  \"items\": [\n\
                    1, // first\n\
                    // second\n\
                    2\n\
                    // after the last item\n\
                  ],\n\
                  \"empty\": { /* nothing yet */ },\n\
                  // after the last key\n\
                }\n\
                // the end";
    let node = read(text);
    assert_eq!(node.comment(), Some("The settings"));
    assert_eq!(
        node.comment_at(CommentPlace::End),
        Some("after the last key\nthe end")
    );
    let author = field(&node, "author");
    assert_eq!(author.comment(), Some("Who wrote it\nand when\nthe owner"));
    let items = field(&node, "items");
    assert_eq!(items.comment(), Some("A boxed\ncomment"));
    let list = items.value().as_array().unwrap();
    assert_eq!(list[0].comment(), Some("first"));
    assert_eq!(list[1].comment(), Some("second"));
    assert_eq!(
        items.comment_at(CommentPlace::End),
        Some("after the last item")
    );
    assert_eq!(
        field(&node, "empty").comment_at(CommentPlace::Start),
        Some("nothing yet")
    );
}

#[test]
fn written_back_the_comments_and_values_read_the_same() {
    let text = "// top\n{\n  // a number\n  count: 3,\n  'odd key': [\n    'x', // trailing\n  ],\n  empty: {\n    // inside\n  },\n}\n";
    let node = read(text);
    let json5 = to_string(&node, Style::Json5, true).unwrap();
    assert_eq!(
        json5,
        "// top\n{\n  // a number\n  count: 3,\n  \"odd key\": [\n    // trailing\n    \"x\"\n  ],\n  empty: {\n    // inside\n  }\n}\n"
    );
    let jsonc = to_string(&node, Style::Jsonc, true).unwrap();
    assert!(jsonc.contains("\"count\": 3"), "{jsonc}");
    for text in [json5, jsonc, to_string(&node, Style::Json5, false).unwrap()] {
        let again = read(&text);
        assert_eq!(
            crate::json::to_string(&again, false).unwrap(),
            crate::json::to_string(&node, false).unwrap(),
            "{text}"
        );
        assert_eq!(field(&again, "count").comment(), Some("a number"), "{text}");
        assert_eq!(again.comment(), Some("top"), "{text}");
    }
}

#[test]
fn a_repeated_key_keeps_the_last_with_a_warning() {
    let parsed = parse("{a: 1, a: 2}").unwrap();
    assert_eq!(field(&parsed.node, "a").value(), &Value::Int(2));
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].message.contains("twice"));
}

#[test]
fn errored_nodes_are_left_out_and_the_root_must_not_be_one() {
    use crate::value::{ErrorKind, ErrorValue};
    let err = || {
        Node::new(Value::Error(ErrorValue {
            kind: ErrorKind::Value,
            message: "x".into(),
            origin: None,
        }))
    };
    let node = Node::new(Value::Array(vec![Node::new(Value::Int(1)), err()]));
    assert_eq!(to_string(&node, Style::Json5, false).unwrap(), "[1]");
    assert!(to_string(&err(), Style::Jsonc, true).is_err());
}
