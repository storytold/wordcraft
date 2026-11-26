use super::cursor::caret_color_at;
use super::pages::view_key;
use super::*;
use crate::table_resize::tests::{H, screen, steps};
// Frames go through the app's input hook (`embed::before_input`) as in the app: command states, fired commands and
// focus requests reach the runtime only that way (`test_harness::frame`).
use crate::test_harness::AppFrames as _;
use std::collections::HashSet;

/// Rests the pointer for as long as the framework's tooltip waits before it shows a tool's tip
/// (`tooltip::DELAY`, the framework's look), at the harness's default step of a quarter second.
fn rest_for_tip(h: &mut H) {
    let frames = (craft::ui::contract::tooltip::DELAY / 0.25).ceil() as usize + 3;
    steps(h, frames);
}

mod markup;
mod overlays;

/// #3931: while Insert › Table's grid asks for a preview the page is laid out with that
/// table; the document, its revision and undo history stay as they were, and without a
/// request the page is the document's own again.
#[test]
fn the_page_shows_the_table_the_grid_previews() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    let cells = |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).count();
    let (doc, rev) = (app.session.doc.clone(), app.session.rev());
    app.canvas.table_preview = Some((2, 3));
    assert_eq!(cells(&shown_layout(&mut app)), 6);
    assert_eq!(app.canvas.table_preview, None, "a request lasts one frame");
    assert_eq!((&app.session.doc, app.session.rev(), app.session.can_undo()), (&doc, rev, false));
    app.canvas.table_preview = Some((2, 3));
    let again = shown_layout(&mut app);
    app.canvas.table_preview = Some((2, 3));
    assert!(std::sync::Arc::ptr_eq(&again, &shown_layout(&mut app)), "a still pointer reuses the preview");
    assert_eq!(cells(&shown_layout(&mut app)), 0, "no request: the document's own page");
}

/// #3931 review: the preview's pages are drawn from the copy they were laid out from. The
/// table moves "Kites" down a block, so the document's own block there is not its paragraph.
#[test]
fn the_previewed_pages_are_drawn_from_the_previewed_document() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    app.session.run("caret.end", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let layout = shown_layout(&mut app);
    let text = |doc: &wordcraft_doc::Document, story, path| doc.para(story, path).map(|p| p.text.clone());
    let mut kites = 0;
    for it in layout.pages.iter().flat_map(|p| &p.items) {
        let Placed::Lines { story, path, .. } = it else { continue };
        let shown = text(page_doc(&app), *story, path);
        assert!(shown.is_some(), "{path:?} names a paragraph of the drawn document");
        kites += usize::from(shown.as_deref() == Some("Kites"));
    }
    assert_eq!(kites, 1, "Kites is drawn as itself");
    assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
    assert!(std::ptr::eq(page_doc(&app), &app.session.doc), "no preview: the document's own pages");
}

/// #3931 review: with a selection the table replaces it in the copy, so the page draws the
/// copy's caret, in the table, and no selection: the document's selection names text that has
/// moved in the preview's layout.
#[test]
fn the_preview_draws_the_copy_caret_and_selection() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    app.session.sel = wordcraft_engine::Selection { anchor: Pos::body(0, 3), focus: Pos::body(1, 2) };
    app.canvas.table_preview = Some((2, 2));
    let layout = shown_layout(&mut app);
    let sel = preview_sel(&app).expect("the preview's selection").clone();
    assert!(sel.is_collapsed(), "the table took the selection's place");
    assert_ne!(sel, app.session.sel);
    let c = layout.caret_on(&sel.focus, 0).expect("the caret is on the preview's page");
    assert!(layout.pages[c.page].items.iter().any(|it| matches!(it, Placed::Lines { path, .. } if path.0.len() == 4 && *path == sel.focus.path)));
    assert!(sel.focus.path.cell().is_some(), "the caret is in the table");
    assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
    assert!(preview_sel(&app).is_none(), "no preview: the document's own selection");
}

/// #3931 review: the preview is laid out again when the caret moves or the view changes with
/// the grid still hovered, so the page shows the table where a click would put it, at the
/// view's width.
#[test]
fn the_preview_follows_the_caret_and_the_view() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
    let tables =
        |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).map(|(_, p)| p.0[0]).collect::<HashSet<u32>>();
    app.session.run("caret.docEnd", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let at_end = tables(&shown_layout(&mut app));
    app.session.run("caret.docStart", &json!({})).unwrap();
    app.canvas.table_preview = Some((2, 2));
    let at_start = tables(&shown_layout(&mut app));
    assert_ne!(at_start, at_end, "the caret moved: so did the table");
    let (_, _, fresh) = app.session.preview_layout("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    assert_eq!(at_start, tables(&fresh));
    app.session.view.mode = wordcraft_layout::ViewMode::Web;
    app.session.view.web_width = 300.0;
    app.canvas.table_preview = Some((2, 2));
    let web = shown_layout(&mut app);
    assert_eq!(web.pages[0].w, app.session.layout().pages[0].w, "laid out at the Web view's width");
}

/// #2545: notes belong to the body editing context, not the faded header/footer context.
#[test]
fn editing_notes_keeps_the_body_and_active_note_opaque() {
    use wordcraft_layout::display::{Draw, page_display};
    for command in ["references.footnote", "references.endnote"] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Body marker.")), Default::default());
        app.run(command, json!({"text": "Note marker."})).unwrap();
        let note = app.session.sel.focus.story;
        assert!(matches!(part_kind(&app, note), Some(PartKind::Footnote | PartKind::Endnote)));
        app.run("insert.header", json!({"text": "Header marker."})).unwrap();
        let header = app.session.sel.focus.story;
        for story in [note, StoryRef::Body, header] {
            app.session.sel = wordcraft_engine::Selection::caret(app.session.doc.end_of(story));
            let layout = app.session.layout();
            for dark in [false, true] {
                app.session.view.dark_mode = dark;
                let opts = page_render_options(&app, dims_body(&app, &layout));
                let draws = page_display(&app.session.doc, &layout.pages[0], &opts.display);
                for marker in ["Body", "Note", "Header"] {
                    let alphas: Vec<f32> = draws
                        .iter()
                        .filter_map(|d| match d {
                            Draw::Glyphs { text, alpha, .. } if text.contains(marker) => Some(*alpha),
                            _ => None,
                        })
                        .collect();
                    assert!(!alphas.is_empty(), "{command}: {marker} is drawn");
                    let active = if marker == "Header" { story == header } else { story != header };
                    assert!(
                        alphas.iter().all(|a| if active { *a == 1.0 } else { *a > 0.0 && *a < 1.0 }),
                        "{command}, {story:?}, dark={dark}, {marker}: {alphas:?}"
                    );
                }
            }
        }
    }
}

/// #2177: only a link to a program-made hidden bookmark hides its name in the hover tip.
#[test]
fn hover_tip_hides_generated_bookmark_names_only() {
    assert!(is_generated_bookmark_link("#_Toc123456"));
    assert!(is_generated_bookmark_link("#_Ref42"));
    assert!(!is_generated_bookmark_link("#Chapter1"));
    assert!(!is_generated_bookmark_link("https://example.com/_x"));
    assert!(!is_generated_bookmark_link(""));
}

/// #299: the caret is black on light backgrounds and white on dark ones, whatever the UI
/// theme: on white paper, a dark page colour, a dark page (Switch Modes) and a highlight.
#[test]
fn caret_contrasts_with_what_lies_under_it() {
    use wordcraft_doc::props::Rgb;
    assert_eq!(caret_contrast(Rgb::WHITE), Color32::BLACK);
    assert_eq!(caret_contrast(Rgb::BLACK), Color32::WHITE);
    assert_eq!(caret_contrast(Rgb(0xFF, 0xFF, 0x00)), Color32::BLACK, "yellow highlight");
    assert_eq!(caret_contrast(Rgb(0x00, 0x00, 0x80)), Color32::WHITE, "dark blue");
    assert_eq!(caret_contrast(Rgb(0x33, 0x33, 0x33)), Color32::WHITE, "dark paper");
    assert_eq!(caret_contrast(Rgb(0xC0, 0xC0, 0xC0)), Color32::BLACK, "light grey");

    let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
    let mut a = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let at = |a: &mut WordApp, off: usize| {
        let layout = a.session.layout();
        let c = layout.caret_on(&Pos::body(0, off), 0).unwrap();
        let view = view_key(a, false, None);
        caret_color_at(a, &layout, c.page, c.x, c.top + c.height / 2.0, view, false)
    };
    assert_eq!(at(&mut a, 2), Color32::BLACK, "white paper");
    a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page colour");
    a.run("design.pageColor", json!({})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::BLACK, "page colour removed");
    a.session.view.dark_mode = true;
    assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page");
    a.session.view.dark_mode = false;
    a.run("select.text", json!({"text": "Hello"})).unwrap();
    a.run("format.highlight", json!({"color": "black"})).unwrap();
    assert_eq!(at(&mut a, 2), Color32::WHITE, "inside a black highlight");
    assert_eq!(at(&mut a, 9), Color32::BLACK, "past it");
    assert_eq!(at(&mut a, 2), Color32::WHITE, "again, from the cache");
}

