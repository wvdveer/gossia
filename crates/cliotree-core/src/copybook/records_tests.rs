use super::*;
use crate::copybook::copybook_tests::CUSTOMER;
use crate::copybook::{CodePage, lower, parse};
use crate::typespec::TypeSpec;

fn spec() -> TypeSpec {
    lower(&parse(CUSTOMER).unwrap(), CodePage::Cp037).unwrap()
}

/// One customer record, written by hand.
fn record() -> Vec<u8> {
    let page = CodePage::Cp037;
    let mut r = Vec::new();
    r.extend(page.encode("004711").unwrap());
    r.extend(page.encode(&format!("{:<20}", "Ann Smith")).unwrap());
    r.extend([0x00, 0x12, 0x34, 0x56, 0x7D]); // -12345.67
    r.extend([0x00, 0x2A]); // 42
    r.extend(page.encode("A").unwrap());
    r.extend(page.encode("   ").unwrap());
    r.extend(page.encode("2").unwrap());
    r.extend(page.encode("0123456789").unwrap());
    r.extend(page.encode("9876543210").unwrap());
    r.extend([0; 10]);
    r.extend(page.encode(&format!("{:<12}", "1 Main St")).unwrap());
    r
}

#[test]
fn records_are_read_by_the_layout() {
    let spec = spec();
    let layout = spec.copybook().unwrap();
    let bytes = record();
    // A file of two records, or of part of one, is refused.
    let two = read(&[record(), record()].concat(), layout, &[]).unwrap_err();
    assert!(
        two.message.contains("a record of CUSTOMER-REC has"),
        "{two}"
    );
    assert!(read(&bytes[1..], layout, &[]).is_err());
    let parsed = read(&bytes, layout, &[]).unwrap();
    let node = parsed.node;
    let at = |p: &str| node.get(&p.parse().unwrap()).unwrap().value().clone();
    assert_eq!(at("CUST_ID"), Value::Int(4711));
    assert_eq!(at("CUST_NAME"), Value::Str(format!("{:<20}", "Ann Smith")));
    assert_eq!(at("BALANCE"), Value::Float(-12345.67));
    assert_eq!(at("VISITS"), Value::Int(42));
    assert_eq!(at("PHONES").as_array().map(<[Node]>::len), Some(2));
    assert_eq!(at("PHONES[1].PHONE"), Value::Str("9876543210".into()));
    assert_eq!(at("ADDRESS_AREA.STREET"), Value::Str("1 Main St   ".into()));
    assert!(node.get(&"ADDRESS_CODES".parse().unwrap()).is_none());
    assert!(parsed.warnings.is_empty());
    // The same bytes written again are the same.
    assert_eq!(write(&node, layout).unwrap(), bytes);
}

#[test]
fn the_other_layout_of_a_redefines_reads_the_same_bytes() {
    let spec = spec();
    let layout = spec.copybook().unwrap();
    let mut bytes = record();
    let street = bytes.len() - 12;
    bytes[street..street + 6].copy_from_slice(&CodePage::Cp037.encode("123456").unwrap());
    let node = read_record(&bytes, layout, &[1]);
    assert_eq!(
        node.get(&"ADDRESS_CODES.POSTCODE".parse().unwrap())
            .unwrap()
            .value(),
        &Value::Int(123_456)
    );
    assert!(node.get(&"ADDRESS_AREA".parse().unwrap()).is_none());
    let mut written = write_record(&node, layout, &Path::root()).unwrap();
    // The FILLER after the postcode comes back as spaces.
    written[street + 6..].copy_from_slice(&bytes[street + 6..]);
    assert_eq!(written, bytes);
}

#[test]
fn bad_bytes_are_kept_as_an_error_and_written_back() {
    let spec = spec();
    let layout = spec.copybook().unwrap();
    let mut bytes = record();
    bytes[..6].copy_from_slice(&[0x40; 6]);
    let node = read_record(&bytes, layout, &[]);
    let Value::Error(e) = node.get(&"CUST_ID".parse().unwrap()).unwrap().value() else {
        panic!("not an error");
    };
    assert!(e.message.contains("X'404040404040'"), "{}", e.message);
    assert_eq!(write_record(&node, layout, &Path::root()).unwrap(), bytes);
}

#[test]
fn a_table_shorter_than_its_room_sets_its_counter_and_low_values() {
    let spec = spec();
    let layout = spec.copybook().unwrap();
    let mut node = read_record(&record(), layout, &[]);
    if let Value::Object(fields) = node.value_mut()
        && let Value::Array(items) = fields["PHONES"].value_mut()
    {
        items.pop();
    }
    let bytes = write_record(&node, layout, &Path::root()).unwrap();
    let back = read_record(&bytes, layout, &[]);
    assert_eq!(
        back.get(&"PHONE_COUNT".parse().unwrap()).unwrap().value(),
        &Value::Int(1)
    );
    assert_eq!(&bytes[48..68], &[0; 20]);
}

