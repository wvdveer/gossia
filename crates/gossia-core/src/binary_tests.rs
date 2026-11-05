use super::*;

fn bytes() -> Vec<u8> {
    let mut b = CodePage::Cp037.encode("HELLO, WORLD 123456").unwrap();
    b.extend([0x00, 0x12, 0x3C]);
    b
}

#[test]
fn each_kind_reads_back_what_it_writes() {
    let b = bytes();
    for kind in BinaryText::ALL {
        let text = kind.write(&b, 11, CodePage::Cp037);
        assert_eq!(kind.read(&text), Some(b.clone()), "{kind:?}: {text}");
        assert_eq!(BinaryText::read_any(&text), Some((kind, b.clone())));
        assert_eq!(BinaryText::from_key(kind.key()), Some(kind));
    }
    assert_eq!(to_hex(&[0xF0, 0x0A]), "F00A");
    assert_eq!(to_base64(&[0xF0, 0x0A]), "8Ao=");
}

#[test]
fn a_dump_shows_sixteen_bytes_a_line_with_their_text() {
    let b = CodePage::Cp037.encode("HELLO, WORLD 123456").unwrap();
    // One record has no heading; several are each headed by their number.
    let text = dump(&b, 19, CodePage::Cp037, usize::MAX);
    assert!(text.starts_with("00000000  "), "{text}");
    let text = dump(&b, 10, CodePage::Cp037, usize::MAX);
    assert!(text.starts_with("Record 0\n00000000  "), "{text}");
    assert!(text.contains("\n\nRecord 1\n0000000A  "), "{text}");
    let text = format!("Record 0\n{}", dump(&b, 19, CodePage::Cp037, usize::MAX));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1],
        "00000000  C8 C5 D3 D3 D6 6B 40 E6  D6 D9 D3 C4 40 F1 F2 F3  HELLO, WORLD 123"
    );
    assert!(lines[2].starts_with("00000010  F4 F5 F6 "), "{text}");
    assert!(lines[2].ends_with(" 456"), "{text}");
    // A control character is a dot.
    assert!(dump(&[0x00, 0xC1], 2, CodePage::Cp037, usize::MAX).ends_with(".A"));
}

#[test]
fn a_dump_cut_short_or_with_other_lines_is_not_read() {
    let b = bytes();
    let short = dump(&b, 11, CodePage::Cp037, 16);
    assert!(short.contains("6 more bytes"), "{short}");
    assert_eq!(BinaryText::Dump.read(&short), None);
    assert_eq!(BinaryText::Dump.read("Dear Ann,\nhello"), None);
    // Hex digits with spaces and line breaks are still hex.
    assert_eq!(from_hex("F0 F1\nc1"), Some(vec![0xF0, 0xF1, 0xC1]));
    assert_eq!(from_hex("F0F"), None);
    assert_eq!(from_base64("8A\no="), Some(vec![0xF0, 0x0A]));
    assert_eq!(from_base64("not base64!"), None);
    // Hex digits are read as hex, though they are Base64 too.
    assert_eq!(
        BinaryText::read_any("F00A"),
        Some((BinaryText::Hex, vec![0xF0, 0x0A]))
    );
}
