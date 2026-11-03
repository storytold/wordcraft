//! Wrapped-line affinity adapted from WordCraft PR #3110, retaining current grapheme navigation.
use super::{dictionaries_turn, run};

// Concurrent dictionary installs invalidate layouts and intentionally clear
// affinity. Keep these geometry assertions in one dictionary generation.
use crate::Session;
use serde_json::json;
use wordcraft_doc::Pos;

/// A Khmer paragraph long enough to wrap, and the first place a line wraps into the next with no
/// space between (the offset after the line's last syllable, the next line's first).
fn khmer_wrap() -> (Session, usize) {
    let text = "សិស្សានុសិស្សត្រូវបានលើកទឹកចិត្តឱ្យអានសៀវភៅជារៀងរាល់ថ្ងៃដើម្បីពង្រីកចំណេះដឹងនិងបណ្តុះគំនិតច្នៃប្រឌិត".repeat(8);
    let mut s = Session::new(wordcraft_doc::Document::from_text(&text));
    let l = s.layout();
    let wrap = (1..text.len()).find(|&off| l.caret_at_line_end(&Pos::body(0, off), 0).is_some()).expect("a line wraps without a space");
    (s, wrap)
}

#[test]
fn end_on_a_line_wrapped_without_a_space_goes_after_its_last_syllable() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let l = s.layout();
    let last = s.doc.para_at(&Pos::body(0, wrap)).map(|p| p.prev_boundary(wrap)).unwrap();
    let line = l.caret_on(&Pos::body(0, last), 0).unwrap();
    // End from the start of that line: after the last syllable, drawn on the same line.
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, wrap));
    let c = s.caret_geometry(&l, &s.sel.focus).unwrap();
    assert_eq!(c.top, line.top, "drawn on the line it ends");
    assert!(c.x > line.x, "after the last syllable");
    // Typing goes after that syllable, not inside the word.
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text.get(last..wrap + 1).unwrap().chars().last(), Some('X'));
    run(&mut s, "edit.undo", json!({}));
    // Home from there goes to the start of that line, not the next one.
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "caret.home", json!({}));
    assert!(s.sel.focus.off < last, "Home stays on the line: {:?}", s.sel.focus);
    // Shift+End selects the last syllable too.
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.end", json!({"extend": true}));
    assert_eq!((s.sel.anchor.off, s.sel.focus.off), (last, wrap));
}

#[test]
fn right_stops_at_the_end_of_a_line_wrapped_without_a_space() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let l = s.layout();
    let para = s.doc.para_at(&Pos::body(0, wrap)).unwrap().clone();
    let (last, after) = (para.prev_boundary(wrap), para.next_boundary(wrap));
    let line = l.caret_on(&Pos::body(0, last), 0).unwrap();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    // Right: after the last syllable, still on that line.
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, wrap));
    assert_eq!(s.caret_geometry(&l, &s.sel.focus).unwrap().top, line.top);
    // Right again: the next line's start, the same place in the text drawn there.
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, wrap));
    let next = l.caret_on(&Pos::body(0, wrap), 0).unwrap();
    assert_eq!(s.caret_geometry(&l, &s.sel.focus), Some(next), "at the next line's start");
    // Then after the next line's first syllable.
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, after));
    // Left comes back the same way: the next line's start, the line's end, its last syllable.
    run(&mut s, "caret.left", json!({}));
    assert_eq!((s.sel.focus.off, s.caret_geometry(&l, &s.sel.focus)), (wrap, Some(next)));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, wrap));
    assert_eq!(s.caret_geometry(&l, &s.sel.focus).unwrap().top, line.top, "at the line's end");
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus, Pos::body(0, last));
    // Down from the end of the line goes to the next line, not the one after it.
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.right", json!({}));
    run(&mut s, "caret.down", json!({}));
    let below = s.caret_geometry(&l, &s.sel.focus).unwrap();
    assert_eq!(below.top, next.top, "one line down: {:?}", s.sel.focus);
}