/// Read Mode › View › Page Color: the reading columns are drawn on white, sepia or a dark
/// page, in place of the document's page color; other views keep the page color.
#[test]
fn read_mode_page_color_sets_the_reading_paper() {
    use wordcraft_doc::props::Rgb;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
    a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
    let paper = |a: &WordApp| page_render_options(a, false).paper_of(&a.session.doc);
    assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "Print Layout shows the page color");
    let print_key = view_key(&a, false, None);
    a.run("view.readMode", json!({})).unwrap();
    assert_eq!(paper(&a), Rgb::WHITE, "None: white, the page color ignored");
    let none_key = view_key(&a, false, None);
    a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
    assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
    assert!(!page_render_options(&a, false).dark, "Sepia keeps the text's colors");
    let sepia_key = view_key(&a, false, None);
    a.run("view.readPageColor", json!({"value": "inverse"})).unwrap();
    assert!(page_render_options(&a, false).dark);
    let d = wordcraft_render::DARK_PAPER;
    assert_eq!(paper(&a), Rgb(d, d, d));
    let inverse_key = view_key(&a, false, None);
    let keys = [print_key, none_key, sepia_key, inverse_key];
    for (i, k) in keys.iter().enumerate() {
        assert!(!keys[..i].contains(k), "each paper has its own view key");
    }
    a.run("view.printLayout", json!({})).unwrap();
    assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "leaving Read Mode brings the page color back");
    assert!(!page_render_options(&a, false).dark);
    // With Switch Modes on, Read Mode's paper still follows the Page color, which is kept from
    // the last entry: Inverse is dark, None is white and Sepia is sepia.
    a.session.view.dark_mode = true;
    a.run("view.readMode", json!({})).unwrap();
    assert!(page_render_options(&a, false).dark, "Inverse is kept");
    a.run("view.readPageColor", json!({"value": "none"})).unwrap();
    assert!(!page_render_options(&a, false).dark);
    assert_eq!(paper(&a), Rgb::WHITE);
    a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
    assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
    a.run("view.printLayout", json!({})).unwrap();
    assert!(page_render_options(&a, false).dark, "Print Layout keeps Switch Modes");
}

/// What the canvas draws over the pages (ink, object drag previews, table drag arrows) and the
/// paper drawn under a page before its raster follow Read Mode's Page color, not Switch Modes.
#[test]
fn drawing_over_the_pages_follows_the_read_mode_page_color() {
    use wordcraft_doc::props::Rgb;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
    a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
    let blank = |a: &WordApp| page_render_options(a, false).blank_paper();
    let grey = Rgb(wordcraft_render::DARK_PAPER, wordcraft_render::DARK_PAPER, wordcraft_render::DARK_PAPER);
    a.session.view.dark_mode = true;
    assert!(dark_page(&a), "Switch Modes in Print Layout");
    assert_eq!(blank(&a), grey);
    a.run("view.readMode", json!({})).unwrap();
    assert!(!dark_page(&a), "None in Read Mode, with Switch Modes on");
    assert_eq!(blank(&a), Rgb::WHITE);
    a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
    assert!(!dark_page(&a));
    assert_eq!(blank(&a), wordcraft_render::SEPIA_PAPER);
    a.session.view.dark_mode = false;
    a.run("view.readPageColor", json!({"value": "inverse"})).unwrap();
    assert!(dark_page(&a), "Inverse in Read Mode, with Switch Modes off");
    assert_eq!(blank(&a), grey);
    a.run("view.printLayout", json!({})).unwrap();
    assert!(!dark_page(&a));
    assert_eq!(blank(&a), Rgb::WHITE, "the paper under the raster, not the page color");
}

/// #299: the caret's colour is worked out only when the caret is drawn: with text selected
/// there is no caret, so the page's draw list isn't built for it.
#[test]
fn caret_colour_is_not_worked_out_while_the_caret_is_hidden() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
    a.run("select.text", json!({"text": "Hello"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert!(h.state().canvas.caret_bg.is_none(), "no caret, no colour lookup");
}

/// Show revisions in balloons: a document whose only revision is a deletion gets the markup
/// area for its balloon; the other modes have nothing to show there, and neither does the view
/// while the Comments pane is open.
#[test]
fn a_deletion_alone_opens_the_markup_area_when_revisions_go_in_balloons() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("document.setText", json!({"text": "keep gone"})).unwrap();
    a.run("review.trackChanges", json!({"value": true})).unwrap();
    a.run("select.range", json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 9}})).unwrap();
    a.run("text.delete", json!({})).unwrap();
    assert_eq!(markup_width(&a), 0.0, "comments and formatting only: nothing to show");
    a.run("review.trackingOptions", json!({"balloons": "revisions"})).unwrap();
    assert!(markup_width(&a) > 0.0, "the deletion's balloon needs the markup area");
    a.run("view.commentsPane", json!({"value": true})).unwrap();
    assert_eq!(markup_width(&a), 0.0);
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("review.trackingOptions", json!({"insertionsDeletions": false})).unwrap();
    assert_eq!(markup_width(&a), 0.0, "deletions not shown at all");
}

/// #549: double-click a word and drag: the selection grows by whole words; triple-click and
/// drag: by whole paragraphs (Word's behaviour).
#[test]
fn multi_click_drag_extends_by_units() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Alpha beta gamma delta\rSecond para\rThird para"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let screen = |h: &mut H, pos: Pos| {
        let c = h.state_mut().session.layout().caret(&pos).expect("caret");
        page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen")
    };
    let button = |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let multi_drag = |h: &mut H, from: Pos2, to: Pos2, clicks: usize| {
        h.event(Event::PointerMoved(from));
        for i in 0..clicks {
            h.event(button(from, true));
            h.app_frame();
            if i + 1 < clicks {
                h.event(button(from, false));
                h.app_frame();
            }
        }
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.app_frame();
        }
        h.event(button(to, false));
        for _ in 0..3 {
            h.app_frame();
        }
    };
    // Double-click inside "beta", drag into "gamma": "beta gamma " is selected.
    let (from, to) = (screen(&mut h, Pos::body(0, 8)), screen(&mut h, Pos::body(0, 13)));
    multi_drag(&mut h, from, to, 2);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!((s, e), (Pos::body(0, 6), Pos::body(0, 17)), "whole words");
    // Let the double-click time pass, then triple-click the second paragraph and drag into the third.
    for _ in 0..40 {
        h.app_frame();
    }
    let (from, to) = (screen(&mut h, Pos::body(1, 3)), screen(&mut h, Pos::body(2, 2)));
    multi_drag(&mut h, from, to, 3);
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(1, 0), Pos::body(2, 10)), "whole paragraphs");
    // A plain double-click (no drag) still selects just the word.
    h.state_mut().run("select.collapse", json!({})).unwrap();
    for _ in 0..40 {
        h.app_frame();
    }
    let at = screen(&mut h, Pos::body(0, 1));
    multi_drag(&mut h, at, at, 2);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!((s, e), (Pos::body(0, 0), Pos::body(0, 6)));
}

/// #3880: the status bar names the page being looked at. With the caret scrolled out of view, that
/// is the page at the top of the window; while the caret is on screen, its page.
#[test]
fn the_status_bar_names_the_page_in_view_not_only_the_caret_page() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for _ in 0..11 {
        a.run("text.insert", json!({"text": "Page text"})).unwrap();
        a.run("layout.break", json!({"kind": "page"})).unwrap();
    }
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    assert_eq!(h.state_mut().session.layout().pages.len(), 12);
    let caret_page_label = "Page 1 of 12";
    assert!(h.query_by_label(caret_page_label).is_some(), "the caret is on page 1 and in view");
    assert!(h.state().canvas.page_rects().any(|(page, _)| page == 0), "page 1 is on screen");
    // Scroll to page 9 without touching the caret.
    let (rev, sel) = (h.state().session.rev(), h.state().session.sel.clone());
    h.state_mut().canvas.view.go_to(8);
    for _ in 0..4 {
        h.app_frame();
    }
    assert!(h.state().canvas.view.offset.1 > 0.0, "the view scrolled down");
    // The page the window shows at its top is the one the status bar names, whatever the caret's page is.
    let top = h.state().canvas.page_rects().min_by(|a, b| a.1.min.y.abs().total_cmp(&b.1.min.y.abs())).map(|(page, _)| page);
    assert_eq!(top, Some(8), "the window shows page 9 at its top");
    assert!(h.query_by_label(caret_page_label).is_none(), "the status bar no longer names the caret's page");
    assert!(h.query_by_label(&format!("Page {} of 12", top.unwrap_or(0) + 1)).is_some(), "it names the page in view");
    assert_eq!((h.state().session.rev(), &h.state().session.sel), (rev, &sel), "scrolling changes neither the document nor the selection");
}

/// `viewed_page`: the caret's page while any of its line is on screen, else the window's top page.
#[test]
fn the_page_in_view_follows_the_caret_while_it_is_on_screen() {
    let tops: Vec<f64> = (0..4).map(|i| f64::from(i) * 110.0).collect();
    let rects: Vec<craft::behavior::kurbo::Rect> = tops.iter().map(|&y| craft::behavior::kurbo::Rect::new(0.0, y, 80.0, y + 100.0)).collect();
    let layout = craft::pages::layout::PagedLayout { rects, size: Default::default(), columns: 1, bands: vec![(0.0, 100.0); 4] };
    // The window's top edge in the layout's coordinates.
    let scrolled = |top: f64| top;
    let caret = |page: usize, y: f64| Some((page, tops[page] + y, tops[page] + y + 10.0));
    // Window 0..150 shows page 0 and the top of page 1.
    assert_eq!(viewed_page(caret(1, 5.0), scrolled(0.0), &layout, 150.0), 1, "the caret is on screen, on page 1");
    assert_eq!(viewed_page(caret(0, 5.0), scrolled(0.0), &layout, 150.0), 0);
    // Scrolled to page 3: the caret on page 0 is off screen.
    assert_eq!(viewed_page(caret(0, 5.0), scrolled(330.0), &layout, 100.0), 3);
    assert_eq!(viewed_page(None, scrolled(220.0), &layout, 100.0), 2, "no caret: the top page");
    assert_eq!(viewed_page(caret(2, 90.0), scrolled(330.0), &layout, 100.0), 3, "a caret just above the window is not in view");
}

