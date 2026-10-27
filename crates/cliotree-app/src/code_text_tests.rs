//! The code editor's text handling: indentation, the procedure box, line
//! numbers.

use crate::code_text::*;

fn at(text: &str, marker: &str) -> usize {
    text[..text.find(marker).unwrap()].chars().count()
}

#[test]
fn enter_keeps_the_indentation_and_adds_a_level_after_a_colon() {
    let text = "def f(x):";
    let e = newline(text, 9, 9, 4);
    assert_eq!(e.text, "def f(x):\n    ");
    assert_eq!((e.start, e.end), (14, 14));

    let text = "def f(x):\n    y = 1";
    let end = text.chars().count();
    let e = newline(text, end, end, 4);
    assert_eq!(e.text, "def f(x):\n    y = 1\n    ");

    // A colon in a comment or a string does not open a block.
    let text = "    x = 1  # note:";
    assert_eq!(newline(text, 18, 18, 4).text, "    x = 1  # note:\n    ");
    let text = "    s = 'a:'";
    assert_eq!(newline(text, 12, 12, 4).text, "    s = 'a:'\n    ");
    // But a colon before a comment does.
    let text = "if x:  # yes";
    assert_eq!(newline(text, 12, 12, 2).text, "if x:  # yes\n  ");

    // Breaking a line in the middle carries the rest down, and a selection is
    // replaced.
    let text = "    ab";
    assert_eq!(newline(text, 5, 5, 4).text, "    a\n    b");
    assert_eq!(newline(text, 4, 6, 4).text, "    \n    ");
}

#[test]
fn tab_indents_to_the_next_stop_or_every_selected_line() {
    let e = indent("x", 0, 0, 4);
    assert_eq!((e.text.as_str(), e.start), ("    x", 4));
    let e = indent("ab", 1, 1, 4);
    assert_eq!((e.text.as_str(), e.start), ("a   b", 4));

    let text = "a\n\nb\nc";
    // From inside "a" to inside "b": lines a, (empty) and b, not c.
    let e = indent(text, 0, at(text, "b") + 1, 4);
    assert_eq!(e.text, "    a\n\n    b\nc");
    assert_eq!((e.start, e.end), (0, at(&e.text, "b") + 1));

    // A selection that ends at the start of a line leaves that line alone.
    let text = "a\nb\n";
    let e = indent(text, 0, 2, 2);
    assert_eq!(e.text, "  a\nb\n");
}

#[test]
fn shift_tab_takes_a_level_away_from_the_lines_touched() {
    let text = "        x = 1";
    let e = dedent(text, 10, 10, 4);
    assert_eq!((e.text.as_str(), e.start), ("    x = 1", 6));

    let text = "  a\n    b\nc";
    let e = dedent(text, 0, text.chars().count(), 4);
    assert_eq!(e.text, "a\nb\nc");
    assert_eq!((e.start, e.end), (0, e.text.chars().count()));

    // A cursor inside the spaces moves to where they end.
    let e = dedent("    x", 2, 2, 4);
    assert_eq!((e.text.as_str(), e.start, e.end), ("x", 0, 0));
}

#[test]
fn the_procedure_box_lists_top_level_functions_with_their_lines() {
    let source = "import math\n\ndef vat(x):\n    def inner():\n        pass\n    return x\n\nasync def later():\n    pass\nclass C:\n    def method(self):\n        pass\ndef _private(): pass\n";
    assert_eq!(
        functions(source),
        [
            ("vat".to_owned(), 2),
            ("later".to_owned(), 7),
            ("_private".to_owned(), 12)
        ]
    );
}

#[test]
fn lines_and_their_numbers() {
    let text = "a\nbb\nc";
    assert_eq!(line_of(text, 0), 0);
    assert_eq!(line_of(text, 3), 1);
    assert_eq!(line_of(text, 5), 2);
    assert_eq!(start_of_line(text, 1), 2);
    assert_eq!(start_of_line(text, 2), 5);
    assert_eq!(start_of_line(text, 9), 6);
    assert_eq!(line_numbers(""), "1");
    assert_eq!(
        line_numbers(&"x\n".repeat(10)),
        " 1\n 2\n 3\n 4\n 5\n 6\n 7\n 8\n 9\n10\n11"
    );
    // Characters, not bytes.
    assert_eq!(byte_of("é=1", 1), 2);
}
