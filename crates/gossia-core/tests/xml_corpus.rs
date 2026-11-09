//! The corpus of common XML documents (DESIGN.md, XML Schemas and
//! namespaces, 0.5.0, "The corpus"). Every document must read and write back
//! as the same tree. A document whose build step is done must also import:
//! an XSD is inferred, the document is read with it, is valid, and is written
//! back as the same text.
//!
//! The documents in `xml-corpus/` are made up, in the shape of the real ones.

use std::path::PathBuf;

use gossia_core::validate::validate;
use gossia_core::xmltree::{self, Child, Element};
use gossia_core::{infer, xml, xsd};

/// The build step that is done. A document of a later step only has to read
/// and write back as a tree.
const DONE: &str = "15c";

/// Each document, and the step after which it imports.
const CORPUS: &[(&str, &str)] = &[
    ("sitemap.xml", "15a"),
    ("atom.xml", "15a"),
    ("kml.xml", "15a"),
    ("project.csproj", "15a"),
    ("docbook.xml", "15a"),
    // `xml:` and `xsi:` attributes beside one namespace.
    ("page.xhtml", "15b"),
    ("pom.xml", "15b"),
    ("track.gpx", "15b"),
    ("model.archimate", "15b"),
    // Children in a varying order.
    ("settings.plist", "15c"),
    ("document.xml", "15c"),
    // Several namespaces, one XSD tab each.
    ("feed.rss", "15d"),
    ("AndroidManifest.xml", "15d"),
    ("envelope.soap.xml", "15d"),
    ("MainWindow.xaml", "15d"),
    // An `xlink:` attribute.
    ("drawing.svg", "15d"),
];

fn text_of(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "xml-corpus", name]
        .iter()
        .collect();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The element without the whitespace that only lays it out: text that is
/// all whitespace, in an element that holds no other text.
fn strip(element: &Element) -> Element {
    let mut e = element.clone();
    let mixed = e.has_text();
    e.children = e
        .children
        .iter()
        .filter(|c| mixed || !matches!(c, Child::Text(t) if t.trim().is_empty()))
        .map(|c| match c {
            Child::Element(inner) => Child::Element(strip(inner)),
            other => other.clone(),
        })
        .collect();
    e
}

fn same_document(name: &str, original: &str, written: &str) {
    let a = xmltree::parse(original).unwrap();
    let b = xmltree::parse(written).unwrap_or_else(|e| panic!("{name}: {e}\n{written}"));
    assert_eq!(a.declaration, b.declaration, "{name}");
    assert_eq!(a.before, b.before, "{name}");
    assert_eq!(a.after, b.after, "{name}");
    assert_eq!(strip(&a.root), strip(&b.root), "{name}\n{written}");
}

#[test]
fn every_document_reads_and_writes_back_as_the_same_tree() {
    for (name, _) in CORPUS {
        let text = text_of(name);
        let doc = xmltree::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        for pretty in [false, true] {
            same_document(name, &text, &xmltree::write(&doc, pretty));
        }
    }
}

#[test]
fn the_documents_of_the_steps_done_import_and_write_back_unchanged() {
    for (name, step) in CORPUS.iter().filter(|(_, step)| *step <= DONE) {
        let text = text_of(name);
        let doc = xmltree::parse(&text).unwrap();
        let inferred = infer::infer_xsd(&doc).unwrap_or_else(|e| panic!("{name} ({step}): {e}"));
        let spec = xsd::lower_str(&inferred.text)
            .unwrap_or_else(|e| panic!("{name}: {e}\n{}", inferred.text));
        let parsed = xml::parse(&text, &spec).unwrap_or_else(|e| panic!("{name}: {e}"));
        let issues = validate(&spec, &parsed.node);
        assert!(issues.is_empty(), "{name}: {issues:?}\n{}", inferred.text);
        let written = xml::to_string(&parsed.node, &spec, &parsed.document, true)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        same_document(name, &text, &written);
        assert_eq!(written, text, "{name}: written back differently");
    }
}

#[test]
fn every_document_of_a_later_step_is_listed_with_a_step_still_to_come() {
    // Keeps the list honest: when a step is done, its documents must import.
    let steps = ["15a", "15b", "15c", "15d"];
    for (name, step) in CORPUS {
        assert!(steps.contains(step), "{name}: {step}");
    }
    let listed: Vec<&str> = CORPUS.iter().map(|(name, _)| *name).collect();
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "xml-corpus"]
        .iter()
        .collect();
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(listed.contains(&name.as_str()), "{name} is not listed");
    }
}
