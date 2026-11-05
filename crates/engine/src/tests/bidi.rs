//! Right-to-left editing commands: paragraph direction, alignment as seen on the page, font
//! commands on Persian text, arrow keys in bidirectional text, DOCX save and open.

use serde_json::json;
use wordcraft_doc::props::Align;
use wordcraft_doc::{Pos, StoryRef};

use crate::Session;

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A session with `text` (paragraphs split on `\n`), every paragraph right to left when `rtl`.
fn session(text: &str, rtl: bool) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": text}));
    if rtl {
        run(&mut s, "select.all", json!({}));
        run(&mut s, "para.rtl", json!({}));
        run(&mut s, "caret.docStart", json!({}));
    }
    s
}

fn props(s: &Session, i: usize) -> wordcraft_doc::props::ParaProps {
    s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap().props.clone()
}

fn caret_x(s: &mut Session) -> f32 {
    let l = s.layout();
    l.caret(&s.sel.focus).unwrap().x
}

#[test]
fn direction_commands_change_only_the_reading_order() {
    let mut s = session("سلام دنیا\nHello", false);
    run(&mut s, "para.alignCenter", json!({}));
    run(&mut s, "para.rtl", json!({}));
    assert_eq!(props(&s, 0).bidi, Some(true));
    assert_eq!(props(&s, 0).align, Some(Align::Center), "direction doesn't touch alignment");
    // Left-to-right again, and `value: false` means the other direction.
    run(&mut s, "para.ltr", json!({}));
    assert_eq!(props(&s, 0).bidi, Some(false));
    run(&mut s, "para.ltr", json!({"value": false}));
    assert_eq!(props(&s, 0).bidi, Some(true));
    // A new right-to-left paragraph starts at the right margin (start-aligned).
    let mut s = session("متن", true);
    assert!((caret_x(&mut s) - 540.0).abs() < 0.5, "caret at {}", caret_x(&mut s));
}

#[test]
fn alignment_buttons_mean_what_they_show_in_rtl_paragraphs() {
    let mut s = session("متن فارسی", true);
    // Align Left in a right-to-left paragraph moves text to the left: its end edge.
    run(&mut s, "para.alignLeft", json!({}));
    assert_eq!(props(&s, 0).align, Some(Align::Right));
    let x_end = s.layout().caret(&Pos::body(0, "متن فارسی".len())).unwrap().x;
    assert!((x_end - 72.0).abs() < 0.5, "text ends at the left margin: {x_end}");
    // Align Right is the start edge; pressing it again keeps it there.
    run(&mut s, "para.alignRight", json!({}));
    assert_eq!(props(&s, 0).align, Some(Align::Left));
    run(&mut s, "para.alignRight", json!({}));
    assert_eq!(props(&s, 0).align, Some(Align::Left));
    // Logical values for scripts.
    run(&mut s, "para.align", json!({"value": "end"}));
    assert_eq!(props(&s, 0).align, Some(Align::Right));
    run(&mut s, "para.align", json!({"value": "right"}));
    assert_eq!(props(&s, 0).align, Some(Align::Left));
}

#[test]
fn para_set_on_page_follows_each_paragraphs_own_direction() {
    // The Paragraph dialog's alignment and indents are as seen on the page (#1346): over a
    // right-to-left and a left-to-right paragraph, Left is each one's own left side.
    let mut s = session("سلام دنیا\nHello", false);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "para.rtl", json!({}));
    run(&mut s, "select.all", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "para.set", json!({"props": {"spaceAfter": 6}, "onPage": {"align": "left", "left": 36}}));
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let (rtl, ltr) = (props(&s, 0), props(&s, 1));
    assert_eq!((rtl.align, rtl.indent_right, rtl.indent_left, rtl.space_after), (Some(Align::Right), Some(36.0), None, Some(6.0)));
    assert_eq!((ltr.align, ltr.indent_left, ltr.indent_right, ltr.space_after), (Some(Align::Left), Some(36.0), None, Some(6.0)));
    // A direction in `props` comes first: both are right to left now, so Right is their start.
    run(&mut s, "para.set", json!({"props": {"bidi": true}, "onPage": {"right": 18}}));
    assert_eq!((props(&s, 1).bidi, props(&s, 1).indent_left, props(&s, 1).indent_right), (Some(true), Some(18.0), None));
    assert!(s.run("para.set", &json!({"props": {}, "onPage": {"align": "middle"}})).is_err());
    // Undo takes each command back whole.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((props(&s, 0).align, props(&s, 0).indent_right, props(&s, 1).indent_left), (None, None, None));
}

