//! Tests of the page's markup surfaces: the rulers, the comment balloons and the context menu,
//! each driven through the app frame by access name and pointer, as a person would.

use super::*;
use crate::table_resize::tests::{H, cell, screen, steps, table_window};
use egui_kittest::kittest::Queryable;

/// A harness around `a` that runs the app's frame at the default step time, after six frames
/// that settle layout.
fn window(a: WordApp) -> H {
    window_dt(a, None)
}

/// [`window`] with a chosen step time (the time one frame adds), for gestures whose outcome
/// depends on how long frames last.
fn window_dt(a: WordApp, dt: Option<f32>) -> H {
    let mut b = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0));
    if let Some(dt) = dt {
        b = b.with_step_dt(dt);
    }
    let mut h = b.build_ui_state(
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
    h
}

/// A pointer button event at `at`.
fn button(at: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() }
}

/// A primary click at `at`, press and release a frame apart, then three frames to settle.
fn click_at(h: &mut H, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.event(button(at, egui::PointerButton::Primary, true));
    h.app_frame();
    h.event(button(at, egui::PointerButton::Primary, false));
    steps(h, 3);
}

/// A primary drag from `from` to `to` in six moves.
fn drag_between(h: &mut H, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.app_frame();
    h.event(button(from, egui::PointerButton::Primary, true));
    h.app_frame();
    for k in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
        h.app_frame();
    }
    h.event(button(to, egui::PointerButton::Primary, false));
    steps(h, 3);
}

/// A PNG picture insertion's parameters, `width` points wide.
fn picture_params(width: u32) -> serde_json::Value {
    let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
    let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img).clone(), image::ImageFormat::Png).unwrap();
    json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": width})
}

// ---------------------------------------------------------------------------------------
// Rulers
// ---------------------------------------------------------------------------------------

/// Dragging a margin edge on the vertical ruler leaves the view where it is. Each step of the
/// drag runs `layout.margins`, which brought the caret back into view: with the caret off
/// screen (the bottom margin is far from the text at the top) the page slid under the pointer
/// mid-drag, and the margin ran far past it (a drag of about 1 cm set it about 6 cm larger).
#[test]
fn dragging_the_bottom_margin_on_the_ruler_keeps_the_view_and_follows_the_pointer() {
    let text = vec!["A line of text."; 20].join("\n");
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text)), Default::default());
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let mut h = window_dt(a, Some(0.25));
    let body = h.state_mut().session.layout().pages[0].body;
    let area = h.state().canvas.canvas_rect().expect("the canvas is shown");
    // Scroll the first page's bottom margin to the middle of the window: the caret, at the
    // top of the page, is off screen.
    let bottom = screen(&mut h, 0.0, body.bottom()).y;
    h.event(egui::Event::PointerMoved(area.center()));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, area.center().y - bottom),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Default::default(),
    });
    h.run_steps(6);
    assert!(screen(&mut h, 0.0, body.y).y < area.min.y, "the caret's line is above the view");
    let offset = h.state().canvas.view.offset;
    let sect = wordcraft_engine::cmd::page::sect(&h.state().session);
    let depth = h.state().session.undo_depth();

    // The bottom margin's splitter, dragged up 36 points in a few moves.
    let start = h.get_by_label("Bottom Margin").rect().center();
    let rise = screen(&mut h, 0.0, body.bottom()).y - screen(&mut h, 0.0, body.bottom() - 36.0).y;
    let end = start - vec2(0.0, rise);
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(2);
    h.event(button(start, egui::PointerButton::Primary, true));
    h.app_frame();
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
        h.app_frame();
        assert!(h.state().canvas.view.offset == offset, "the view moved during the drag (step {k})");
    }
    h.event(button(end, egui::PointerButton::Primary, false));
    h.run_steps(3);
    assert!(h.state().canvas.view.offset == offset, "the view moved after the drag");
    let after = wordcraft_engine::cmd::page::sect(&h.state().session);
    let grown = after.margin_bottom - sect.margin_bottom;
    assert!((grown - 36.0).abs() <= 9.0, "the margin follows the pointer: {} → {}", sect.margin_bottom, after.margin_bottom);
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
}

