//! Tests of comments on the nodes of an XML tab (DESIGN.md, "Comments and
//! structure fidelity", "Comments in the UI").

use std::sync::Arc;

use cliotree_core::{Path, Value, json, jsonschema, xml, xsd};

use crate::fake::FakeEngine;
use crate::{Calc, ExportFormat};

const XSD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="order">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="customer" type="xs:string"/>
        <xs:element name="item" type="xs:integer" maxOccurs="unbounded"/>
        <xs:element name="total" type="xs:integer"/>
      </xs:sequence>
      <xs:attribute name="id" type="xs:integer"/>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

const XML: &str = r#"<order id="7">
  <!-- Who pays -->
  <customer>Ada</customer>
  <item>2</item>
  <item>3</item>
  <total>0</total>
</order>"#;

fn p(text: &str) -> Path {
    if text.is_empty() {
        Path::root()
    } else {
        text.parse().unwrap()
    }
}

fn book() -> (Calc, FakeEngine) {
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    let spec = xsd::lower_str(XSD).unwrap();
    let node = xml::parse(XML, &spec).unwrap().node;
    calc.add_sheet("Order", spec, node).unwrap();
    calc.mark_saved();
    (calc, engine)
}

#[test]
fn a_comment_is_inserted_edited_and_deleted_each_as_one_undoable_entry() {
    let (mut calc, _) = book();
    assert_eq!(calc.comment("Order", &p("customer")), Some(" Who pays "));
    let revision = calc.revision();

    calc.set_comment("Order", &p("item[1]"), Some("Three\nof them"))
        .unwrap();
    assert_eq!(calc.comment("Order", &p("item[1]")), Some("Three\nof them"));
    assert!(calc.revision() > revision, "the window is drawn again");
    assert!(calc.is_modified());
    assert_eq!(calc.undo_label(), Some("Insert comment on Order.item[1]"));

    calc.set_comment("Order", &p("item[1]"), Some("Three"))
        .unwrap();
    assert_eq!(calc.undo_label(), Some("Edit comment on Order.item[1]"));
    calc.set_comment("Order", &p("customer"), None).unwrap();
    assert_eq!(calc.undo_label(), Some("Delete comment on Order.customer"));
    assert_eq!(calc.comment("Order", &p("customer")), None);
    // Only white space is no comment.
    calc.set_comment("Order", &p("item[1]"), Some("  "))
        .unwrap();
    assert_eq!(calc.comment("Order", &p("item[1]")), None);

    calc.undo().unwrap();
    assert_eq!(calc.comment("Order", &p("item[1]")), Some("Three"));
    calc.undo().unwrap();
    assert_eq!(calc.comment("Order", &p("customer")), Some(" Who pays "));
    calc.undo().unwrap();
    calc.undo().unwrap();
    assert_eq!(calc.comment("Order", &p("item[1]")), None);
    assert!(!calc.is_modified());
    calc.redo().unwrap();
    assert_eq!(calc.comment("Order", &p("item[1]")), Some("Three\nof them"));

    // The same text again is no edit.
    let label = calc.undo_label().map(str::to_owned);
    calc.set_comment("Order", &p("item[1]"), Some("Three\nof them"))
        .unwrap();
    assert_eq!(calc.undo_label().map(str::to_owned), label);
}