/// #856: the left margin beside body text is Word's selection bar. A click selects the line,
/// a double-click the paragraph, a triple-click (or Ctrl/⌘+click) the document; dragging
/// selects whole lines and Shift+click extends by lines.
#[test]
fn selection_bar_clicks_select_lines() {
    use egui::{Event, Modifiers, PointerButton};
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(8);
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": format!("{}\rSecond para\rThird para", long.trim_end())})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    // A point in the left margin beside the line `li` lines below the one holding `pos`.
    let margin = |h: &mut H, pos: Pos, li: f32| {
        let c = h.state_mut().session.layout().caret(&pos).expect("caret");
        page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height * (li + 0.5)).expect("on screen")
    };
    let line_start = |h: &mut H, at: Pos2| {
        let placed = h.state().canvas.shown.pages.iter().find(|p| p.rect.contains(at)).expect("page").clone();
        let p = placed.to_page(at);
        let layout = h.state_mut().session.layout();
        layout.selection_bar_at(placed.page, p.x as f32, p.y as f32).expect("in the selection bar")
    };
    let span = |h: &mut H, pos: &Pos| h.state_mut().session.layout().line_span(pos, 0).expect("line");
    let button = |at: Pos2, pressed: bool, modifiers: Modifiers| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    let clicks_drag = |h: &mut H, from: Pos2, to: Pos2, clicks: usize, mods: Modifiers| {
        h.event(Event::PointerMoved(from));
        for i in 0..clicks {
            h.event(button(from, true, mods));
            h.app_frame();
            if i + 1 < clicks {
                h.event(button(from, false, mods));
                h.app_frame();
            }
        }
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.app_frame();
        }
        h.event(button(to, false, mods));
        for _ in 0..3 {
            h.app_frame();
        }
    };
    let pause = |h: &mut H| {
        for _ in 0..40 {
            h.app_frame();
        }
    };
    // Click beside the second line: that whole line is selected, not a caret at its start.
    let at = margin(&mut h, Pos::body(0, 0), 1.0);
    let l2 = line_start(&mut h, at);
    assert!(l2.off > 0 && l2.path == wordcraft_doc::Path::top(0));
    clicks_drag(&mut h, at, at, 1, Modifiers::NONE);
    let (_, l2_end, _) = span(&mut h, &l2);
    assert_eq!(h.state().session.sel.ordered(), (l2.clone(), l2_end.clone()), "the line");
    // Double-click: the paragraph, mark included.
    pause(&mut h);
    clicks_drag(&mut h, at, at, 2, Modifiers::NONE);
    assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(1, 0)), "the paragraph");
    // Triple-click: the whole document.
    pause(&mut h);
    clicks_drag(&mut h, at, at, 3, Modifiers::NONE);
    assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(2, 10)), "everything");
    // Drag from beside the first line down past the third: three whole lines.
    pause(&mut h);
    let (from, to) = (margin(&mut h, Pos::body(0, 0), 0.0), margin(&mut h, Pos::body(0, 0), 2.0));
    let l3 = line_start(&mut h, to);
    clicks_drag(&mut h, from, to, 1, Modifiers::NONE);
    let (_, l3_end, _) = span(&mut h, &l3);
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), l3_end), "three lines");
    // Shift+click beside "Second para" extends by lines, to that paragraph's mark.
    pause(&mut h);
    let second = margin(&mut h, Pos::body(1, 0), 0.0);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.app_frame();
    clicks_drag(&mut h, second, second, 1, Modifiers::SHIFT);
    h.event(Event::ModifiersChanged(Modifiers::default()));
    h.app_frame();
    let sel = h.state().session.sel.clone();
    assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), Pos::body(2, 0)), "extended by lines");
    // Hovering the bar shows the arrow, not the I-beam.
    h.event(Event::PointerMoved(at));
    h.app_frame();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
}

/// A right-to-left paragraph's selection bar is the right margin; the left margin beside it
/// is nothing special (a click there is the caret's).
#[test]
fn selection_bar_of_a_right_to_left_paragraph_is_on_the_right() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "First para\r\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e9}\u{5dc}\u{5d5}\u{5dd}\rThird para"})).unwrap();
    a.run("caret.set", json!({"pos": serde_json::to_value(Pos::body(1, 0)).unwrap()})).unwrap();
    a.run("para.rtl", json!({})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(1, 0)).expect("caret");
    let (body_x, page_w) = {
        let l = h.state_mut().session.layout();
        (l.pages[c.page].body.right(), l.pages[c.page].w)
    };
    let y = c.top + c.height / 2.0;
    let right = page_to_screen(h.state_mut(), c.page, (body_x + page_w) / 2.0, y).expect("on screen");
    let left = page_to_screen(h.state_mut(), c.page, 30.0, y).expect("on screen");
    let click = |h: &mut H, at: Pos2| {
        h.event(Event::PointerMoved(at));
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.app_frame();
        }
    };
    h.event(Event::PointerMoved(right));
    h.app_frame();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the bar's arrow on the right");
    click(&mut h, right);
    let (s, e) = h.state().session.sel.ordered();
    assert_eq!(s, Pos::body(1, 0), "the line is selected");
    assert!(e > s && (e.path == wordcraft_doc::Path::top(1) || e == Pos::body(2, 0)), "{s:?}..{e:?}");
    // Beside it on the left: no bar (not the bar's arrow, and a caret, not a line selection).
    // The cursor there is the text cursor, or none at all on some platforms' test windows.
    h.event(Event::PointerMoved(left));
    h.app_frame();
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    click(&mut h, left);
    assert!(h.state().session.sel.is_collapsed(), "a click in the left margin beside it is a caret, not a line");
}

/// #3453: while the Format Painter is armed the pointer over the text says so, and it goes
/// back once the painter is off. The selection bar keeps its arrow while it is armed.
#[test]
fn an_armed_format_painter_changes_the_pointer() {
    use craft::behavior::cursor::{Cursor, CursorIcon, PaintedCursor};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Paint this formatting"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let c = h.state_mut().session.layout().caret(&Pos::body(0, 6)).expect("caret");
    let over_text = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
    let over_bar = page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height / 2.0).expect("on screen");
    // The cursor the pass settled on, painted cursors included.
    let cursor_after = |h: &mut H, cmd: Option<&str>, at: Pos2| -> Option<Cursor> {
        if let Some(cmd) = cmd {
            h.state_mut().run(cmd, json!({})).unwrap();
        }
        h.event(egui::Event::PointerMoved(at));
        h.app_frame();
        craft::ui::runtime::cursor::current(&h.ctx)
    };
    // The shell the harness runs with decides whether the I-beam is painted (a native Windows shell) or the system's.
    let painted = {
        let env = craft::ui::runtime::env::Env::of_root(&h.ctx);
        let shell = env.view().shell();
        shell.os == craft::host::target::Os::Windows && shell.runtime == craft::host::target::Runtime::Native
    };
    let text = if painted { Cursor::Painted(PaintedCursor::IBeam) } else { Cursor::System(CursorIcon::Text) };
    assert_eq!(cursor_after(&mut h, None, over_text), Some(text.clone()), "no painter yet");
    assert_eq!(cursor_after(&mut h, Some("edit.formatPainter"), over_text), Some(Cursor::System(CursorIcon::Copy)), "armed");
    assert_eq!(cursor_after(&mut h, Some("edit.formatPainter"), over_text), Some(text), "turned off");
    h.state_mut().run("edit.formatPainter", json!({})).unwrap();
    let bar = cursor_after(&mut h, None, over_bar);
    assert!(matches!(bar, None | Some(Cursor::System(CursorIcon::Default))), "the selection bar keeps its arrow: {bar:?}");
}

/// #4410: with the Format Painter armed, a double-click on a word paints that word, and the
/// painter goes off again (it was only dragging across the text that painted it).
#[test]
fn double_clicking_a_word_with_the_format_painter_paints_it() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Bold source then target words"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("select.text", json!({"text": "Bold"})).unwrap();
    a.run("format.bold", json!({})).unwrap();
    a.run("edit.formatPainter", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    assert!(h.state().session.painter.is_some(), "armed");
    let word_off = "Bold source then ".len() + 2;
    let c = h.state_mut().session.layout().caret(&Pos::body(0, word_off)).expect("caret");
    let at = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
    h.event(Event::PointerMoved(at));
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.app_frame();
    }
    for _ in 0..3 {
        h.app_frame();
    }
    let s = &h.state().session;
    let para = s.doc.para_at(&Pos::body(0, 0)).expect("paragraph");
    assert_eq!(para.props_of_char("Bold source then t".len()).bold, Some(true), "the double-clicked word is painted");
    assert_ne!(para.props_of_char("Bold source ".len()).bold, Some(true), "the other words are not");
    assert!(s.painter.is_none(), "the painter is used up");
}

/// A typed character is laid out and painted in the frame that delivers the key, not the next
/// one: the pages that frame drew are the layout with the typed character.
#[test]
fn a_keystroke_is_painted_in_its_own_frame() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    let ctx = h.ctx.clone();
    h.state_mut().canvas.focus_page(&ctx);
    for _ in 0..6 {
        h.app_frame();
    }
    assert!(h.state().canvas.focused(&ctx), "the canvas has the keyboard focus");
    h.event(egui::Event::Text("x".into()));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Some textx");
    let latest = h.state_mut().session.layout();
    let painted = h.state().canvas.page_keys.layout.clone();
    assert!(painted.is_some_and(|l| std::sync::Arc::ptr_eq(&l, &latest)), "the frame painted the layout with the typed character");
}

/// #960: double-clicking the top margin of a document without a header creates one and
/// edits it (the bottom margin: a footer), as in Word; typing goes into it, and
/// double-clicking the body leaves it. One left empty is taken away again.
#[test]
fn double_click_margin_creates_header_and_footer() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let double_click = |h: &mut H, at: Pos2| {
        h.event(Event::PointerMoved(at));
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.app_frame();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.app_frame();
        }
        // Let the double-click time pass before the next one.
        for _ in 0..40 {
            h.app_frame();
        }
    };
    let kind = |h: &H| part_kind(h.state(), h.state().session.sel.focus.story);
    let page = h.state_mut().session.layout().pages[0].clone();
    assert!(page.header_story.is_none() && page.footer_story.is_none());
    let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
    let body = page_to_screen(h.state_mut(), 0, x, page.body.y + 6.0).expect("body on screen");
    // Top margin: a new header, the caret in it, typing goes there.
    let at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
    double_click(&mut h, at);
    assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
    let header = h.state().session.sel.focus.story;
    h.event(Event::Text("Running head".into()));
    for _ in 0..3 {
        h.app_frame();
    }
    assert_eq!(h.state().session.doc.plain_text(header), "Running head");
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Body text");
    // Double-click the body: back in it, the header kept.
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(header));
    // Bottom margin: a new footer; left empty, leaving takes it away again.
    let at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
    double_click(&mut h, at);
    assert_eq!(kind(&h), Some(PartKind::Footer), "editing a new footer");
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert!(h.state().session.doc.last_section.footers.default.is_none(), "the empty footer is gone");
    // And a written footer stays.
    double_click(&mut h, at);
    let footer = h.state().session.sel.focus.story;
    assert_eq!(kind(&h), Some(PartKind::Footer));
    h.event(Event::Text("Page foot".into()));
    h.app_frame();
    double_click(&mut h, body);
    assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
    assert_eq!(h.state().session.doc.plain_text(footer), "Page foot");
}