/// #1878: in Multiple Pages both rulers belong to the caret's page: the margin splitters of the
/// horizontal ruler lie over that page's width and the vertical ruler's over its height, and
/// they follow the caret to another page of the row and to the next row.
#[test]
fn the_rulers_span_the_caret_page_in_multiple_pages() {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "One"})).unwrap();
    for text in ["Two", "Three", "Four"] {
        a.run("insert.pageBreak", json!({})).unwrap();
        a.run("text.insert", json!({"text": text})).unwrap();
    }
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("edit.goto", json!({"page": 1})).unwrap();
    a.run("view.multiplePages", json!({})).unwrap();
    let mut h = window(a);
    h.run_steps(4);
    // The ruler's splitters: left and right across the top, top and bottom down the side.
    let across = |h: &H| (h.get_by_label("Left Margin").rect().center().x, h.get_by_label("Right Margin").rect().center().x);
    let down = |h: &H| (h.get_by_label("Top Margin").rect().center().y, h.get_by_label("Bottom Margin").rect().center().y);
    let within_x = |h: &H, r: Rect| {
        let (l, rt) = across(h);
        r.min.x <= l && l <= rt && rt <= r.max.x
    };
    let within_y = |h: &H, r: Rect| {
        let (t, b) = down(h);
        r.min.y <= t && t <= b && b <= r.max.y
    };
    // Every page of the layout on screen, those scrolled out of view too (production's `page_rects`).
    let rects: Vec<Rect> = h.state().canvas.laid_page_rects().to_vec();
    assert!(rects.len() == 4 && (rects[0].min.y - rects[1].min.y).abs() < 1.0 && rects[1].min.x > rects[0].min.x, "pages side by side: {rects:?}");
    assert!(within_x(&h, rects[0]) && within_y(&h, rects[0]), "the rulers span page 1");
    assert!(!within_x(&h, rects[1]), "and not page 2, which sits beside it");

    // The caret moves to the second page of the row: the horizontal ruler follows it.
    h.state_mut().run("edit.goto", json!({"page": 2})).unwrap();
    h.run_steps(4);
    let rects: Vec<Rect> = h.state().canvas.laid_page_rects().to_vec();
    assert!(within_x(&h, rects[1]), "the horizontal ruler follows the caret to page 2");
    assert!(within_y(&h, rects[1]));
    assert!(!within_x(&h, rects[0]));

    // And down to the next row: the vertical ruler moves to that page's height.
    let below = (0..rects.len()).find(|i| rects[*i].min.y > rects[0].min.y + 1.0).expect("a second row of pages");
    h.state_mut().run("edit.goto", json!({"page": below + 1})).unwrap();
    h.run_steps(4);
    let rects: Vec<Rect> = h.state().canvas.laid_page_rects().to_vec();
    assert!(within_y(&h, rects[below]), "the vertical ruler spans page {}", below + 1);
    assert!(within_x(&h, rects[below]));
    assert!(!within_y(&h, rects[0]), "page 1 is in the row above");
}

/// Tab stops on the ruler: the corner selector picks the kind, a click sets a stop, dragging
/// moves it, dragging it off clears it, and each gesture is one undo step.
#[test]
fn ruler_tab_stops_are_set_moved_and_cleared_with_the_pointer() {
    use wordcraft_doc::props::TabAlign;
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Name\tScore"})).unwrap();
    a.run("view.ruler", json!({"value": true})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = window_dt(a, Some(1.0 / 60.0));
    // The middle of the ruler's strip, over a point `d` points from the text area's left edge:
    // the strip's height is that of the margin splitters, which sit on it.
    let on_ruler = |h: &mut H, d: f32| {
        let body_x = h.state_mut().session.layout().pages[0].body.x;
        let x = page_to_screen(h.state_mut(), 0, body_x + d, 0.0).expect("on screen").x;
        pos2(x, h.get_by_label("Left Margin").rect().center().y)
    };
    // The ruler snaps stops to its 1/16 inch grid.
    let step = 4.5_f32;
    let near = |stops: &[(f32, TabAlign)], want: f32| stops.len() == 1 && stops[0].1 == TabAlign::Center && (stops[0].0 - want).abs() <= step;
    // Lets the double-click time pass before the next press.
    let settle = |h: &mut H| steps(h, 30);
    let stops = |h: &H| {
        let s = &h.state().session;
        let para = s.doc.para_at(&s.sel.focus).expect("a paragraph");
        s.doc.styles.resolve_para(&para.props).tabs.iter().map(|t| (t.pos, t.align)).collect::<Vec<_>>()
    };
    let marker = |h: &H| h.query_all_by_label_contains("Tab at").next().map(|n| n.rect().center());

    // The selector starts at Left; a click moves it to Center.
    assert_eq!(h.state().canvas.tab_kind, TabAlign::Left);
    let selector = h.query_all_by_label_contains("Tab Selector").next().expect("the tab selector").rect().center();
    click_at(&mut h, selector);
    settle(&mut h);
    assert_eq!(h.state().canvas.tab_kind, TabAlign::Center);

    // A click on the ruler sets a centre stop at 2 inches (on the 1/16 inch grid), one step.
    let depth = h.state().session.undo_depth();
    let at = on_ruler(&mut h, 144.0);
    click_at(&mut h, at);
    settle(&mut h);
    let first = stops(&h);
    assert!(near(&first, 144.0), "the stop is set where the ruler was clicked: {first:?}");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "setting a stop is one step");
    assert!(marker(&h).is_some(), "the stop has its marker on the ruler");

    // Dragging it to 3 inches moves it, keeping its kind, in one step.
    let (from, to) = (marker(&h).expect("the marker"), on_ruler(&mut h, 216.0));
    let depth = h.state().session.undo_depth();
    drag_between(&mut h, from, pos2(to.x, from.y));
    let moved = stops(&h);
    assert!(near(&moved, 216.0) && moved[0].0 > first[0].0, "the stop moved right to the pointer, keeping its kind: {moved:?}");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "moving a stop is one step");

    // Dragging it down into the page clears it, in one step.
    let from = marker(&h).expect("the marker");
    let depth = h.state().session.undo_depth();
    drag_between(&mut h, from, from + vec2(0.0, 80.0));
    assert!(stops(&h).is_empty(), "the stop came off: {:?}", stops(&h));
    assert_eq!(h.state().session.undo_depth(), depth + 1, "clearing a stop is one step");

    // Undo brings back the moved stop, then the first one.
    assert!(h.state_mut().session.undo());
    assert_eq!(stops(&h), moved);
    assert!(h.state_mut().session.undo());
    assert_eq!(stops(&h), first);
}

