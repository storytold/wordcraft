//! Right-to-left editing commands: paragraph direction, alignment as seen on the page, font
//! commands on Persian text, arrow keys in bidirectional text, DOCX save and open. Table and
//! section direction commands (#66) set `bidiVisual` and section `rtl` without touching text.

use serde_json::json;
use wordcraft_doc::props::Align;
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef, Table, para_block};

use crate::{Selection, Session};

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

/// A session whose caret sits in the first cell of a two-column table.
fn table_session(rtl: bool) -> Session {
    let mut t = Table::new(1, 2, 200.0);
    t.props.rtl = rtl;
    t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("العمود الأول", Default::default()))];
    t.rows[0].cells[1].blocks = vec![para_block(Paragraph::with_text("second", Default::default()))];
    let mut d = wordcraft_doc::Document::new();
    d.body = vec![Block::Table(t).into()];
    let mut s = Session::new(d);
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0]), off: 0 });
    s
}

#[test]
fn table_direction_sets_toggles_and_undoes() {
    let mut s = table_session(false);
    assert!(s.run("table.direction", &json!({})).is_ok(), "toggles on");
    assert!(s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.rtl);
    run(&mut s, "table.direction", json!({"direction": "ltr"}));
    assert!(!s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.rtl);
    run(&mut s, "table.direction", json!({"direction": "rtl"}));
    assert!(s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.rtl);
    assert!(s.run("table.direction", &json!({"direction": "sideways"})).is_err(), "unknown values are rejected");
    // Undo and redo cross the direction change.
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.rtl);
    run(&mut s, "edit.redo", json!({}));
    assert!(s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.rtl);
    // Cell text is untouched by the table's direction.
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("العمود الأول") && text.contains("second"), "logical text kept: {text:?}");
}

#[test]
fn table_direction_needs_a_table() {
    let mut s = session("متن", true);
    let r = s.run("table.direction", &json!({}));
    assert!(r.is_err(), "disabled outside a table: {r:?}");
}

#[test]
fn section_direction_sets_toggles_and_undoes() {
    let mut s = session("one\ntwo", false);
    assert!(!s.doc.last_section.rtl);
    run(&mut s, "layout.sectionDirection", json!({}));
    assert!(s.doc.last_section.rtl, "toggles on");
    // Layout follows: the body starts at the right with the gutter on the binding side.
    let l = s.layout();
    assert!((l.pages[0].body.x - 72.0).abs() < 0.5, "no gutter: body at the left margin");
    run(&mut s, "layout.sectionDirection", json!({"direction": "ltr"}));
    assert!(!s.doc.last_section.rtl);
    run(&mut s, "layout.sectionDirection", json!({"direction": "rtl"}));
    assert!(s.doc.last_section.rtl);
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.last_section.rtl);
    run(&mut s, "edit.redo", json!({}));
    assert!(s.doc.last_section.rtl);
}

#[test]
fn rtl_table_layout_mirrors_columns_but_keeps_reading_order() {
    let mut s = table_session(true);
    let l = s.layout();
    let x0 = l.caret(&Pos { story: StoryRef::Body, path: Path(vec![0, 0, 0, 0]), off: 0 }).unwrap().x;
    let x1 = l.caret(&Pos { story: StoryRef::Body, path: Path(vec![0, 0, 1, 0]), off: 0 }).unwrap().x;
    assert!(x0 > x1, "first logical column on the right ({x0} vs {x1})");
    // Tab still walks logical cells; the stored order never reverses.
    run(&mut s, "text.tab", json!({}));
    assert_eq!(s.sel.focus.path.0, vec![0, 0, 1, 0]);
    run(&mut s, "text.backTab", json!({}));
    assert_eq!(s.sel.focus.path.0, vec![0, 0, 0, 0]);
}
