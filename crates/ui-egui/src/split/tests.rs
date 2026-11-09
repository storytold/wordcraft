//! View › Split (#3254) drawn and used: two panes on one document, each scrolled on its own and
//! with its own caret; a press in the other pane works in it; Remove Split, the bar's
//! double-click and a drag onto an edge close the pane Word closes.

use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::Session;

use super::{BAR, MIN_PANE};
use crate::tests::FrameDriver;
use crate::{Services, WordApp};

/// The window the tests draw.
fn screen() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))
}

/// A long document (120 paragraphs over several pages), drawn for a few frames, the caret at
/// its start.
fn long_document() -> (FrameDriver, WordApp) {
    let text: Vec<String> = (0..120).map(|i| format!("Paragraph {i} of the long document, with enough words to fill its line.")).collect();
    let mut a = WordApp::new(Session::new(Document::from_text(&text.join("\n"))), Services::default());
    let mut d = FrameDriver::new();
    frames(&mut d, &mut a, 3);
    (d, a)
}

/// `n` frames without input.
fn frames(d: &mut FrameDriver, a: &mut WordApp, n: usize) {
    for _ in 0..n {
        d.frame(a, Vec::new(), screen());
    }
}

/// The document split, drawn until it settles; returns where the upper pane, the bar and the
/// lower pane are.
fn split(d: &mut FrameDriver, a: &mut WordApp) -> [Rect; 3] {
    a.run("view.split", json!({"value": true})).unwrap();
    frames(d, a, 3);
    rects(a)
}

/// Where the upper pane, the bar and the lower pane were drawn last.
fn rects(a: &WordApp) -> [Rect; 3] {
    a.canvas.split.as_ref().and_then(|s| s.rects()).expect("the window is split")
}

/// A primary press or release at `at`.
fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A primary click at `at`, over two frames, then one more frame.
fn click(d: &mut FrameDriver, a: &mut WordApp, at: Pos2) {
    d.frame(a, vec![Event::PointerMoved(at), button(at, true)], screen());
    d.frame(a, vec![button(at, false)], screen());
    frames(d, a, 1);
}

/// The bar dragged from its middle to `to`, released there.
fn drag_bar(d: &mut FrameDriver, a: &mut WordApp, to: Pos2) {
    let from = rects(a)[1].center();
    d.frame(a, vec![Event::PointerMoved(from), button(from, true)], screen());
    d.frame(a, vec![Event::PointerMoved(pos2(from.x, from.y + (to.y - from.y) / 2.0))], screen());
    d.frame(a, vec![Event::PointerMoved(to)], screen());
    d.frame(a, vec![button(to, false)], screen());
    frames(d, a, 2);
}

/// A touchpad scroll of `dy` points over `at` (it lands in the same frame).
fn scroll(d: &mut FrameDriver, a: &mut WordApp, at: Pos2, dy: f32) {
    let wheel = |y: f32, phase| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, y),
        phase,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    };
    d.frame(a, vec![Event::PointerMoved(at)], screen());
    d.frame(a, vec![wheel(-dy, egui::TouchPhase::Start)], screen());
    d.frame(a, vec![wheel(0.0, egui::TouchPhase::End)], screen());
    a.canvas.wheel = Default::default();
    if let Some(s) = a.canvas.split.as_mut() {
        s.canvas.wheel = Default::default();
    }
    frames(d, a, 1);
}

/// The other pane's caret.
fn other_caret(a: &WordApp) -> Pos {
    a.canvas.split.as_ref().and_then(|s| s.other_view()).map(|v| v.selection().focus.clone()).expect("the window is split")
}

/// A point inside the first line of text on the first page drawn in a pane whose page rects are
/// `pages` and whose own area is `pane`.
fn on_text(pages: &[Rect], pane: Rect) -> Pos2 {
    let page = pages.iter().find(|r| r.intersects(pane)).copied().expect("a page in the pane");
    // One inch of margin at 100% (96 px), a little into the first line.
    let at = pos2(page.min.x + 130.0, page.min.y + 104.0);
    assert!(pane.contains(at), "{at:?} is in the pane {pane:?}");
    at
}