/// #2826: editing the default header, then turning on Different First Page: a double-click in
/// page 1's top margin goes into page 1's own (new) header, and typing lands there.
#[test]
fn double_click_reaches_the_first_page_header_from_another_header() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("insert.header", json!({"text": "Head"})).unwrap();
    let head = a.session.sel.focus.story;
    a.run("layout.differentFirstPage", json!({"value": true})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let page = h.state_mut().session.layout().pages[0].clone();
    assert!(page.header_story.is_none());
    let at = page_to_screen(h.state_mut(), 0, page.body.x + 20.0, page.body.y / 2.0).expect("top margin on screen");
    h.event(Event::PointerMoved(at));
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.app_frame();
    }
    let first = h.state().session.sel.focus.story;
    assert_eq!(part_kind(h.state(), first), Some(PartKind::Header));
    assert_ne!(first, head, "page 1's own header, not the default one");
    h.event(Event::Text("Cover".into()));
    for _ in 0..3 {
        h.app_frame();
    }
    assert_eq!(h.state().session.doc.plain_text(first), "Cover");
    assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(first));
}

/// #1026: while editing the header, double-clicking the bottom margin switches to the
/// footer (and back), as in Word, instead of selecting a word in the header.
#[test]
fn double_click_switches_between_header_and_footer() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Body text"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let double_click = |h: &mut H, at: Pos2| {
        h.event(Event::PointerMoved(at));
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.app_frame();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.app_frame();
        }
        for _ in 0..40 {
            h.app_frame();
        }
    };
    let kind = |h: &H| part_kind(h.state(), h.state().session.sel.focus.story);
    let page = h.state_mut().session.layout().pages[0].clone();
    let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
    let head_at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
    let foot_at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
    double_click(&mut h, head_at);
    assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
    let header = h.state().session.sel.focus.story;
    h.event(Event::Text("Head".into()));
    for _ in 0..3 {
        h.app_frame();
    }
    // Header -> footer.
    double_click(&mut h, foot_at);
    assert_eq!(kind(&h), Some(PartKind::Footer), "header -> footer band double-click switches");
    assert!(h.state().session.sel.is_collapsed(), "no word selected");
    assert_eq!(h.state().session.doc.plain_text(header), "Head");
    h.event(Event::Text("Foot".into()));
    for _ in 0..3 {
        h.app_frame();
    }
    let footer = h.state().session.sel.focus.story;
    // Footer -> header.
    double_click(&mut h, head_at);
    assert_eq!(h.state().session.sel.focus.story, header, "footer -> header band double-click switches");
    assert!(h.state().session.sel.is_collapsed(), "no word selected");
    assert_eq!(h.state().session.doc.plain_text(footer), "Foot");
    assert_eq!(h.state().session.doc.plain_text(header), "Head");
}

#[test]
fn only_web_and_email_links_are_followed() {
    for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
        assert!(is_followable_link(ok), "{ok}");
    }
    for bad in [
        "file:///etc/passwd",
        "file://C:/Windows/System32/calc.exe",
        "C:\\Windows\\System32\\calc.exe",
        "/usr/bin/xterm",
        "javascript:alert(1)",
        "ms-msdt:/id",
        "smb://host/share",
        " https://example.com",
        "\thttps://example.com",
        "https\u{0}://example.com",
        "",
        "example.com",
    ] {
        assert!(!is_followable_link(bad), "{bad:?}");
    }
}

/// Ctrl+click on a link to a bookmark puts the caret at the bookmark's start, as Word does;
/// only Go To selects the bookmark's text (#1236).
#[test]
fn a_link_to_a_bookmark_puts_the_caret_at_its_start() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Alpha beta gamma\nSee here")), Default::default());
    a.run("select.text", json!({"text": "beta"})).unwrap();
    a.run("insert.bookmark", json!({"name": "Spot"})).unwrap();
    a.run("select.text", json!({"text": "here"})).unwrap();
    a.run("insert.link", json!({"url": "#Spot"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // Between the link's "h" and "e".
    let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(1), off: 5 }).unwrap();
    let at = screen(&mut h, c.x, c.top + c.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.app_frame();
    let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
    h.event_modifiers(press, egui::Modifiers::COMMAND);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h, 3);
    let s = &h.state().session;
    let (start, _) = wordcraft_engine::cmd::insert::bookmark_range(&s.doc, "Spot").unwrap();
    assert!(s.sel.is_collapsed(), "a caret, not a selection: {:?}", s.sel);
    assert_eq!(s.sel.focus, start, "at the bookmark's start");
}

/// #2945: only a `REF` or `PAGEREF` with `\h` and a bookmark makes a link.
#[test]
fn only_hyperlinked_reference_fields_make_links() {
    let field = |instr: &str| wordcraft_doc::InlineObject::Field {
        instr: instr.into(),
        result: "x".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    assert_eq!(field_link(&field(r"REF _Ref000000001 \h")).as_deref(), Some("#_Ref000000001"));
    assert_eq!(field_link(&field(r"PAGEREF Spot \h")).as_deref(), Some("#Spot"));
    assert_eq!(field_link(&field(r"ref Spot \* MERGEFORMAT \H")).as_deref(), Some("#Spot"));
    for plain in [r"REF Spot", r"REF \h", "REF", "", r"PAGE \h", r"TOC \h", r"SEQ Figure \h"] {
        assert_eq!(field_link(&field(plain)), None, "{plain:?}");
    }
}

/// #2945: a cross-reference (`REF … \h`) is a link to its target: Ctrl+click on it goes there,
/// and its hover tip names no bookmark.
#[test]
fn ctrl_click_on_a_cross_reference_goes_to_its_target() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Rock\nSee ")), Default::default());
    a.run("select.text", json!({"text": "Rock"})).unwrap();
    a.run("references.caption", json!({"label": "Figure", "text": "A rock"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("insert.crossReference", json!({"to": "figure", "target": "Figure 1", "show": "number"})).unwrap();
    let caption = a.session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Caption"))).unwrap();
    let last = a.session.doc.body.len() - 1;
    assert_ne!(caption, last);
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // The field sits after "See ": click between its two edges.
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    let (before, after) = {
        let l = h.state_mut().session.layout();
        let pos = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(last), off };
        (l.caret(&pos(4)).unwrap(), l.caret(&pos(4 + obj)).unwrap())
    };
    let at = screen(&mut h, (before.x + after.x) / 2.0, before.top + before.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    h.app_frame();
    let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
    h.event_modifiers(press, egui::Modifiers::COMMAND);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h, 3);
    let s = &h.state().session;
    assert_eq!(s.sel.focus.path.last(), caption, "the caret went to the caption: {:?}", s.sel);
}

/// #3368: right-clicking a link dismisses the hover tooltip. A tooltip draws above the
/// context menu and covered its first items (Cut, Copy, Paste, Font…), so the menu looked
/// truncated; the tooltip must stand down while the menu owns the pointer.
#[test]
fn right_clicking_a_link_hides_the_hover_tooltip() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("See here")), Default::default());
    a.run("select.text", json!({"text": "here"})).unwrap();
    a.run("insert.link", json!({"url": "https://example.com/very/long/tracking/url?with=parameters&and=still-more"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    // Between the link's "h" and "e".
    let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 }).unwrap();
    let at = screen(&mut h, c.x, c.top + c.height / 2.0);
    h.event(egui::Event::PointerMoved(at));
    rest_for_tip(&mut h);
    let tips = |h: &H| h.query_all_by_label_contains("Click to follow link").count();
    let rows = |h: &H, label: &str| h.query_all_by_label_contains(label).count();
    assert!(tips(&h) > 0, "hovering the link shows its target");
    let (cut, copy, paste) = (rows(&h, "Cut"), rows(&h, "Copy"), rows(&h, "Paste"));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.app_frame();
    }
    steps(&mut h, 2);
    assert!(
        rows(&h, "Cut") > cut && rows(&h, "Copy") > copy && rows(&h, "Paste") > paste,
        "the context menu is open: its Cut, Copy and Paste rows are visible"
    );
    assert_eq!(tips(&h), 0, "the link tooltip is gone, so it can't cover the menu");
}

/// #4501 follow-up: a press the canvas handles (here Ctrl+click following a link) is done with
/// the pointer, so the link tooltip stands down that frame; plain hovering shows it again.
#[test]
fn a_handled_press_skips_the_link_tooltip() {
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("See here")), Default::default());
    a.run("select.text", json!({"text": "here"})).unwrap();
    a.run("insert.link", json!({"url": "https://example.com/"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 }).unwrap();
    let at = screen(&mut h, c.x, c.top + c.height / 2.0);
    h.event(Event::PointerMoved(at));
    rest_for_tip(&mut h);
    let tips = |h: &H| h.query_all_by_label_contains("Click to follow link").count();
    assert!(tips(&h) > 0, "hovering the link shows its target");
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::COMMAND });
    h.app_frame();
    assert_eq!(tips(&h), 0, "the Ctrl+click was handled: no tooltip that frame");
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::COMMAND });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    rest_for_tip(&mut h);
    assert!(tips(&h) > 0, "hovering again shows the tooltip");
}

