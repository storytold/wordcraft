//! Theme colours through the editing commands (storytold/wordcraft#6256): a colour chosen
//! replaces the theme colour it had, one that looks the same included, where other formatting
//! keeps it; a new theme recolours what follows its colours without that counting as a
//! formatting change; Compare sees a theme colour that replaced a fixed one; and content that
//! Paste, Compare or a style copy brings in from under another theme keeps the colours it shows.

use std::sync::Arc;

use serde_json::json;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::{CharColorSources, CharProps, ColorSource, ThemeColor, ThemeRef};
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, RevisionKind, Rgb, StoryRef, Table, TextColor, para_block};

use crate::{Selection, Session};

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Text in `theme`, as `doc`'s theme gives it, the record holding the colour the file stored
/// beside it.
fn in_theme(doc: &Document, theme: ThemeRef) -> CharProps {
    let shown = theme.resolve(&doc.settings.theme_colors, None).map(TextColor::Rgb);
    let source = ColorSource { theme, value: Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), shown };
    CharProps { color: shown, color_sources: Some(Box::new(CharColorSources { color: Some(source), ..Default::default() })), ..Default::default() }
}

/// Text in `accent1` with a shade ([`in_theme`]).
fn themed(doc: &Document) -> CharProps {
    in_theme(doc, ThemeRef { color: ThemeColor::Accent1, tint: None, shade: Some(0xBF) })
}

