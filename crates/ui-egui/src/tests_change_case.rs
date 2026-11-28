//! The Change Case picker: one list for Shift+F3, the ribbon, the mini toolbar and the
//! right-click menu. Text changes only when a row is applied, so Original is the text as it was
//! when the picker opened (#3483).

use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_doc::{Document, StoryRef};

use crate::{Services, WordApp};

fn harness(text: &str) -> egui_kittest::Harness<'static, WordApp> {
    let a = WordApp::new(wordcraft_engine::Session::new(Document::from_text(text)), Services::default());
    crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(move |_| a)
}

fn body(h: &egui_kittest::Harness<'static, WordApp>) -> String {
    h.state().session.doc.plain_text(StoryRef::Body).trim_end().to_string()
}

fn select_all(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(2);
}

fn shift_f3(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F3);
    h.run_steps(2);
}

fn is_open(h: &egui_kittest::Harness<'static, WordApp>) -> bool {
    h.state().canvas.case_picker.is_some()
}

#[test]
fn shift_f3_opens_the_picker_and_enter_applies_the_next_mode() {
    let mut h = harness("hello world");
    select_all(&mut h);
    shift_f3(&mut h);
    assert!(is_open(&h), "Shift+F3 opened the picker");
    assert!(h.query_by_label("Original").is_some(), "Original is the first row");
    assert_eq!(body(&h), "hello world", "opening the picker changes nothing");

    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(!is_open(&h), "Enter closed the picker");
    assert_eq!(body(&h), "HELLO WORLD", "the next mode (UPPERCASE) was preselected");
}

#[test]
fn original_escape_and_the_caret_leave_the_text_alone() {
    let mut h = harness("Hello World");
    select_all(&mut h);
    shift_f3(&mut h);
    h.get_by_label("Original").click();
    h.run_steps(2);
    assert!(!is_open(&h));
    assert_eq!(body(&h), "Hello World", "Original leaves the text as it was");

    shift_f3(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!is_open(&h));
    assert_eq!(body(&h), "Hello World", "Esc cancels");
}

#[test]
fn repeated_shift_f3_moves_the_highlight() {
    let mut h = harness("Hello World");
    select_all(&mut h);
    shift_f3(&mut h);
    // Title Case, so the picker starts on lowercase; another Shift+F3 moves to UPPERCASE.
    shift_f3(&mut h);
    assert_eq!(body(&h), "Hello World", "the second Shift+F3 only moves the highlight");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(body(&h), "HELLO WORLD");
}

#[test]
fn arrows_move_the_highlight_without_previewing() {
    let mut h = harness("Hello World");
    select_all(&mut h);
    shift_f3(&mut h);
    // "Hello World" is Title Case, so the next mode is lowercase: one Down reaches UPPERCASE.
    assert_eq!(body(&h), "Hello World");
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(body(&h), "Hello World", "moving the highlight does not preview");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(body(&h), "HELLO WORLD");

    shift_f3(&mut h);
    h.key_press(egui::Key::Home);
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(body(&h), "HELLO WORLD", "Home selects Original");
}

#[test]
fn clicking_a_row_applies_once_and_undo_restores_the_text() {
    let mut h = harness("Hello World");
    select_all(&mut h);
    shift_f3(&mut h);
    h.get_by_label("lowercase").click();
    h.run_steps(2);
    assert_eq!(body(&h), "hello world");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(body(&h), "Hello World", "one undo step restores the original case");
}

#[test]
fn ribbon_mini_toolbar_and_context_menu_open_the_same_picker() {
    // Ribbon Aa: the accessible control with the highest place on screen.
    let mut h = harness("hello world");
    select_all(&mut h);
    h.run_steps(4);
    let top = h.get_all_by_label("Change Case").min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect("the ribbon Aa is shown");
    top.click();
    h.run_steps(2);
    assert!(is_open(&h), "the ribbon Aa opens the picker");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    // The mini toolbar sits over the page, below the ribbon.
    h.run_steps(4);
    let mini = h.get_all_by_label("Change Case").max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect("the mini-toolbar Aa is shown");
    mini.click();
    h.run_steps(2);
    assert!(is_open(&h), "the mini-toolbar Aa opens the picker");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    // Shift+F10 opens the page menu; its Change Case item uses the same picker.
    h.state_mut().run("ui.contextMenu", json!({})).unwrap();
    h.run_steps(3);
    let menu_item =
        h.get_all_by_label("Change Case").max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect("the context-menu item is shown");
    menu_item.click();
    h.run_steps(2);
    assert!(is_open(&h), "the context menu opens the picker");
}

#[test]
fn a_collapsed_caret_in_a_word_opens_the_picker() {
    let mut h = harness("hello world");
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
    h.run_steps(2);
    shift_f3(&mut h);
    assert!(is_open(&h));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(body(&h), "HELLO world", "the word at the caret is the target");
}
