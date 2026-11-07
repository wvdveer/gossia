use super::*;

#[test]
fn folded_lines_parameters_and_groups_are_read() {
    let text = "BEGIN:VCARD\r\nitem1.EMAIL;TYPE=work,pref;TYPE=voice:a@b.c\r\nNOTE:one long\r\n  line\r\nX-Q;LABEL=\"a:b;c,d\":v\r\nEND:VCARD\r\n";
    let top = parse(text).unwrap();
    assert_eq!(top.len(), 1);
    let card = &top[0];
    assert_eq!(card.name, "VCARD");
    let email = card.get("EMAIL").unwrap();
    assert_eq!(email.value, "a@b.c");
    assert_eq!(email.params_all("TYPE"), ["work", "pref", "voice"]);
    assert_eq!(card.get("NOTE").unwrap().text(), "one long line");
    assert_eq!(card.get("X-Q").unwrap().param("LABEL"), Some("a:b;c,d"));
}

#[test]
fn escapes_and_lists() {
    let p = Property {
        name: "X".into(),
        params: Vec::new(),
        value: r"a\, b,c\;d\nnext\\".into(),
    };
    assert_eq!(p.text(), "a, b,c;d\nnext\\");
    assert_eq!(p.list(','), ["a, b", "c;d\nnext\\"]);
    assert_eq!(escape("a;b,c\\d\ne"), r"a\;b\,c\\d\ne");
}

#[test]
fn vcard_2_1_bare_types_and_quoted_printable() {
    let text = "BEGIN:VCARD\nVERSION:2.1\nTEL;HOME;VOICE:123\nNOTE;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:Caf=C3=A9 =\nau lait\nN;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:M=FCller\nEND:VCARD\n";
    let card = &parse(text).unwrap()[0];
    assert_eq!(
        card.get("TEL").unwrap().params_all("TYPE"),
        ["HOME", "VOICE"]
    );
    assert_eq!(card.get("NOTE").unwrap().text(), "Café au lait");
    assert_eq!(card.get("N").unwrap().text(), "Müller");
}

#[test]
fn nesting_and_mistakes_are_named() {
    let top = parse("BEGIN:A\nBEGIN:B\nX:1\nEND:B\nEND:A\n").unwrap();
    assert_eq!(top[0].children[0].get("X").unwrap().value, "1");
    assert!(parse("BEGIN:A\nEND:B\n").unwrap_err().contains("line 2"));
    assert!(parse("BEGIN:A\n").unwrap_err().contains("never ended"));
    assert!(
        parse("BEGIN:A\nno colon\nEND:A\n")
            .unwrap_err()
            .contains("line 2")
    );
}

#[test]
fn lines_are_folded_at_75_bytes_without_splitting_a_character() {
    let line = format!("SUMMARY:{}", "é".repeat(60));
    let folded = fold(&line);
    for piece in folded.split("\r\n").filter(|p| !p.is_empty()) {
        assert!(piece.len() <= 75, "{piece:?}");
    }
    let back = parse(&format!("BEGIN:X\r\n{folded}END:X\r\n")).unwrap();
    assert_eq!(back[0].get("SUMMARY").unwrap().value, "é".repeat(60));
}