/// Split divides the window in the middle and works in the new, lower pane, as Word does; both
/// panes show the document from the same place, each in a page state of its own. The ribbon's
/// command reports the split.
#[test]
fn split_shows_two_panes_and_works_in_the_lower_one() {
    let (mut d, mut a) = long_document();
    let whole = a.canvas.canvas_rect.unwrap();
    let [top, bar, bottom] = split(&mut d, &mut a);
    assert_eq!(bar.height(), BAR);
    assert!((top.height() - bottom.height()).abs() <= 1.0, "split in the middle: {} and {}", top.height(), bottom.height());

    let s = a.canvas.split.as_ref().unwrap();
    // Each pane has its rulers; the pages under them fill the window's page area.
    let upper = s.other_canvas().canvas_rect.unwrap();
    assert_eq!((upper.min.y, a.canvas.canvas_rect.unwrap().max.y), (whole.min.y, whole.max.y), "the panes share the window's page area");
    assert!(s.active_bottom(), "the lower pane is worked in");
    let (active, other) = (a.canvas.canvas_rect.unwrap(), s.other_canvas().canvas_rect.unwrap());
    assert!(bottom.contains_rect(active) && top.contains_rect(other), "{active:?} {other:?}");
    assert_ne!(a.canvas.pane, s.other_canvas().pane, "each pane has its own page widget");
    let (upper_pages, lower_pages) = (&s.other_canvas().page_rects, &a.canvas.page_rects);
    assert!(!upper_pages.is_empty() && !lower_pages.is_empty(), "both panes draw pages");
    let (u, l) = (upper_pages[0].min.y - top.min.y, lower_pages[0].min.y - bottom.min.y);
    assert!((u - l).abs() < 1.0, "both panes start at the same place: {u} and {l}");
    assert!(a.session.view.split);
    assert_eq!(a.run("view.state", json!({})).unwrap()["split"], json!(true));
}

/// The wheel scrolls the pane under the pointer only, whichever is worked in.
#[test]
fn each_pane_scrolls_on_its_own() {
    let (mut d, mut a) = long_document();
    let [top, _, bottom] = split(&mut d, &mut a);
    let other = |a: &WordApp| a.canvas.split.as_ref().unwrap().other_canvas().scroll_offset.y;
    let (upper, lower) = (other(&a), a.canvas.scroll_offset.y);

    scroll(&mut d, &mut a, top.center(), 300.0);
    let scrolled = other(&a);
    assert!(scrolled - upper > 200.0, "the upper pane scrolled: {upper} → {scrolled}");
    assert_eq!(a.canvas.scroll_offset.y, lower, "the lower pane didn't");
    assert!(a.canvas.split.as_ref().unwrap().active_bottom(), "scrolling doesn't change the pane worked in");

    scroll(&mut d, &mut a, bottom.center(), 120.0);
    assert!(a.canvas.scroll_offset.y - lower > 60.0, "the lower pane scrolled: {lower} → {}", a.canvas.scroll_offset.y);
    assert_eq!(other(&a), scrolled, "the upper pane stayed");
}