/// Text in `accent2`, which [`other_theme`] gives the colour the default theme gives it ([`in_theme`]).
fn in_accent2(doc: &Document) -> CharProps {
    in_theme(doc, ThemeRef { color: ThemeColor::Accent2, tint: None, shade: None })
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

/// A new document whose theme gives `accent1` another colour than the default theme's.
fn other_theme() -> Document {
    let mut d = Document::new();
    d.settings.theme_colors[4] = Rgb(0x99, 0x33, 0x66);
    d
}

/// The colour of the first character of the first paragraph.
fn first_colour(d: &Document) -> Option<TextColor> {
    d.body.first().and_then(|b| b.as_para()).and_then(|p| p.props_of_char(0).color)
}

/// `d` saved as a DOCX and opened again.
fn reopened(d: &Document) -> Document {
    let bytes = crate::io::save_bytes("theme-colours.docx", d).unwrap();
    crate::io::open_bytes("theme-colours.docx", &bytes).unwrap()
}

/// Text copied in one open document and pasted into another keeps the colours it shows, once
/// saved and reopened too: as a fixed colour where the other document's theme gives its theme
/// colour another colour, with its theme colour where it gives the same.
#[test]
fn text_pasted_into_another_open_document_keeps_the_colours_it_shows() {
    for (into, follows) in [(Document::new(), true), (other_theme(), false)] {
        let mut s = Session::new(document());
        let shown = first_colour(&s.doc);
        select(&mut s, 0, 12);
        run(&mut s, "edit.copy", json!({}));
        s.add_document(into);
        run(&mut s, "edit.paste", json!({}));
        assert_eq!(first_colour(&s.doc), shown, "pasted");
        assert_eq!(record(&s, 0).is_some(), follows, "the theme colour stays only where it gives that colour");
        assert_eq!(first_colour(&reopened(&s.doc)), shown, "saved and reopened");
    }
}

/// Formatting picked up in one open document, by Copy Formatting or the Format Painter, and
/// pasted onto text in another keeps the colours it shows, as pasted text does.
#[test]
fn formatting_picked_up_in_another_open_document_keeps_the_colours_it_shows() {
    for pick in ["edit.copyFormat", "edit.formatPainter"] {
        for (mut into, follows) in [(Document::new(), true), (other_theme(), false)] {
            into.body = vec![para_block(Paragraph::with_text("Plain", CharProps::default()))];
            let mut s = Session::new(document());
            let shown = first_colour(&s.doc);
            select(&mut s, 0, 12);
            run(&mut s, pick, json!({}));
            s.add_document(into);
            select(&mut s, 0, 5);
            run(&mut s, "edit.pasteFormat", json!({}));
            assert_eq!(first_colour(&s.doc), shown, "{pick}: painted");
            assert_eq!(record(&s, 0).is_some(), follows, "{pick}: the theme colour stays only where it gives that colour");
            assert_eq!(first_colour(&reopened(&s.doc)), shown, "{pick}: saved and reopened");
        }
    }
}

/// Table cells pasted with the caret in a table cell (their rows inserted or the cells laid
/// over) keep the colours they show, as pasted text does.
#[test]
fn table_cells_pasted_into_a_table_under_another_theme_keep_the_colours_they_show() {
    let props = themed(&Document::new());
    let shown = props.color;
    let mut copied = Table::new(1, 2, 200.0);
    for (i, cell) in copied.rows[0].cells.iter_mut().enumerate() {
        cell.blocks = vec![para_block(Paragraph::with_text(&format!("Themed cell {i}"), props.clone()))];
    }
    let mut d = other_theme();
    d.body = vec![Arc::new(Block::Table(Table::new(1, 2, 200.0))), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    s.clipboard = Some(Fragment { blocks: vec![Block::Table(copied)], ..Default::default() });
    run(&mut s, "edit.paste", json!({}));
    let cell = |d: &Document| {
        let p = d.body[0].as_table().and_then(|t| t.rows[0].cells[0].blocks[0].as_para()).cloned().unwrap_or_default();
        (p.text.clone(), p.props_of_char(0).color)
    };
    assert_eq!(cell(&s.doc), ("Themed cell 0".to_string(), shown), "pasted into the table");
    assert_eq!(cell(&reopened(&s.doc)).1, shown, "saved and reopened");
}

/// Compared with a revision under another theme, the result (this document, under its own
/// theme) shows the revised text in the colours it shows there, once saved and reopened too.
#[test]
fn compare_with_a_revision_under_another_theme_keeps_the_colours_it_shows() {
    let mut revised = other_theme();
    revised.body = vec![para_block(Paragraph::with_text("Themed Other", themed(&revised)))];
    let shown = first_colour(&revised);
    let path = std::env::temp_dir().join(format!("wc-compare-other-theme-{}.docx", std::process::id()));
    crate::io::save_path(&path, &revised).unwrap();
    let mut base = Document::new();
    base.body = vec![para_block(Paragraph::with_text("Themed Other", CharProps::default()))];
    let mut s = Session::new(base);
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    r.unwrap();
    assert_eq!(first_colour(&s.doc), shown, "compared");
    assert_eq!(first_colour(&reopened(&s.doc)), shown, "saved and reopened");
}

/// Compared with a revision under another theme, text in a theme colour that both themes give
/// the same colour, on both sides, isn't a change, and keeps its theme colour.
#[test]
fn compare_under_another_theme_finds_no_change_in_a_theme_colour_both_give_alike() {
    let same_text = |mut d: Document| {
        d.body = vec![para_block(Paragraph::with_text("Same text", in_accent2(&d)))];
        reopened(&d)
    };
    let path = std::env::temp_dir().join(format!("wc-compare-shared-colour-{}.docx", std::process::id()));
    crate::io::save_path(&path, &same_text(other_theme())).unwrap();
    let mut s = Session::new(same_text(Document::new()));
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    assert_eq!(r.unwrap()["changes"], 0, "no change");
    assert!(record(&s, 0).is_some(), "still in accent2");
}

/// Styles copied from a template under another theme, by Update Styles and by the Organizer,
/// look as they did there, once saved and reopened too: in a theme colour the two themes give
/// different colours, the colour shown, fixed; in one they give the same colour, that theme
/// colour.
#[test]
fn styles_copied_from_a_template_under_another_theme_keep_the_colours_they_show() {
    let mut template = other_theme();
    let ids = ["Heading1", "Heading2"];
    let mut shown = Vec::new();
    for (id, props) in ids.into_iter().zip([themed(&template), in_accent2(&template)]) {
        shown.push(props.color);
        let st = Arc::make_mut(&mut template.styles).get_mut(id).unwrap_or_else(|| panic!("{id}"));
        st.chr.color = props.color;
        st.chr.color_sources = props.color_sources;
    }
    let path = std::env::temp_dir().join(format!("wc-template-other-theme-{}.dotx", std::process::id()));
    crate::io::save_path(&path, &template).unwrap();
    let file = path.to_string_lossy().to_string();
    let colours = |d: &Document| ids.map(|id| d.styles.get(id).and_then(|st| st.chr.color)).to_vec();
    let follows = |d: &Document| ids.map(|id| d.styles.get(id).is_some_and(|st| st.chr.color_sources.is_some())).to_vec();
    let mut seen = Vec::new();
    for (id, params) in [("tools.updateStyles", json!({"path": file})), ("styles.copyFrom", json!({"path": file, "styles": ids}))] {
        let mut s = Session::new(Document::new());
        let r = s.run(id, &params);
        seen.push((id, r.map(|_| (colours(&s.doc), follows(&s.doc), colours(&reopened(&s.doc)))).map_err(|e| e.to_string())));
    }
    let _ = std::fs::remove_file(&path);
    for (id, got) in seen {
        assert_eq!(
            got,
            Ok((shown.clone(), vec![false, true], shown.clone())),
            "{id}: copied (accent2 still a theme colour), then saved and reopened"
        );
    }
}