// ---------------------------------------------------------------------------------------
// Comment balloons and drafts
// ---------------------------------------------------------------------------------------

/// #1348: an app with one comment ("Original") on some text, its balloon on screen.
fn comment_harness() -> (H, u32) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Some text here"})).unwrap();
    a.run("select.text", json!({"text": "text"})).unwrap();
    let id = a.run("review.newComment", json!({"text": "Original"})).unwrap()["id"].as_u64().unwrap() as u32;
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    (window(a), id)
}

/// Click comment `id`'s balloon (from no selected balloon): the card is selected and its own
/// edit has the keyboard in its field.
fn click_balloon(h: &mut H, id: u32) {
    h.state_mut().canvas.comments.selected = craft::behavior::Selection::default();
    steps(h, 2);
    let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
    click_at(h, card.left_top() + vec2(30.0, 8.0));
    assert_eq!(h.state().canvas.selected_balloon(), Some(id), "the click selected the balloon");
    // The selected card opens its own edit, a framework text field, which takes the keyboard by a click in it
    // (none opens where comments can't change).
    use egui::accesskit::Role;
    let field = h
        .query_by_role_and_label(Role::MultilineTextInput, "Edit")
        .or_else(|| h.query_by_role_and_label(Role::TextInput, "Edit"))
        .map(|n| n.rect().center());
    if let Some(at) = field {
        click_at(h, at);
        assert_eq!(h.state().canvas.selected_balloon(), Some(id), "the card stays selected");
    }
}

/// Types `text` into the field that has the keyboard.
fn type_in(h: &mut H, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.app_frame();
}

/// Leave the comment field by clicking on the page (which also leaves the balloon).
fn leave_field(h: &mut H) {
    let at = h.state().canvas.page_rects().next().map(|(_, r)| r.center()).expect("a page is shown");
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(button(at, egui::PointerButton::Primary, pressed));
        h.app_frame();
    }
    steps(h, 3);
}

/// The text of comment `id` in the document.
fn comment_text(app: &WordApp, id: u32) -> String {
    let part = app.session.doc.comments[&id].part;
    app.session.doc.plain_text(StoryRef::Part(part))
}

/// The text of the comment edit still being typed, if there is one.
fn draft_text(app: &WordApp) -> Option<String> {
    app.canvas.comments.editing.as_ref().map(|(_, _, text)| text.clone())
}

/// #1348: a comment edit typed, then refused (the document became read-only).
fn refused_draft() -> (H, u32) {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Refused ");
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    (h, id)
}

