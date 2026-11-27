//! Theme colours through the editing commands (storytold/wordcraft#6256): a colour chosen
//! replaces the theme colour it had, one that looks the same included, where other formatting
//! keeps it; a new theme recolours what follows its colours without that counting as a
//! formatting change; and Compare sees a theme colour that replaced a fixed one.

use serde_json::json;
use wordcraft_doc::props::{CharColorSources, CharProps, ColorSource, ThemeColor, ThemeRef};
use wordcraft_doc::{Document, Paragraph, Pos, RevisionKind, Rgb, TextColor};

use crate::{Selection, Session};

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Text in `accent1` with a shade, as `doc`'s theme gives it, the record holding the colour the
/// file stored beside it.
fn themed(doc: &Document) -> CharProps {
    let theme = ThemeRef { color: ThemeColor::Accent1, tint: None, shade: Some(0xBF) };
    let shown = theme.resolve(&doc.settings.theme_colors, None).map(TextColor::Rgb);
    let source = ColorSource { theme, value: Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), shown };
    CharProps { color: shown, color_sources: Some(Box::new(CharColorSources { color: Some(source), ..Default::default() })), ..Default::default() }
}

/// One paragraph, "Themed Other", all in [`themed`].
fn document() -> Document {
    let mut d = Document::new();
    let props = themed(&d);
    d.body = vec![wordcraft_doc::para_block(Paragraph::with_text("Themed Other", props))];
    d
}

/// The text colour's theme colour record at `off` in the first paragraph.
fn record(s: &Session, off: usize) -> Option<ColorSource> {
    s.doc.body[0].as_para().and_then(|p| p.props_of_char(off).color_sources.as_deref().and_then(|c| c.color))
}

fn select(s: &mut Session, from: usize, to: usize) {
    s.sel = Selection { anchor: Pos::body(0, from), focus: Pos::body(0, to) };
}

/// Choosing the colour the text already looks like is a colour chosen: the theme colour goes
/// there, and only there; bold keeps it; undo brings it back.
#[test]
fn a_colour_chosen_replaces_the_theme_colour_even_one_that_looks_the_same() {
    let mut s = Session::new(document());
    let Some(TextColor::Rgb(shown)) = s.doc.body[0].as_para().and_then(|p| p.props_of_char(0).color) else { panic!("a colour") };
    let theirs = record(&s, 0);
    assert!(theirs.is_some());
    select(&mut s, 0, 12);
    run(&mut s, "format.bold", json!({"value": true}));
    assert_eq!((record(&s, 0), record(&s, 7)), (theirs, theirs), "bold sets no colour");
    select(&mut s, 0, 6);
    run(&mut s, "format.color", json!({"color": shown.hex()}));
    assert_eq!(s.doc.body[0].as_para().and_then(|p| p.props_of_char(0).color), Some(TextColor::Rgb(shown)));
    assert_eq!((record(&s, 0), record(&s, 7)), (None, theirs), "only the text it was chosen for");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(record(&s, 0), theirs);
}

/// A new theme shows its colours in the text that follows the theme's, and with Track Changes on
/// that isn't a formatting change of the text: its theme colour is the same.
#[test]
fn a_new_theme_recolours_theme_colours_without_a_tracked_change() {
    let mut s = Session::new(document());
    s.doc.settings.track_changes = true;
    let before = s.doc.body[0].as_para().and_then(|p| p.props_of_char(0).color);
    run(&mut s, "design.theme", json!({"name": "Studio"}));
    let theme = ThemeRef { color: ThemeColor::Accent1, tint: None, shade: Some(0xBF) };
    let now = s.doc.body[0].as_para().and_then(|p| p.props_of_char(0).color);
    assert_eq!(now, theme.resolve(&s.doc.settings.theme_colors, None).map(TextColor::Rgb));
    assert_ne!(now, before, "Studio's first accent isn't Craft's");
    assert!(record(&s, 0).is_some_and(|r| r.holds(now)), "still following accent1");
    // The theme's own style changes are tracked as before; the text that follows it isn't changed.
    let runs = s.doc.body[0].as_para().map(|p| p.runs.iter().map(|r| r.props.fmt_change.is_some()).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(runs, [false], "no tracked formatting change on the text");
}

/// Compared with a revision that gives text the theme colour its fixed colour looked like,
/// the change is found, and the revised text keeps its theme colour.
#[test]
fn compare_finds_a_theme_colour_that_looks_like_the_fixed_one_it_replaced() {
    let revised = document();
    let shown = themed(&revised).color;
    let mut base = Document::new();
    base.body = vec![wordcraft_doc::para_block(Paragraph::with_text("Themed Other", CharProps { color: shown, ..Default::default() }))];
    let path = std::env::temp_dir().join(format!("wc-compare-theme-{}.docx", std::process::id()));
    crate::io::save_path(&path, &revised).unwrap();
    let mut s = Session::new(base);
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    assert_eq!(r.unwrap()["changes"], 1);
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
    let props = s.doc.body[0].as_para().map(|p| p.props_of_char(0).clone()).unwrap_or_default();
    assert!(props.color_sources.as_deref().and_then(|c| c.color).is_some(), "the revised text's theme colour");
    let old = props.fmt_change.as_ref().map(|ch| ch.old.clone()).unwrap_or_default();
    assert_eq!((old.color, old.color_sources), (shown, None), "the fixed colour before");
}
