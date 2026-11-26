//! View › Split (#3254) drawn and used: two panes on one document, each scrolled on its own and
//! with its own caret; a press in the other pane makes it the pane worked in; Remove Split closes
//! the pane Word closes; the divider moves with a drag.

use craft::behavior::kurbo::Point;
use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::Session;

use crate::WordApp;
// Frames go through the app's input hook (`embed::before_input`) as in the app (`test_harness::frame`).
use crate::test_harness::AppFrames as _;

type H = egui_kittest::Harness<'static, WordApp>;

/// A long document (120 paragraphs over several pages) in a whole window, drawn for a few frames,
/// the caret at its start.
fn long_document() -> H {
    let text: Vec<String> = (0..120).map(|i| format!("Paragraph {i} of the long document, with enough words to fill its line.")).collect();
    let mut a = WordApp::new(Session::new(Document::from_text(&text.join("\n"))), Default::default());
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    a.run("caret.docStart", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(vec2(1280.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    frames(&mut h, 6);
    h
}

/// `n` frames without input.
fn frames(h: &mut H, n: usize) {
    for _ in 0..n {
        h.app_frame();
    }
}

/// The document split, drawn until it settles; returns where the upper and the lower pane are.
fn split(h: &mut H) -> [Rect; 2] {
    h.state_mut().run("view.split", json!({"value": true})).unwrap();
    frames(h, 4);
    rects(h)
}

/// Where the upper and the lower pane were drawn last.
fn rects(h: &H) -> [Rect; 2] {
    h.state().canvas.split.as_ref().and_then(|s| s.rects()).expect("the window is split")
}

/// The pane not worked in.
fn other(h: &H) -> &super::SplitPane {
    h.state().canvas.split.as_ref().expect("the window is split")
}

/// The other pane's caret.
fn other_caret(h: &H) -> Pos {
    other(h).other_view().map(|v| v.selection().focus.clone()).expect("the other pane has its view")
}

/// A primary press or release at `at`.
fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A primary click at `at`, then a few frames (long enough that a next click is not a double-click).
fn click(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(button(at, true));
    h.app_frame();
    h.event(button(at, false));
    frames(h, 40);
}

/// A wheel scroll of `dy` points down the document with the pointer over `at`.
fn scroll(h: &mut H, at: Pos2, dy: f32) {
    h.event(Event::PointerMoved(at));
    h.app_frame();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -dy),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    frames(h, 6);
}

/// The point on screen, in a pane that shows the pages `pages`, of the middle of the line holding
/// `pos` at its x.
fn on_text(h: &mut H, pages: &[craft::pages::ui::PlacedPage], pos: &Pos) -> Pos2 {
    let c = h.state_mut().session.layout().caret(pos).expect("the position has a caret");
    let placed = pages.iter().find(|p| p.page == c.page).expect("its page is in the pane");
    placed.to_screen(Point::new(f64::from(c.x), f64::from(c.top + c.height / 2.0)))
}

/// Split divides the window in the middle and works in the new, lower pane, as Word does; both
/// panes show the document from the same place, each with a page view of its own. The command
/// reports the split.
#[test]
fn split_shows_two_panes_and_works_in_the_lower_one() {
    let mut h = long_document();
    let whole = h.state().canvas.canvas_rect().unwrap();
    let [top, bottom] = split(&mut h);
    assert!((top.height() - bottom.height()).abs() <= 2.0, "split in the middle: {} and {}", top.height(), bottom.height());
    assert!((top.min.y - whole.min.y).abs() < 1.0 && (bottom.max.y - whole.max.y).abs() < 1.0, "the panes share the window's page area");

    let s = other(&h);
    assert!(s.active_bottom(), "the lower pane is worked in");
    let (active, upper) = (h.state().canvas.canvas_rect().unwrap(), s.other_canvas().canvas_rect().unwrap());
    assert!(bottom.contains_rect(active) && top.contains_rect(upper), "{active:?} in {bottom:?}, {upper:?} in {top:?}");
    assert_ne!(h.state().canvas.canvas_id(), s.other_canvas().canvas_id(), "each pane has its own page view");
    let (upper_pages, lower_pages) = (&s.other_canvas().shown.pages, &h.state().canvas.shown.pages);
    assert!(!upper_pages.is_empty() && !lower_pages.is_empty(), "both panes draw pages");
    let (u, l) = (upper_pages[0].rect.min.y - upper.min.y, lower_pages[0].rect.min.y - active.min.y);
    assert!((u - l).abs() < 1.0, "both panes start at the same place: {u} and {l}");
    assert!(h.state().session.view.split);
    assert_eq!(h.state_mut().run("view.state", json!({})).unwrap()["split"], json!(true));
}

/// The wheel scrolls the pane under the pointer only, whichever is worked in.
#[test]
fn each_pane_scrolls_on_its_own() {
    let mut h = long_document();
    let [top, bottom] = split(&mut h);
    let upper_offset = |h: &H| other(h).other_canvas().view.offset.1;
    let (upper, lower) = (upper_offset(&h), h.state().canvas.view.offset.1);

    scroll(&mut h, top.center(), 300.0);
    let scrolled = upper_offset(&h);
    assert!(scrolled - upper > 200.0, "the upper pane scrolled: {upper} → {scrolled}");
    assert_eq!(h.state().canvas.view.offset.1, lower, "the lower pane didn't");
    assert!(other(&h).active_bottom(), "scrolling doesn't change the pane worked in");

    // A new wheel gesture: one still running stays with the pane it started over, wherever the pointer goes.
    frames(&mut h, 30);
    scroll(&mut h, bottom.center(), 200.0);
    assert!(h.state().canvas.view.offset.1 - lower > 100.0, "the lower pane scrolled: {lower} → {}", h.state().canvas.view.offset.1);
    assert_eq!(upper_offset(&h), scrolled, "the upper pane stayed");
}

/// Each pane has its own caret and zoom. A press in the other pane makes it the pane worked in,
/// with its own caret and the keyboard (the press itself places nothing, as craft-ui's split views
/// have it); a click there then places its caret, and typing lands there, while the pane left
/// keeps its caret and zoom.
#[test]
fn a_press_in_the_other_pane_works_in_it_with_its_own_caret() {
    let mut h = long_document();
    let [top, _] = split(&mut h);
    // The lower pane: its caret far down the document, zoomed in.
    h.state_mut().run("caret.set", json!({"pos": Pos::body(80, 4)})).unwrap();
    h.state_mut().run("view.zoom", json!({"value": 120})).unwrap();
    frames(&mut h, 4);
    assert_eq!(other_caret(&h), Pos::body(0, 0), "the upper pane's caret stays at the start");

    let upper_pages = other(&h).other_canvas().shown.pages.clone();
    let target = Pos::body(0, 10);
    let at = on_text(&mut h, &upper_pages, &target);
    assert!(top.contains(at), "{at:?} is in the upper pane {top:?}");
    click(&mut h, at);
    assert!(!other(&h).active_bottom(), "the upper pane is worked in");
    assert_eq!(h.state().session.sel.focus, Pos::body(0, 0), "with its own caret: the press only chose the pane");
    assert_eq!(h.state().session.view.zoom, 1.0, "and its own zoom");
    assert_eq!(other_caret(&h), Pos::body(80, 4), "the lower pane keeps its caret");
    assert!((other(&h).other_view().unwrap().view().zoom - 1.2).abs() < 1e-6, "and its zoom");
    let ctx = h.ctx.clone();
    assert!(h.state().canvas.focused(&ctx), "the upper pane has the keyboard");

    click(&mut h, at);
    let caret = h.state().session.sel.focus.clone();
    assert!(h.state().session.sel.is_collapsed());
    assert!(caret.path == target.path && caret.off.abs_diff(target.off) <= 1, "the caret is where it was clicked: {caret:?}");
    h.event(Event::Text("Q".into()));
    frames(&mut h, 2);
    let para = |h: &H, i: usize| h.state().session.doc.para_at(&Pos::body(i, 0)).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(para(&h, 0).chars().nth(caret.off), Some('Q'), "typed in the upper pane at its caret: {}", para(&h, 0));
    assert!(!para(&h, 80).contains('Q'), "not at the lower pane's caret");
    assert_eq!(h.state().session.doc.plain_text(StoryRef::Body).matches('Q').count(), 1);
    assert_eq!(other_caret(&h), Pos::body(80, 4), "the lower pane's caret stayed");
}

/// Remove Split closes the pane being worked in and leaves the other, as Word's does: here the
/// lower pane, worked in after Split, closes, and the upper pane fills the window with its own
/// caret and place, and the keyboard.
#[test]
fn remove_split_leaves_the_pane_not_worked_in() {
    let mut h = long_document();
    let whole = h.state().canvas.canvas_rect().unwrap();
    let [top, _] = split(&mut h);
    scroll(&mut h, top.center(), 400.0);
    let upper = other(&h).other_canvas().view.offset.1;
    h.state_mut().run("caret.set", json!({"pos": Pos::body(90, 2)})).unwrap();
    frames(&mut h, 2);

    assert_eq!(h.state_mut().run("view.split", json!({})).unwrap(), json!({"value": false}));
    frames(&mut h, 4);
    assert!(h.state().canvas.split.is_none());
    assert_eq!(h.state().session.sel.focus, Pos::body(0, 0), "the upper pane's caret");
    assert!((h.state().canvas.view.offset.1 - upper).abs() < 1.0, "and its place: {} vs {upper}", h.state().canvas.view.offset.1);
    let rect = h.state().canvas.canvas_rect().unwrap();
    assert!((rect.min.y - whole.min.y).abs() < 1.0 && (rect.max.y - whole.max.y).abs() < 1.0, "one pane fills the window again: {rect:?}");
    let ctx = h.ctx.clone();
    assert!(h.state().canvas.focused(&ctx), "and has the keyboard");
}

/// The divider between the panes moves with a drag, and the panes follow it; the split stays on.
#[test]
fn the_divider_moves_with_a_drag() {
    let mut h = long_document();
    let [top, bottom] = split(&mut h);
    let from = pos2(top.center().x, (top.max.y + bottom.min.y) / 2.0);
    let to = pos2(from.x, top.min.y + (bottom.max.y - top.min.y) * 0.7);
    h.event(Event::PointerMoved(from));
    h.app_frame();
    h.event(button(from, true));
    h.app_frame();
    for k in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
        h.app_frame();
    }
    h.event(button(to, false));
    frames(&mut h, 3);
    let [upper, lower] = rects(&h);
    assert!((upper.max.y - to.y).abs() <= 6.0, "the divider is where it was dropped: {upper:?}, {to:?}");
    assert!((lower.max.y - bottom.max.y).abs() < 1.0 && lower.height() < bottom.height(), "the lower pane gave up the room: {lower:?}");
    assert!(h.state().session.view.split);
    assert!(other(&h).active_bottom(), "a drag on the divider doesn't change the pane worked in");
}

/// Read Mode shows the document in one pane and keeps the split for when it ends, with each
/// pane's own caret; Split is unavailable while it is on.
#[test]
fn read_mode_shows_one_pane_and_keeps_the_split() {
    let mut h = long_document();
    let [top, bottom] = split(&mut h);
    h.state_mut().run("caret.set", json!({"pos": Pos::body(50, 0)})).unwrap();
    frames(&mut h, 2);
    h.state_mut().run("view.readMode", json!({})).unwrap();
    frames(&mut h, 4);
    assert!(h.state_mut().run("view.split", json!({"value": false})).is_err(), "Split is unavailable in Read Mode");
    let shown = h.state().canvas.canvas_rect().expect("the columns show");
    assert!(shown.height() > (bottom.max.y - top.min.y) / 2.0 + 20.0, "one pane: {shown:?}");
    assert!(h.state().canvas.split.as_ref().is_some_and(|s| s.rects().is_none()), "the split is kept, not drawn");

    h.state_mut().run("view.printLayout", json!({})).unwrap();
    frames(&mut h, 4);
    let [_, lower] = rects(&h);
    assert!(lower.contains_rect(h.state().canvas.canvas_rect().unwrap()), "two panes again, the lower one worked in");
    assert_eq!((h.state().session.sel.focus.clone(), other_caret(&h)), (Pos::body(50, 0), Pos::body(0, 0)), "each with its caret");
}

/// A document shown in a split window's place (File › New in the web app's one window) starts
/// with one pane: the other pane belongs to the document that was shown, which finds both of its
/// panes, with their carets and places, when it is shown again. Closing it closes both.
#[test]
fn a_new_document_starts_with_one_pane_and_the_split_one_keeps_both() {
    let mut h = long_document();
    let [top, _] = split(&mut h);
    scroll(&mut h, top.center(), 400.0);
    h.state_mut().run("caret.set", json!({"pos": Pos::body(30, 1)})).unwrap();
    frames(&mut h, 2);
    let upper = other(&h).other_canvas().view.offset.1;
    let split_doc = h.state().session.document_id();

    h.state_mut().run("file.new", json!({})).unwrap();
    frames(&mut h, 4);
    assert!(!h.state().session.view.split);
    assert!(h.state().canvas.split.is_none(), "no pane is left over from the other document");

    h.state_mut().run("view.switchWindows", json!({"id": split_doc})).unwrap();
    frames(&mut h, 4);
    let s = h.state().canvas.split.as_ref().expect("both panes are back");
    assert!(s.active_bottom(), "the lower pane is still worked in");
    assert_eq!((h.state().session.sel.focus.clone(), other_caret(&h)), (Pos::body(30, 1), Pos::body(0, 0)), "each with its caret");
    let back = other(&h).other_canvas().view.offset.1;
    assert!((back - upper).abs() < 1.0, "the upper pane's place: {back} vs {upper}");

    h.state_mut().run("file.close", json!({})).unwrap();
    frames(&mut h, 4);
    assert_ne!(h.state().session.document_id(), split_doc);
    assert!(!h.state().session.view.split && h.state().canvas.split.is_none(), "the panes closed with their document");
}

/// An edit made in the pane worked in shows in the other at once: the other pane draws its pages
/// again from the edited document, and only then.
#[test]
fn an_edit_shows_in_both_panes() {
    let mut h = long_document();
    split(&mut h);
    let ctx = h.ctx.clone();
    assert!(h.state().canvas.focused(&ctx), "the lower pane has the keyboard");
    let drawn = |h: &H| other(h).other_canvas().page_keys.drawn_layout().cloned().expect("the upper pane drew its pages");
    let before = drawn(&h);
    frames(&mut h, 3);
    assert!(std::sync::Arc::ptr_eq(&before, &drawn(&h)), "nothing laid out again while nothing changes");

    h.event(Event::Text("Typed in the lower pane. ".into()));
    frames(&mut h, 3);
    assert!(h.state().session.doc.plain_text(StoryRef::Body).starts_with("Typed in the lower pane. "));
    assert!(!std::sync::Arc::ptr_eq(&before, &drawn(&h)), "the upper pane drew the edited pages");
}
