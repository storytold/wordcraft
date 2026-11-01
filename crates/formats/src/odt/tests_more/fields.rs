//! Text fields (ODF 1.3 §7): page number and count, dates and times, document information and
//! bookmark references open as WordCraft fields and are written back as ODF fields.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, Paragraph, para_block};

/// Every field in the body: (code, cached result).
pub(super) fn fields(d: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        if let Some(p) = b.as_para() {
            for o in &p.objects {
                if let InlineObject::Field { instr, result, .. } = o {
                    out.push((instr.clone(), result.clone()));
                }
            }
        }
    }
    out
}

fn body(xml: &str) -> Vec<u8> {
    package(
        &format!(r#"<office:document-content {NS_T}><office:body><office:text>{xml}</office:text></office:body></office:document-content>"#),
        None,
    )
}

#[test]
fn fields_open_as_fields_with_their_shown_text() {
    let d = import(&body(concat!(
        r#"<text:p>Page <text:page-number text:select-page="current">3</text:page-number> of <text:page-count>9</text:page-count>.</text:p>"#,
        r#"<text:p><text:date style:data-style-name="N1" text:date-value="2026-10-31">31/10/2026</text:date> <text:time>12:00</text:time></text:p>"#,
        r#"<text:p><text:title>My title</text:title>|<text:subject>Subj</text:subject>|<text:initial-creator>Ada</text:initial-creator>|<text:file-name>f.odt</text:file-name></text:p>"#,
        r#"<text:p>See <text:bookmark-ref text:reference-format="page" text:ref-name="intro">2</text:bookmark-ref> and <text:bookmark-ref text:reference-format="text" text:ref-name="intro">Intro</text:bookmark-ref></text:p>"#,
    )))
    .unwrap();
    let got = fields(&d);
    let want = [
        ("PAGE", "3"),
        ("NUMPAGES", "9"),
        ("DATE", "31/10/2026"),
        ("TIME", "12:00"),
        ("TITLE", "My title"),
        ("SUBJECT", "Subj"),
        ("AUTHOR", "Ada"),
        ("FILENAME", "f.odt"),
        ("PAGEREF intro \\h", "2"),
        ("REF intro \\h", "Intro"),
    ];
    assert_eq!(got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>(), want);
    // The text around the fields stays where it was.
    let p0 = d.body[0].as_para().unwrap();
    assert_eq!(p0.text.replace(wordcraft_doc::para::OBJ, "#"), "Page # of #.");
}

#[test]
fn a_fixed_date_and_unknown_fields_are_plain_text() {
    let d = import(&body(r#"<text:p><text:date text:fixed="true">1 May</text:date>, <text:variable-get text:name="v">4</text:variable-get>, <text:page-number text:select-page="next">5</text:page-number></text:p>"#)).unwrap();
    assert!(fields(&d).is_empty());
    assert_eq!(d.body[0].as_para().unwrap().text, "1 May, 4, 5");
}

fn field_para(parts: &[(&str, Option<&str>)]) -> std::sync::Arc<wordcraft_doc::Block> {
    // Text pieces, and fields given as (code, Some(result)).
    let mut p = Paragraph::new();
    for (s, field) in parts {
        match field {
            Some(result) => {
                p.text.push(wordcraft_doc::para::OBJ);
                p.objects.push(InlineObject::Field {
                    instr: s.to_string(),
                    result: result.to_string(),
                    locked: false,
                    dirty: false,
                    code: false,
                    form: String::new(),
                });
            }
            None => p.text.push_str(s),
        }
    }
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    para_block(p)
}

#[test]
fn fields_round_trip() {
    let mut d = Document::new();
    d.body = vec![field_para(&[
        ("Page ", None),
        ("PAGE", Some("1")),
        (" of ", None),
        ("NUMPAGES", Some("2")),
        (" ", None),
        ("DATE \\@ \"d MMMM yyyy\"", Some("31 October 2026")),
        (" ", None),
        ("TITLE", Some("T")),
        (" ", None),
        ("PAGEREF intro \\h", Some("4")),
    ])];
    let xml = export(&d).unwrap();
    let back = import(&xml).unwrap();
    let got: Vec<String> = fields(&back).into_iter().map(|(c, _)| c).collect();
    // The date keeps its picture (as a date style).
    assert_eq!(got, ["PAGE", "NUMPAGES", "DATE \\@ \"d MMMM yyyy\"", "TITLE", "PAGEREF intro \\h"]);
    assert_eq!(back.body[0].as_para().unwrap().text.matches(wordcraft_doc::para::OBJ).count(), 5);
}

#[test]
fn a_field_odf_has_no_element_for_keeps_its_result_as_text() {
    let mut d = Document::new();
    d.body = vec![field_para(&[("x ", None), ("SECTIONPAGES", Some("7")), (" y", None)])];
    let back = import(&export(&d).unwrap()).unwrap();
    assert!(fields(&back).is_empty());
    assert_eq!(back.body[0].as_para().unwrap().text, "x 7 y");
}

#[test]
fn hostile_fields_do_not_panic() {
    // Unclosed, nested and empty fields, and a long reference name.
    let long = "n".repeat(100_000);
    for xml in [
        "<text:p><text:page-number>".to_string(),
        "<text:p><text:page-number><text:page-count>1</text:page-count></text:page-number></text:p>".to_string(),
        "<text:p><text:page-number/><text:bookmark-ref/></text:p>".to_string(),
        format!(r#"<text:p><text:bookmark-ref text:reference-format="page" text:ref-name="{long}">1</text:bookmark-ref></text:p>"#),
    ] {
        let d = import(&body(&xml)).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn an_element_inside_a_field_leaves_the_formatting_after_it_alone() {
    // A span inside a field was skipped at its start but still ended, popping the enclosing
    // span's formatting: the text after the field lost its bold.
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="B">x<text:page-number><text:span>1</text:span></text:page-number>y</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let p = d.body[0].as_para().unwrap();
    assert_eq!(fields(&d), [("PAGE".to_string(), "1".to_string())]);
    assert!(p.runs.iter().all(|r| r.props.bold == Some(true)), "{:?}", p.runs);
}

#[test]
fn a_paragraph_ending_inside_a_field_still_ends() {
    let d = import(&body("<text:p>a<text:page-number>1</text:p><text:p>b</text:p>")).unwrap();
    let texts: Vec<String> = d.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "#")).collect();
    assert_eq!(texts, ["a#", "b"]);
}

#[test]
fn custom_properties_and_user_fields_are_docproperty_fields() {
    // A custom property (`meta:user-defined`) shown by `text:user-defined`, and a user field
    // (`text:user-field-decl`) shown by `text:user-field-get`: Word's custom document properties
    // and `DOCPROPERTY` fields.
    let meta = r#"<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"><office:meta><meta:user-defined meta:name="Project" meta:value-type="string">Kiln</meta:user-defined><meta:user-defined meta:name="Budget" meta:value-type="float">1200.5</meta:user-defined></office:meta></office:document-meta>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:user-field-decls><text:user-field-decl text:name="Owner" office:value-type="string" office:string-value="Ada"/></text:user-field-decls><text:p>For <text:user-defined text:name="Project">Kiln</text:user-defined> by <text:user-field-get text:name="Owner">Ada</text:user-field-get>.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("meta.xml", meta.as_bytes())]);
    let d = crate::odt::import(&pkg).unwrap();
    let props: Vec<(String, String, String)> = d.custom_props.iter().map(|p| (p.name.clone(), p.kind.clone(), p.value.clone())).collect();
    assert_eq!(
        props,
        [
            ("Project".into(), "lpwstr".into(), "Kiln".into()),
            ("Budget".into(), "r8".into(), "1200.5".into()),
            ("Owner".into(), "lpwstr".into(), "Ada".into())
        ]
    );
    let instrs: Vec<String> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let wordcraft_doc::para::InlineObject::Field { instr, .. } = o { Some(instr.clone()) } else { None })
        .collect();
    assert_eq!(instrs, [r#"DOCPROPERTY "Project""#, r#"DOCPROPERTY "Owner""#]);
    // Saved: the properties in `meta.xml`, the fields as `text:user-defined`.
    let saved = crate::odt::export(&d).unwrap();
    let m = crate::odt::tests::entry(&saved, "meta.xml");
    assert!(m.contains(r#"<meta:user-defined meta:name="Budget" meta:value-type="float">1200.5</meta:user-defined>"#), "{m}");
    assert!(m.contains(r#"meta:name="Owner""#), "{m}");
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:user-defined text:name="Owner">Ada</text:user-defined>"#), "{c}");
    assert_eq!(crate::odt::import(&saved).unwrap().custom_props.len(), 3);
}