/// With Hide White Space on, a click in a page's band puts the caret on that page, and a click
/// on the cut margins either side of the seam lands on the page whose band it is nearest to.
#[test]
fn hiding_white_space_stacks_page_bands_and_keeps_hit_testing() {
    use egui::{Event, PointerButton};
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for _ in 0..2 {
        a.run("text.insert", json!({"text": "Page text"})).unwrap();
        a.run("layout.break", json!({"kind": "page"})).unwrap();
    }
    a.run("text.insert", json!({"text": "Last page"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("view.hideWhiteSpace", json!({"value": true})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 2400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.app_frame();
    }
    let layout = h.state_mut().session.layout();
    assert_eq!(layout.pages.len(), 3);
    let (body0, body1) = (layout.pages[0].body, layout.pages[1].body);
    // A line on page 2. The page breaks are characters of the one paragraph, so its offsets reach every page.
    let len = h.state().session.doc.para_at(&Pos::body(0, 0)).map_or(0, |p| p.text.len());
    let second = (0..=len).find_map(|off| layout.caret(&Pos::body(0, off)).filter(|c| c.page == 1)).expect("a line on page 2");
    let click = |h: &mut H, at: Pos2| {
        h.event(Event::PointerMoved(at));
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.app_frame();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.app_frame();
        }
        // Let the double-click time pass before the next click.
        for _ in 0..40 {
            h.app_frame();
        }
        let focus = h.state().session.sel.focus.clone();
        h.state_mut().session.layout().caret(&focus).map(|c| c.page)
    };
    let on_second = page_to_screen(h.state_mut(), 1, second.x + 1.0, second.top + second.height / 2.0).expect("page 2 on screen");
    assert_eq!(click(&mut h, on_second), Some(1), "a click in page 2's band is on page 2");
    // The pages' rects overlap across the seam, each over the margin the other hides. (A press in the cut margins
    // themselves is in the footer or header area, which from the body leaves the caret where it is, #2103.)
    // Just inside the bottom of page 1's text area, where page 2's hidden top margin lies too: page 1's.
    let below_first = page_to_screen(h.state_mut(), 0, body0.x + 20.0, body0.bottom() - 2.0).expect("page 1 on screen");
    assert!(h.state().canvas.shown.pages.iter().filter(|p| p.rect.contains(below_first)).count() == 2, "both pages lie under the point");
    assert_eq!(click(&mut h, below_first), Some(0), "the band of page 1 there belongs to page 1");
    // Just inside the top of page 2's text area, where page 1's hidden bottom margin lies too: page 2's.
    let above_second = page_to_screen(h.state_mut(), 1, body1.x + 20.0, body1.y + 2.0).expect("page 2 on screen");
    assert!(h.state().canvas.shown.pages.iter().filter(|p| p.rect.contains(above_second)).count() == 2, "both pages lie under the point");
    assert_eq!(click(&mut h, above_second), Some(1), "the band of page 2 there belongs to page 2");
}

/// Kittest's `MissingGlyphPolicy::Panic` is right: a missing glyph is a bug. But the interface
/// fonts need not carry Japanese (craft-fonts is optional, and a CI Linux runner has no
/// Japanese system font), and these IME tests compose Japanese through the interface font
/// chain. Draw a stand-in for exactly those characters; every other missing glyph still panics.
fn kana_test_glyphs(ctx: &egui::Context) {
    ctx.add_glyph_rasterizer(egui::GlyphRasterizer::new("wordcraft::canvas::tests::kana", |req| {
        let kana =
            *req.family == egui::FontFamily::Proportional && !req.cluster.is_empty() && req.cluster.chars().all(|c| matches!(c, 'に' | 'ほ' | 'ん'));
        kana.then(|| egui::RasterizedGlyph {
            bitmap: egui::GlyphBitmap {
                image: egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                offset_px: egui::vec2(0.0, 0.0),
                is_color: false,
            },
            advance_px: (req.font_size_px * 0.5).max(1.0),
        })
    }));
}

/// The fixture above must work with no Japanese font anywhere: only a bundled Latin face, the
/// strict missing-glyph policy, and the kana stand-in.
#[test]
fn the_ime_fixture_draws_kana_without_a_japanese_font() {
    let ctx = crate::test_harness::context();
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    let mut fonts = egui::FontDefinitions::empty();
    fonts
        .font_data
        .insert("Latin".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"))));
    fonts.families.insert(egui::FontFamily::Proportional, vec!["Latin".into()]);
    ctx.set_fonts(fonts);
    kana_test_glyphs(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let g = ui.painter().layout_no_wrap("にほん".into(), crate::theme::regular(14.0), egui::Color32::WHITE);
        assert_eq!(g.rows.first().map_or(0, |r| r.row.glyphs.len()), 3, "one glyph per composed kana");
    });
    output.textures_delta.clear();
}

/// While an input method composes, the caret and the candidate window sit at its cursor inside
/// the composition: the end of the range it reports, or the end of the text with no range. The
/// range comes from the OS and is clamped, whatever it holds.
#[test]
fn the_caret_follows_the_input_method_cursor_in_the_composition() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "The"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    // Added to the harness's context, not as the builder's context setup, which would replace the one that installs
    // the craft runtime.
    kana_test_glyphs(&h.ctx);
    for _ in 0..8 {
        h.app_frame();
    }
    let ime_x = |h: &H| h.output().platform_output.ime.as_ref().map(|ime| ime.cursor_rect.min.x);
    let start = ime_x(&h).expect("the canvas places the candidate window");
    let mut compose = |range: Option<std::ops::Range<usize>>| {
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: range }));
        h.app_frame();
        h.app_frame();
        ime_x(&h).expect("the canvas places the candidate window")
    };
    let before = compose(Some(0..0));
    assert_eq!(before, start, "a cursor at the composition's start is the caret before composing");
    let end = compose(None);
    let mid = compose(Some(1..1));
    assert!(end > mid && mid > before, "the cursor moves through the composition: {before} {mid} {end}");
    assert_eq!(compose(Some(0..99)), end, "a range past the text ends at its end");
    assert_eq!(compose(Some(std::ops::Range { start: 5, end: 1 })), mid, "a reversed range uses its end");
    assert_eq!(compose(Some(usize::MAX..usize::MAX)), end);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("にほん".into())));
    h.app_frame();
    assert_eq!(h.state().canvas.ime_preedit_range, None, "the commit clears the range");
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("にほん"));
}

/// A harness of `app` for the composition tests, past the frames the canvas needs to take focus.
fn composing_harness(app: WordApp) -> H {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h
}

/// The composition `text` as this frame drew it.
fn drawn_composition(h: &H, text: &str) -> Option<egui::epaint::TextShape> {
    h.output().shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.clone()),
        _ => None,
    })
}

/// The font family and size the first section of `t` is set in.
fn composition_font(t: &egui::epaint::TextShape) -> Option<egui::FontId> {
    t.galley.job.sections.first().map(|s| s.format.font_id.clone())
}

/// A Japanese, Chinese or Korean composition in Latin text is drawn in the text's face, whose
/// chain goes on with the interface fonts, CJK included: the kana come from there. Production
/// borrowed a document face for them instead, but only one of 16 MB or less, and on Windows
/// Japanese falls back to Microsoft YaHei (19 MB): the composition was a box until committed.
#[test]
fn a_cjk_composition_uses_the_cjk_ui_font() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "Hello"})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    let mut h = composing_harness(app);
    assert!(h.state().canvas.focused(&h.ctx.clone()), "the canvas takes focus when the window opens");
    let layout = h.state_mut().session.layout();
    let (run, _) = layout.typing_style(&h.state().session.sel.focus, 0).expect("the run text typed here takes");
    assert!(!run.requested_face.covers('に'), "the document's Latin face has no kana: {:?}", run.requested_face);
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: None }));
    for _ in 0..4 {
        h.app_frame();
    }
    assert_eq!(h.state().canvas.ime_preedit, "にほん");
    let drawn = drawn_composition(&h, "にほん").expect("the composition is drawn");
    let row = drawn.galley.rows.first().expect("a row");
    assert_eq!(row.glyphs.len(), 3, "one glyph per composed kana");
    let family = composition_font(&drawn).map(|f| f.family).expect("a font");
    assert_eq!(family, egui::FontFamily::Name(format!("face+ui:{}", super::caret::face_key(run.requested_face)).into()), "in the text's face");
    // The framework serves the face with the interface fonts behind it; what the face lacks is drawn from them, so each
    // kana has a full-width glyph (a missing one would be a narrow box or nothing).
    let size = composition_font(&drawn).map(|f| f.size).expect("a size");
    assert!(
        row.glyphs.iter().all(|g| g.advance_width >= size * 0.8),
        "the kana come from the interface fonts: {:?}",
        row.glyphs.iter().map(|g| g.advance_width).collect::<Vec<_>>()
    );
}

/// The composition is drawn in the font and size of the text it goes into, as Word draws it, its
/// first baseline on the line's. It was drawn in the interface font at 0.8 of the line's height,
/// which made Latin letters look larger and heavier than the same letters once committed.
#[test]
fn the_composition_takes_the_documents_font_and_size() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "Hello"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("format.size", json!({"size": 20})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    let mut h = composing_harness(app);
    let layout = h.state_mut().session.layout();
    let (run, base) = layout.typing_style(&h.state().session.sel.focus, 0).expect("the run text typed here takes");
    assert_eq!(run.size, 20.0, "the size of the text before the caret");
    let caret = layout.caret(&h.state().session.sel.focus).expect("a caret");
    assert!(base > caret.top && base < caret.top + caret.height, "the baseline lies within the caret: {base} in {caret:?}");
    // The caret's IME rectangle gives the caret's top and the page's scale on screen, before the composition moves it.
    let ime = h.output().platform_output.ime.as_ref().map(|i| i.cursor_rect).expect("the caret's IME rectangle");
    let scale = ime.height() / caret.height;
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "tottori".into(), active_range_chars: None }));
    for _ in 0..4 {
        h.app_frame();
    }
    let drawn = drawn_composition(&h, "tottori").expect("the composition is drawn");
    let font = composition_font(&drawn).expect("a font");
    assert_eq!(font.family, egui::FontFamily::Name(format!("face+ui:{}", super::caret::face_key(run.requested_face)).into()), "the text's face");
    assert!((font.size - 20.0 * scale).abs() < 0.01, "{} at scale {scale}", font.size);
    let row = drawn.galley.rows.first().expect("a row");
    let baseline = drawn.pos.y + row.pos.y + row.glyphs.first().expect("a glyph").pos.y;
    let want = ime.min.y + (base - caret.top) * scale;
    assert!((baseline - want).abs() < 0.5, "drawn baseline {baseline}, the line's {want}");
    // A composition isn't text until it is committed.
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Hello");
}