/// Each pane has its own caret and zoom. A press in the other pane works in it: the caret goes
/// where it was pressed, and typing lands there, while the pane left keeps its caret and zoom.
#[test]
fn a_press_in_the_other_pane_works_in_it_with_its_own_caret() {
    let (mut d, mut a) = long_document();
    let [top, _, _] = split(&mut d, &mut a);
    // The lower pane: its caret far down the document, zoomed in.
    a.run("caret.set", json!({"pos": Pos::body(80, 4)})).unwrap();
    a.run("view.zoom", json!({"value": 120})).unwrap();
    frames(&mut d, &mut a, 3);
    assert_eq!(other_caret(&a), Pos::body(0, 0), "the upper pane's caret stays at the start");

    let upper_pages = a.canvas.split.as_ref().unwrap().other_canvas().page_rects.clone();
    click(&mut d, &mut a, on_text(&upper_pages, top));
    let s = a.canvas.split.as_ref().unwrap();
    assert!(!s.active_bottom(), "the upper pane is worked in");
    assert_eq!(a.session.sel.focus.path, Pos::body(0, 0).path, "the caret is in the first paragraph, where it was pressed");
    assert!(a.session.sel.is_collapsed());
    assert_eq!(a.session.view.zoom, 1.0, "the upper pane's zoom");
    assert_eq!(other_caret(&a), Pos::body(80, 4), "the lower pane keeps its caret");
    assert!((s.other_view().unwrap().view().zoom - 1.2).abs() < 1e-6, "and its zoom");
    assert!(a.canvas.focused, "the upper pane has the keyboard");

    let at = a.session.sel.focus.clone();
    d.frame(&mut a, vec![Event::Text("Q".into())], screen());
    frames(&mut d, &mut a, 1);
    let para = |a: &WordApp, i: usize| a.session.doc.para_at(&Pos::body(i, 0)).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(para(&a, 0).chars().nth(at.off), Some('Q'), "typed in the upper pane at its caret: {}", para(&a, 0));
    assert!(!para(&a, 80).contains('Q'), "not at the lower pane's caret");
    assert_eq!(a.session.doc.plain_text(StoryRef::Body).matches('Q').count(), 1);
}

/// Remove Split closes the pane being worked in and leaves the other, as Word's does: here the
/// lower pane, worked in after Split, closes, and the upper pane fills the window with its own
/// caret and place.
#[test]
fn remove_split_leaves_the_pane_not_worked_in() {
    let (mut d, mut a) = long_document();
    let whole = a.canvas.canvas_rect.unwrap();
    let [top, _, _] = split(&mut d, &mut a);
    scroll(&mut d, &mut a, top.center(), 400.0);
    let upper = a.canvas.split.as_ref().unwrap().other_canvas().scroll_offset.y;
    a.run("caret.set", json!({"pos": Pos::body(90, 2)})).unwrap();
    frames(&mut d, &mut a, 2);

    assert_eq!(a.run("view.split", json!({})).unwrap(), json!({"value": false}));
    frames(&mut d, &mut a, 3);
    assert!(a.canvas.split.is_none());
    assert_eq!(a.session.sel.focus, Pos::body(0, 0), "the upper pane's caret");
    assert!((a.canvas.scroll_offset.y - upper).abs() < 1.0, "and its place: {} vs {upper}", a.canvas.scroll_offset.y);
    assert_eq!(a.canvas.canvas_rect, Some(whole), "one pane fills the window again");
    assert!(a.canvas.focused, "and has the keyboard");
}

/// The bar moves with a drag; a double-click on it is Remove Split; dragged onto the window's top
/// edge it closes the upper pane, and the lower one stays, whichever was worked in.
#[test]
fn the_bar_moves_and_closes_the_pane_it_squeezes_out() {
    let (mut d, mut a) = long_document();
    let whole = a.canvas.canvas_rect.unwrap();
    let [top, bar, bottom] = split(&mut d, &mut a);
    let full_h = bottom.max.y - top.min.y;
    drag_bar(&mut d, &mut a, pos2(bar.center().x, top.min.y + full_h * 0.7));
    let [moved, _, _] = rects(&a);
    // The bar's middle follows the pointer.
    assert!((moved.max.y + BAR / 2.0 - (top.min.y + full_h * 0.7)).abs() <= 1.0, "the bar is where it was dropped: {moved:?}");
    assert!(a.session.view.split);

    // Double-click: Remove Split; the upper pane (not worked in) stays.
    let [upper, bar, _] = rects(&a);
    let upper_pane = a.canvas.split.as_ref().unwrap().other_canvas().pane;
    for _ in 0..2 {
        d.frame(&mut a, vec![Event::PointerMoved(bar.center()), button(bar.center(), true)], screen());
        d.frame(&mut a, vec![button(bar.center(), false)], screen());
    }
    frames(&mut d, &mut a, 2);
    assert!(!a.session.view.split && a.canvas.split.is_none(), "a double-click removes the split");
    assert_eq!(a.canvas.pane, upper_pane, "the pane not worked in stays");
    assert_eq!(a.canvas.canvas_rect, Some(whole), "and fills the window");

    // Dragged onto the top edge, the upper pane closes and the lower one (worked in) stays.
    split(&mut d, &mut a);
    a.run("caret.set", json!({"pos": Pos::body(70, 1)})).unwrap();
    frames(&mut d, &mut a, 2);
    let lower_pane = a.canvas.pane;
    drag_bar(&mut d, &mut a, pos2(bar.center().x, upper.min.y + MIN_PANE / 2.0));
    assert!(!a.session.view.split && a.canvas.split.is_none(), "dragged onto an edge, the split ends");
    assert_eq!((a.canvas.pane, a.session.sel.focus.clone()), (lower_pane, Pos::body(70, 1)), "the lower pane stays");
}

