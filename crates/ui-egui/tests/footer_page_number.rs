//! Double-clicking a footer's page number edits the footer on that page, like Word, and does
//! not scroll away (#2103).

use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

/// A four-page document (page breaks between the paragraphs) with a page-number footer.
fn harness() -> egui_kittest::Harness<'static, WordApp> {
    let mut d = wordcraft_doc::Document::from_text("p1\np2\np3\np4");
    for i in 1..4 {
        d.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap().props.page_break_before = Some(true);
    }
    let app = WordApp::new(Session::new(d), Services::default());
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1100.0, 950.0))
        // The default step is 250 ms, which turns a double-click into two single clicks.
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    for _ in 0..6 {
        h.step();
    }
    h.state_mut().run("insert.footer", json!({ "preset": "pageNumber" })).unwrap();
    for _ in 0..4 {
        h.step();
    }
    // Creating the footer leaves the app editing it; a user clicking through pages has closed it
    // and put the caret back in the body.
    h.state_mut().run("insert.closeHeader", json!({})).unwrap();
    for _ in 0..4 {
        h.step();
    }
    assert_eq!(h.state().session.sel.focus.story, wordcraft_doc::StoryRef::Body);
    h
}

/// Wheel the canvas until the last page's bottom (its footer) is in view, then let the eased
/// scroll settle so the aimed position stays put.
fn scroll_to_last_page(h: &mut egui_kittest::Harness<'static, WordApp>, last: usize) {
    let center = egui::pos2(550.0, 450.0);
    let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 950.0));
    for _ in 0..200 {
        let on_screen = h.state().canvas.page_rects.get(last).is_some_and(|r| r.is_finite() && r.bottom() <= view.bottom() && r.top() >= view.top());
        if on_screen {
            break;
        }
        h.hover_at(center);
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    // The wheel eases out over a few frames: wait until the page stops moving.
    let mut prev: Option<egui::Rect> = None;
    for _ in 0..80 {
        h.step();
        let cur = h.state().canvas.page_rects.get(last).copied();
        if cur.is_some() && cur == prev {
            break;
        }
        prev = cur;
    }
    let on_screen = h.state().canvas.page_rects.get(last).is_some_and(|r| r.is_finite() && r.bottom() <= view.bottom() && r.bottom() >= view.top());
    assert!(on_screen, "the last page's footer never came into view");
}

#[test]
fn double_clicking_a_page_number_edits_the_footer_without_scrolling_away() {
    let mut h = harness();
    let last = h.state_mut().session.layout().pages.len() - 1;
    assert!(last >= 2, "the document has {last} pages after the breaks");
    scroll_to_last_page(&mut h, last);
    h.step();

    // The footer band of the last page: below its body, above the page edge.
    let layout = h.state_mut().session.layout().clone();
    let scale = h.state().canvas.scale;
    let page = &layout.pages[last];
    let y_pt = (page.body.bottom() + page.h) / 2.0;
    let rect = h.state().canvas.page_rects[last];
    let at = egui::pos2(rect.center().x, rect.min.y + y_pt * scale);
    assert!(layout.header_footer_zone(last, y_pt).is_some(), "the sweep point is in the footer zone of page {last}");

    // A fresh canvas starts with the caret in the body, as a user clicking through pages has it.
    assert_eq!(h.state().session.sel.focus.story, wordcraft_doc::StoryRef::Body);
    let page_before = last;

    h.hover_at(at);
    h.step();
    // Two clicks back to back. (drop_at also lifts the pointer, which resets egui's click
    // counter, so the presses and releases are fed directly.)
    for _ in 0..2 {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
    }

    let app = h.state_mut();
    let layout = app.session.layout().clone();
    assert!(
        wordcraft_ui_egui::canvas::editing_header_footer(app, &layout),
        "double-clicking the page number edits the footer (sel story {:?}, page hint {})",
        app.session.sel.focus.story,
        app.session.page_hint
    );
    assert_eq!(app.session.page_hint, page_before, "the view stays on the clicked page");
    // editing_header_footer is true for a header story too: pin the Footer story itself, the
    // one the double-clicked page number lives in (CodeRabbit review, PR #2871).
    let footer = layout.pages[page_before].footer_story.expect("the page shows the page-number footer");
    assert_eq!(app.session.sel.focus.story, wordcraft_doc::StoryRef::Part(footer), "the caret is in the footer story of the clicked page");
}