/// The composition's size never fails egui, which draws each glyph into its font atlas and
/// panics on one wider than the atlas. In 1,638-point text at 500% the text's size is about
/// 11,000 points, so the composition is drawn smaller.
#[test]
fn a_composition_at_the_largest_size_and_zoom_is_drawn_smaller() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("document.setText", json!({"text": "W"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("format.size", json!({"size": 1638})).unwrap();
    app.run("caret.docEnd", json!({})).unwrap();
    app.run("view.zoom", json!({"value": 500})).unwrap();
    let mut h = composing_harness(app);
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "WWW".into(), active_range_chars: None }));
    for _ in 0..4 {
        h.app_frame();
    }
    let size = drawn_composition(&h, "WWW").and_then(|t| composition_font(&t)).map(|f| f.size);
    let most = (h.ctx.input(|i| i.max_texture_side) as f32 / 4.0).min(1024.0) / h.ctx.pixels_per_point();
    assert!(size.is_some_and(|s| s > 1.0 && s <= most), "{size:?}, at most {most}");
}

/// The outgoing IME descriptor is built from settled, visible logical text. A pre-edit does
/// not change the model; an accepted commit alone earns `InputMethod`; ordinary typing earns
/// `Other`. The commit mixes Korean, Japanese, ASCII and an emoji.
#[test]
fn ime_descriptor_reports_visible_logical_text_and_exact_cause() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("ab")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    // Added to the harness's context, not as the builder's context setup, which would replace the one that installs
    // the craft runtime.
    kana_test_glyphs(&h.ctx);
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
    h.app_frame();
    let text = |h: &H| h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).map(|p| p.text.clone());
    let cause = |h: &H| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
    assert_eq!(text(&h), Some("ab".into()), "the committed text is available");
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
    h.app_frame();
    assert_eq!(text(&h), Some("ab".into()), "the pre-edit never enters the committed model");
    assert_eq!(cause(&h), Some(ImeChangeCause::Other), "a pre-edit is no mutation");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("한日本A😀".into())));
    h.app_frame();
    assert!(text(&h).unwrap_or_default().contains("한日本A😀"), "the commit is in the visible text");
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted commit is the IME's change");
    h.event(egui::Event::Text("x".into()));
    h.app_frame();
    assert_eq!(cause(&h), Some(ImeChangeCause::Other), "ordinary typing is an application change");
}

/// Tracked deletions never go to the input method. The caret may straddle one and the
/// selection direction survives the visible mapping.
#[test]
fn ime_descriptor_hides_tracked_deletions_and_preserves_reversed_selection() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Paragraph, Run, para_block};
    let mut p = Paragraph::new();
    p.text = "abDELcd".into();
    p.runs = vec![
        Run { len: 2, props: CharProps::default() },
        Run { len: 3, props: CharProps { del: Some(0), ..Default::default() } },
        Run { len: 2, props: CharProps::default() },
    ];
    let mut doc = wordcraft_doc::Document::from_text("x");
    doc.body = vec![para_block(p)];
    let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 2), focus: wordcraft_doc::Pos::body(0, 7) };
    h.app_frame();
    let payload = |h: &H| h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).cloned();
    let p = payload(&h).expect("the visible text is available");
    assert_eq!(p.text, "abcd");
    assert_eq!(p.cursor, 4);
    assert_eq!(p.anchor, 2, "the reversed selection stays reversed");
}

/// Byte deletion keeps the selection out of both erased sides, then the commit replaces it;
/// both accepted mutations are `InputMethod` receipts over bounded snapshots.
#[test]
fn ime_byte_deletion_preserves_the_selection_and_earns_input_method() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("abcd")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 3), focus: wordcraft_doc::Pos::body(0, 1) };
    h.app_frame();
    let cause = |h: &H| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 1 }));
    h.app_frame();
    let body = |h: &H| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
    assert_eq!(body(&h), "bc", "the selected text is preserved");
    assert_eq!(h.state().session.sel.anchor.off, 2, "the selection anchor moved with the erased prefix");
    assert_eq!(h.state().session.sel.focus.off, 0);
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted deletion is the IME's change");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("X".into())));
    h.app_frame();
    assert_eq!(body(&h), "X", "the commit replaces the preserved selection");
    assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "delete plus commit chain into one IME change");
}

/// A selection wider than the protocol limit is honestly unavailable, not truncated.
#[test]
fn ime_descriptor_is_unavailable_for_a_too_wide_selection() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.app_frame();
    let ime = h.output().platform_output.ime.clone().expect("the canvas owns the IME context");
    assert!(ime.surrounding_text.available().is_none(), "a 5000-byte selection is unavailable");
}

/// Replacing the document starts a new IME context; caret motion inside it does not.
#[test]
fn ime_generation_changes_on_document_replacement_but_not_on_caret_motion() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    let generation = |h: &H| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
    let first = generation(&h).expect("an IME context");
    h.state_mut().run("caret.right", json!({})).unwrap();
    h.app_frame();
    assert_eq!(generation(&h), Some(first), "caret motion keeps the context");
    h.state_mut().session.set_document(wordcraft_doc::Document::from_text("new"));
    h.app_frame();
    assert!(generation(&h) > Some(first), "a replaced document starts a new context");
}

/// An equation's structured tree has no reversible surrounding text: report `Unavailable`
/// while keeping the stable editing-session context and candidate geometry through caret
/// motion. A real content commit is proved by the engine's checked revision and earns
/// `InputMethod`.
#[test]
fn ime_equation_is_unavailable_with_a_stable_session_context() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    assert!(app.session.math.is_some(), "the equation is being edited");
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    let ime = h.output().platform_output.ime.clone().expect("the equation places its candidate window");
    assert!(ime.surrounding_text.available().is_none(), "no fabricated equation surroundings");
    let generation = ime.context_generation;
    h.state_mut().run("equation.move", json!({"dir": "left"})).unwrap();
    h.app_frame();
    let moved = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(moved.context_generation, generation, "caret motion keeps the equation's IME context");
    assert!(moved.surrounding_text.available().is_none());
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
    h.app_frame();
    let committed = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(committed.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the content commit");
    assert!(committed.surrounding_text.available().is_none());
}

/// A transformed equation commit (build-up or Math AutoCorrect) is `Other`: the engine's
/// per-command outcome marks it transformed even though the revision moved and content
/// changed. The equation really built up; no InputMethod receipt is fabricated.
#[test]
fn ime_transformed_equation_commits_are_other() {
    use egui::ImeChangeCause;
    for (typed, latex) in [("a/b ", "\\frac{a}{b}"), ("\\alpha ", "\\alpha")] {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({})).unwrap();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..8 {
            h.app_frame();
        }
        h.event(egui::Event::Ime(egui::ImeEvent::Commit(typed.into())));
        h.app_frame();
        let info = h.state_mut().run("equation.get", json!({})).unwrap();
        assert_eq!(info["latex"].as_str(), Some(latex), "the commit was transformed: {typed:?}");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "an automatic equation rewrite is not the IME's own edit: {typed:?}");
    }
}

/// A literal IME commit after an earlier AutoCorrect is still the IME's own change. The
/// stale `"AutoFormat"` undo label left by that earlier step must not classify this command.
#[test]
fn ime_literal_commit_after_autoformat_is_input_method() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    for text in ["Word", " ", "(c)", " "] {
        app.run("text.insert", json!({"text": text})).unwrap();
    }
    assert_eq!(app.session.undo_label(), Some("AutoFormat"), "the earlier AutoCorrect left its undo label");
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Word \u{a9} x");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the literal commit is not the earlier AutoFormat");
}

/// A real allocator exhaustion (not a hand-built stamp) keeps the engine revision absent;
/// an equation commit and a wide body deletion still happen but can never certify a receipt.
#[test]
fn exhausted_ime_revisions_never_certify_equation_or_wide_body_edits() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.exhaust_ime_state_revision();
    h.app_frame();
    assert_eq!(h.state_mut().session.ime_state_revision(), None, "the allocator is exhausted");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
    h.app_frame();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "xy", "the real commit still happened");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "without a revision the change cannot be certified");

    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.app_frame();
    h.state_mut().session.exhaust_ime_state_revision();
    h.app_frame();
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.app_frame();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(body.matches('a').count(), 4999, "the wide body delete still happened");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "the exhausted revision cannot prove the delete");
}

/// A body replaced under the same document id starts a new IME context and interrupts the
/// live composition, even when the replacement text (and so the bounded window) is identical.
#[test]
fn ime_generation_changes_on_body_replacement_with_the_same_text() {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    // Added to the harness's context, not as the builder's context setup, which would replace the one that installs
    // the craft runtime.
    kana_test_glyphs(&h.ctx);
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
    h.app_frame();
    assert_eq!(h.state().canvas.ime_preedit.as_str(), "に");
    let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
    h.state_mut().run("document.setText", json!({"text": "hello"})).unwrap();
    h.app_frame();
    let ime = h.output().platform_output.ime.clone().expect("the canvas still owns the IME context");
    assert!(ime.context_generation > first, "a replaced body starts a new context");
    assert!(ime.should_interrupt_composition, "the old composition is interrupted");
    assert!(h.state().canvas.ime_preedit.is_empty(), "the old pre-edit is not kept");
    assert!(ime.surrounding_text.available().is_some(), "the new body still has surroundings");
}