#[test]
fn numbers_of_every_usage_go_both_ways() {
    let text = "01 R.\n 05 Z PIC S9(3).\n 05 L PIC S9(3) SIGN LEADING SEPARATE.\n \
                05 T PIC S9(3) SIGN TRAILING SEPARATE.\n 05 P PIC 9(4) COMP-3.\n \
                05 B PIC S9(9) BINARY.\n 05 N PIC 9(4) COMP-5.\n 05 D PIC 9(3)V9(2).\n";
    let spec = lower(&parse(text).unwrap(), CodePage::Cp037).unwrap();
    let layout = spec.copybook().unwrap();
    assert_eq!(layout.length, 3 + 4 + 4 + 3 + 4 + 2 + 5);
    let record = crate::json::parse(
        r#"{"Z": -12, "L": -5, "T": 7, "P": 1234, "B": -100000, "N": 65535, "D": 123.45}"#,
    )
    .unwrap()
    .node;
    let bytes = write_record(&record, layout, &Path::root()).unwrap();
    assert_eq!(&bytes[..3], &[0xF0, 0xF1, 0xD2]);
    assert_eq!(&bytes[3..7], &[0x60, 0xF0, 0xF0, 0xF5]);
    assert_eq!(&bytes[7..11], &[0xF0, 0xF0, 0xF7, 0x4E]);
    assert_eq!(&bytes[11..14], &[0x01, 0x23, 0x4F]);
    assert_eq!(&bytes[18..20], &[0xFF, 0xFF]);
    let back = read_record(&bytes, layout, &[]);
    assert_eq!(
        crate::json::to_string(&back, false).unwrap(),
        crate::json::to_string(&record, false).unwrap()
    );
}

#[test]
fn values_that_do_not_fit_are_refused_naming_the_field() {
    let spec = lower(
        &parse("01 R.\n 05 A PIC X(2).\n 05 N PIC 9(2).\n").unwrap(),
        CodePage::Cp037,
    )
    .unwrap();
    let layout = spec.copybook().unwrap();
    let write = |json: &str| {
        write_record(
            &crate::json::parse(json).unwrap().node,
            layout,
            &Path::root(),
        )
    };
    assert!(
        write(r#"{"A": "abc", "N": 1}"#)
            .unwrap_err()
            .message
            .contains("A:")
    );
    assert!(
        write(r#"{"A": "€", "N": 1}"#)
            .unwrap_err()
            .message
            .contains("has no")
    );
    assert!(
        write(r#"{"A": "a", "N": 100}"#)
            .unwrap_err()
            .message
            .contains("N:")
    );
    assert!(write(r#"{"A": "a", "N": -1}"#).is_err());
    assert_eq!(
        write(r#"{"N": 7}"#).unwrap(),
        [0x40, 0x40, 0xF0, 0xF7],
        "an absent text is spaces"
    );
    let e = read(&[0x40; 5], layout, &[]).unwrap_err();
    assert!(e.message.contains("a record of R has 4"), "{e}");
}

#[test]
fn a_record_whose_top_item_has_occurs_is_that_table() {
    let text = "10 RATES OCCURS 3.\n 15 CODE PIC X(2).\n 15 RATE PIC 9V99.\n";
    let spec = lower(&parse(text).unwrap(), CodePage::Cp037).unwrap();
    let layout = spec.copybook().unwrap();
    assert_eq!(layout.length, 15);
    assert!(matches!(
        spec.get(spec.root()).kind,
        crate::typespec::Kind::Array { .. }
    ));
    let page = CodePage::Cp037;
    let bytes = page.encode("EU125US099GB110").unwrap();
    let node = read(&bytes, layout, &[]).unwrap().node;
    assert_eq!(node.value().as_array().map(<[Node]>::len), Some(3));
    assert_eq!(
        node.get(&"[1].RATE".parse().unwrap()).unwrap().value(),
        &Value::Float(0.99)
    );
    assert_eq!(write(&node, layout).unwrap(), bytes);
    // A table of plain values is a list of them.
    let spec = lower(&parse("05 CODES PIC X(2) OCCURS 2.\n").unwrap(), page).unwrap();
    let node = read(&page.encode("ABCD").unwrap(), spec.copybook().unwrap(), &[])
        .unwrap()
        .node;
    assert_eq!(
        node.get(&"[1]".parse().unwrap()).unwrap().value(),
        &Value::Str("CD".into())
    );
    // A table whose counter is outside the record cannot be read.
    let text = "10 T OCCURS 1 TO 3 DEPENDING ON N.\n 15 A PIC X.\n";
    assert!(lower(&parse(text).unwrap(), page).is_err());
}