/// Dragged onto the bottom edge, the bar closes the lower pane though it is the one worked in:
/// the upper pane stays, with its own caret.
#[test]
fn the_bar_dragged_to_the_bottom_closes_the_lower_pane() {
    let (mut d, mut a) = long_document();
    let [_, bar, bottom] = split(&mut d, &mut a);
    a.run("caret.set", json!({"pos": Pos::body(60, 3)})).unwrap();
    frames(&mut d, &mut a, 2);
    drag_bar(&mut d, &mut a, pos2(bar.center().x, bottom.max.y - MIN_PANE / 2.0));
    assert!(!a.session.view.split && a.canvas.split.is_none());
    assert_eq!(a.session.sel.focus, Pos::body(0, 0), "the upper pane stays, with its caret");
}

/// Read Mode shows the document in one pane and keeps the split for when it ends, with each
/// pane's own caret; Split is unavailable while it is on.
#[test]
fn read_mode_shows_one_pane_and_keeps_the_split() {
    let (mut d, mut a) = long_document();
    split(&mut d, &mut a);
    a.run("caret.set", json!({"pos": Pos::body(50, 0)})).unwrap();
    frames(&mut d, &mut a, 2);
    let whole = egui::Rect::from_min_max(rects(&a)[0].min, rects(&a)[2].max);
    a.run("view.readMode", json!({})).unwrap();
    frames(&mut d, &mut a, 3);
    assert!(a.run("view.split", json!({"value": false})).is_err(), "Split is unavailable in Read Mode");
    assert!(a.canvas.canvas_rect.is_some_and(|r| r.height() > whole.height() / 2.0 + MIN_PANE), "one pane: {:?}", a.canvas.canvas_rect);
    assert!(a.canvas.split.as_ref().is_some_and(|s| s.rects().is_none()), "the split is kept, not drawn");
    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::SHIFT);
    assert_eq!(a.session.sel.focus, Pos::body(50, 0), "F6 doesn't change to a pane that isn't shown");

    a.run("view.printLayout", json!({})).unwrap();
    frames(&mut d, &mut a, 3);
    let [_, _, bottom] = rects(&a);
    assert!(bottom.contains_rect(a.canvas.canvas_rect.unwrap()), "two panes again, the lower one worked in");
    assert_eq!((a.session.sel.focus.clone(), other_caret(&a)), (Pos::body(50, 0), Pos::body(0, 0)), "each with its caret");
}