/// #1085: a selected balloon's buttons work (pressing them is not a click on the page that
/// leaves the balloon): a new comment with no text resolves and deletes from its balloon.
#[test]
fn an_empty_comment_balloon_deletes() {
    for body in ["", "Some text here"] {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        if !body.is_empty() {
            a.run("text.insert", json!({"text": body})).unwrap();
            a.run("select.text", json!({"text": "text"})).unwrap();
        }
        let id = a.run("review.newComment", json!({})).unwrap()["id"].as_u64().unwrap() as u32;
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = window(a);
        click_balloon(&mut h, id);
        let resolve = h.get_by_label("Resolve").rect().center();
        click_at(&mut h, resolve);
        assert!(h.state().session.doc.comments[&id].resolved, "{body:?}: Resolve worked");
        assert_eq!(h.state().canvas.selected_balloon(), Some(id), "{body:?}: the balloon stays selected");
        let del = h.get_by_label("Delete").rect().center();
        click_at(&mut h, del);
        assert!(h.state().session.doc.comments.is_empty(), "{body:?}: the empty comment was deleted");
    }
}

/// Typing in a balloon goes to the comment as one undo step, written when the field is left.
#[test]
fn a_balloon_edits_its_comment() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    let depth = h.state().session.undo_depth();
    for piece in ["Edited", " twice"] {
        type_in(&mut h, piece);
    }
    assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while typing");
    leave_field(&mut h);
    let text = comment_text(h.state(), id);
    assert!(text.contains("Edited twice") && text.contains("Original"), "{text:?}");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the edit");
    assert!(h.state_mut().session.undo());
    assert_eq!(comment_text(h.state(), id), "Original");
}

/// #1348: an edit the document refuses (here read-only protection) keeps what was typed and
/// says why; once the edit is allowed, leaving the field again writes it.
#[test]
fn a_refused_comment_edit_keeps_the_draft() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, "Edited");
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    leave_field(&mut h);
    assert_eq!(comment_text(h.state(), id), "Original", "the protected document refused the edit");
    assert!(draft_text(h.state()).is_some_and(|d| d.contains("Edited")), "the draft store still holds the typed text");
    let status = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
    assert!(status.contains("protected"), "the refusal is shown: {status:?}");
    h.state_mut().session.doc.settings.protection = None;
    click_balloon(&mut h, id);
    leave_field(&mut h);
    let text = comment_text(h.state(), id);
    assert!(text.contains("Edited"), "the typed text survived the refusal: {text:?}");
}

/// #3268: clicking a balloon's Reply button, typing a reply and clicking away posts it as a
/// reply comment under the original, not a draft that silently disappears.
#[test]
fn replying_to_a_comment_from_its_balloon_posts_the_reply() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    h.get_by_label("Reply").click();
    steps(&mut h, 3);
    type_in(&mut h, "A reply");
    leave_field(&mut h);
    let replies: Vec<u32> = h.state().session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k).collect();
    assert_eq!(replies.len(), 1, "{:?}", h.state().session.doc.comments);
    assert_eq!(comment_text(h.state(), replies[0]), "A reply");
}

/// #1348: a draft typed in one document never lands in another document's comment that happens
/// to have the same id.
#[test]
fn a_comment_draft_stays_with_its_document() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, " stale");
    // An agent replaces the document while the balloon is still being typed in.
    h.state_mut().session.set_document(wordcraft_doc::Document::new());
    let a = h.state_mut();
    a.run("text.insert", json!({"text": "Other words here"})).unwrap();
    a.run("select.text", json!({"text": "words"})).unwrap();
    let fresh = a.run("review.newComment", json!({"text": "Fresh"})).unwrap()["id"].as_u64().unwrap() as u32;
    assert_eq!(fresh, id, "the new document reuses the comment id");
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    steps(&mut h, 3);
    assert!(draft_text(h.state()).is_none_or(|d| !d.contains("stale")), "the draft is not the new document's");
    click_balloon(&mut h, fresh);
    leave_field(&mut h);
    assert_eq!(comment_text(h.state(), fresh), "Fresh", "the old document's draft didn't leak in");
}

/// #1348: saving while a comment is still being typed saves what was typed.
#[test]
fn saving_writes_a_pending_comment_draft_first() {
    let (mut h, id) = comment_harness();
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    let path = std::env::temp_dir().join(format!("wordcraft-1348-save-{}.docx", std::process::id()));
    h.state_mut().run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
    let text = comment_text(h.state(), id);
    assert!(text.contains("Typed"), "the draft was written before saving: {text:?}");
    assert!(!h.state().session.dirty, "and saved");
    let saved = wordcraft_engine::io::open_path(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let part = saved.comments.values().next().expect("the comment was saved").part;
    assert!(saved.plain_text(StoryRef::Part(part)).contains("Typed"), "the file has the typed text");
}

/// #1348: closing a document whose only change is a comment still being typed asks to save.
#[test]
fn closing_with_a_pending_comment_draft_asks_to_save() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    assert!(!h.state_mut().close_requested(), "the typed comment is an unsaved change");
    assert!(h.state().closing.is_some(), "the save prompt asks about it");
}

