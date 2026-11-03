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
fn a_fixed_date_is_a_locked_field_and_unknown_fields_are_plain_text() {
    // A fixed date (§19.807 `text:fixed`) keeps what it shows: a locked field, written back fixed.
    let d = import(&body(r#"<text:p><text:date text:fixed="true">1 May</text:date>, <text:conditional-text text:condition="ooow:1" text:string-value-if-true="4" text:string-value-if-false="5">4</text:conditional-text>, <text:page-number text:select-page="next">5</text:page-number></text:p>"#)).unwrap();
    assert_eq!(fields(&d), [("DATE".to_string(), "1 May".to_string())]);
    let locked = d.body[0].as_para().unwrap().objects.iter().any(|o| matches!(o, InlineObject::Field { locked: true, .. }));
    assert!(locked);
    // Written back: with no picture to read its value under, as its text (a reader would show a
    // fixed date without a value as today's).
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(!c.contains("<text:date") && c.contains("1 May"), "{c}");
    // With a picture, fixed at the value it shows (§19.871).
    let n = r#"xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#;
    let d = import(&package(
        &format!(
            r#"<office:document-content {NS_T} {n}><office:automatic-styles><number:date-style style:name="N1"><number:day/><number:text> </number:text><number:month number:style="long" number:textual="true"/><number:text> </number:text><number:year number:style="long"/></number:date-style><number:time-style style:name="N2"><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/></number:time-style></office:automatic-styles><office:body><office:text><text:p><text:date text:fixed="true" style:data-style-name="N1" text:date-value="2026-05-01">1 May 2026</text:date> <text:time text:fixed="true" style:data-style-name="N2" text:time-value="2026-05-01T14:30:00">14:30</text:time></text:p></office:text></office:body></office:document-content>"#
        ),
        None,
    ))
    .unwrap();
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"text:date-value="2026-05-01""#) && c.contains(">1 May 2026</text:date>"), "{c}");
    assert!(c.contains(r#"text:time-value="1899-12-30T14:30:00""#) && c.contains(">14:30</text:time>"), "{c}");
    assert_eq!(c.matches(r#"text:fixed="true""#).count(), 2, "{c}");
    // A fixed date with neither a value nor text (LibreOffice's templates) shows the day: a
    // date field.
    let d = import(&body(r#"<text:p><text:date text:fixed="true"/>.</text:p>"#)).unwrap();
    assert_eq!(fields(&d).len(), 1);
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
    // Saved: the properties in `meta.xml` with `text:user-defined` fields, the user field as one.
    let saved = crate::odt::export(&d).unwrap();
    let m = crate::odt::tests::entry(&saved, "meta.xml");
    assert!(m.contains(r#"<meta:user-defined meta:name="Budget" meta:value-type="float">1200.5</meta:user-defined>"#), "{m}");
    assert!(!m.contains(r#"meta:name="Owner""#), "{m}");
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:user-defined text:name="Project">Kiln</text:user-defined>"#), "{c}");
    assert!(c.contains(r#"<text:user-field-get text:name="Owner">Ada</text:user-field-get>"#), "{c}");
    assert_eq!(crate::odt::import(&saved).unwrap().custom_props.len(), 3);
}

#[test]
fn custom_date_properties_are_full_utc_dates() {
    for (v, kind, want) in [
        ("2026-11-01", "filetime", "2026-11-01T00:00:00Z"),
        ("2026-11-01T10:30:00", "filetime", "2026-11-01T10:30:00Z"),
        ("2026-11-01T10:30:00Z", "filetime", "2026-11-01T10:30:00Z"),
        ("soon", "lpwstr", "soon"),
    ] {
        let p = crate::odt::fields::custom_prop("When", Some("date"), v).unwrap();
        assert_eq!((p.kind.as_str(), p.value.as_str()), (kind, want), "{v}");
    }
}

#[test]
fn bibliography_marks_are_citations_of_sources() {
    // `text:bibliography-mark`: a citation carrying its source, which becomes the document's.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>As shown <text:bibliography-mark text:identifier="Smi20" text:bibliography-type="book" text:author="Smith, Jane" text:title="Kilns" text:year="2020" text:publisher="Clay Press">[Smi20]</text:bibliography-mark>, and again <text:bibliography-mark text:identifier="Smi20" text:bibliography-type="book">[Smi20]</text:bibliography-mark>.</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    assert_eq!(d.sources.len(), 1);
    let s = &d.sources[0];
    assert_eq!(
        (s.tag.as_str(), s.kind.as_str(), s.author.as_str(), s.title.as_str(), s.year.as_str(), s.publisher.as_str()),
        ("Smi20", "book", "Smith, Jane", "Kilns", "2020", "Clay Press")
    );
    let fields: Vec<(String, String)> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Field { instr, result, .. } = o { Some((instr.clone(), result.clone())) } else { None })
        .collect();
    assert_eq!(fields, [("CITATION Smi20".to_string(), "[Smi20]".to_string()), ("CITATION Smi20".to_string(), "[Smi20]".to_string())]);
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(
        c.contains(r#"<text:bibliography-mark text:identifier="Smi20" text:bibliography-type="book" text:author="Smith, Jane" text:title="Kilns" text:year="2020" text:publisher="Clay Press">[Smi20]</text:bibliography-mark>"#),
        "{c}"
    );
}

const VARIABLES: &str = r#"<text:variable-decls><text:variable-decl office:value-type="string" text:name="who"/></text:variable-decls><text:p>Set: <text:variable-set text:name="who" office:value-type="string" office:string-value="Ada &quot;AL&quot;">Ada "AL"</text:variable-set>. Hidden: <text:variable-set text:name="who" text:display="none" office:value-type="string" office:string-value="Grace"></text:variable-set>. Get: <text:variable-get text:name="who">Grace</text:variable-get>. Number: <text:variable-set text:name="n" office:value-type="float" office:value="42" text:display="none"/><text:variable-get text:name="n">42</text:variable-get>.</text:p>"#;

#[test]
fn variables_open_as_set_and_ref_fields() {
    let d = import(&body(VARIABLES)).unwrap();
    let want: Vec<(String, String)> = [
        (r#"SET who "Ada \"AL\"""#, ""),
        ("REF who", r#"Ada "AL""#),
        (r#"SET who "Grace""#, ""),
        ("REF who", "Grace"),
        (r#"SET n "42""#, ""),
        ("REF n", "42"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(fields(&d), want);
    // The text around them, as LibreOffice shows it.
    assert_eq!(d.body[0].as_para().unwrap().text.replace(wordcraft_doc::para::OBJ, ""), "Set: . Hidden: . Get: . Number: .");
}

#[test]
fn variables_are_written_back_declared() {
    let d = import(&body(VARIABLES)).unwrap();
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    for want in [
        r#"<text:variable-decls><text:variable-decl office:value-type="string" text:name="n"/><text:variable-decl office:value-type="string" text:name="who"/></text:variable-decls>"#,
        r#"<text:variable-set text:name="who" text:display="none" office:value-type="string" office:string-value="Ada &quot;AL&quot;"/>"#,
        r#"<text:variable-get text:name="who">Grace</text:variable-get>"#,
    ] {
        assert!(c.contains(want), "{want} in {c}");
    }
    assert_eq!(fields(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), fields(&d));
}

#[test]
fn hostile_variables_never_panic() {
    let long = "v".repeat(100_000);
    for xml in [
        r#"<text:p><text:variable-set/><text:variable-get/></text:p>"#.to_string(),
        r#"<text:p><text:variable-set text:name="a b" office:string-value="x"/><text:variable-set text:name="q&quot;" office:string-value="x"/></text:p>"#.to_string(),
        format!(r#"<text:p><text:variable-set text:name="v" office:string-value="{long}"/></text:p>"#),
        r#"<text:p><text:variable-set text:name="v" text:formula="ooow:\\&quot;"><text:variable-set text:name="w"/></text:variable-set></text:p>"#.to_string(),
    ] {
        let d = import(&body(&xml)).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn a_date_in_the_locales_order_shows_so() {
    // LibreOffice's letters: year, month by name and day, `number:automatic-order`: shown in
    // the locale's order, "November 2, 2026" in US English.
    let styles = format!(
        r#"<office:document-styles {NS_T} xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"><office:styles><number:date-style style:name="N112" number:automatic-order="true"><number:year number:style="long"/><number:month number:style="long" number:textual="true"/><number:day/></number:date-style><number:date-style style:name="N2" number:automatic-order="true"><number:day/><number:month/><number:year number:style="long"/></number:date-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p><text:date style:data-style-name="N112" text:date-value="2026-11-02">2026November2</text:date> <text:date style:data-style-name="N2" text:date-value="2026-11-02">2112026</text:date></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let codes: Vec<String> = fields(&d).into_iter().map(|(c, _)| c).collect();
    assert_eq!(codes, [r#"DATE \@ "MMMM d, yyyy""#, r#"DATE \@ "M/d/yyyy""#]);
}

#[test]
fn user_fields_stay_user_fields() {
    // LibreOffice's report templates: user fields (§7.4.7 `text:user-field-decl`, §7.4.9
    // `text:user-field-get`) are written back as such, not as document properties.
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:user-field-decls><text:user-field-decl office:value-type="string" office:string-value="Group one" text:name="GroupField1"/><text:user-field-decl office:value-type="float" office:value="3" text:name="Count"/></text:user-field-decls><text:p>Group: <text:user-field-get text:name="GroupField1">Group one</text:user-field-get>, <text:user-field-get text:name="Count">3</text:user-field-get></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    assert!(d.custom_props.iter().all(|p| p.odf_user_field), "{:?}", d.custom_props);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(r#"<text:user-field-decl text:name="GroupField1" office:value-type="string" office:string-value="Group one"/>"#), "{c}");
    assert!(c.contains(r#"office:value-type="float" office:value="3""#), "{c}");
    assert!(c.contains(r#"<text:user-field-get text:name="GroupField1">Group one</text:user-field-get>"#), "{c}");
    let meta = crate::odt::tests::entry(&saved, "meta.xml");
    assert!(!meta.contains("GroupField1"), "not a document property too: {meta}");
    let back = import(&saved).unwrap();
    assert_eq!(back.custom_props, d.custom_props);
}

#[test]
fn sender_fields_stay_fields() {
    // LibreOffice's letter and fax templates: the sender's details (§7.3.6), shown as last filled
    // in (WordCraft's `SENDER` field code), written back as the same fields.
    let d = import(&body(r#"<text:p><text:sender-firstname>Ada</text:sender-firstname> <text:sender-lastname>Lovelace</text:sender-lastname>, <text:sender-email>ada@example.org</text:sender-email></text:p>"#)).unwrap();
    assert_eq!(
        fields(&d),
        [("SENDER firstname".into(), "Ada".into()), ("SENDER lastname".into(), "Lovelace".into()), ("SENDER email".into(), "ada@example.org".into())]
    );
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(
        c.contains("<text:sender-firstname>Ada</text:sender-firstname>") && c.contains("<text:sender-email>ada@example.org</text:sender-email>"),
        "{c}"
    );
}

#[test]
fn a_field_that_follows_the_document_says_so() {
    // §19.807: not fixed unless said; LibreOffice reads an author field without `text:fixed` as
    // fixed (an empty one stayed empty instead of showing the author), so WordCraft says it, as
    // LibreOffice does. Sender fields it writes without, and so does WordCraft.
    let d = import(&body(r#"<text:p><text:author-name text:fixed="false"/> <text:sender-firstname>Ann</text:sender-firstname></text:p>"#)).unwrap();
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:author-name text:fixed="false""#), "{c}");
    assert!(c.contains("<text:sender-firstname>Ann</text:sender-firstname>"), "{c}");
}