#[test]
fn a_click_after_the_last_letter_of_a_line_wrapped_without_a_space_stays_on_that_line() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let l = s.layout();
    let end = l.caret_at_line_end(&Pos::body(0, wrap), 0).unwrap();
    let start = l.caret_on(&Pos::body(0, wrap), 0).unwrap();
    // The click lands on the place in the text both carets show; where it was decides which.
    for (y, want) in [(end.top + end.height / 2.0, end), (start.top + start.height / 2.0, start)] {
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}, "page": end.page, "y": y}));
        assert_eq!(s.caret_geometry(&l, &s.sel.focus), Some(want), "clicked at y {y}");
    }
    // Clicked past the line's end with the mouse's own hit test.
    run(&mut s, "caret.set", json!({"page": end.page, "x": end.x + 30.0, "y": end.top + end.height / 2.0}));
    assert_eq!(s.sel.focus, Pos::body(0, wrap));
    assert_eq!(s.caret_geometry(&l, &s.sel.focus), Some(end));
}

#[test]
fn end_and_right_on_spaced_text_are_unchanged() {
    let _dictionaries = dictionaries_turn();
    let mut s = Session::new(wordcraft_doc::Document::from_text(&"Typography is the art and technique of arranging type. ".repeat(6)));
    let l = s.layout();
    assert!((1..s.doc.para_at(&Pos::body(0, 0)).unwrap().len()).all(|off| l.caret_at_line_end(&Pos::body(0, off), 0).is_none()));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.line_end, None);
}

#[test]
fn edits_selection_and_reflow_clear_wrap_affinity() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}}));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert!(s.line_end.is_none());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}}));
    run(&mut s, "caret.left", json!({}));
    run(&mut s, "select.all", json!({}));
    assert!(s.line_end.is_none());
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}}));
    run(&mut s, "caret.left", json!({}));
    assert!(s.line_end.is_some());
    s.view.mode = wordcraft_layout::ViewMode::Web;
    s.view.web_width = 200.0;
    s.layout();
    assert!(s.line_end.is_none());
}

#[test]
fn hostile_page_does_not_wrap_to_an_existing_page() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let layout = s.layout();
    let end = layout.caret_at_line_end(&Pos::body(0, wrap), 0).unwrap();
    let para = layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| if let wordcraft_layout::Placed::Lines { para, .. } = item { Some(para) } else { None })
        .unwrap();
    assert!(!para.wraps_into(usize::MAX, wrap));
    assert!(s.run("caret.set", &json!({"page": u64::MAX, "x": end.x, "y": end.top})).is_err());
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}, "page": u64::MAX, "y": end.top}));
    assert!(s.line_end.is_none());
}

#[test]
fn cached_reflow_and_window_views_do_not_leak_affinity() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    // Prime a second layout; switching back must clear affinity even on a cache hit.
    s.view.mode = wordcraft_layout::ViewMode::Web;
    s.view.web_width = 200.0;
    s.layout();
    s.view.mode = wordcraft_layout::ViewMode::Print;
    s.layout();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}}));
    run(&mut s, "caret.left", json!({}));
    assert!(s.line_end.is_some());
    s.export_layout();
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)), "export does not move the editing caret");
    let v = s.take_view();
    assert!(s.line_end.is_none());
    s.put_view(v);
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    s.view.mode = wordcraft_layout::ViewMode::Web;
    s.layout();
    assert!(s.line_end.is_none());
}

#[test]
fn affinity_is_owned_by_its_document() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let first = s.document_id();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}}));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    let second = s.add_document(wordcraft_doc::Document::from_text("Another document"));
    assert!(s.line_end.is_none());
    s.activate(first).unwrap();
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    let layout = s.layout();
    assert_eq!(s.caret_geometry(&layout, &s.sel.focus), layout.caret_at_line_end(&s.sel.focus, s.page_hint));
    s.activate(second).unwrap();
    assert!(s.line_end.is_none());
}