/// #1348: an agent can't close the document while comment text is still being typed unless
/// it passes `force`, as with unsaved changes. New and Open open beside it, losing nothing.
#[test]
fn an_agent_cannot_close_the_document_over_a_pending_comment_draft() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.dirty = false;
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    let open = json!({"path": "/nonexistent/other.docx"});
    assert!(crate::control::close_refused(h.state(), "file.open", &open).is_none());
    assert!(crate::control::close_refused(h.state(), "file.new", &json!({})).is_none());
    assert!(crate::control::close_refused(h.state(), "file.close", &json!({})).is_some());
    assert!(crate::control::close_refused(h.state(), "file.close", &json!({"force": true})).is_none());
    assert!(crate::control::close_refused(h.state(), "text.insert", &json!({"text": "x"})).is_none());
    // The same for the command that names the document it closes (#3274).
    let doc = h.state().session.document_id();
    assert!(crate::control::close_refused(h.state(), "file.closeDocument", &json!({})).is_some());
    assert!(crate::control::close_refused(h.state(), "file.closeDocument", &json!({"id": doc})).is_some());
    assert!(crate::control::close_refused(h.state(), "file.closeDocument", &json!({"force": true})).is_none());
}

/// #1348: a draft the document refuses doesn't keep the window open: closing asks to save,
/// and Don't Save drops the draft and closes.
#[test]
fn a_refused_comment_draft_can_be_dropped_on_close() {
    let (mut h, _) = refused_draft();
    assert!(!h.state_mut().close_requested(), "the refused draft is an unsaved change");
    assert!(h.state().closing.is_some(), "the save prompt asks about it");
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(h.state_mut().close_requested(), "Don't Save closes");
}

/// #1348: New with a refused draft opens beside it, asking nothing; the draft waits with its
/// document and is there when that document is shown again.
#[test]
fn a_refused_comment_draft_stays_with_its_document_on_new() {
    let (mut h, _) = refused_draft();
    let document = h.state().session.document_id();
    h.state_mut().run("file.new", json!({})).unwrap();
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "nothing is asked");
    assert_ne!(h.state().session.document_id(), document, "New made a document");
    assert!(!crate::panes::has_comment_drafts(h.state()), "the draft isn't the new document's");
    h.state_mut().run("view.switchWindows", json!({"id": document})).unwrap();
    assert!(crate::panes::has_comment_drafts(h.state()), "the draft is back with its document");
}

/// #1348: Save with a refused draft saves the rest of the document, keeps the draft and says why.
#[test]
fn saving_with_a_refused_comment_draft_saves_the_document() {
    let (mut h, _) = refused_draft();
    h.state_mut().session.dirty = true;
    let path = std::env::temp_dir().join(format!("wordcraft-1348-refused-{}.docx", std::process::id()));
    let r = h.state_mut().run("file.save", json!({"path": path.to_string_lossy()}));
    let saved = wordcraft_engine::io::open_path(&path);
    let _ = std::fs::remove_file(&path);
    assert!(r.is_ok(), "{r:?}");
    assert!(saved.is_ok(), "the document was saved");
    assert!(!h.state().session.dirty);
    assert!(draft_text(h.state()).is_some_and(|d| d.contains("Refused")), "the draft stays");
    let status = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
    assert!(status.contains("protected"), "the refusal is shown: {status:?}");
}

/// #1348: in a read-only document the comment fields can't be typed in. The shared read-only
/// case of the balloon component cannot reach this: whether comments may change is the
/// document's protection.
#[test]
fn comment_fields_are_read_only_where_comments_cant_change() {
    let (mut h, id) = comment_harness();
    h.state_mut().session.doc.settings.protection = Some("readOnly".into());
    h.app_frame();
    click_balloon(&mut h, id);
    type_in(&mut h, "Typed ");
    assert_eq!(draft_text(h.state()), None, "nothing was typed");
    assert!(!crate::panes::has_comment_drafts(h.state()), "and no draft is pending");
    assert_eq!(comment_text(h.state(), id), "Original");
}

// ---------------------------------------------------------------------------------------
// The context menu
// ---------------------------------------------------------------------------------------