/// Undo (and a version restore) puts a whole body back: the IME generation follows it too.
#[test]
fn ime_generation_changes_on_undo_and_version_restore() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one")), Default::default());
    app.run("file.versions", json!({"save": "v1"})).unwrap();
    app.run("document.setText", json!({"text": "two"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    let generation = |h: &H| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
    let first = generation(&h).expect("an IME context");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.app_frame();
    assert!(generation(&h) > Some(first), "Undo swaps the body back");
    let after_undo = generation(&h);
    h.state_mut().run("file.versions", json!({"restore": 0})).unwrap();
    h.app_frame();
    assert!(generation(&h) > after_undo, "a version restore replaces the body");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one");
}

/// With smart quotes off the engine inserts the quote as typed: that is the input method's
/// own edit and earns an `InputMethod` receipt (it used to be forced to `Other`).
#[test]
fn ime_apostrophe_with_smart_quotes_off_is_an_input_method_change() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("tools.autocorrect", json!({"smartQuotes": false})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "'", "the quote is not curled");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine inserted it unchanged");
}

/// With smart quotes on, the same commit is AutoFormat work and stays `Other` (the existing
/// contract), so the fix distinguishes the two instead of always assuming a transformation.
#[test]
fn ime_apostrophe_with_smart_quotes_on_stays_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    // Smart quotes are on by default.
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "\u{2018}", "curled");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "AutoFormat changed the text");
}

/// A real IME-only literal commit is the input method's own edit.
#[test]
fn ime_literal_commit_is_input_method() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("literal".into())));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "literal");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
}

/// A formatting change before an IME commit in the same frame must force `Other`, even
/// though an available body stamp cannot see the style change.
#[test]
fn para_alignment_before_ime_commit_is_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
    h.app_frame();
    h.state_mut().run("para.alignCenter", json!({})).unwrap();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.app_frame();
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
    assert_eq!(
        h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
        Some(wordcraft_doc::props::Align::Center),
        "the application command really ran"
    );
}

/// A formatting change after an accepted IME commit in the same frame must also force
/// `Other`: the pending application signal survives the receipt and is drained at observation.
#[test]
fn para_alignment_after_ime_commit_is_other() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
    h.app_frame();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.event(egui::Event::Key {
        key: egui::Key::E,
        physical_key: Some(egui::Key::E),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.app_frame();
    // The chord fires its command in that pass; the app runs fired commands when the next pass begins.
    h.app_frame();
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
    assert_eq!(
        h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
        Some(wordcraft_doc::props::Align::Center),
        "the application command really ran"
    );
}

/// A selected non-empty SmartArt graphic consumes an IME commit as an application edit: its
/// item changes, the body does not, and the frame is `Other`.
#[test]
fn smart_art_item_ime_commit_is_other_and_does_not_reach_the_body() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.app_frame();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
    assert_eq!(body, "", "the commit must not fall through into the body");
    let item = wordcraft_engine::cmd::smart_art::selected_smart_art(&h.state().session).and_then(|g| g.items.first().map(|i| i.text.clone()));
    assert_eq!(item.as_deref(), Some("Ax"), "the SmartArt item really changed");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other);
}

/// A selected graphic with no items consumes the commit without editing anything. The body
/// stays empty and no InputMethod receipt is fabricated.
#[test]
fn empty_smart_art_ime_commit_is_consumed_without_body_insertion() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
    // `insert.smartArt` refuses an empty item list, but an imported/edited model may hold
    // none: make the selected graphic empty while keeping its identity.
    let at = app.session.sel.ordered().0;
    if let Some(wordcraft_doc::InlineObject::Graphic { graphic, .. }) =
        app.session.doc.para_mut(at.story, &at.path).expect("paragraph").object_at_mut(at.off)
        && let Some(spec) = std::sync::Arc::make_mut(graphic).smart_art.as_mut()
    {
        std::sync::Arc::make_mut(spec).items.clear();
    }
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    let depth = h.state().session.undo_depth();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
    h.app_frame();
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "", "no body insertion");
    assert_eq!(h.state().session.undo_depth(), depth, "consuming an empty graphic changes no document");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_eq!(ime.change_cause, ImeChangeCause::Other, "no receipt and no application edit");
}

/// A wide selection's surrounding text is unavailable; deleting inside it still earns
/// `InputMethod` when both engine revisions are present and strictly increase.
#[test]
fn ime_wide_unavailable_deletion_uses_engine_revisions() {
    use egui::ImeChangeCause;
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    // A wide selection with one character before it: the counted deletion is outside the
    // selection, so the selection stays wide (unavailable) after it.
    h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
    h.app_frame();
    let before = h.output().platform_output.ime.clone().expect("an IME context");
    assert!(before.surrounding_text.available().is_none(), "the wide selection is unavailable");
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.app_frame();
    let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(body.matches('a').count(), 4999);
    let after = h.output().platform_output.ime.clone().expect("an IME context");
    assert!(after.surrounding_text.available().is_none(), "the selection is still wide after the deletion");
    assert_eq!(after.change_cause, ImeChangeCause::InputMethod, "the engine revisions prove the change");
}

/// A transactional equation delete that cannot make progress is refused as a whole and
/// interrupts the composition without a receipt.
#[test]
fn ime_equation_delete_at_start_is_refused_without_a_receipt() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    app.run("equation.move", json!({"dir": "home"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    // A composition is under way: the framework interrupts a composition it has seen (a pre-edit the page shows).
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "z".into(), active_range_chars: None }));
    h.app_frame();
    assert_eq!(h.state().canvas.ime_preedit.as_str(), "z");
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
    h.app_frame();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "x+y", "nothing is deleted");
    let ime = h.output().platform_output.ime.clone().expect("an IME context");
    assert_ne!(ime.change_cause, ImeChangeCause::InputMethod);
    assert!(ime.should_interrupt_composition, "the refused equation delete interrupts the composition");
}

/// A byte-counted replacement inside an equation is refused as a whole: nothing is deleted,
/// only the immediately paired stale `Text`/`Commit` is dropped and the composition is
/// interrupted. A later direct commit in the same raw batch or a later frame is fresh input
/// and applies, so the adjacent-pair bound never swallows it.
#[test]
fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    let linear = |h: &mut H| h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    // Harness::event steps each queued event in a separate frame. Insert the complete
    // protocol replacement into RawInput to exercise a single native input batch.
    h.input_mut().events.extend([
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
        egui::Event::Ime(egui::ImeEvent::Commit("Z".into())),
        egui::Event::Ime(egui::ImeEvent::Commit("W".into())),
    ]);
    h.app_frame();
    assert_eq!(linear(&mut h), "x+yW", "the adjacent stale commit is dropped and the next direct commit applies");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh direct commit is the input method's change");
    assert!(ime.should_interrupt_composition, "the refused byte deletion still interrupts the stale composition");

    // An adjacent `Text` + `Commit` pair is one replacement; a third commit is fresh.
    let base = linear(&mut h);
    h.input_mut().events.extend([
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
        egui::Event::Text("Q".into()),
        egui::Event::Ime(egui::ImeEvent::Commit("Q2".into())),
        egui::Event::Ime(egui::ImeEvent::Commit("R".into())),
    ]);
    h.app_frame();
    assert_eq!(linear(&mut h), format!("{base}R"), "the adjacent pair is dropped, not every later commit");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
    assert!(ime.should_interrupt_composition);

    // A refused deletion with no replacement in its batch must not swallow a later direct
    // commit, even with no pre-edit before it.
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
    h.app_frame();
    let base = linear(&mut h);
    assert_eq!(linear(&mut h), "x+yWR", "the refused deletion alone changed nothing");
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("Y".into())));
    h.app_frame();
    assert_eq!(linear(&mut h), format!("{base}Y"), "a later direct commit applies without a pre-edit");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh commit is the input method's change");
    assert!(!ime.should_interrupt_composition, "the fresh commit is not treated as stale");
}

/// A satisfied transactional equation delete commits content and earns one `InputMethod`
/// receipt over the checked unavailable revisions.
#[test]
fn ime_equation_delete_is_one_input_method_change() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
    h.app_frame();
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "x+");
    let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the delete");
}

/// An application change in an equation context and an IME commit in the same frame are
/// `Other` in both orders, just as for body text.
#[test]
fn ime_equation_context_mixes_with_application_changes() {
    use egui::ImeChangeCause;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    h.state_mut().run("equation.justify", json!({"jc": "left"})).unwrap();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
    h.app_frame();
    let before = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(before.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
    let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
    assert_eq!(linear, "xy");

    h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
    h.app_frame();
    let accepted = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(accepted.change_cause, ImeChangeCause::InputMethod);
    h.state_mut().run("equation.justify", json!({"jc": "right"})).unwrap();
    h.app_frame();
    let after = h.output().platform_output.ime.clone().expect("still editing");
    assert_eq!(after.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
}

/// Replacing the edited equation under the same document id gives it a fresh session token:
/// the IME context changes and the old composition is interrupted.
#[test]
fn ime_equation_replacement_starts_a_new_context() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..8 {
        h.app_frame();
    }
    // A composition is under way in the old equation: the framework interrupts a composition it has seen.
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "w".into(), active_range_chars: None }));
    h.app_frame();
    assert_eq!(h.state().canvas.ime_preedit.as_str(), "w");
    let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
    h.state_mut().run("insert.equation", json!({"linear": "z"})).unwrap();
    h.app_frame();
    let ime = h.output().platform_output.ime.clone().expect("the new equation owns the context");
    assert!(ime.context_generation > first, "a new equation editing session is a new context");
    assert!(ime.should_interrupt_composition, "the old equation composition is interrupted");
    assert!(h.state().session.math.is_some(), "the new equation is being edited");
}

