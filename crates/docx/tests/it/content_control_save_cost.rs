//! What saving a document with many content controls costs (#13).
//!
//! A form-shaped document: a control per paragraph, inline, or a control around two paragraphs.
//! The writer used to look for each paragraph's markers in the whole set of markers, once per
//! paragraph, and to compare every control with every other one, so the save was quadratic in the
//! number of controls.

use std::time::Instant;

use wordcraft_doc::control::{ContentControl, ControlKind};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, para_block};
use wordcraft_docx::write;

/// One paragraph per control, the control inline in it.
fn inline_controls(n: usize) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    let props = CharProps::default();
    for _ in 0..n {
        let mut p = Paragraph::with_text("value: ", props.clone());
        let off = p.len();
        p.insert_object(off, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props)
            .expect("insert the start marker");
        let off = p.len();
        p.insert_object(off, InlineObject::ControlEnd, &props).expect("insert the end marker");
        doc.body.push(para_block(p));
    }
    doc
}

/// Two paragraphs per control, the control written around both of them.
fn block_controls(n: usize) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    let props = CharProps::default();
    for _ in 0..n {
        let mut open = Paragraph::new();
        let off = open.len();
        open.insert_object(off, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props)
            .expect("insert the start marker");
        doc.body.push(para_block(open));
        let mut close = Paragraph::with_text("value", props.clone());
        let off = close.len();
        close.insert_object(off, InlineObject::ControlEnd, &props).expect("insert the end marker");
        doc.body.push(para_block(close));
    }
    doc
}

/// How much more saving the bigger document costs than the smaller one, for the bigger one
/// `scale` times the size of the smaller one's: ~`scale` when linear, ~`scale`² when quadratic.
/// Each is the cheapest of several runs after a warm-up, the runs alternating so a busy spell
/// slows both sizes alike; the smaller document is saved `scale` times over per run, so both
/// timings are long against the clock's resolution.
fn cost_ratio(scale: usize, mut small: impl FnMut(), mut big: impl FnMut()) -> (f64, f64, f64) {
    small();
    big();
    let mut small_scaled = || (0..scale).for_each(|_| small());
    let (mut t1, mut tk) = (f64::INFINITY, f64::INFINITY);
    for _ in 0..5 {
        let a = Instant::now();
        small_scaled();
        t1 = t1.min(a.elapsed().as_secs_f64() / scale as f64);
        let b = Instant::now();
        big();
        tk = tk.min(b.elapsed().as_secs_f64());
    }
    (t1, tk, tk / t1.max(1e-9))
}

/// A document with many inline content controls saves in the time the controls take: the writer
/// looked for each paragraph's markers in the whole set of markers, once per paragraph, so a
/// document with 32,000 controls took four seconds to save where it takes a tenth of one now.
#[test]
fn a_document_with_many_inline_content_controls_saves_in_linear_time() {
    let scale = 4;
    let small = inline_controls(8_000);
    let big = inline_controls(8_000 * scale);
    let (t1, tk, ratio) = cost_ratio(
        scale,
        || {
            write(&small).expect("write");
        },
        || {
            write(&big).expect("write");
        },
    );
    eprintln!("t(n) {t1:.4} s, t(4n) {tk:.4} s, ratio {ratio:.1}");
    assert!(ratio < 8.0, "four times the controls took {ratio:.1}x the time ({t1:.4} s → {tk:.4} s): not linear");
}

/// The same for the controls written around whole paragraphs: the writer compared every control
/// with every other one to find the ones that clash, so the save was quadratic in their number.
#[test]
fn a_document_with_many_block_content_controls_saves_in_linear_time() {
    let scale = 4;
    let small = block_controls(8_000);
    let big = block_controls(8_000 * scale);
    let (t1, tk, ratio) = cost_ratio(
        scale,
        || {
            write(&small).expect("write");
        },
        || {
            write(&big).expect("write");
        },
    );
    eprintln!("t(n) {t1:.4} s, t(4n) {tk:.4} s, ratio {ratio:.1}");
    assert!(ratio < 8.0, "four times the controls took {ratio:.1}x the time ({t1:.4} s → {tk:.4} s): not linear");
}

/// The controls are written where they were: the plan that decides whether a control is written
/// inline or around whole paragraphs gives the same answer as it did before it was made faster.
#[test]
fn the_controls_are_written_where_they_were() {
    // Inline controls: both markers in the same paragraph, so the control is written in it.
    let bytes = write(&inline_controls(50)).expect("write");
    let back = wordcraft_docx::read(&bytes).expect("read back");
    let ranges = back.control_ranges(wordcraft_doc::StoryRef::Body);
    assert_eq!(ranges.len(), 50, "every control survives the round trip");
    for r in &ranges {
        assert_eq!(r.start.path, r.end.path, "the control is written inline: {:?}", r.start.path);
    }

    // Block controls: the markers in different paragraphs, so the control is written around them.
    let bytes = write(&block_controls(50)).expect("write");
    let back = wordcraft_docx::read(&bytes).expect("read back");
    let ranges = back.control_ranges(wordcraft_doc::StoryRef::Body);
    assert_eq!(ranges.len(), 50, "every control survives the round trip");
    for r in &ranges {
        assert_ne!(r.start.path, r.end.path, "the control is written around the paragraphs");
    }
}

