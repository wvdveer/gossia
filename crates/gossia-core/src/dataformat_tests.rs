use super::*;

#[test]
fn formats_are_named_by_code_and_extension() {
    for format in DataFormat::ALL {
        assert_eq!(DataFormat::from_code(format.code()), Some(format));
        // A file of YAML documents is told apart by its text.
        let by_text =
            DataFormat::from_extension(format.extension()).map(|f| f.refine("a: 1\n---\nb: 2\n"));
        let expected = match format {
            DataFormat::Yaml => DataFormat::YamlDocuments,
            _ => format,
        };
        assert_eq!(by_text, Some(expected));
    }
    assert_eq!(DataFormat::Yaml.refine("a: 1\n"), DataFormat::Yaml);
    assert_eq!(DataFormat::from_extension("YML"), Some(DataFormat::Yaml));
    assert_eq!(
        DataFormat::from_extension("NDJSON"),
        Some(DataFormat::JsonLines)
    );
    assert_eq!(DataFormat::from_extension("txt"), None);
    assert!(DataFormat::Json5.has_comments() && !DataFormat::JsonLines.has_comments());
    assert!(!DataFormat::Xml.json_shaped());
}

#[test]
fn json_lines_are_an_array_of_one_value_a_line() {
    let parsed = DataFormat::JsonLines
        .read("{\"a\": 1}\n\n{\"a\": 10000000000000000000}\n[1]\n")
        .unwrap();
    let items = parsed.node.value().as_array().unwrap();
    assert_eq!(items.len(), 3, "the blank line is passed over");
    assert_eq!(parsed.warnings.len(), 1);
    assert_eq!(parsed.warnings[0].path.to_string(), "[1].a");
    let e = DataFormat::JsonLines.read("{}\n{\"a\": \n").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("not a JSON value"), "{e}");
    assert_eq!(
        DataFormat::JsonLines.write(&parsed.node, true).unwrap(),
        "{\"a\":1}\n{\"a\":1e+19}\n[1]\n"
    );
    let object = DataFormat::Json.read("{}").unwrap().node;
    assert!(DataFormat::JsonLines.write(&object, true).is_err());
}

#[test]
fn every_json_shaped_format_reads_back_what_it_writes() {
    let rows = r#"[{"name": "Ann", "n": 1.5, "ok": true, "tags": ["x"]}, {"name": "Bo"}]"#;
    for format in DataFormat::JSON_SHAPED {
        // TOML needs a table at the top; the others take the rows as they are.
        let text = if format == DataFormat::Toml {
            format!(r#"{{"rows": {rows}}}"#)
        } else {
            rows.to_owned()
        };
        let node = DataFormat::Json.read(&text).unwrap().node;
        let plain = json::to_string(&node, false).unwrap();
        for pretty in [true, false] {
            let text = format.write(&node, pretty).unwrap();
            assert!(text.ends_with('\n'));
            let again = format.read(&text).unwrap().node;
            assert_eq!(json::to_string(&again, false).unwrap(), plain, "{format:?}");
        }
    }
}