#[test]
fn later_hebrew_does_not_skip_an_earlier_ltr_wrap_affinity() {
    let _dictionaries = dictionaries_turn();
    let text = format!("{} שלום", "សិស្សានុសិស្សត្រូវបានលើកទឹកចិត្តឱ្យអានសៀវភៅជារៀងរាល់ថ្ងៃ".repeat(8));
    let mut s = Session::new(wordcraft_doc::Document::from_text(&text));
    let layout = s.layout();
    let wrap = text.char_indices().map(|(off, _)| off).find(|&off| layout.caret_at_line_end(&Pos::body(0, off), 0).is_some()).unwrap();
    let last = s.doc.para_at(&Pos::body(0, wrap)).unwrap().prev_boundary(wrap);
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, wrap);
    assert!(s.line_end.is_none());
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, last);
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, wrap);
    assert!(s.line_end.is_none());
}

#[test]
fn pointer_selection_can_change_visual_side_without_changing_range() {
    let _dictionaries = dictionaries_turn();
    let (mut s, wrap) = khmer_wrap();
    let layout = s.layout();
    let pos = Pos::body(0, wrap);
    let end = layout.caret_at_line_end(&pos, 0).unwrap();
    let start = layout.caret_on(&pos, 0).unwrap();
    let last = s.doc.para_at(&pos).unwrap().prev_boundary(wrap);
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    for caret in [end, start, end] {
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": wrap}, "extend": true, "page": caret.page, "y": caret.top + caret.height / 2.0}));
        assert_eq!((s.sel.anchor.off, s.sel.focus.off), (last, wrap));
        assert_eq!(s.caret_geometry(&layout, &pos), Some(caret));
    }
}

#[test]
fn page_movement_and_reporting_use_the_visible_side_of_a_wrap() {
    let _dictionaries = dictionaries_turn();
    for view in ["view.webLayout", "view.draft", "view.outline"] {
        let (mut s, _) = khmer_wrap();
        run(&mut s, view, json!({}));
        let layout = s.layout();
        let para = s.doc.para_at(&Pos::body(0, 0)).unwrap().clone();
        let wraps: Vec<_> = (1..para.len()).filter(|&off| layout.caret_at_line_end(&Pos::body(0, off), 0).is_some()).collect();
        assert!(wraps.len() >= 2, "{view}");
        let first = wraps[0];
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": para.prev_boundary(first)}}));
        run(&mut s, "caret.end", json!({}));
        let before = s.caret_geometry(&layout, &s.sel.focus);
        run(&mut s, "caret.pageUp", json!({}));
        assert_eq!(s.line_end, Some(Pos::body(0, first)), "{view}: no line above; keep the visual caret");
        assert_eq!(s.caret_geometry(&layout, &s.sel.focus), before);

        let last = *wraps.last().unwrap();
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": para.prev_boundary(last)}}));
        run(&mut s, "caret.end", json!({}));
        assert_eq!(s.line_end, Some(Pos::body(0, last)));
        run(&mut s, "caret.pageDown", json!({}));
        assert!(s.sel.focus.off > last, "{view}: move onto the final line, not just to its start");
    }

    let (mut s, _) = khmer_wrap();
    let text = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.repeat(8);
    run(&mut s, "document.setText", json!({"text": text}));
    let layout = s.layout();
    assert!(layout.pages.len() >= 3, "relative Go To must not be hidden by last-page clamping");
    let (wrap, end) = text
        .char_indices()
        .find_map(|(off, _)| {
            let pos = Pos::body(0, off);
            let end = layout.caret_at_line_end(&pos, 0)?;
            let start = layout.caret_on(&pos, 0)?;
            (end.page == 0 && start.page == 1).then_some((off, end))
        })
        .expect("an unspaced wrap crosses the first page boundary");
    let last = s.doc.para_at(&Pos::body(0, 0)).unwrap().prev_boundary(wrap);
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": last}}));
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.line_end, Some(Pos::body(0, wrap)));
    let summary = run(&mut s, "document.layout", json!({}));
    assert_eq!(summary["caret"]["page"], end.page, "reported and drawn caret pages agree");
    run(&mut s, "edit.goto", json!({"pageBy": 1}));
    assert_eq!(s.caret_geometry(&layout, &s.sel.focus).unwrap().page, end.page + 1, "Go To advances one visible page");
}
