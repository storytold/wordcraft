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

// Word draws a `w:rtl` run with the complex-script font, size, bold and italic alone: a bold,
// italic, 20 pt Courier New Hebrew run with no complex-script values was drawn, once marked
// `w:rtl`, plain, in the theme's complex-script font and at its default size (measured in Word
// for Windows). WordCraft falls back to the ordinary values, so the pieces the writer marks get
// those as complex-script values wherever nothing sets one.

/// The `w:rPr` of each run of the first paragraph of `xml`.
fn run_props(xml: &str) -> Vec<String> {
    let body = xml.split("<w:body>").nth(1).unwrap();
    let para = body.split("</w:p>").next().unwrap();
    para.split("<w:r>").skip(1).map(|r| r.split("</w:r>").next().unwrap().split("<w:t").next().unwrap().to_string()).collect()
}

/// How WordCraft draws each character of `d`'s first paragraph: its font, size, bold and italic,
/// the complex-script ones for a right-to-left letter.
fn drawn(d: &Document) -> Vec<(String, f32, bool, bool)> {
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    let mut out = Vec::new();
    for (range, props) in p.run_ranges() {
        let r = d.styles.resolve_char(p.props.style.as_deref(), props);
        for c in p.text.get(range).unwrap().chars() {
            let rc = if wordcraft_doc::bidi::has_rtl(&c.to_string()) { r.complex() } else { r.clone() };
            out.push((rc.font, rc.size, rc.bold, rc.italic));
        }
    }
    out
}

#[test]
fn right_to_left_pieces_keep_the_font_size_bold_and_italic_they_were_drawn_with() {
    let courier = CharProps { font: Some("Courier New".into()), size: Some(20.0), bold: Some(true), italic: Some(true), ..Default::default() };
    let d = doc_of("שלום WordCraft עולם", true, courier);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let runs = run_props(&document_xml(&bytes));
    assert_eq!(runs.len(), 3, "{runs:?}");
    for r in &runs {
        let rtl = r.contains("<w:rtl/>");
        assert_eq!(r.contains(r#"w:cs="Courier New""#), rtl, "{r}");
        assert_eq!(r.contains("<w:bCs/>"), rtl, "{r}");
        assert_eq!(r.contains("<w:iCs/>"), rtl, "{r}");
        assert_eq!(r.contains(r#"<w:szCs w:val="40"/>"#), rtl, "{r}");
    }
    // Read back, every character is drawn as it was.
    let r = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(drawn(&r), drawn(&d));
}

#[test]
fn inherited_ordinary_values_become_the_complex_script_ones() {
    // The run sets nothing: its font, size and bold come from the document defaults.
    let mut d = doc_of("English with עברית inside.", false, CharProps::default());
    d.styles.default_chr.font = Some("Courier New".into());
    d.styles.default_chr.size = Some(18.0);
    d.styles.default_chr.bold = Some(true);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let runs = run_props(&document_xml(&bytes));
    let rtl: Vec<&String> = runs.iter().filter(|r| r.contains("<w:rtl/>")).collect();
    assert_eq!(rtl.len(), 1, "{runs:?}");
    for want in [r#"w:cs="Courier New""#, "<w:bCs/>", r#"<w:szCs w:val="36"/>"#] {
        assert!(rtl[0].contains(want), "{want}: {}", rtl[0]);
    }
    assert!(!rtl[0].contains("<w:iCs"), "not italic: {}", rtl[0]);
    assert_eq!(drawn(&wordcraft_docx::read(&bytes).unwrap()), drawn(&d));
}

#[test]
fn a_run_already_marked_complex_script_gets_no_complex_script_values() {
    // `w:cs` already has Word draw the run with its complex-script values (a bold run with no
    // `w:bCs` is plain), so marking it `w:rtl` changes nothing and nothing is added.
    let d = doc_of(
        "שלום WordCraft",
        true,
        CharProps { cs: Some(true), font: Some("Courier New".into()), size: Some(20.0), bold: Some(true), italic: Some(true), ..Default::default() },
    );
    let runs = run_props(&document_xml(&wordcraft_docx::write(&d).unwrap()));
    let rtl = runs.iter().find(|r| r.contains("<w:rtl/>")).unwrap_or_else(|| panic!("{runs:?}"));
    for added in ["w:cs=\"Courier New\"", "<w:bCs", "<w:iCs", "<w:szCs"] {
        assert!(!rtl.contains(added), "{added}: {rtl}");
    }
}

#[test]
fn table_style_formatting_becomes_the_complex_script_values() {
    // The table style makes its text bold and 20 pt, and its header row italic: WordCraft draws
    // the Hebrew in the cells so, and the pieces marked `w:rtl` must say so in complex-script
    // values for Word to draw them alike.
    use wordcraft_doc::styles::{PartFormat, Style, StyleKind, TablePart};
    let mut st = Style {
        id: "HebrewTable".into(),
        name: "HebrewTable".into(),
        kind: StyleKind::Table,
        chr: CharProps { bold: Some(true), size: Some(20.0), ..Default::default() },
        ..Default::default()
    };
    st.set_table_part(TablePart::FirstRow, PartFormat { chr: CharProps { italic: Some(true), ..Default::default() }, ..Default::default() });
    let mut d = Document::new();
    d.styles.upsert(st);
    let mut t = wordcraft_doc::Table::new(2, 1, 468.0);
    t.props.style = Some("HebrewTable".into());
    for row in &mut t.rows {
        for cell in &mut row.cells {
            cell.blocks = vec![para_block(Paragraph::with_text("שלום", CharProps::default()))];
        }
    }
    d.body = vec![std::sync::Arc::new(wordcraft_doc::Block::Table(t)), para_block(Paragraph::with_text("", CharProps::default()))];
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    let cells: Vec<&str> = xml.split("<w:tc>").skip(1).map(|c| c.split("</w:tc>").next().unwrap()).collect();
    assert_eq!(cells.len(), 2);
    for (row, cell) in cells.iter().enumerate() {
        let run = cell.split("<w:r>").nth(1).unwrap_or_else(|| panic!("row {row}: {cell}"));
        assert!(run.contains("<w:rtl/>"), "row {row}: {run}");
        assert!(run.contains("<w:bCs/>") && run.contains(r#"<w:szCs w:val="40"/>"#), "row {row}: {run}");
        assert_eq!(run.contains("<w:iCs/>"), row == 0, "only the header row is italic: {run}");
    }
}

#[test]
fn complex_script_values_already_set_are_not_repeated() {
    let mut d = doc_of("שלום WordCraft", true, CharProps { size: Some(20.0), bold: Some(true), ..arial() });
    d.styles.default_chr.font_cs = Some("Arial".into());
    d.styles.default_chr.bold_cs = Some(false);
    // `arial()` sets the run's own complex-script font and size; the defaults set bold.
    let runs = run_props(&document_xml(&wordcraft_docx::write(&d).unwrap()));
    let rtl = runs.iter().find(|r| r.contains("<w:rtl/>")).unwrap();
    assert!(rtl.contains(r#"<w:szCs w:val="32"/>"#) && !rtl.contains(r#"<w:szCs w:val="40"/>"#), "the run's own szCs: {rtl}");
    assert!(!rtl.contains("<w:bCs"), "bold comes from the defaults' bCs: {rtl}");
}
