//! Fields with formats and references (ODF 1.3 §7.3–7.7, §16.27 data styles): dates and times
//! in their date style (Word's `\@` picture), sequences numbering figures and tables (`SEQ`,
//! with a bookmark over the caption's label and number for references to it), chapter fields
//! (`STYLEREF`), reference marks and references to them and to sequences (`REF`, `PAGEREF`).

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, OBJ};

const NS_N: &str = r#"xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#;

/// Each paragraph's fields (code, result) and bookmarks ("[name", "]name").
fn items(d: &Document) -> Vec<Vec<String>> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| {
            p.objects
                .iter()
                .filter_map(|o| match o {
                    InlineObject::Field { instr, result, .. } => Some(format!("{instr} = {result}")),
                    InlineObject::BookmarkStart { name } => Some(format!("[{name}")),
                    InlineObject::BookmarkEnd { name } => Some(format!("]{name}")),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// LibreOffice's own (from a document it saved).
fn open() -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_N}><office:automatic-styles>
<number:date-style style:name="N1"><number:day-of-week number:style="long"/><number:text>, </number:text><number:month number:style="long" number:textual="true"/><number:text> </number:text><number:day number:style="long"/><number:text>, </number:text><number:year number:style="long"/></number:date-style>
<number:date-style style:name="N2"><number:day number:style="long"/><number:text>/</number:text><number:month number:style="long"/><number:text>/</number:text><number:year/></number:date-style>
<number:time-style style:name="N3"><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text> </number:text><number:am-pm/></number:time-style>
<number:time-style style:name="N4"><number:hours/><number:text> o'clock</number:text></number:time-style>
</office:automatic-styles><office:body><office:text>
<text:h text:outline-level="1">Chapter one</text:h>
<text:p><text:date style:data-style-name="N1">Sunday, November 01, 2026</text:date> <text:date style:data-style-name="N2">01/11/26</text:date> <text:time style:data-style-name="N3">08:45 AM</text:time> <text:time style:data-style-name="N4">8 o'clock</text:time></text:p>
<text:p>In <text:chapter text:display="name" text:outline-level="1">Chapter one</text:chapter> (<text:chapter text:display="number" text:outline-level="2"/>)</text:p>
<text:p>Figure <text:sequence text:ref-name="refFigure0" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">1</text:sequence>: a cat</text:p>
<text:p>Table <text:sequence text:ref-name="refTable0" text:name="Table" text:formula="ooow:Table+1" style:num-format="I">I</text:sequence>: prices</text:p>
<text:p>Marked <text:reference-mark-start text:name="m1"/>here<text:reference-mark-end text:name="m1"/> and <text:reference-mark text:name="m2"/>.</text:p>
<text:p>See <text:sequence-ref text:reference-format="category-and-value" text:ref-name="refFigure0">Figure 1</text:sequence-ref> on page <text:sequence-ref text:reference-format="page" text:ref-name="refFigure0">1</text:sequence-ref>, <text:reference-ref text:reference-format="text" text:ref-name="m1">here</text:reference-ref> (<text:reference-ref text:reference-format="page" text:ref-name="m1">1</text:reference-ref>).</text:p>
</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn formatted_and_referring_fields_open_as_words() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        items(&open()),
        [
            vec![],
            s(&[
                r#"DATE \@ "dddd, MMMM dd, yyyy" = Sunday, November 01, 2026"#,
                r#"DATE \@ "dd/MM/yy" = 01/11/26"#,
                r#"TIME \@ "hh:mm AM/PM" = 08:45 AM"#,
                r#"TIME \@ "H' o''clock'" = 8 o'clock"#,
            ]),
            s(&[r#"STYLEREF "Heading 1" = Chapter one"#, r#"STYLEREF "Heading 2" \n = "#]),
            // The bookmark over the label and number is what references to the caption show.
            s(&["[refFigure0", r#"SEQ Figure \* ARABIC = 1"#, "]refFigure0"]),
            s(&["[refTable0", r#"SEQ Table \* ROMAN = I"#, "]refTable0"]),
            s(&["[m1", "]m1", "[m2", "]m2"]),
            s(&[r#"REF refFigure0 \h = Figure 1"#, r#"PAGEREF refFigure0 \h = 1"#, r#"REF m1 \h = here"#, r#"PAGEREF m1 \h = 1"#]),
        ]
    );
    // The caption's bookmark starts before its label.
    let p = open().body[3].as_para().unwrap().clone();
    assert!(p.text.starts_with(OBJ), "{:?}", p.text);
}

#[test]
fn formatted_and_referring_fields_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    for el in ["<number:date-style ", "<text:sequence ", "<text:sequence-decl ", "<text:chapter ", "style:data-style-name="] {
        assert!(c.contains(el), "{el} in {c}");
    }
    assert_eq!(items(&import(&bytes).unwrap()), items(&d));
}

#[test]
fn hostile_formats_and_references_do_not_panic() {
    for body in [
        r#"<text:p><text:date style:data-style-name="missing">x</text:date><text:sequence>no name</text:sequence><text:chapter text:outline-level="99999"/></text:p>"#,
        r#"<text:p><text:sequence text:name="A B" text:ref-name="">1</text:sequence><text:sequence-ref text:ref-name="">?</text:sequence-ref><text:reference-mark-end text:name="never"/></text:p>"#,
    ] {
        let content = format!(
            r#"<office:document-content {NS_T} {NS_N}><office:automatic-styles><number:date-style style:name="N1"><number:text>{}</number:text></number:date-style></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#,
            "'\"\\".repeat(500)
        );
        let d = import(&package(&content, None)).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn a_page_number_keeps_its_own_format() {
    // On a page numbered i, ii…, a page number field may show 1, 2… (or another format).
    let content = format!(
        r#"<office:document-content {NS_T} {NS_N}><office:body><office:text><text:p><text:page-number style:num-format="1" text:select-page="current">1</text:page-number> <text:page-number style:num-format="A" text:select-page="current">A</text:page-number> <text:page-number text:select-page="current">i</text:page-number></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let got = items(&d);
    assert_eq!(got[0], [r#"PAGE \* Arabic = 1"#, r#"PAGE \* ALPHABETIC = A"#, "PAGE = i"]);
    assert_eq!(items(&import(&export(&d).unwrap()).unwrap()), got);
}