#[test]
fn font_commands_reach_persian_text() {
    let mut s = session("کتاب خوب", true);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "format.size", json!({"size": 18}));
    run(&mut s, "format.font", json!({"name": "Vazirmatn"}));
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    let c = p.props_of_char(0).clone();
    let rc = s.doc.styles.resolve_char(None, &c).complex();
    assert_eq!((rc.font.as_str(), rc.size, rc.bold), ("Vazirmatn", 18.0, true), "the complex-script formatting follows");
    // Saved and reopened, Word sees bold and size for the Persian text too.
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
    let d = crate::io::open_bytes("x.docx", &bytes).unwrap();
    let c = d.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props_of_char(0).clone();
    let rc = d.styles.resolve_char(None, &c).complex();
    assert_eq!((rc.font.as_str(), rc.size, rc.bold), ("Vazirmatn", 18.0, true));
    assert_eq!(d.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.bidi, Some(true));
}

#[test]
fn font_commands_override_inherited_complex_script_properties_explicitly() {
    use wordcraft_doc::props::CharProps;

    let mut s = session("שלום ABC", true);
    s.doc.styles.default_chr.size_cs = Some(30.0);
    s.doc.styles.default_chr.bold_cs = Some(true);
    s.doc.styles.default_chr.italic_cs = Some(true);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({"value": false}));
    run(&mut s, "format.italic", json!({"value": false}));
    run(&mut s, "format.size", json!({"size": 18}));
    for (command, expected_size) in [
        (None, 18.0),
        (Some("format.growFont"), 20.0),
        (Some("format.shrinkFont"), 18.0),
        (Some("format.growFont1"), 19.0),
        (Some("format.shrinkFont1"), 18.0),
    ] {
        if let Some(command) = command {
            run(&mut s, command, json!({}));
        }
        let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let c = p.props_of_char(0);
        assert_eq!((c.size_cs, c.bold_cs, c.italic_cs), (Some(expected_size), Some(false), Some(false)));
        let d = crate::io::open_bytes("x.docx", &crate::io::save_bytes("x.docx", &s.doc).unwrap()).unwrap();
        let c = d.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props_of_char(0);
        let cs = d.styles.resolve_char(None, c).complex();
        assert_eq!((cs.size, cs.bold, cs.italic), (expected_size, false, false));
    }
    // The font dialog's command keeps formatting both scripts when given one value.
    run(&mut s, "format.set", json!({"props": {"size": 10, "bold": true, "italic": true}}));
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    let c: &CharProps = p.props_of_char(0);
    assert_eq!((c.size_cs, c.bold_cs, c.italic_cs), (Some(10.0), Some(true), Some(true)));
    // Callers can still supply distinct values explicitly, including false.
    run(&mut s, "format.set", json!({"props": {"size": 12, "sizeCs": 18, "bold": true, "boldCs": false, "italic": true, "italicCs": false}}));
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    let c = p.props_of_char(0);
    assert_eq!((c.size, c.size_cs, c.bold_cs, c.italic_cs), (Some(12.0), Some(18.0), Some(false), Some(false)));
}