#[test]
fn what_cannot_have_a_comment_is_refused_with_the_reason() {
    let (mut calc, _) = book();
    let refused = |calc: &Calc, path: &str| calc.comment_refusal("Order", &p(path)).unwrap();
    assert!(refused(&calc, "[\"@id\"]").contains("attribute"));
    assert!(refused(&calc, "item").contains("each of its items"));
    assert!(refused(&calc, "item[5]").contains("not there"));
    assert_eq!(calc.comment_refusal("Order", &p("")), None, "the root can");
    assert_eq!(calc.comment_refusal("Order", &p("item[0]")), None);

    let error = calc
        .set_comment("Order", &p("total"), Some("a -- b"))
        .unwrap_err();
    assert!(error.to_string().contains("two hyphens"), "{error}");
    let error = calc
        .set_comment("Order", &p("total"), Some("ends -"))
        .unwrap_err();
    assert!(error.to_string().contains("end with a hyphen"), "{error}");
    assert!(
        calc.set_comment("Order", &p("[\"@id\"]"), Some("x"))
            .is_err()
    );
    assert!(!calc.is_modified());

    // A JSON tab has no comments.
    let spec =
        jsonschema::lower_str(r#"{"type": "object", "properties": {"a": {"type": "integer"}}}"#)
            .unwrap();
    calc.add_sheet("J", spec, json::parse(r#"{"a": 1}"#).unwrap().node)
        .unwrap();
    assert!(
        calc.comment_refusal("J", &p("a"))
            .unwrap()
            .contains("only on XML tabs")
    );
}

#[test]
fn a_comment_stays_through_a_formula_and_is_written_as_xml() {
    let (mut calc, engine) = book();
    calc.set_comment("Order", &p("total"), Some("Sum of the items"))
        .unwrap();
    calc.set_formula("Order", &p("total"), "Order.item[0] + Order.item[1]")
        .unwrap();
    calc.recalc(&engine);
    assert_eq!(
        calc.data("Order")
            .unwrap()
            .get(&p("total"))
            .unwrap()
            .value(),
        &Value::Int(5)
    );
    assert_eq!(
        calc.comment("Order", &p("total")),
        Some("Sum of the items"),
        "a result keeps the comment"
    );
    // Changing a comment recalculates nothing.
    let runs = engine.evaluations();
    calc.set_comment("Order", &p("item[0]"), Some("First"))
        .unwrap();
    assert_eq!(calc.pending(), 0);
    calc.recalc(&engine);
    assert_eq!(engine.evaluations(), runs);
    // Undoing the comment on the formula's node leaves its result alone.
    calc.set_comment("Order", &p("total"), Some("Total"))
        .unwrap();
    calc.undo().unwrap();
    assert_eq!(calc.comment("Order", &p("total")), Some("Sum of the items"));
    assert_eq!(
        calc.data("Order")
            .unwrap()
            .get(&p("total"))
            .unwrap()
            .value(),
        &Value::Int(5)
    );

    let text = calc.export_text("Order", ExportFormat::Xml, true).unwrap();
    assert!(text.contains("<!--First-->\n  <item>2</item>"), "{text}");
    assert!(
        text.contains("<!--Sum of the items-->\n  <total>5</total>"),
        "{text}"
    );
}

/// Comments inside an element (at its start, before its text, and at its
/// end) are read, set in one entry with the one before it, undone, and
/// written back in their places.
#[test]
fn the_comments_inside_an_element_are_kept_and_edited() {
    use cliotree_core::CommentPlace::{Before, End, Start};
    let engine = FakeEngine::new();
    let mut calc = Calc::new(Arc::new(engine.clone()));
    let spec = xsd::lower_str(XSD).unwrap();
    let text = "<order><customer><!--first-->Ada<!--then--></customer>\
                <item>2</item><total>0</total><!--last--></order>";
    let node = xml::parse(text, &spec).unwrap().node;
    calc.add_sheet("Order", spec, node).unwrap();
    let customer = p("customer");
    assert_eq!(calc.comment_at("Order", &customer, Start), Some("first"));
    assert_eq!(calc.comment_at("Order", &customer, End), Some("then"));
    assert_eq!(calc.comment_at("Order", &p(""), End), Some("last"));
    assert!(calc.has_comments("Order", &customer));
    assert!(!calc.has_comments("Order", &p("total")));

    calc.set_comments(
        "Order",
        &customer,
        &[
            (Before, Some("who")),
            (Start, None),
            (End, Some("then, again")),
        ],
    )
    .unwrap();
    assert_eq!(calc.undo_label(), Some("Edit comment on Order.customer"));
    assert_eq!(calc.comment("Order", &customer), Some("who"));
    assert_eq!(calc.comment_at("Order", &customer, Start), None);
    // Refused text changes nothing.
    assert!(
        calc.set_comments(
            "Order",
            &customer,
            &[(Before, Some("x")), (End, Some("a--b"))]
        )
        .is_err()
    );
    assert_eq!(calc.comment("Order", &customer), Some("who"));

    let written = calc.export_text("Order", ExportFormat::Xml, false).unwrap();
    assert!(
        written.contains("<!--who--><customer>Ada<!--then, again--></customer>"),
        "{written}"
    );
    assert!(written.contains("<!--last--></order>"), "{written}");

    calc.undo().unwrap();
    assert_eq!(calc.comment("Order", &customer), None);
    assert_eq!(calc.comment_at("Order", &customer, Start), Some("first"));
    assert_eq!(calc.comment_at("Order", &customer, End), Some("then"));
}
