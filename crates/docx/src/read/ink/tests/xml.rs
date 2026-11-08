use super::{Graphic, ink, read};

/// Decode an XML fixture with a fresh bounded work allowance.
fn decode(text: &str) -> Option<Graphic> {
    read(text.as_bytes(), &mut 2_000_000)
}

/// Encode a fixture as BOM-marked UTF-16 in the requested byte order.
fn utf16(text: &str, little: bool) -> Vec<u8> {
    let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
    for unit in text.encode_utf16() {
        bytes.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
    }
    bytes
}

/// Namespace identity follows decoded URIs and scoped Unicode prefixes, not prefix spelling.
#[test]
fn namespace_uris_decode_references_and_unicode_prefixes_keep_their_identity() {
    let expected = ink("<trace>0 0,10 10</trace>");
    assert_eq!(decode(r#"<筆:ink xmlns:筆="http://www.w3.org/2003/Ink&#77;L"><筆:trace>0 0,10 10</筆:trace></筆:ink>"#), expected);
    assert_eq!(
        decode(
            r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace xmlns:i="http://www.w3.org/2003/InkML" xmlns="urn:other"><i:trace>0 0,10 10</i:trace></trace></ink>"#
        ),
        None
    );
    assert_eq!(
        decode(r#"<ink xmlns="http://www.w3.org/2003/InkML"><i:trace xmlns:i="http://www.w3.org/2003/InkML">0 0,10 10</i:trace></ink>"#),
        expected
    );
}

/// Only valid qualified XML IDs establish local references, with XML whitespace trimmed.
#[test]
fn only_qualified_xml_ids_supply_local_references() {
    let good = r##"<definitions><brush xml:id="筆"><brushProperty name="color" value="#2563EB"/></brush></definitions><trace brushRef="#筆">0 0,10 10</trace>"##;
    assert!(ink(good).is_some());
    assert_eq!(ink(&good.replace("xml:id=", "id=")), None);
    assert_eq!(ink(&good.replace("xml:id=", "bad:id=")), None);
    assert_eq!(ink(&good.replace("xml:id=", "other:id=").replace("<definitions>", "<definitions xmlns:other=\"urn:other\">")), None);
    assert_eq!(ink(&good.replace("xml:id=\"筆\"", "xml:id=\" \n筆\r \"")), ink(good));
    for id in ["", "1bad", "a:b", "a b", "-bad", "a\u{a0}b"] {
        assert!(ink(&good.replace("xml:id=\"筆\"", &format!("xml:id=\"{id}\""))).is_none(), "{id:?}");
    }
}

/// Undeclared prefixes and duplicate raw or expanded attributes invalidate the whole part.
#[test]
fn undeclared_prefixes_and_duplicate_expanded_attributes_refuse_the_part() {
    for body in [
        r#"<trace bad:type="penUp">0 0,1 1</trace>"#,
        r#"<trace type="penDown" type="penUp">0 0,1 1</trace>"#,
        r#"<trace type="penDown"other="value">0 0,1 1</trace>"#,
        r#"<annotationXML xmlns:a="urn:same" xmlns:b="urn:same" a:x="1" b:x="2"/><trace>0 0,1 1</trace>"#,
        r#"<annotation xmlns:a="urn:one" xmlns:a="urn:two"/><trace>0 0,1 1</trace>"#,
        r#"<annotation xmlns:a="urn:one" a:b:c="1"/><trace>0 0,1 1</trace>"#,
    ] {
        assert!(ink(body).is_none(), "{body}");
    }
    assert!(ink(r#"<annotationXML xmlns:a="urn:one" xmlns:b="urn:two" a:x="1" b:x="2"/><trace>0 0,1 1</trace>"#).is_some());
}

/// Invalid namespace bindings, XML names and attribute values are rejected rather than repaired.
#[test]
fn invalid_namespace_bindings_and_names_are_not_repaired() {
    for body in [
        r#"<annotation xmlns:xml="urn:wrong"/>"#,
        r#"<annotation xmlns:xmlns="urn:wrong"/>"#,
        r#"<annotation xmlns:p="http://www.w3.org/XML/1998/namespace"/>"#,
        r#"<annotation xmlns="http://www.w3.org/2000/xmlns/"/>"#,
        r#"<annotation xmlns:p="urn:has space"/>"#,
        r#"<annotation 1bad="x"/>"#,
        r#"<annotation xmlns:="urn:bad"/>"#,
        r#"<1bad/>"#,
        r#"<annotation a="raw<value"/>"#,
        r#"<annotation a="&undefined;"/>"#,
    ] {
        assert!(ink(&format!("{body}<trace>0 0,1 1</trace>")).is_none(), "{body}");
    }
}

/// XML 1.1 normalizes literal newlines while preserving referenced characters and XML 1.0 differences.
#[test]
fn xml_11_normalizes_literal_newlines_but_not_character_references() {
    let text = "<?xml version=\"1.1\"?><ink xmlns=\"http://www.w3.org/2003/InkML\" documentID=\"a\r\u{85}b\u{2028}c&#x85;d&amp;#10;\"><annotation>a\r\u{85}b\u{2028}c&#x85;d</annotation><trace>0 0,1 1</trace></ink>";
    let root = super::super::xml::parse(text.as_bytes(), &mut 2_000_000).unwrap();
    assert_eq!(root.attr("documentID"), Some("a b c\u{85}d&#10;"));
    assert_eq!(root.child("ink:annotation").unwrap().text(), "a\nb\nc\u{85}d");
    assert_eq!(decode(text), ink("<trace>0 0,1 1</trace>"));
    let markup = "<?xml\u{85}version=\"1.1\"?><ink\u{2028}xmlns=\"http://www.w3.org/2003/InkML\"><trace\u{85}type=\"penDown\">0 0,1 1</trace></ink>";
    assert_eq!(decode(markup), ink("<trace>0 0,1 1</trace>"));
    assert!(decode(&markup.replace("1.1", "1.0")).is_none());
    let xml10 = super::super::xml::parse(text.replace("1.1", "1.0").as_bytes(), &mut 2_000_000).unwrap();
    assert_eq!(xml10.attr("documentID"), Some("a \u{85}b\u{2028}c\u{85}d&#10;"));
}

/// Restricted XML 1.1 characters require references; invalid code points remain forbidden.
#[test]
fn xml_11_restricted_characters_need_references_and_xml_10_refuses_them() {
    let text =
        "<?xml version=\"1.1\"?><ink xmlns=\"http://www.w3.org/2003/InkML\"><annotation a=\"&#x1;\">&#x1;</annotation><trace>0 0,1 1</trace></ink>";
    assert_eq!(decode(text), ink("<trace>0 0,1 1</trace>"));
    assert_eq!(decode(&text.replace("1.1", "1.0")), None);
    assert_eq!(decode(&text.replace("&#x1;", "\u{1}")), None);
    for reference in ["&#0;", "&#xFFFE;", "&#xD800;", "&#x110000;"] {
        assert_eq!(decode(&text.replace("&#x1;", reference)), None, "{reference}");
    }
}

/// Prefix undeclaration is scoped to its element and is accepted only in XML 1.1.
#[test]
fn prefix_undeclaration_is_scoped_and_requires_xml_11() {
    let text = "<?xml version=\"1.1\"?><ink xmlns=\"http://www.w3.org/2003/InkML\" xmlns:p=\"urn:other\"><annotationXML xmlns:p=\"\"/><annotationXML p:name=\"ok\"/><trace>0 0,1 1</trace></ink>";
    assert!(decode(text).is_some());
    assert!(decode(&text.replace("1.1", "1.0")).is_none());
    assert!(decode(&text.replace("xmlns:p=\"\"/>", "xmlns:p=\"\"><p:name/></annotationXML>")).is_none());
}

/// Malformed declarations, multiple roots and illegal content outside the root refuse preview.
#[test]
fn document_envelopes_and_declarations_must_be_well_formed() {
    let body = r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,1 1</trace></ink>"#;
    for text in [
        format!("junk{body}"),
        format!("{body}junk"),
        format!("{body}{body}"),
        format!(" {body}<?xml version=\"1.0\"?>"),
        format!(" <?xml version=\"1.0\"?>{body}"),
        format!("<?xml version=\"1.2\"?>{body}"),
        format!("<?xml version=\"1.0\" extra=\"x\"?>{body}"),
        format!("<?xml version=\"1.0\" standalone=\"maybe\"?>{body}"),
        format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?>{body}"),
        format!("<?xml version=\"1.0\" version=\"1.0\"?>{body}"),
        format!("<!-- invalid -- comment -->{body}"),
        format!("<![CDATA[ ]]>{body}"),
        body.replace("<trace>", "<trace>]]>"),
        format!("&#32;{body}"),
    ] {
        assert!(decode(&text).is_none(), "{text}");
    }
    assert!(decode(&format!("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?><!-- ok -->\n{body}\n<?done ok?>")).is_some());
}

/// UTF-8 and both UTF-16 orders preserve geometry but reject malformed code units and bytes.
#[test]
fn utf8_and_both_utf16_orders_are_strict_and_keep_the_same_geometry() {
    let text = "<?xml version=\"1.1\" encoding=\"UTF-16\"?><ink xmlns=\"http://www.w3.org/2003/InkML\"><annotation>筆\u{1f58b}</annotation><trace>0 0,1 1</trace></ink>";
    let expected = ink("<trace>0 0,1 1</trace>");
    for little in [false, true] {
        let bytes = utf16(text, little);
        assert_eq!(read(&bytes, &mut 2_000_000), expected);
        assert!(read(&bytes[..bytes.len() - 1], &mut 2_000_000).is_none());
        let mut malformed = bytes.clone();
        malformed.extend(if little { [0x00, 0xd8] } else { [0xd8, 0x00] });
        assert!(read(&malformed, &mut 2_000_000).is_none());
    }
    let mut invalid_utf8 = text.replace("UTF-16", "UTF-8").into_bytes();
    invalid_utf8.push(0xff);
    assert!(read(&invalid_utf8, &mut 2_000_000).is_none());
}

/// Opaque metadata cannot inject rendering definitions or continuations, but IDs must remain unique.
#[test]
fn metadata_cannot_supply_rendering_definitions_or_continuations() {
    for hidden in [
        r##"<brush xml:id="p"><brushProperty name="color" value="#FF0000"/></brush>"##,
        r#"<context xml:id="p"/>"#,
        r#"<trace xml:id="p">0 0,10 10</trace>"#,
    ] {
        let reference = if hidden.starts_with("<brush") {
            "brushRef"
        } else if hidden.starts_with("<context") {
            "contextRef"
        } else {
            "traceDataRef"
        };
        let use_ = if reference == "traceDataRef" {
            r##"<traceView traceDataRef="#p"/>"##.to_string()
        } else {
            format!(r##"<trace {reference}="#p">0 0,1 1</trace>"##)
        };
        assert!(ink(&format!("<definitions><annotationXML>{hidden}</annotationXML></definitions>{use_}")).is_none());
    }
    assert_eq!(ink(r#"<annotationXML><trace>not trace data</trace></annotationXML><trace>0 0,1 1</trace>"#), ink("<trace>0 0,1 1</trace>"));
    assert_eq!(
        ink(
            r##"<annotationXML xml:base="https://example.invalid/metadata"><context contextRef="#metadata-only"/></annotationXML><trace>0 0,1 1</trace>"##
        ),
        ink("<trace>0 0,1 1</trace>")
    );
    assert!(ink(r#"<annotationXML><item xml:id="same"/></annotationXML><trace xml:id="same">0 0,1 1</trace>"#).is_none());
}

/// DTDs and rebased fragment references refuse decoding without resolving external resources.
#[test]
fn dtd_dependencies_and_rebased_fragment_references_retain_fallback() {
    let body = r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,1 1</trace></ink>"#;
    for dtd in [
        r#"<!DOCTYPE ink SYSTEM "https://example.invalid/ink.dtd">"#,
        r#"<!DOCTYPE ink [<!ENTITY data "0 0,1 1">]>"#,
        r#"<!DOCTYPE ink [<!ENTITY data SYSTEM "file:///etc/passwd">]>"#,
    ] {
        assert!(decode(&format!("{dtd}{body}")).is_none());
    }
    let referenced =
        r##"<definitions><brush xml:id="p"/></definitions><trace xml:base="https://example.invalid/other.xml" brushRef="#p">0 0,1 1</trace>"##;
    assert!(ink(referenced).is_none());
    assert!(ink(&referenced.replace("xml:base=\"https://example.invalid/other.xml\"", "xml:base=\"\"")).is_some());
}

/// Attribute counts and exact shared work budgets are enforced before accepting a preview.
#[test]
fn parser_attributes_and_shared_work_are_bounded_before_tree_construction() {
    let attrs = (0..64).map(|i| format!(" a{i}=\"x\"")).collect::<String>();
    assert!(ink(&format!("<annotation{attrs}/><trace>0 0,1 1</trace>")).is_some());
    assert!(ink(&format!("<annotation{attrs} extra=\"x\"/><trace>0 0,1 1</trace>")).is_none());
    let text = r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,1 1</trace></ink>"#;
    let mut work = 2_000_000;
    assert!(read(text.as_bytes(), &mut work).is_some());
    let spent = 2_000_000 - work;
    assert!(read(text.as_bytes(), &mut { spent }).is_some());
    let mut short = spent - 1;
    assert!(read(text.as_bytes(), &mut short).is_none());
    assert!(short < spent - 1, "completed operations still consume work on failure");
}

proptest::proptest! {
    /// Arbitrary byte sequences never panic during bounded XML and ink decoding.
    #[test]
    fn arbitrary_xml_bytes_never_panic(bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..2048)) {
        let _ = read(&bytes, &mut 20_000);
    }
}
