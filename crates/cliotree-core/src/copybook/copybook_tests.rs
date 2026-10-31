use super::*;
use crate::typespec::Kind;

pub(super) const CUSTOMER: &str = "\
000100* The customer master record
000200 01  CUSTOMER-REC.
000300     05  CUST-ID            PIC 9(6).
000400* Name as printed
000500     05  CUST-NAME          PIC X(20).
000600     05  BALANCE            PIC S9(7)V99 COMP-3.
000700     05  VISITS             PIC 9(4) COMP.
000800     05  STATUS             PIC X.
000900         88  ACTIVE         VALUE 'A'.
001000     05  FILLER             PIC X(3).
001100     05  PHONE-COUNT        PIC 9.
001200     05  PHONES OCCURS 0 TO 3 TIMES DEPENDING ON PHONE-COUNT.
001300         10  PHONE          PIC X(10).
001400     05  ADDRESS-AREA.
001500         10  STREET         PIC X(12).
001600     05  ADDRESS-CODES REDEFINES ADDRESS-AREA.
001700         10  POSTCODE       PIC 9(6).
001800         10  FILLER         PIC X(6).
";

#[test]
fn a_copybook_is_read_in_fixed_form_with_its_comments() {
    let copybook = parse(CUSTOMER).unwrap();
    assert_eq!(copybook.items.len(), 1);
    let record = &copybook.items[0];
    assert_eq!(record.name, "CUSTOMER-REC");
    assert_eq!(
        record.comment.as_deref(),
        Some("The customer master record")
    );
    let names: Vec<&str> = record.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "CUST-ID",
            "CUST-NAME",
            "BALANCE",
            "VISITS",
            "STATUS",
            "FILLER",
            "PHONE-COUNT",
            "PHONES",
            "ADDRESS-AREA",
            "ADDRESS-CODES"
        ]
    );
    assert_eq!(record.items[1].comment.as_deref(), Some("Name as printed"));
    assert_eq!(record.items[2].usage, Some(Usage::Comp3));
    assert_eq!(record.items[2].picture.as_deref(), Some("S9(7)V99"));
    assert_eq!(
        record.items[4].comment.as_deref(),
        Some("88 ACTIVE VALUE 'A'.")
    );
    let phones = record.items[7].occurs.as_ref().unwrap();
    assert_eq!(
        (phones.min, phones.max, phones.depending_on.as_deref()),
        (Some(0), 3, Some("PHONE-COUNT"))
    );
    assert_eq!(record.items[9].redefines.as_deref(), Some("ADDRESS-AREA"));
}

#[test]
fn free_form_and_values_are_read() {
    let copybook =
        parse("01 REC.\n  05 A PIC X(5) VALUE 'it''s'.\n  05 B PIC S9(3)V9 VALUE -1.5.\n").unwrap();
    let items = &copybook.items[0].items;
    assert_eq!(items[0].value.as_deref(), Some("'it''s'"));
    assert_eq!(items[1].value.as_deref(), Some("-1.5"));
}

#[test]
fn a_copybook_is_written_back_and_reads_the_same() {
    let copybook = parse(CUSTOMER).unwrap();
    let text = to_text(&copybook);
    assert!(text.lines().all(|l| l.chars().count() <= 72), "{text}");
    assert!(text.contains("       01  CUSTOMER-REC.\n"), "{text}");
    assert!(text.contains("      * Name as printed\n"), "{text}");
    assert_eq!(parse(&text).unwrap(), copybook, "{text}");
}

#[test]
fn the_tree_of_a_schema_tab_holds_the_copybook() {
    let copybook = parse(CUSTOMER).unwrap();
    let node = to_node(&copybook, CodePage::Cp1140);
    let (back, page) = from_node(&node).unwrap();
    assert_eq!(back, copybook);
    assert_eq!(page, CodePage::Cp1140);
}

#[test]
fn a_copybook_lowers_to_one_record() {
    let spec = lower(&parse(CUSTOMER).unwrap(), CodePage::Cp037).unwrap();
    let layout = spec.copybook().unwrap();
    // 6 + 20 + 5 + 2 + 1 + 3 + 1 + 30 + 12
    assert_eq!(layout.length, 80);
    assert_eq!(layout.redefines.len(), 1);
    // The root is the record itself, not a list of them.
    let Kind::Object { fields } = &spec.get(spec.root()).kind else {
        panic!("not an object");
    };
    let keys: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        keys,
        [
            "CUST_ID",
            "CUST_NAME",
            "BALANCE",
            "VISITS",
            "STATUS",
            "PHONE_COUNT",
            "PHONES",
            "ADDRESS_AREA",
            "ADDRESS_CODES"
        ]
    );
    assert!(!fields[7].required && !fields[8].required);
    assert!(matches!(spec.get(fields[2].ty).kind, Kind::Number(_)));
}

#[test]
fn copybooks_that_cannot_be_used_say_why() {
    let bad = |text: &str| lower(&parse(text).unwrap(), CodePage::Cp037).unwrap_err();
    assert!(bad("01 R.\n 05 A PIC X.\n 05 B REDEFINES C PIC X.\n").contains("redefines C"));
    assert!(bad("01 R.\n 05 A PIC 9(19).\n").contains("18 digits"));
    assert!(bad("01 R.\n 05 A PIC X COMP-3.\n").contains("characters"));
    assert!(bad("01 R.\n 05 T OCCURS 2 DEPENDING ON N PIC X.\n").contains("not in the record"));
    assert!(bad("01 R.\n 05 A-B PIC X.\n 05 A_B PIC X.\n").contains("two items"));
    assert!(parse("01 R.\n 05 A PIC X SYNC.\n").is_err());
    assert!(parse("01 R.\n 05 A COMP-1.\n").is_err());
    assert!(bad("      * only a comment\n").contains("no record"));
    assert!(parse("01 R.\n 05 A PIC X\n").is_err(), "no period");
}