/// A right-click at `at`, the button pressed and released a frame apart.
fn right_click(h: &mut H, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.app_frame();
    for pressed in [true, false] {
        h.event(button(at, egui::PointerButton::Secondary, pressed));
        h.app_frame();
    }
    steps(h, 2);
}

/// Where the open context menu's entry holding `label` is (an entry's label also has its
/// shortcut or submenu arrow, and the ribbon above the page may show the same name).
fn menu_entry(h: &H, label: &str) -> Option<Pos2> {
    let top = h.state().canvas.page_rects().next().map_or(0.0, |(_, r)| r.min.y);
    h.query_all_by_label_contains(label).map(|n| n.rect()).find(|r| r.min.y >= top).map(|r| r.center())
}

/// Picks `path` in the open context menu with the mouse: over each submenu on the way, then a
/// click on the last entry.
fn pick(h: &mut H, path: &[&str]) {
    let Some((last, submenus)) = path.split_last() else { return };
    for label in submenus {
        let at = menu_entry(h, label).expect(label);
        h.event(egui::Event::PointerMoved(at));
        steps(h, 2);
    }
    let at = menu_entry(h, last).expect(last);
    h.event(egui::Event::PointerMoved(at));
    h.app_frame();
    for pressed in [true, false] {
        h.event(button(at, egui::PointerButton::Primary, pressed));
        h.app_frame();
    }
    steps(h, 2);
}

/// Whether the open context menu has an entry holding each of `labels` (`true`) or none
/// (`false`).
fn assert_menu(h: &H, labels: &[&str], there: bool) {
    for label in labels {
        assert_eq!(menu_entry(h, label).is_some(), there, "{label} in the menu: {there}");
    }
}

/// The screen point at the centre of the object at `pos`, on its page.
fn object_centre(h: &mut H, pos: &Pos) -> Pos2 {
    let rect = h.state_mut().session.layout().object(pos, 0).expect("the object is laid out").rect;
    let page = h.state_mut().session.layout().caret(pos).map_or(0, |c| c.page);
    page_to_screen(h.state_mut(), page, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("on screen")
}

/// The screen point of the caret at `pos`.
fn caret_point(h: &mut H, pos: &Pos) -> Pos2 {
    let c = h.state_mut().session.layout().caret(pos).expect("laid out");
    page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen")
}

/// A document with a table of contents and one heading added after it, so the table is stale.
fn toc_window() -> (H, usize) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
    a.run("caret.docStart", json!({})).unwrap();
    a.run("references.toc", json!({})).unwrap();
    let h = window(a);
    let entry =
        h.state().session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1"))).expect("a TOC entry");
    (h, entry)
}

/// Appends a Heading 1 paragraph named `name` to the body.
fn add_heading(h: &mut H, name: &str) {
    use wordcraft_doc::{Block, Paragraph, Path, props::CharProps};
    let end = h.state().session.doc.body.len();
    let heading = Paragraph::with_text(name, CharProps::default()).styled("Heading1");
    h.state_mut().session.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(heading)).unwrap();
    h.state_mut().session.relayout();
    steps(h, 6);
}

/// A picture selected in a plain document, and where to right-click it.
fn picture_target() -> (H, Pos2) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Text before the picture "})).unwrap();
    a.run("insert.picture", picture_params(160)).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    let mut h = window(a);
    let at = object_centre(&mut h, &pos);
    (h, at)
}

/// A rectangle shape selected in a plain document, and where to right-click it.
fn shape_target() -> (H, Pos2) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    a.run("text.insert", json!({"text": "Text before the shape "})).unwrap();
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
    let mut h = window(a);
    let at = object_centre(&mut h, &pos);
    (h, at)
}

/// The caret on a table of contents entry of a stale table.
fn toc_target() -> (H, Pos2) {
    let (mut h, entry) = toc_window();
    add_heading(&mut h, "Fresh heading");
    let at = caret_point(&mut h, &Pos::body(entry, 2));
    (h, at)
}

/// A second paragraph to paste into, with the first paragraph's word copied.
fn paste_target() -> (H, Pos2) {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Bold\nTarget")), Default::default());
    a.run("select.text", json!({"text": "Bold"})).unwrap();
    a.run("edit.copy", json!({})).unwrap();
    let mut h = window(a);
    let at = caret_point(&mut h, &Pos::body(1, 6));
    (h, at)
}

/// A table window with the caret in the cell at row 1, column 2.
fn cell_target() -> (H, Pos2) {
    let mut h = table_window();
    let b = cell(&mut h, 1, 2);
    let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
    (h, at)
}