#[test]
fn arrow_keys_move_visually_in_rtl_text() {
    let fa = "سلام دنیا";
    let mut s = session(fa, true);
    // From the start (the right edge), Left moves forward through the text.
    let x0 = caret_x(&mut s);
    run(&mut s, "caret.left", json!({}));
    assert!(s.sel.focus.off > 0, "Left moves to the next character in right-to-left text");
    assert!(caret_x(&mut s) < x0, "and the caret moves left on the screen");
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, 0);
    // Right at the start edge goes nowhere in the first paragraph (no previous one).
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, 0);
    // Left all the way reaches the end, then the next paragraph.
    let mut s = session(&format!("{fa}\nبعدی"), true);
    // (Stepping until the paragraph changes: the لا ligature is one caret position for two letters.)
    let mut steps = 0;
    while s.sel.focus.path.last() == 0 && steps < 40 {
        run(&mut s, "caret.left", json!({}));
        steps += 1;
    }
    assert!(steps <= fa.chars().count() + 1, "{steps} steps");
    assert_eq!(s.sel.focus.path.last(), 1, "past the left end of the line: the next paragraph");
    assert_eq!(s.sel.focus.off, 0, "at its start (right edge)");
    // Shift+Left selects forward in logical order.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.left", json!({"extend": true}));
    let (a, b) = s.sel.ordered();
    assert_eq!((a.off, b.off > 0), (0, true));
    // Left with a selection collapses to its left edge: the logical end.
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, b.off);
}

#[test]
fn arrow_keys_in_mixed_text_visit_each_position() {
    // A left-to-right paragraph with a Persian word: Right walks across the screen.
    let t = "ab سلام cd";
    let mut s = session(t, false);
    let mut xs = vec![caret_x(&mut s)];
    for _ in 0..30 {
        let before = s.sel.focus.clone();
        run(&mut s, "caret.right", json!({}));
        if s.sel.focus == before {
            break;
        }
        xs.push(caret_x(&mut s));
    }
    assert!(xs.windows(2).all(|w| w[1] > w[0]), "each Right moves right: {xs:?}");
    assert_eq!(s.sel.focus.off, t.len(), "ends at the end of the paragraph");
}

#[test]
fn word_moves_follow_reading_order() {
    let mut s = session("یک دو سه", true);
    run(&mut s, "caret.wordLeft", json!({}));
    assert!(s.sel.focus.off > 0, "Ctrl+Left goes to the next word in right-to-left text");
    run(&mut s, "caret.wordRight", json!({}));
    assert_eq!(s.sel.focus.off, 0);
}

#[test]
fn typing_persian_keeps_logical_order() {
    let mut s = session("", true);
    for w in ["سلام", " ", "WordCraft", " ", "۱۴۰۳"] {
        run(&mut s, "text.insert", json!({"text": w}));
    }
    assert_eq!(s.doc.plain_text(StoryRef::Body), "سلام WordCraft ۱۴۰۳");
    // The caret after typing a number at the end sits right after it on the screen.
    let l = s.layout();
    let c = l.caret(&s.sel.focus).unwrap();
    assert!(c.x < 540.0 && c.x > 72.0);
    // select.text finds Persian text.
    run(&mut s, "select.text", json!({"text": "سلام"}));
    assert_eq!(s.selected_text(), "سلام");
}

#[test]
fn explicit_alignment_sets_without_toggling() {
    for rtl in [false, true] {
        let mut s = session("متن", rtl);
        for v in ["center", "center", "right", "right", "left", "left"] {
            run(&mut s, "para.align", json!({"value": v}));
            let shown = props(&s, 0).align.unwrap_or_default().visual(rtl);
            let want = match v {
                "center" => Align::Center,
                "right" => Align::Right,
                _ => Align::Left,
            };
            assert_eq!(shown, want, "{v} twice stays {v} (rtl={rtl})");
        }
    }
}

