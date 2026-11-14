//! Document information fields (ODF 1.3 §7.3, §7.5): author initials, template name, revision
//! number and editing time open as Word's USERINITIALS, TEMPLATE, REVNUM and EDITTIME, and a file
//! or template name shown with its path as `\p`; all are written back as the same ODF elements.

use super::fields::fields;
use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};

fn body(xml: &str) -> Vec<u8> {
    package(
        &format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>{xml}</text:p></office:text></office:body></office:document-content>"#
        ),
        None,
    )
}

fn codes(xml: &str) -> Vec<(String, String)> {
    fields(&import(&body(xml)).unwrap())
}

#[test]
fn doc_info_elements_open_as_word_fields() {
    let table = [
        (r#"<text:author-initials>AL</text:author-initials>"#, "USERINITIALS", "AL"),
        (r#"<text:template-name text:display="name-and-extension">Letter.ott</text:template-name>"#, "TEMPLATE", "Letter.ott"),
        (r#"<text:template-name text:display="full">/t/Letter.ott</text:template-name>"#, "TEMPLATE \\p", "/t/Letter.ott"),
        (r#"<text:template-name>Letter.ott</text:template-name>"#, "TEMPLATE", "Letter.ott"),
        (r#"<text:file-name text:display="full">/d/f.odt</text:file-name>"#, "FILENAME \\p", "/d/f.odt"),
        (r#"<text:file-name text:display="name-and-extension">f.odt</text:file-name>"#, "FILENAME", "f.odt"),
        (r#"<text:file-name>f.odt</text:file-name>"#, "FILENAME", "f.odt"),
        (r#"<text:editing-cycles>7</text:editing-cycles>"#, "REVNUM", "7"),
        (r#"<text:editing-duration text:duration="PT1H5M">65</text:editing-duration>"#, "EDITTIME", "65"),
    ];
    for (xml, code, result) in table {
        assert_eq!(codes(xml), [(code.to_string(), result.to_string())], "{xml}");
    }
}

#[test]
fn displays_word_has_no_field_for_stay_text() {
    for (xml, text) in [
        (r#"<text:template-name text:display="path">/t</text:template-name>"#, "/t"),
        (r#"<text:template-name text:display="title">My letter</text:template-name>"#, "My letter"),
        (r#"<text:template-name text:display="area">Mine</text:template-name>"#, "Mine"),
        (r#"<text:template-name text:display="name">Letter</text:template-name>"#, "Letter"),
        (r#"<text:file-name text:display="path">/d</text:file-name>"#, "/d"),
        (r#"<text:file-name text:display="name">f</text:file-name>"#, "f"),
    ] {
        let d = import(&body(xml)).unwrap();
        assert!(fields(&d).is_empty(), "{xml}");
        assert_eq!(d.body[0].as_para().unwrap().text, text, "{xml}");
    }
}

#[test]
fn word_fields_are_written_as_their_odf_elements() {
    let xml = concat!(
        r#"<text:author-initials>AL</text:author-initials>|"#,
        r#"<text:template-name text:display="name-and-extension">Letter.ott</text:template-name>|"#,
        r#"<text:template-name text:display="full">/t/Letter.ott</text:template-name>|"#,
        r#"<text:file-name text:display="name-and-extension">f.odt</text:file-name>|"#,
        r#"<text:file-name text:display="full">/d/f.odt</text:file-name>|"#,
        r#"<text:editing-cycles>7</text:editing-cycles>|"#,
        r#"<text:editing-duration>65</text:editing-duration>"#,
    );
    let d = import(&body(xml)).unwrap();
    let c = content_of(&export(&d).unwrap());
    for want in [
        r#"<text:author-initials text:fixed="false">AL</text:author-initials>"#,
        r#"<text:template-name text:display="name-and-extension">Letter.ott</text:template-name>"#,
        r#"<text:template-name text:display="full">/t/Letter.ott</text:template-name>"#,
        r#"<text:file-name text:display="name-and-extension">f.odt</text:file-name>"#,
        r#"<text:file-name text:display="full">/d/f.odt</text:file-name>"#,
        r#"<text:editing-cycles>7</text:editing-cycles>"#,
        r#"<text:editing-duration>65</text:editing-duration>"#,
    ] {
        assert!(c.contains(want), "missing {want} in {c}");
    }
}

#[test]
fn doc_info_fields_round_trip() {
    let want = ["USERINITIALS", "TEMPLATE", "TEMPLATE \\p", "FILENAME", "FILENAME \\p", "REVNUM", "EDITTIME"];
    let xml = concat!(
        r#"<text:author-initials>AL</text:author-initials> "#,
        r#"<text:template-name text:display="name-and-extension">L.ott</text:template-name> "#,
        r#"<text:template-name text:display="full">/t/L.ott</text:template-name> "#,
        r#"<text:file-name text:display="name-and-extension">f.odt</text:file-name> "#,
        r#"<text:file-name text:display="full">/d/f.odt</text:file-name> "#,
        r#"<text:editing-cycles>7</text:editing-cycles> "#,
        r#"<text:editing-duration>65</text:editing-duration>"#,
    );
    let d = import(&body(xml)).unwrap();
    let back = import(&export(&d).unwrap()).unwrap();
    let got: Vec<String> = fields(&back).into_iter().map(|(c, _)| c).collect();
    assert_eq!(got, want);
}

#[test]
fn switches_are_case_insensitive_and_others_ignored() {
    use wordcraft_doc::para::{InlineObject, OBJ, Run};
    let mut p = wordcraft_doc::Paragraph::new();
    p.text.push(OBJ);
    p.objects.push(InlineObject::Field {
        instr: "FILENAME \\P \\* Upper".to_string(),
        result: "/D/F.ODT".to_string(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    });
    p.runs = vec![Run { len: p.text.len(), props: Default::default() }];
    let mut d = wordcraft_doc::Document::new();
    d.body = vec![wordcraft_doc::para_block(p)];
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:file-name text:display="full">/D/F.ODT</text:file-name>"#), "{c}");
}
