//! Tables of contents and other generated indexes (ODF 1.3 §8: `text:table-of-content`,
//! `text:illustration-index`, `text:table-index`, `text:alphabetical-index`,
//! `text:bibliography`): opened as WordCraft's (Word's `TOC`, `INDEX` and `BIBLIOGRAPHY`
//! fields around their entries, in the TOC styles), so Update Table works on them, and written
//! back as ODF indexes LibreOffice updates.

use crate::odt::tests::{NS_T, content_of, package, styles_of};
use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, Paragraph, para_block};

const NS_X: &str = r#"xmlns:xlink="http://www.w3.org/1999/xlink""#;

/// Each body paragraph: (its text, its style id, the fields in it as "instr", "[instr" for a
/// start, "]" for an end).
fn paras(d: &Document) -> Vec<(String, String, Vec<String>)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| {
            let fields = p
                .objects
                .iter()
                .filter_map(|o| match o {
                    InlineObject::Field { instr, .. } => Some(instr.clone()),
                    InlineObject::FieldStart { instr, .. } => Some(format!("[{instr}")),
                    InlineObject::FieldEnd => Some("]".into()),
                    _ => None,
                })
                .collect();
            (p.text.replace(OBJ, ""), p.props.style.clone().unwrap_or_default(), fields)
        })
        .collect()
}

fn open(body: &str) -> Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Contents_20_Heading" style:display-name="Contents Heading" style:family="paragraph"><style:text-properties fo:font-size="16pt" fo:font-weight="bold"/></style:style><style:style style:name="Contents_20_1" style:display-name="Contents 1" style:family="paragraph"/><style:style style:name="Contents_20_2" style:display-name="Contents 2" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.2in"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T} {NS_X}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Contents_20_1"/><style:style style:name="P2" style:family="paragraph" style:parent-style-name="Contents_20_2"/></office:automatic-styles><office:body><office:text>{body}<text:p>After.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

/// LibreOffice's own table of contents (a document it saved), its templates cut short.
const TOC: &str = r##"<text:table-of-content text:style-name="Sect1" text:protected="true" text:name="Table of Contents1"><text:table-of-content-source text:outline-level="3"><text:index-title-template text:style-name="Contents_20_Heading">Table of Contents</text:index-title-template><text:table-of-content-entry-template text:outline-level="1" text:style-name="Contents_20_1"><text:index-entry-text/><text:index-entry-page-number/></text:table-of-content-entry-template></text:table-of-content-source><text:index-body><text:index-title text:style-name="Sect1" text:name="Table of Contents1_Head"><text:p text:style-name="Contents_20_Heading">Table of Contents</text:p></text:index-title><text:p text:style-name="P1"><text:a xlink:type="simple" xlink:href="#__RefHeading___Toc4">Chapter 1<text:tab/>1</text:a></text:p><text:p text:style-name="P2"><text:a xlink:type="simple" xlink:href="#__RefHeading___Toc6">Section 1.1<text:tab/>2</text:a></text:p></text:index-body></text:table-of-content>"##;

const TOC_INSTR: &str = r#"TOC \o "1-3" \h \z \u"#;

#[test]
fn a_table_of_contents_opens_as_wordcrafts() {
    let d = open(TOC);
    assert_eq!(
        paras(&d),
        [
            // Its title carries the field, as a table WordCraft inserts does.
            ("Table of Contents".to_string(), "TOCHeading".to_string(), vec![TOC_INSTR.to_string()]),
            ("Chapter 1\t1".into(), "TOC1".into(), vec![]),
            ("Section 1.1\t2".into(), "TOC2".into(), vec![]),
            ("After.".into(), String::new(), vec![]),
        ]
    );
    // LibreOffice's Contents styles are WordCraft's TOC styles, with their look from the file.
    let toc2 = d.styles.get("TOC2").unwrap();
    assert_eq!(toc2.name, "TOC 2");
    assert!(toc2.para.indent_left.is_some_and(|l| (l - 14.4).abs() < 0.01), "{:?}", toc2.para.indent_left);
    assert!(d.styles.get("Contents1").is_none() && d.styles.get("Contents2").is_none());
}