/// A table window with the whole table selected, and a point inside it.
fn table_target() -> (H, Pos2) {
    let (mut h, at) = cell_target();
    h.state_mut().run("table.selectTable", json!({})).unwrap();
    steps(&mut h, 2);
    (h, at)
}

/// A picture in a table cell, and where to right-click it.
fn cell_picture_target() -> (H, Pos2, Pos) {
    let mut h = table_window();
    h.state_mut().run("caret.end", json!({})).unwrap();
    h.state_mut().run("insert.picture", picture_params(60)).unwrap();
    let pos = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap().0;
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos::body(1, 0));
    steps(&mut h, 4);
    let at = object_centre(&mut h, &pos);
    (h, at, pos)
}

/// One row of [`the_context_menu_rows_follow_the_target`]: how the target is made, the point to
/// right-click, a check before the menu opens, and the rows the menu has and lacks.
struct Target {
    name: &'static str,
    make: fn() -> (H, Pos2),
    before: fn(&H),
    present: &'static [&'static str],
    absent: &'static [&'static str],
}

/// No check before the menu opens.
fn nothing(_: &H) {}

/// #1500: a shape's Format Shape… row opens a pane, but the pane is closed until it is
/// picked, and Shape Fill applies to the selected shape.
fn shape_commands_ready(h: &H) {
    assert!(
        craft::ui::runtime::commands::resolve(&h.ctx, craft::keymap::CommandRef::new("shape.fill")).enabled,
        "Shape Fill applies to the selected shape"
    );
    assert!(!h.state().session.view.format_shape_pane, "the pane starts closed");
}

/// The context menu's rows follow what was right-clicked, as in Word: a picture offers Wrap
/// Text and Insert Caption… (#513, #1230), a shape Format Shape… (#1500), a table of contents
/// entry Update Field and Remove Table of Contents (#1197, #2179), text after a copy the
/// ribbon's Paste items (#2583), a cell the cell's items and the whole table Distribute,
/// AutoFit and Insert Caption… (never Select or Cell Alignment). Choosing a row is the
/// command's own wiring, tested with the command; here only the rows are asserted, except the
/// caption of a picture in a cell, which asks the dialog for a Figure label (not Table).
#[test]
fn the_context_menu_rows_follow_the_target() {
    let targets = [
        Target { name: "picture", make: picture_target, before: nothing, present: &["Wrap Text", "Insert Caption…"], absent: &["Update Field"] },
        // A selected shape is an object like a picture: Wrap Text is its row too (#513).
        Target {
            name: "shape",
            make: shape_target,
            before: shape_commands_ready,
            present: &["Format Shape…", "Wrap Text"],
            absent: &["Update Field"],
        },
        Target {
            name: "TOC entry",
            make: toc_target,
            before: nothing,
            present: &["Paste", "Update Field", "Remove Table of Contents"],
            absent: &["Format Shape…", "Wrap Text"],
        },
        Target {
            name: "text after a copy",
            make: paste_target,
            before: nothing,
            present: &["Paste", "Keep Text Only", "Merge Formatting", "Paste Special…"],
            absent: &["Update Field", "Wrap Text"],
        },
        Target {
            name: "cell",
            make: cell_target,
            before: nothing,
            present: &["Insert", "Delete", "Split Cells", "Text Direction", "Table Properties…"],
            absent: &["Merge Cells", "Distribute Rows", "AutoFit", "Select", "Cell Alignment"],
        },
        Target {
            name: "whole table",
            make: table_target,
            before: nothing,
            present: &["Delete", "Distribute Rows", "Distribute Columns", "AutoFit", "Insert Caption…", "Table Properties…"],
            absent: &["Split Cells", "Merge Cells"],
        },
    ];
    for t in targets {
        let (mut h, at) = (t.make)();
        (t.before)(&h);
        assert_menu(&h, &["Wrap Text", "Update Field", "Distribute Rows"], false);
        right_click(&mut h, at);
        assert_menu(&h, t.present, true);
        assert_menu(&h, t.absent, false);
        if t.name == "cell" {
            assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 2)), "the caret moved to the cell");
        }
        if t.name == "whole table" {
            assert!(h.state().session.cell_range().is_some(), "a right-click on the selected table keeps it");
        }
        if t.name == "shape" {
            assert!(!h.state().session.view.format_shape_pane, "the pane stays closed until the row is picked");
        }
    }

    // A picture in a table cell: Insert Caption… proposes a Figure caption, and OK puts it
    // under the picture in that cell.
    let (mut h, at, pos) = cell_picture_target();
    right_click(&mut h, at);
    assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some(), "the right-click selected the picture");
    assert_menu(&h, &["Insert Caption…", "Wrap Text"], true);
    pick(&mut h, &["Insert Caption…"]);
    let open = h.state().dialog_host.open_dialogs();
    let caption = open.iter().find(|d| d.key.as_deref() == Some("caption")).expect("the caption dialog is open");
    let label = caption.fields.as_ref().and_then(|f| f.get("label")).and_then(|l| l.as_str()).map(str::to_owned);
    assert_eq!(label.as_deref(), Some("Figure"), "a picture in a table gets a figure caption");
    h.get_by_label("OK").click();
    steps(&mut h, 3);
    let (table, r, c) = pos.path.cell().unwrap();
    let paras: Vec<String> = h.state().session.doc.table(StoryRef::Body, &table).unwrap().rows[r].cells[c]
        .blocks
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| p.plain_text())
        .collect();
    assert_eq!(paras.last().map(String::as_str), Some("Figure 1"), "{paras:?}");
}

