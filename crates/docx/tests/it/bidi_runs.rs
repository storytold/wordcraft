//! Runs of mixed right-to-left and left-to-right text (#1543). WordCraft lays a paragraph out with
//! the Unicode Bidirectional Algorithm (UAX #9) from its text and direction alone, while Word
//! orders a run's characters by the run's `w:rtl` (ECMA-376 Part 1 §17.3.2.30): text in a run
//! without it is laid out left to right, so Hebrew words around a Latin name, and the punctuation
//! between them, change places. The writer therefore splits such a run where the resolved
//! direction changes and marks the right-to-left pieces `w:rtl`, as Word does with text typed
//! in a right-to-left paragraph.

use std::io::Read;

use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, para_block};

/// `word/document.xml` of a written package.
fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// The runs of the first paragraph of `xml`: (has `w:rtl`, text with `w:br` as `\n`).
fn runs(xml: &str) -> Vec<(bool, String)> {
    let body = xml.split("<w:body>").nth(1).unwrap();
    let para = body.split("</w:p>").next().unwrap();
    para.split("<w:r>")
        .skip(1)
        .map(|r| {
            let r = r.split("</w:r>").next().unwrap();
            let rtl = r.contains("<w:rtl/>");
            let mut text = String::new();
            for part in r.split('<').skip(1) {
                if part.starts_with("w:br/>") {
                    text.push('\n');
                } else if part.starts_with("w:t ") || part.starts_with("w:t>") {
                    text.push_str(part.split_once('>').unwrap().1);
                }
            }
            (rtl, text)
        })
        .collect()
}

fn arial() -> CharProps {
    CharProps { font: Some("Arial".into()), font_cs: Some("Arial".into()), size: Some(16.0), size_cs: Some(16.0), ..Default::default() }
}

fn doc_of(text: &str, rtl_para: bool, props: CharProps) -> Document {
    let mut p = Paragraph::with_text(text, props);
    if rtl_para {
        p.props.bidi = Some(true);
    }
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    d
}

fn written_runs(d: &Document) -> Vec<(bool, String)> {
    runs(&document_xml(&wordcraft_docx::write(d).unwrap()))
}

fn owned(v: &[(bool, &str)]) -> Vec<(bool, String)> {
    v.iter().map(|(r, t)| (*r, (*t).to_string())).collect()
}

#[test]
fn rtl_paragraph_splits_latin_out_of_rtl_runs() {
    let d = doc_of("עברית WordCraft 2026 ואחריה עוד עברית.", true, arial());
    assert_eq!(written_runs(&d), owned(&[(true, "עברית "), (false, "WordCraft 2026"), (true, " ואחריה עוד עברית.")]));
}

#[test]
fn rtl_paragraph_brackets_email_and_final_stop_take_the_paragraph_direction() {
    let d = doc_of("לפני (ABC 123) אחרי; דוא״ל test@example.com.", true, arial());
    assert_eq!(written_runs(&d), owned(&[(true, "לפני ("), (false, "ABC 123"), (true, ") אחרי; דוא״ל "), (false, "test@example.com"), (true, ".")]));
}

#[test]
fn line_breaks_stay_in_the_split_runs() {
    let d = doc_of("שלום עולם.\nעברית WordCraft 2026 ואחריה.", true, arial());
    assert_eq!(written_runs(&d), owned(&[(true, "שלום עולם.\nעברית "), (false, "WordCraft 2026"), (true, " ואחריה.")]));
}