#[test]
fn an_untitled_index_is_a_field_around_its_entries() {
    let d = open(
        r#"<text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="2"/><text:index-body><text:p text:style-name="P1">One<text:tab/>1</text:p><text:p text:style-name="P1">Two<text:tab/>2</text:p></text:index-body></text:table-of-content><text:alphabetical-index text:name="A"><text:alphabetical-index-source/><text:index-body><text:p>apple, 1</text:p></text:index-body></text:alphabetical-index><text:illustration-index text:name="F"><text:illustration-index-source text:caption-sequence-name="Drawing"/><text:index-body><text:p>Drawing 1: a cat<text:tab/>3</text:p></text:index-body></text:illustration-index>"#,
    );
    let p = paras(&d);
    assert_eq!(p[0].2, [r#"[TOC \o "1-2" \h \z \u"#]);
    assert_eq!((p[1].0.as_str(), p[1].2.as_slice()), ("Two\t2", ["]".to_string()].as_slice()));
    assert_eq!(p[2].2, ["[INDEX", "]"]);
    assert_eq!(p[3].2, [r#"[TOC \h \z \c "Drawing""#, "]"]);
}

#[test]
fn indexes_round_trip() {
    for body in [
        TOC.to_string(),
        r#"<text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="2"/><text:index-body><text:p text:style-name="P1">One</text:p></text:index-body></text:table-of-content><text:bibliography text:name="B"><text:bibliography-source/><text:index-body><text:p>Doe, J. (2020).</text:p></text:index-body></text:bibliography>"#.to_string(),
    ] {
        let d = open(&body);
        let bytes = export(&d).unwrap();
        let c = content_of(&bytes);
        assert!(c.contains("<text:table-of-content ") && c.contains("<text:index-body>"), "{c}");
        assert_eq!(paras(&import(&bytes).unwrap()), paras(&d), "{c}");
    }
}

#[test]
fn a_wordcraft_table_of_contents_is_written_as_an_odf_one() {
    // As References › Table of Contents makes it: a heading holding the field, then entries.
    let mut head = Paragraph::with_text("Contents", Default::default()).styled("TOCHeading");
    head.insert_object(
        head.len(),
        InlineObject::Field { instr: TOC_INSTR.into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
        &Default::default(),
    )
    .unwrap();
    let mut d = Document::new();
    d.body = vec![
        para_block(head),
        para_block(Paragraph::with_text("Intro\t1", Default::default()).styled("TOC1")),
        para_block(Paragraph::with_text("Detail\t2", Default::default()).styled("TOC2")),
        para_block(Paragraph::with_text("Body text.", Default::default())),
    ];
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    assert!(c.contains(r#"<text:table-of-content-source text:outline-level="3">"#), "{c}");
    assert!(c.contains("<text:index-title ") && c.contains(r#"text:style-name="Contents_20_1""#), "{c}");
    // The TOC styles are LibreOffice's Contents styles.
    assert!(styles_of(&bytes).contains(r#"style:name="Contents_20_2" style:display-name="Contents 2""#), "{}", styles_of(&bytes));
    let back = paras(&import(&bytes).unwrap());
    assert_eq!(back, paras(&d));
}

#[test]
fn hostile_indexes_do_not_panic() {
    let deep = r#"<text:table-of-content text:name="T"><text:index-body>"#.repeat(50);
    for body in [
        deep.as_str(),
        r#"<text:table-of-content><text:index-body><text:index-title><text:index-title><text:p>x"#,
        r#"<text:index-body><text:p>stray</text:p></text:index-body>"#,
        r#"<text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="99999999999"/><text:index-body><table:table><table:table-row><table:table-cell><text:p>cell</text:p></table:table-cell></table:table-row></table:table></text:index-body></text:table-of-content>"#,
        r#"<text:alphabetical-index><text:index-body></text:index-body></text:alphabetical-index>"#,
    ] {
        let d = open(body);
        let _ = export(&d).unwrap();
    }
}

#[test]
fn index_marks_are_xe_fields_and_written_back() {
    // An entry at a point (`text:alphabetical-index-mark`, its keys the levels above it) and one
    // over text (`-mark-start` … `-mark-end`, the entry being the marked text).
    let body = r#"<text:p>Fruit <text:alphabetical-index-mark text:string-value="apple" text:key1="Fruit"/>is sweet; <text:alphabetical-index-mark-start text:id="m1"/>bananas<text:alphabetical-index-mark-end text:id="m1"/> too.</text:p><text:p><text:alphabetical-index-mark text:string-value="a &quot;quoted&quot; one: here"/></text:p>"#;
    let content =
        format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    let d = import(&crate::odt::tests::package(&content, None)).unwrap();
    let xe: Vec<String> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let wordcraft_doc::para::InlineObject::Field { instr, .. } = o { Some(instr.clone()) } else { None })
        .collect();
    assert_eq!(xe, [r#"XE "Fruit:apple""#, r#"XE "bananas""#, r#"XE "a quoted one here""#]);
    // The marked text stays text.
    assert!(d.body.iter().filter_map(|b| b.as_para()).any(|p| p.text.contains("bananas too.")));
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"<text:alphabetical-index-mark text:string-value="apple" text:key1="Fruit"/>"#), "{c}");
    assert!(c.contains(r#"<text:alphabetical-index-mark text:string-value="bananas"/>"#), "{c}");
}