#[test]
fn typing_kurdish_in_new_document_advances_caret_smoothly() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let mut first_x = None;
    let mut last_x = 0.0;
    for ch in ["م", "ن", " ", "ن", "ا", "و", "م", " ", "ه", "ێ", "ڤ", "ا", "ر", "ه"] {
        run(&mut s, "text.insert", json!({"text": ch}));
        let l = s.layout();
        let c = l.caret(&s.sel.focus).unwrap();
        if first_x.is_none() {
            first_x = Some(c.x);
        }
        last_x = c.x;
        // The caret must not jump back to the left margin (72.0), but stay near the right margin (540.0).
        assert!(c.x > 400.0, "caret should not be at left margin: {}", c.x);
    }
    assert!(last_x < first_x.unwrap(), "caret should advance leftwards overall");
    assert_eq!(props(&s, 0).bidi, Some(true), "paragraph auto-detected as right-to-left");
}

/// The runs of the first paragraph in a saved DOCX: (has `w:rtl`, text).
fn saved_runs(s: &Session) -> Vec<(bool, String)> {
    use std::io::Read;
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
    let mut xml = String::new();
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    let para = xml.split("<w:body>").nth(1).unwrap().split("</w:p>").next().unwrap().to_string();
    para.split("<w:r>")
        .skip(1)
        .map(|r| {
            let r = r.split("</w:r>").next().unwrap();
            let text: String = r.split("<w:t xml:space=\"preserve\">").skip(1).filter_map(|t| t.split("</w:t>").next()).collect();
            (r.contains("<w:rtl/>"), text)
        })
        .collect()
}

#[test]
fn latin_typed_into_reopened_hebrew_is_saved_left_to_right() {
    // #1543: a saved file's Hebrew runs say `w:rtl`. Latin typed next to them after reopening
    // takes its direction from its own letters, as on screen, so it's saved in a run of its own
    // without `w:rtl` (Word would otherwise lay it out right to left again).
    let s = session("לפני אחרי", true);
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
    let mut s = Session::new(crate::io::open_bytes("x.docx", &bytes).unwrap());
    assert!(s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().runs.iter().all(|r| r.props.rtl == Some(true)));
    s.sel = crate::Selection::caret(Pos::new(StoryRef::Body, wordcraft_doc::Path::top(0), "לפני ".len()));
    run(&mut s, "text.insert", json!({"text": "Foo Bar 2026 "}));
    let runs = saved_runs(&s);
    assert!(runs.contains(&(false, "Foo Bar 2026".to_string())), "{runs:?}");
    assert!(runs.iter().all(|(rtl, t)| !rtl || !t.chars().any(|c| c.is_ascii_alphanumeric())), "{runs:?}");
    assert_eq!(runs.iter().map(|(_, t)| t.as_str()).collect::<String>(), "לפני Foo Bar 2026 אחרי");
}

#[test]
fn a_latin_link_inserted_into_reopened_hebrew_is_saved_left_to_right() {
    // Other insertions start from the caret's formatting too (#1543): a link's Latin text
    // inserted between reopened Hebrew words isn't saved as right-to-left.
    let s = session("לפני אחרי", true);
    let bytes = crate::io::save_bytes("x.docx", &s.doc).unwrap();
    let mut s = Session::new(crate::io::open_bytes("x.docx", &bytes).unwrap());
    s.sel = crate::Selection::caret(Pos::new(StoryRef::Body, wordcraft_doc::Path::top(0), "לפני ".len()));
    run(&mut s, "insert.link", json!({"url": "https://example.com", "text": "WordCraft site"}));
    let runs = saved_runs(&s);
    assert!(runs.contains(&(false, "WordCraft site".to_string())), "{runs:?}");
    // The caret's own formatting, as Reveal Formatting shows it, still says right to left.
    s.sel = crate::Selection::caret(Pos::new(StoryRef::Body, wordcraft_doc::Path::top(0), "לפ".len()));
    s.pending = None; // moving the caret by hand doesn't drop the link's pending formatting
    assert_eq!(s.caret_props().rtl, Some(true));
    assert_eq!(s.typing_props().rtl, None);
    assert_eq!(s.typing_props().lang_bidi.as_deref(), Some("he-IL"), "the bidi language stays");
}