/// A page drawn without a dragged object must not pass for the page with it: the drag's start
/// and end change the page's texture key once each, so the page is rendered again then and
/// only then. Without this, the old raster — the object still on it where it was — stays while
/// the preview follows the pointer, and the object shows twice.
#[test]
fn the_view_key_changes_when_a_drag_takes_an_object_off_the_page() {
    let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
    let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
    let object = Pos::body(0, 5);
    let hidden = view_key(&app, false, Some(&object));
    assert_ne!(hidden, view_key(&app, false, None), "the page without the dragged object is rendered again");
    assert_eq!(hidden, view_key(&app, false, Some(&object)), "the same drag keeps the key: no re-render every frame");
}

#[test]
fn screen_paper_color_changes_raster_and_cache_without_changing_export_paper() {
    use wordcraft_doc::props::Rgb;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let layout = app.session.layout();
    let page = &layout.pages[0];
    let print_before = wordcraft_render::render_page(&app.session.doc, page, 0.1, &Default::default());
    let mut keys = vec![view_key(&app, false, None)];
    for (value, expected) in [("green", Rgb(0xCC, 0xE8, 0xCF)), ("cream", Rgb(0xFA, 0xF0, 0xDC)), ("gray", Rgb(0xE8, 0xEA, 0xED))] {
        app.run("view.paperColor", json!({"value":value})).unwrap();
        let key = view_key(&app, false, None);
        assert!(!keys.contains(&key), "each preset invalidates page and tile caches");
        keys.push(key);
        let opts = page_render_options(&app, false);
        assert_eq!(opts.paper_of(&app.session.doc), expected);
        let screen = wordcraft_render::render_page(&app.session.doc, page, 0.1, &opts);
        assert_eq!(&screen.pixels[..4], &[expected.0, expected.1, expected.2, 255]);
        let print = wordcraft_render::render_page(&app.session.doc, page, 0.1, &Default::default());
        assert_eq!(print.pixels, print_before.pixels, "screen tints cannot leak into output rasters");
    }
    app.run("view.darkMode", json!({"value":true})).unwrap();
    let d = wordcraft_render::DARK_PAPER;
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(d, d, d));
    app.run("view.darkMode", json!({"value":false})).unwrap();
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(0xE8, 0xEA, 0xED));
    app.run("design.pageColor", json!({"color":"#102040"})).unwrap();
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(0x10, 0x20, 0x40));
    app.run("view.readMode", json!({})).unwrap();
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(0xE8, 0xEA, 0xED));
    app.run("view.darkMode", json!({"value":true})).unwrap();
    assert!(!page_render_options(&app, false).dark, "Switch Modes is ignored in Read Mode");
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(0xE8, 0xEA, 0xED));
    app.run("view.darkMode", json!({"value":false})).unwrap();
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(0xE8, 0xEA, 0xED));
    app.run("view.readPageColor", json!({"value":"sepia"})).unwrap();
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), wordcraft_render::SEPIA_PAPER);
    app.run("view.readPageColor", json!({"value":"inverse"})).unwrap();
    let d = wordcraft_render::DARK_PAPER;
    assert_eq!(page_render_options(&app, false).paper_of(&app.session.doc), Rgb(d, d, d));
}

/// #1961 review: what a picture draws must be in its page's fingerprint itself. Reset Picture on
/// one never edited keeps its media and clears only its corrections and crop, and undo puts them
/// back; the anchor paragraph's layout pointer also moves then, but only as long as the old
/// layout isn't freed and its address reused, so this looks at the picture's items alone.
#[test]
fn a_pictures_corrections_and_crop_change_its_page_key() {
    use wordcraft_doc::para::{Anchor, Float, Wrap};
    use wordcraft_doc::picfx::PicFx;
    let mut d = wordcraft_doc::Document::new();
    let media = d.add_media(b"not really a png".to_vec(), "png");
    let float = Float { wrap: Wrap::Square, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, ..Default::default() };
    let mut p = wordcraft_doc::Paragraph::new();
    let pic = wordcraft_doc::para::InlineObject::Image {
        media,
        w: 40.0,
        h: 20.0,
        alt: String::new(),
        float,
        crop: [0.0; 4],
        fx: PicFx::new(0, 0, 80_000),
        ole: None,
    };
    p.insert_object(0, pic, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let mut s = wordcraft_engine::Session::new(d);
    let key = |s: &mut wordcraft_engine::Session| {
        let layout = s.layout();
        let items: Vec<Placed> = layout.pages[0].items.iter().filter(|i| matches!(i, Placed::Image { .. })).cloned().collect();
        assert_eq!(items.len(), 1, "the picture is an item of its page");
        pages::page_content_key(&Page { items, ..Default::default() })
    };
    let faded = key(&mut s);
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 3}})).unwrap();
    s.run("picture.reset", &json!({})).unwrap();
    assert_ne!(key(&mut s), faded, "Reset Picture changes only the corrections");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(key(&mut s), faded, "undo gives back the faded picture");
    let at = |crop: [f32; 4], fx: PicFx| Page {
        items: vec![Placed::Image {
            rect: Default::default(),
            media: "m".into(),
            crop,
            story: StoryRef::Body,
            path: Default::default(),
            off: 0,
            spin: Default::default(),
            fx,
        }],
        ..Default::default()
    };
    let plain = pages::page_content_key(&at([0.0; 4], PicFx::default()));
    assert_ne!(plain, pages::page_content_key(&at([0.1, 0.0, 0.0, 0.0], PicFx::default())), "crop");
    assert_ne!(plain, pages::page_content_key(&at([0.0; 4], PicFx::new(10_000, 0, 0))), "brightness");
}

/// #5267: a picture linked from its context menu (Link…) keeps the link, shows it on hover and
/// follows it on Ctrl/⌘+click, in line with the text and floating over it.
#[test]
fn a_picture_linked_from_its_context_menu_shows_and_follows_its_link() {
    use egui_kittest::kittest::Queryable;
    for floating in [false, true] {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Text before the picture "})).unwrap();
        a.run("select.text", json!({"text": "before"})).unwrap();
        a.run("insert.bookmark", json!({"name": "Spot"})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img).clone(), image::ImageFormat::Png).unwrap();
        a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
        if floating {
            a.run("arrange.wrap", json!({"wrap": "inFrontOfText"})).unwrap();
        }
        let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        let rect = h.state_mut().session.layout().object(&pos, 0).expect("the picture is laid out").rect;
        let at = page_to_screen(h.state_mut(), 0, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("the picture is on screen");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.app_frame();
        }
        h.app_frame();
        h.get_by_label_contains("Link…").click();
        steps(&mut h, 4);
        h.event(egui::Event::Text("https://example.com/picture".into()));
        steps(&mut h, 3);
        h.get_by_label_contains("OK").click();
        steps(&mut h, 4);
        let s = &h.state().session;
        let link = s.doc.para_at(&pos).map(|p| p.props_of_char(pos.off).link.clone());
        assert_eq!(link, Some(Some("https://example.com/picture".to_string())), "floating {floating}: the link is on the picture");
        // Hovering the picture shows where it goes.
        h.event(egui::Event::PointerMoved(at + egui::vec2(1.0, 0.0)));
        rest_for_tip(&mut h);
        assert!(h.query_by_label_contains("https://example.com/picture").is_some(), "floating {floating}: the hover tip names the link");
        // Ctrl/⌘+click follows it, instead of picking the picture for a copy: here a link to a
        // place in the document, which moves the caret (a web address goes to the browser).
        h.state_mut().run("insert.link", json!({"bookmark": "Spot"})).unwrap();
        steps(&mut h, 2);
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
        h.event_modifiers(press, egui::Modifiers::COMMAND);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        steps(&mut h, 3);
        let s = &h.state().session;
        let (start, _) = wordcraft_engine::cmd::insert::bookmark_range(&s.doc, "Spot").unwrap();
        assert!(s.sel.is_collapsed() && s.sel.focus == start, "floating {floating}: the caret went to the bookmark: {:?}", s.sel);
        assert!(h.state().canvas.object_gesture().is_none(), "floating {floating}: no copy-drag started");
    }
}

/// #5267: inside a linked text box, hovering shows the box's link and Ctrl/⌘+click follows it,
/// although a click there reaches the box's text rather than the box.
#[test]
fn a_linked_text_box_shows_and_follows_its_link_from_inside() {
    use egui_kittest::kittest::Queryable;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Text before the box "})).unwrap();
    a.run("select.text", json!({"text": "before"})).unwrap();
    a.run("insert.bookmark", json!({"name": "Spot"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("insert.textBox", json!({"text": "Boxed words"})).unwrap();
    let link_box = |a: &mut WordApp, link: serde_json::Value| {
        let bx = a.session.layout().find_object(0, |o| o.text_box.is_some()).expect("the box");
        let end = Pos { off: bx.off + wordcraft_doc::para::OBJ.len_utf8(), ..bx.pos() };
        a.run("select.range", json!({"anchor": bx.pos(), "focus": end})).unwrap();
        a.run("insert.link", link).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        bx.pos()
    };
    let pos = link_box(&mut a, json!({"url": "https://example.com/box"}));
    let link = a.session.doc.para_at(&pos).map(|p| p.props_of_char(pos.off).link.clone());
    assert_eq!(link, Some(Some("https://example.com/box".to_string())), "the link is on the text box");
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    steps(&mut h, 6);
    let rect = h.state_mut().session.layout().find_object(0, |o| o.text_box.is_some()).expect("the box is laid out").rect;
    let at = page_to_screen(h.state_mut(), 0, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("the box is on screen");
    h.event(egui::Event::PointerMoved(at));
    rest_for_tip(&mut h);
    assert!(h.query_by_label_contains("https://example.com/box").is_some(), "the hover tip names the box's link");
    // Ctrl/⌘+click inside the box follows it: here a place in the document, which moves the caret.
    link_box(h.state_mut(), json!({"bookmark": "Spot"}));
    steps(&mut h, 2);
    let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
    h.event_modifiers(press, egui::Modifiers::COMMAND);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    steps(&mut h, 3);
    let s = &h.state().session;
    let (start, _) = wordcraft_engine::cmd::insert::bookmark_range(&s.doc, "Spot").unwrap();
    assert!(s.sel.is_collapsed() && s.sel.focus == start, "the caret went to the bookmark: {:?}", s.sel);
}