#[test]
fn code_pages_turn_text_into_bytes_and_back() {
    assert_eq!(CodePage::Cp037.encode("A1 ").unwrap(), [0xC1, 0xF1, 0x40]);
    assert_eq!(CodePage::Cp1140.decode(&[0x9F]), "€");
    assert_eq!(CodePage::Cp037.encode("€"), Err('€'));
    assert_eq!(CodePage::Cp285.decode(&[0x5B]), "£");
    for page in CodePage::ALL {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(page.encode(&page.decode(&all)).unwrap(), all, "{page:?}");
        assert_eq!(CodePage::from_code(page.code()), Some(page));
    }
}

#[test]
fn the_tree_of_a_copybook_conforms_to_its_meta_schema() {
    let node = to_node(&parse(CUSTOMER).unwrap(), CodePage::Cp037);
    let issues = crate::validate::validate(crate::metaschema::copybook_meta(), &node);
    assert!(issues.is_empty(), "{issues:?}");
    let spec = lower_node(&node).unwrap();
    assert_eq!(spec.copybook().unwrap().length, 80);
}

/// IBM MQ's message descriptor, as its copybook CMQMDV has it: its record at
/// level 10, since it is copied in under an 01 of the program, and comment
/// lines of two stars (owner's report, 2026-11-01).
const MQMD: &str = "\
**   MQMD structure\n\
  10 MQMD.\n\
**    Structure identifier\n\
   15 MQMD-STRUCID          PIC X(4).\n\
**    Structure version number\n\
   15 MQMD-VERSION          PIC S9(9) BINARY.\n\
**    Options for report messages\n\
   15 MQMD-REPORT           PIC S9(9) BINARY.\n\
**    Message type\n\
   15 MQMD-MSGTYPE          PIC S9(9) BINARY.\n\
**    Message lifetime\n\
   15 MQMD-EXPIRY           PIC S9(9) BINARY.\n\
**    Feedback or reason code\n\
   15 MQMD-FEEDBACK         PIC S9(9) BINARY.\n\
**    Numeric encoding of message data\n\
   15 MQMD-ENCODING         PIC S9(9) BINARY.\n\
**    Character set identifier of message data\n\
   15 MQMD-CODEDCHARSETID   PIC S9(9) BINARY.\n\
**    Format name of message data\n\
   15 MQMD-FORMAT           PIC X(8).\n\
**    Message priority\n\
   15 MQMD-PRIORITY         PIC S9(9) BINARY.\n\
**    Message persistence\n\
   15 MQMD-PERSISTENCE      PIC S9(9) BINARY.\n\
**    Message identifier\n\
   15 MQMD-MSGID            PIC X(24).\n\
**    Correlation identifier\n\
   15 MQMD-CORRELID         PIC X(24).\n\
**    Backout counter\n\
   15 MQMD-BACKOUTCOUNT     PIC S9(9) BINARY.\n\
**    Name of reply queue\n\
   15 MQMD-REPLYTOQ         PIC X(48).\n\
**    Name of reply queue manager\n\
   15 MQMD-REPLYTOQMGR      PIC X(48).\n\
**    User identifier\n\
   15 MQMD-USERIDENTIFIER   PIC X(12).\n\
**    Accounting token\n\
   15 MQMD-ACCOUNTINGTOKEN  PIC X(32).\n\
**    Application data relating to identity\n\
   15 MQMD-APPLIDENTITYDATA PIC X(32).\n\
**    Type of application that put the message\n\
   15 MQMD-PUTAPPLTYPE      PIC S9(9) BINARY.\n\
**    Name of application that put the message\n\
   15 MQMD-PUTAPPLNAME      PIC X(28).\n\
**    Date when message was put\n\
   15 MQMD-PUTDATE          PIC X(8).\n\
**    Time when message was put\n\
   15 MQMD-PUTTIME          PIC X(8).\n\
**    Application data relating to origin\n\
   15 MQMD-APPLORIGINDATA   PIC X(4).\n\
**    Group identifier\n\
   15 MQMD-GROUPID          PIC X(24).\n\
**    Sequence number of logical message within group\n\
   15 MQMD-MSGSEQNUMBER     PIC S9(9) BINARY.\n\
**    Offset of data in physical message from start of logical message\n\
   15 MQMD-OFFSET           PIC S9(9) BINARY.\n\
**    Message flags\n\
   15 MQMD-MSGFLAGS         PIC S9(9) BINARY.\n\
**    Length of original message\n\
   15 MQMD-ORIGINALLENGTH   PIC S9(9) BINARY.\n";

#[test]
fn a_copybook_copied_in_under_an_01_has_its_records_at_its_own_level() {
    let book = parse(MQMD).unwrap();
    assert_eq!(record_names(&book), ["MQMD"]);
    assert_eq!(book.items[0].level, 10);
    assert_eq!(book.items[0].comment.as_deref(), Some("MQMD structure"));
    assert_eq!(
        book.items[0].items[0].comment.as_deref(),
        Some("Structure identifier")
    );
    let spec = lower(&book, CodePage::Cp037).unwrap();
    // MQMD version 2 is 364 bytes.
    assert_eq!(spec.copybook().unwrap().length, 364);
    // Written and read back, as text and as the schema tab's tree.
    assert_eq!(parse(&to_text(&book)).unwrap(), book);
    let (back, _) = from_node(&to_node(&book, CodePage::Cp037)).unwrap();
    assert_eq!(back, book);
}