#[test]
fn split_runs_keep_their_formatting() {
    let d = doc_of("עברית WordCraft עברית", true, arial());
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    let body = xml.split("<w:body>").nth(1).unwrap();
    assert_eq!(body.matches("<w:r>").count(), 3);
    assert_eq!(body.matches(r#"<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>"#).count(), 4, "three runs and the mark: {body}");
    assert_eq!(body.matches(r#"<w:szCs w:val="32"/>"#).count(), 4, "{body}");
}

#[test]
fn hebrew_inside_an_ltr_paragraph_is_marked_right_to_left() {
    let d = doc_of("English with עברית מעורבת inside.", false, CharProps::default());
    assert_eq!(written_runs(&d), owned(&[(false, "English with "), (true, "עברית מעורבת"), (false, " inside.")]));
}

#[test]
fn text_without_rtl_characters_is_one_run() {
    let d = doc_of("Plain English, 2026.", true, arial());
    assert_eq!(written_runs(&d), owned(&[(false, "Plain English, 2026.")]));
    let d = doc_of("Plain English, 2026.", false, arial());
    assert_eq!(written_runs(&d), owned(&[(false, "Plain English, 2026.")]));
}

#[test]
fn explicit_run_direction_is_kept() {
    // A run that already says its direction is written as it is, unsplit.
    let d = doc_of("עברית WordCraft", true, CharProps { rtl: Some(true), ..arial() });
    assert_eq!(written_runs(&d), owned(&[(true, "עברית WordCraft")]));
    let d = doc_of("עברית WordCraft", true, CharProps { rtl: Some(false), ..arial() });
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"<w:rtl w:val="0"/>"#), "{xml}");
    assert_eq!(runs(&xml).len(), 1);
}

#[test]
fn mixed_paragraph_round_trips_text_and_direction() {
    let text = "לפני (ABC 123) אחרי; דוא״ל test@example.com.\nעברית WordCraft 2026 ואחריה עוד עברית.";
    let d = doc_of(text, true, arial());
    let bytes = wordcraft_docx::write(&d).unwrap();
    let r = wordcraft_docx::read(&bytes).unwrap();
    let p = r.body.iter().find_map(|b| b.as_para()).unwrap();
    assert_eq!(p.text, text);
    assert_eq!(p.props.bidi, Some(true));
    for run in &p.runs {
        assert_eq!(run.props.font.as_deref(), Some("Arial"));
        assert_eq!(run.props.size, Some(16.0));
    }
    assert!(p.runs.iter().any(|r| r.props.rtl == Some(true)));
    // Saving again writes the same runs: the read-back direction is now explicit.
    assert_eq!(written_runs(&r), written_runs(&d));
}

// Word gives right-to-left text without a `w:lang w:bidi` its default complex-script language
// (Arabic), so its status bar, proofing and hyphenation call Hebrew "Arabic". Right-to-left
// pieces the writer marks say their language (§17.3.2.20), unless the run or a style already does.

/// `w:bidi` attribute of each run's `w:lang` in the first paragraph: (has `w:rtl`, bidi language).
fn run_langs(xml: &str) -> Vec<(bool, Option<String>)> {
    let body = xml.split("<w:body>").nth(1).unwrap();
    let para = body.split("</w:p>").next().unwrap();
    para.split("<w:r>")
        .skip(1)
        .map(|r| {
            let r = r.split("</w:r>").next().unwrap();
            let lang = r.split("<w:lang ").nth(1).and_then(|l| l.split("w:bidi=\"").nth(1)).map(|v| v.split('"').next().unwrap().to_string());
            (r.contains("<w:rtl/>"), lang)
        })
        .collect()
}

fn langs(d: &Document) -> Vec<(bool, Option<String>)> {
    run_langs(&document_xml(&wordcraft_docx::write(d).unwrap()))
}

fn he(rtl: bool) -> (bool, Option<String>) {
    (rtl, rtl.then(|| "he-IL".to_string()))
}

#[test]
fn hebrew_pieces_say_they_are_hebrew() {
    let d = doc_of("לפני (ABC 123) אחרי; דוא״ל test@example.com.", true, arial());
    // The final stop has no letters of its own; it takes the paragraph's language.
    assert_eq!(langs(&d), vec![he(true), he(false), he(true), he(false), he(true)]);
    let d = doc_of("English with עברית מעורבת inside.", false, CharProps::default());
    assert_eq!(langs(&d), vec![he(false), he(true), he(false)]);
}

#[test]
fn arabic_pieces_say_they_are_arabic() {
    let d = doc_of("مرحبا WordCraft بالعالم", true, arial());
    let ar = Some("ar-SA".to_string());
    assert_eq!(langs(&d), vec![(true, ar.clone()), (false, None), (true, ar)]);
}

#[test]
fn a_language_the_run_already_has_is_kept() {
    let d = doc_of("سلام WordCraft دنیا", true, CharProps { lang_bidi: Some("fa-IR".into()), ..arial() });
    let fa = Some("fa-IR".to_string());
    assert_eq!(langs(&d), vec![(true, fa.clone()), (false, fa.clone()), (true, fa)]);
    // A right-to-left proofing language on the run (w:val) names the right-to-left text too.
    let d = doc_of("שלום WordCraft", true, CharProps { lang: Some("yi".into()), ..arial() });
    assert_eq!(langs(&d), vec![(true, Some("yi".into())), (false, None)]);
}

#[test]
fn an_inherited_bidi_language_is_not_repeated() {
    let mut d = doc_of("سلام WordCraft دنیا", true, arial());
    d.styles.default_chr.lang_bidi = Some("fa-IR".into());
    assert_eq!(langs(&d), vec![(true, None), (false, None), (true, None)]);
}