/// #2182: after Update Field has refreshed a table of contents once, the context menu on it
/// still offers Update Field, so a later heading change can be brought in again.
#[test]
fn context_menu_on_a_toc_still_offers_update_field_after_an_update() {
    let (mut h, _) = toc_window();
    for round in 0..2 {
        let name = format!("Fresh heading {round}");
        add_heading(&mut h, &name);
        let entry = h
            .state()
            .session
            .doc
            .body
            .iter()
            .position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1")))
            .expect("a TOC entry");
        let at = caret_point(&mut h, &Pos::body(entry, 2));
        right_click(&mut h, at);
        assert_menu(&h, &["Paste"], true);
        assert!(menu_entry(&h, "Update Field").is_some(), "round {round}: Update Field is offered");
        pick(&mut h, &["Update Field"]);
        let text = h.state().session.doc.plain_text(StoryRef::Body);
        assert_eq!(text.matches(&name).count(), 2, "round {round}: the new heading is listed: {text}");
    }
}

/// With just the caret in a cell, right-click › Delete › Delete Table removes the table, in
/// one undo step (#1198: the menu offered only Delete Cells there).
#[test]
fn a_right_click_in_a_cell_deletes_the_table() {
    let mut h = table_window();
    let b = cell(&mut h, 0, 1);
    let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
    right_click(&mut h, at);
    assert!(h.state().session.cell_range().is_none(), "just the caret");
    let depth = h.state().session.undo_depth();
    pick(&mut h, &["Delete", "Delete Table"]);
    assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()));
    assert_eq!(h.state().session.undo_depth(), depth + 1);
}

/// With a column's cells selected, a right-click on one of them keeps them: the menu is a
/// column's (Merge Cells, Distribute) and its Text Direction turns just the column, in one
/// undo step; one on a cell beside them drops them, though that cell lies between the
/// selection's ends in the text, and the menu is a cell's again.
#[test]
fn a_right_click_keeps_selected_cells_only_on_them() {
    use wordcraft_doc::props::TextDirection;
    let mut h = table_window();
    h.state_mut().run("table.selectColumn", json!({})).unwrap();
    assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 0))));
    let on = cell(&mut h, 1, 0);
    let at = screen(&mut h, on.x + on.w / 2.0, on.y + on.h / 2.0);
    right_click(&mut h, at);
    assert_eq!(h.state().session.cell_range().map(|r| r.cols), Some((0, 0)), "still the column");
    assert_menu(&h, &["Merge Cells", "Distribute Rows", "Distribute Columns", "Text Direction"], true);
    assert_menu(&h, &["Split Cells", "AutoFit"], false);
    let depth = h.state().session.undo_depth();
    pick(&mut h, &["Text Direction"]);
    assert_eq!(h.state().session.undo_depth(), depth + 1);
    let t = h.state().session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    let dir = |r: usize, c: usize| t.rows[r].cells[c].props.text_direction;
    assert_eq!(
        [dir(0, 0), dir(1, 0), dir(0, 1), dir(1, 2)],
        [TextDirection::Down, TextDirection::Down, TextDirection::Horizontal, TextDirection::Horizontal]
    );
    let beside = cell(&mut h, 0, 2);
    let at = screen(&mut h, beside.x + beside.w / 2.0, beside.y + beside.h / 2.0);
    right_click(&mut h, at);
    assert!(h.state().session.cell_range().is_none(), "the column is no longer selected");
    assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((0, 2)));
    assert_menu(&h, &["Split Cells"], true);
    assert_menu(&h, &["Merge Cells"], false);
}