/// The names of the parts of a package, in the order they are in it.
fn part_names(bytes: &[u8]) -> Vec<String> {
    let z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    z.file_names().map(str::to_string).collect()
}

/// The body of the document part of a package.
fn document_body(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut xml = String::new();
    {
        use std::io::Read;
        z.by_name("word/document.xml").expect("the document part").read_to_string(&mut xml).expect("read");
    }
    let rest = xml.split_once("<w:body>").expect("the body").1;
    let body = rest.split_once("</w:body>").expect("the end of the body").0;
    format!("<w:body>{body}</w:body>")
}

/// A document whose controls are written in an order and a place worth pinning: two inline
/// controls in one paragraph, and a control around a whole paragraph.
fn pinned_doc() -> Document {
    let props = CharProps::default();
    let mut doc = Document::new();
    doc.body.clear();
    // "before [a] between [b] after", the two controls inline in one paragraph.
    let mut p = Paragraph::with_text("before ", props.clone());
    let mut at = p.len();
    p.insert_object(at, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::Text { multi_line: false })) }, &props)
        .expect("the first marker");
    at = p.len();
    p.insert_text(at, "inside a", &props).expect("text");
    at = p.len();
    p.insert_object(at, InlineObject::ControlEnd, &props).expect("the first end marker");
    at = p.len();
    p.insert_text(at, " between ", &props).expect("text");
    at = p.len();
    p.insert_object(at, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props)
        .expect("the second marker");
    at = p.len();
    p.insert_text(at, "inside b", &props).expect("text");
    at = p.len();
    p.insert_object(at, InlineObject::ControlEnd, &props).expect("the second end marker");
    at = p.len();
    p.insert_text(at, " after", &props).expect("text");
    doc.body.push(para_block(p));
    // A control around a whole paragraph.
    let mut open = Paragraph::new();
    let at = open.len();
    open.insert_object(at, InlineObject::ControlStart { control: Box::new(ContentControl::new(ControlKind::RichText)) }, &props)
        .expect("the block marker");
    doc.body.push(para_block(open));
    let mut inside = Paragraph::with_text("in the block", props.clone());
    let at = inside.len();
    inside.insert_object(at, InlineObject::ControlEnd, &props).expect("the block end marker");
    doc.body.push(para_block(inside));
    doc
}

/// The body of the document part the writer writes for [`pinned_doc`], byte for byte: what the
/// writer wrote before it was made faster. The counts and the paragraphs in the test above say
/// that the controls are where they belong; this says that the rest is where it was too, down to
/// the order and the place of the markers inside a paragraph.
///
/// The body is what the plan decides: which controls are written inline and which around whole
/// paragraphs, and where. Nothing outside it can change with the plan, so it is what is pinned.
///
/// Regenerate it when the writer's output changes on purpose: write [`pinned_doc`] and take the
/// body of the document part.
#[test]
fn the_document_body_is_written_as_it_was() {
    let bytes = write(&pinned_doc()).expect("write");
    assert_eq!(document_body(&bytes), PINNED_BODY);
    // The parts are the same ones, so nothing is added to the package and nothing is lost.
    assert_eq!(part_names(&bytes), PINNED_PARTS);
}

/// The body of the document part of [`pinned_doc`], as the writer wrote it before it was made
/// faster.
const PINNED_PARTS: &[&str] = &[
    "[Content_Types].xml",
    "_rels/.rels",
    "word/document.xml",
    "word/_rels/document.xml.rels",
    "word/styles.xml",
    "word/settings.xml",
    "word/theme/theme1.xml",
    "docProps/core.xml",
    "docProps/app.xml",
];

const PINNED_BODY: &str = r#"<w:body><w:p><w:r><w:t xml:space="preserve">before </w:t></w:r><w:sdt><w:sdtPr><w:text/></w:sdtPr><w:sdtContent><w:r><w:t xml:space="preserve">inside a</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t xml:space="preserve"> between </w:t></w:r><w:sdt><w:sdtPr></w:sdtPr><w:sdtContent><w:r><w:t xml:space="preserve">inside b</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t xml:space="preserve"> after</w:t></w:r></w:p><w:sdt><w:sdtPr></w:sdtPr><w:sdtContent><w:p></w:p><w:p><w:r><w:t xml:space="preserve">in the block</w:t></w:r></w:p></w:sdtContent></w:sdt><w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720" w:num="1"/></w:sectPr></w:body>"#;