/// A document shown in a split window's place (File › New in the web app's one window) starts
/// with one pane: the other pane belongs to the document that was shown, which finds both of
/// its panes, with their carets and places, when it is shown again. Closing it closes both.
#[test]
fn a_new_document_starts_with_one_pane_and_the_split_one_keeps_both() {
    let (mut d, mut a) = long_document();
    let [top, _, _] = split(&mut d, &mut a);
    scroll(&mut d, &mut a, top.center(), 400.0);
    a.run("caret.set", json!({"pos": Pos::body(30, 1)})).unwrap();
    frames(&mut d, &mut a, 2);
    let upper = a.canvas.split.as_ref().unwrap().other_canvas().scroll_offset.y;
    let split_doc = a.session.document_id();

    a.run("file.new", json!({})).unwrap();
    frames(&mut d, &mut a, 3);
    assert!(!a.session.view.split);
    assert!(a.canvas.split.is_none(), "no pane is left over from the other document");

    a.run("view.switchWindows", json!({"id": split_doc})).unwrap();
    frames(&mut d, &mut a, 3);
    let s = a.canvas.split.as_ref().expect("both panes are back");
    assert!(s.active_bottom(), "the lower pane is still worked in");
    assert_ne!(a.canvas.pane, s.other_canvas().pane, "each pane has its own page widget");
    assert_eq!((a.session.sel.focus.clone(), other_caret(&a)), (Pos::body(30, 1), Pos::body(0, 0)), "each with its caret");
    let back = a.canvas.split.as_ref().unwrap().other_canvas().scroll_offset.y;
    assert!((back - upper).abs() < 1.0, "the upper pane's place: {back} vs {upper}");

    a.run("file.close", json!({})).unwrap();
    frames(&mut d, &mut a, 3);
    assert_ne!(a.session.document_id(), split_doc);
    assert!(!a.session.view.split && a.canvas.split.is_none(), "the panes closed with their document");
}

/// A key press with `modifiers`, then two frames.
fn key(d: &mut FrameDriver, a: &mut WordApp, key: egui::Key, modifiers: egui::Modifiers) {
    let press = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    d.frame(a, vec![press], screen());
    frames(d, a, 2);
}

/// An edit made in the pane worked in shows in the other at once: the other pane draws its pages
/// again from the edited document.
#[test]
fn an_edit_shows_in_both_panes() {
    let (mut d, mut a) = long_document();
    split(&mut d, &mut a);
    frames(&mut d, &mut a, 2);
    let other = |a: &WordApp| a.canvas.split.as_ref().unwrap().other_canvas().rasters;
    let before = other(&a);
    frames(&mut d, &mut a, 2);
    assert_eq!(other(&a), before, "nothing to draw again while nothing changes");

    d.frame(&mut a, vec![Event::Text("Typed in the lower pane. ".into())], screen());
    frames(&mut d, &mut a, 2);
    assert!(a.session.doc.plain_text(StoryRef::Body).starts_with("Typed in the lower pane. "));
    assert!(other(&a) > before, "the upper pane drew the edited page: {} rasters", other(&a) - before);
}

/// F6 moves the keyboard from the upper pane to the lower one and Shift+F6 back, as Word's F6
/// goes through a split window's panes; with no task pane open they go round the two panes, and
/// with one open F6 from the lower pane goes on to it. Typing follows the keyboard.
#[test]
fn f6_moves_between_the_panes() {
    let (mut d, mut a) = long_document();
    split(&mut d, &mut a);
    let lower = |a: &WordApp| a.canvas.split.as_ref().unwrap().active_bottom();
    assert!(lower(&a) && a.canvas.focused);

    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::SHIFT);
    assert!(!lower(&a) && a.canvas.focused, "Shift+F6: the upper pane has the keyboard");
    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::NONE);
    assert!(lower(&a) && a.canvas.focused, "F6: back to the lower pane");
    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::NONE);
    assert!(!lower(&a) && a.canvas.focused, "no task pane: F6 goes round to the upper pane");

    a.run("caret.set", json!({"pos": Pos::body(40, 0)})).unwrap();
    frames(&mut d, &mut a, 1);
    d.frame(&mut a, vec![Event::Text("Z".into())], screen());
    frames(&mut d, &mut a, 1);
    assert_eq!(a.session.doc.para_at(&Pos::body(40, 0)).map(|p| p.text.starts_with('Z')), Some(true), "typed in the upper pane");
    assert_eq!(other_caret(&a), Pos::body(0, 0), "the lower pane's caret stayed");

    // With the Navigation pane open, F6 from the lower pane goes on to it.
    a.run("view.navigationPane", json!({"value": true})).unwrap();
    frames(&mut d, &mut a, 2);
    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::NONE);
    assert!(lower(&a) && a.canvas.focused, "F6 from the upper pane: the lower pane");
    key(&mut d, &mut a, egui::Key::F6, egui::Modifiers::NONE);
    assert!(lower(&a) && !a.canvas.focused, "F6 from the lower pane: the task pane");
}
