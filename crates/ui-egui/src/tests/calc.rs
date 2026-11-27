//! Inline calculations in the interface: the source box (Insert › Calculation and a double-click
//! on a calculation), the unique-name prompt, Insert › Calculated Value, Calculation Options and
//! the bar of a calculation edited outside WordCraft.

use egui::{Event, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use wordcraft_doc::calc::{CalcRegion, CalcState, UnitSystem};
use wordcraft_doc::{InlineObject, Pos, StoryRef};

use super::*;
use crate::calc_ui::{self, CalcDialog};

/// Every calculation region in the body, in document order, with where it sits.
fn regions(a: &WordApp) -> Vec<(Pos, CalcRegion)> {
    let doc = &a.session.doc;
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(para) = doc.para(StoryRef::Body, &path) else { continue };
        for (off, o) in para.objects_with_offsets() {
            if let InlineObject::Equation { calc: Some(c), .. } = o {
                out.push((Pos { story: StoryRef::Body, path: path.clone(), off }, (**c).clone()));
            }
        }
    }
    out
}

fn sources(a: &WordApp) -> Vec<String> {
    regions(a).into_iter().map(|(_, c)| c.source).collect()
}

/// The open source box, if the dialog is one.
fn calc_box(a: &WordApp) -> Option<calc_ui::CalcBox> {
    match a.calc_dialog.as_ref() {
        Some(CalcDialog::Source(b)) => Some(b.clone()),
        _ => None,
    }
}

fn ready(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = harness(a);
    h.run_steps(6);
    h
}

/// Insert › Calculation opens the box (from the ribbon button's command without a source).
fn open_box(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.state_mut().run("insert.calculation", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().calc_dialog.as_ref().map(CalcDialog::name), Some("calculation"));
}

fn type_text(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

fn enter(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// Typing a calculation and Enter inserts it with that source; the text goes into the box, not
/// the page, and `:=` shows as `≔` while it is typed.
#[test]
fn calc_box_commits_source() {
    let mut h = ready(app());
    open_box(&mut h);
    // `:` then `=`: the box borrows the math font for `≔` and shows it.
    type_text(&mut h, "P :");
    type_text(&mut h, "= 1500 lbf");
    h.run_steps(3);
    let font = egui::TextStyle::Body.resolve(&h.ctx.global_style());
    assert!(h.ctx.fonts_mut(|f| f.has_glyph(&font, '\u{2254}')), "the interface can draw the definition mark while the box is open");
    let b = calc_box(h.state()).expect("the box is open");
    assert_eq!(b.source, "P \u{2254} 1500 lbf", "`:=` shows as the definition mark");
    enter(&mut h);
    assert!(h.state().calc_dialog.is_none(), "Enter commits and closes the box");
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
    assert_eq!(regions(h.state())[0].1.state, CalcState::Fresh);
    let texts: Vec<String> = h
        .state()
        .session
        .doc
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| h.state().session.doc.para(StoryRef::Body, p))
        .map(|p| p.text.clone())
        .collect();
    assert_eq!(texts, ["\u{FFFC}"], "nothing typed reached the page: one paragraph holding the calculation");
    // Esc cancels: nothing more is inserted.
    open_box(&mut h);
    type_text(&mut h, "Q := 2 ft");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none());
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
}

/// Double-clicking a calculation opens the box with its source; Enter runs `calc.setSource`.
#[test]
fn double_click_opens_source() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Load "})).unwrap();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " here"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = timed(a);
    let (at, region) = regions(h.state()).remove(0);
    let p = on_screen(&mut h, &at);
    double_click(&mut h, p);
    let b = calc_box(h.state()).expect("the double-click opens the source box");
    assert_eq!(b.editing, Some(region.id));
    assert_eq!(calc_ui::stored_source(&b.source), "P := 1500 lbf");
    assert!(h.state().session.math.is_none(), "not the equation editor");
    // Replace the source and commit.
    select_all_in_box(&mut h);
    type_text(&mut h, "P := 2000 lbf");
    assert_eq!(calc_ui::stored_source(&calc_box(h.state()).expect("the source box is open").source), "P := 2000 lbf");
    enter(&mut h);
    assert!(h.state().calc_dialog.is_none());
    assert_eq!(sources(h.state()), ["P := 2000 lbf"]);
    assert_eq!(regions(h.state())[0].1.id, region.id, "the same region");
}

/// A harness whose frames advance the clock (egui needs it to see a double-click or a hover).
fn timed(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            crate::test_harness::driver_logic(&ctx, app);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    h
}

/// The middle of the calculation at `at`, on screen.
fn on_screen(h: &mut egui_kittest::Harness<'static, WordApp>, at: &Pos) -> Pos2 {
    let eq = h.state_mut().session.layout().equation_box(at, 0).expect("the calculation is laid out");
    crate::canvas::page_to_screen(h.state_mut(), eq.page, eq.x + eq.math.width / 2.0, eq.base - 2.0).expect("on screen")
}

fn set_state(a: &mut WordApp, index: usize, state: CalcState) {
    let (at, _) = regions(a).remove(index);
    if let Ok(p) = a.session.doc.para_mut(at.story, &at.path)
        && let Some(InlineObject::Equation { calc: Some(c), .. }) = p.object_at_mut(at.off)
    {
        c.state = state;
    }
}

fn double_click(h: &mut egui_kittest::Harness<'static, WordApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
}

fn select_all_in_box(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.event(Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    h.run_steps(1);
}

/// A name already defined: the prompt says so and offers `P_1`; Rename inserts the calculation
/// under that name and leaves the first one alone. Nothing is inserted under the clashing name.
#[test]
fn duplicate_name_prompts_for_rename() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    let mut h = ready(a);
    open_box(&mut h);
    type_text(&mut h, "P := 2 lbf");
    enter(&mut h);
    let b = calc_box(h.state()).expect("the box stays open");
    let prompt = b.prompt.expect("the unique-name prompt shows");
    assert_eq!(prompt.new_name, "P_1");
    assert!(prompt.message.starts_with("P is already defined on page 1"), "{}", prompt.message);
    assert_eq!(sources(h.state()), ["P := 1500 lbf"], "nothing inserted under the clashing name");
    assert!(h.query_by_label("New name").is_some(), "the New name field is drawn");
    assert!(h.query_by_label("Go to existing").is_some(), "a duplicate can be gone to");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none(), "Rename commits");
    assert_eq!(sources(h.state()), ["P := 1500 lbf", "P_1 := 2 lbf"]);
}

/// Each name of `[a; b] := …` gets the same prompt as a single definition: here the second one,
/// renamed in place.
#[test]
fn a_name_of_several_prompts_for_rename() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    let mut h = ready(a);
    open_box(&mut h);
    type_text(&mut h, "[q; P] := [1; 2]");
    enter(&mut h);
    let b = calc_box(h.state()).expect("the box stays open");
    let prompt = b.prompt.expect("the unique-name prompt shows");
    assert_eq!(prompt.name, "P");
    assert_eq!(prompt.new_name, "P_1");
    assert!(prompt.message.starts_with("P is already defined on page 1"), "{}", prompt.message);
    assert_eq!(sources(h.state()), ["P := 1500 lbf"], "nothing inserted under the clashing name");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none(), "Rename commits");
    assert_eq!(sources(h.state()), ["P := 1500 lbf", "[q; P_1] := [1; 2]"]);
}

/// A unit's name: the prompt gives the reserved-name message (and no Go to existing).
#[test]
fn unit_name_prompts_for_rename() {
    let mut h = ready(app());
    open_box(&mut h);
    type_text(&mut h, "A := 2 in^2");
    enter(&mut h);
    let prompt = calc_box(h.state()).and_then(|b| b.prompt).expect("the prompt shows");
    assert_eq!(prompt.message, "A is a unit, function or constant name — choose another name");
    assert_eq!(prompt.new_name, "A_1");
    assert!(sources(h.state()).is_empty(), "nothing inserted");
    assert!(h.query_by_label("Go to existing").is_none(), "only a duplicate has one to go to");
    // Cancel goes back to the box with the source as typed.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    h.run_steps(3);
    let b = calc_box(h.state()).expect("back in the box");
    assert!(b.prompt.is_none());
    assert_eq!(calc_ui::stored_source(&b.source), "A := 2 in^2");
}

/// Calculation Options: unit system, precision and manual recalculation, applied by OK as one
/// `calc.settings`.
#[test]
fn options_dialog_sets_settings() {
    let mut a = app();
    a.run("calc.settings", json!({})).unwrap();
    assert_eq!(a.calc_dialog.as_ref().map(CalcDialog::name), Some("calcOptions"));
    assert!(a.session.doc.calc_settings.is_none(), "opening the dialog changes nothing");
    let mut h = ready(a);
    h.get_by_label("SI (metric)").click();
    h.run_steps(2);
    h.get_by_label("Recalculate manually").click();
    h.run_steps(2);
    h.get_by_label("Gram (SI): gravity is g_0").click();
    h.run_steps(2);
    if let Some(d) = h.state_mut().calc_dialog.as_mut()
        && let CalcDialog::Options(f) = d
    {
        f.precision = 5;
    }
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none());
    let s = h.state().session.doc.calc_settings.clone().expect("settings set");
    assert_eq!((s.system, s.precision, s.manual), (UnitSystem::Si, 5, true));
    assert_eq!(s.g, wordcraft_doc::calc::GMeans::Gram);
}

/// Insert › Calculated Value lists the defined names and inserts the chosen one's value.
#[test]
fn calc_value_picker_inserts_a_value() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("insert.calcValue", json!({})).unwrap();
    let Some(d) = a.calc_dialog.clone() else { panic!("no dialog: {:?}", a.calc_dialog.as_ref().map(CalcDialog::name)) };
    let CalcDialog::Value(mut f) = d else { panic!("not the picker") };
    assert_eq!(f.names, ["P"]);
    assert_eq!(f.name, "P");
    f.unit = "kip".into();
    f.precision_on = true;
    f.precision = 2;
    assert_eq!(f.params(), json!({"name": "P", "unit": "kip", "precision": 2}));
    a.calc_dialog = None;
    a.run("insert.calcValue", f.params()).unwrap();
    let fields: Vec<String> = a
        .session
        .doc
        .body
        .iter()
        .filter_map(|b| match &**b {
            wordcraft_doc::Block::Para(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { instr, result, .. } => Some(format!("{instr} → {result}")),
            _ => None,
        })
        .collect();
    assert_eq!(fields.len(), 1, "{fields:?}");
    assert!(fields[0].contains("1.5"), "{fields:?}");
}

/// The rename suggestion: `<name>_1`, `<name>_2`, … the first free one.
#[test]
fn calc_rename_suggestion_takes_the_first_free_name() {
    let mut a = app();
    for s in ["P := 1 lbf", "P_1 := 2 lbf", "P_2 := 3 lbf"] {
        a.run("insert.calculation", json!({"source": s})).unwrap();
        a.run("text.newParagraph", json!({})).unwrap();
    }
    assert_eq!(calc_ui::suggest_name(&mut a.session, "P", None).as_deref(), Some("P_3"));
    assert_eq!(calc_ui::suggest_name(&mut a.session, "A_b", None).as_deref(), Some("A_b_1"));
    assert_eq!(calc_ui::rename_source("P := 2 lbf", "P", "P_3").as_deref(), Some("P_3 := 2 lbf"));
    assert_eq!(calc_ui::rename_source("  f(x) \u{2254} x^2", "f", "g").as_deref(), Some("  g(x) \u{2254} x^2"));
    assert_eq!(calc_ui::rename_source("Px := 1", "P", "Q"), None, "only the whole name");
    assert_eq!(calc_ui::defined_names_of("σ := P/A_b ="), ["σ"]);
    assert!(calc_ui::defined_names_of("P/2 =").is_empty(), "a bare evaluation defines nothing");
    assert_eq!(calc_ui::defined_names_of("[a; b_1] := Find(x, y)"), ["a", "b_1"]);
    // A name of several, renamed where it stands, whole names only.
    assert_eq!(calc_ui::rename_source("[a; b] := v", "b", "b_1").as_deref(), Some("[a; b_1] := v"));
    assert_eq!(calc_ui::rename_source(" [ab; b; c] \u{2254} v", "b", "B").as_deref(), Some(" [ab; B; c] \u{2254} v"));
    assert_eq!(calc_ui::rename_source("[ab; c] := v", "b", "B"), None, "only a whole name");
}

/// `:=` shows as `≔` when the font has it, with the cursor kept in place; never without.
#[test]
fn calc_define_mark_shows_when_the_font_has_it() {
    let mark = "P \u{2254} 1";
    assert_eq!(calc_ui::show_define_mark("P := 1", 4, true), (mark.to_string(), 3), "after `:=`, after the mark");
    assert_eq!(calc_ui::show_define_mark("P := 1", 6, true), (mark.to_string(), 5));
    assert_eq!(calc_ui::show_define_mark("P := 1", 3, true), (mark.to_string(), 3), "between `:` and `=`: after the mark");
    assert_eq!(calc_ui::show_define_mark("P := 1", 1, true), (mark.to_string(), 1));
    assert_eq!(calc_ui::show_define_mark(mark, 3, false), ("P := 1".to_string(), 4), "no glyph: back to ASCII");
    assert_eq!(calc_ui::stored_source(mark), "P := 1", "stored as ASCII");
}

/// `∧` and `∨` show as the words `and` and `or` where the font can't draw them (never as a box),
/// with the cursor kept in place and quoted texts left alone.
#[test]
fn calc_logic_marks_spell_out_without_coverage() {
    let (and, or) = ('\u{2227}', '\u{2228}');
    assert_eq!(calc_ui::spell_logic(&format!("a {and} b"), 4), ("a and b".to_string(), 6), "spaces kept, cursor kept before b");
    assert_eq!(calc_ui::spell_logic(&format!("a{or}b"), 2), ("a or b".to_string(), 5), "spaces added; the cursor follows the mark");
    assert_eq!(calc_ui::spell_logic(&format!("x > 1 {and}"), 7), ("x > 1 and ".to_string(), 10), "inserted at the end: room to go on");
    assert_eq!(calc_ui::spell_logic(&format!("{or}"), 0), ("or ".to_string(), 0));
    assert_eq!(calc_ui::spell_logic(&format!("c := \"a {and} b\" == t {or} u"), 5).0, "c := \"a \u{2227} b\" == t or u", "texts are left alone");
    assert_eq!(calc_ui::spell_logic("a and b", 3), ("a and b".to_string(), 3), "nothing to spell: unchanged");
    // Two marks in a row get one space between them.
    assert_eq!(calc_ui::spell_logic(&format!("{and}{or}"), 2).0, "and or ");
    // The Insert Symbol buttons say the words where the symbols can't be drawn.
    assert_eq!(calc_ui::symbol_label(&and.to_string(), false), "and");
    assert_eq!(calc_ui::symbol_label(&or.to_string(), false), "or");
    assert_eq!(calc_ui::symbol_label(&and.to_string(), true), and.to_string());
    assert_eq!(calc_ui::symbol_label("≤", false), "≤", "other symbols are the interface's own");
    assert!(calc_ui::logic_covered(|_| true));
    assert!(!calc_ui::logic_covered(|c| c != or), "both symbols must be drawable");
    // A box that can't draw them rewrites its source, keeping the cursor.
    let mut b = calc_ui::CalcBox::new(None, "x ", None);
    b.insert_symbol(&and.to_string());
    b.spell_logic_marks();
    assert_eq!(b.source, "x and ");
    assert_eq!(b.cursor, Some(6));
}

/// What the box shows for `\wedge` and the Insert Symbol buttons follows what the interface can
/// draw: the symbol when it can (the box borrows the math face), else the word; never a glyph
/// that would draw as a box. Either way the calculation means the same.
#[test]
fn calc_box_shows_logic_marks_only_when_drawable() {
    let (and, or) = ('\u{2227}', '\u{2228}');
    let mut h = ready(app());
    open_box(&mut h);
    type_text(&mut h, "a > 1 \\wedge ");
    h.run_steps(3);
    let font = egui::TextStyle::Body.resolve(&h.ctx.global_style());
    let drawable = h.ctx.fonts_mut(|f| f.has_glyph(&font, and) && f.has_glyph(&font, or));
    let source = calc_box(h.state()).expect("open").source;
    if drawable {
        assert_eq!(source, format!("a > 1 {and}"), "drawn from the borrowed face");
    } else {
        assert_eq!(source, "a > 1 and ", "no glyph: the word");
    }
    type_text(&mut h, "b < 2");
    h.get_by_label("Insert Symbol").click();
    h.run_steps(3);
    let label = if drawable { or.to_string() } else { "or".to_string() };
    h.get_by_label(&label).click();
    h.run_steps(3);
    let source = calc_box(h.state()).expect("open").source;
    assert_eq!(source.contains(or), drawable, "{source}");
    assert!(drawable || source.ends_with("b < 2 or "), "{source}");
    assert!(drawable || !source.contains(and), "{source}");
    // Both spellings are the same calculation.
    type_text(&mut h, "1 = 1");
    enter(&mut h);
    assert_eq!(regions(h.state()).len(), 1, "the calculation was inserted");
}

/// Hovering a region: an error shows its message, a stale result says how to update it.
#[test]
fn calc_hover_text_follows_the_state() {
    assert_eq!(calc_ui::hover_text(&CalcState::Error("units don't match".into())).as_deref(), Some("units don't match"));
    assert_eq!(calc_ui::hover_text(&CalcState::Stale).as_deref(), Some("Out of date — recalculate (References › Recalculate)"));
    assert_eq!(calc_ui::hover_text(&CalcState::Fresh), None);
    assert_eq!(calc_ui::hover_text(&CalcState::EditedOutside), None, "that one has its bar");
}

/// A region edited outside WordCraft: with the caret on it, its bar offers Restore from
/// calculation and Keep Word's equation, which run `calc.resolveEdited`.
#[test]
fn calc_edited_outside_bar_resolves_the_region() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf ="})).unwrap();
    a.run("text.newParagraph", json!({})).unwrap();
    a.run("insert.calculation", json!({"source": "Q := 2 lbf ="})).unwrap();
    let set_edited = |a: &mut WordApp| {
        for path in a.session.doc.para_paths(StoryRef::Body) {
            if let Ok(p) = a.session.doc.para_mut(StoryRef::Body, &path) {
                for o in &mut p.objects {
                    if let InlineObject::Equation { calc: Some(c), .. } = o {
                        c.state = CalcState::EditedOutside;
                    }
                }
            }
        }
    };
    set_edited(&mut a);
    let (first, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection::caret(first);
    assert!(calc_ui::bar_target(&a).is_some(), "the caret is on an edited region");
    let mut h = ready(a);
    h.get_by_label("Restore from calculation").click();
    h.run_steps(3);
    let r = regions(h.state());
    assert_eq!(r[0].1.state, CalcState::Fresh, "restored from its source");
    assert_eq!(r[1].1.state, CalcState::EditedOutside, "the other is left alone");
    let second = r[1].0.clone();
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(second);
    h.run_steps(3);
    h.get_by_label("Keep Word's equation").click();
    h.run_steps(3);
    let r = regions(h.state());
    assert_eq!(r.len(), 1, "kept as a plain equation, no longer a calculation");
}

/// The bar doesn't show over a dialog (the source box opens where it would be).
#[test]
fn calc_edited_outside_bar_hides_behind_a_dialog() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf ="})).unwrap();
    set_state(&mut a, 0, CalcState::EditedOutside);
    let (at, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection::caret(at);
    let mut h = ready(a);
    assert!(h.query_by_label("Restore from calculation").is_some(), "the bar shows");
    h.state_mut().run("insert.calculation", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Restore from calculation").is_none(), "not while the box is open");
}

/// Hovering an errored calculation shows its message; hovering one edited outside WordCraft
/// brings up its bar (the caret elsewhere).
#[test]
fn calc_hover_shows_the_error_and_the_edited_bar() {
    let mut a = app();
    a.run("text.insert", json!({"text": "Bad "})).unwrap();
    a.run("insert.calculation", json!({"source": "x := 1 lbf + 2 ft =", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " and "})).unwrap();
    a.run("insert.calculation", json!({"source": "Q := 2 lbf =", "display": false})).unwrap();
    a.run("text.insert", json!({"text": " end"})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    set_state(&mut a, 1, CalcState::EditedOutside);
    a.run("caret.docStart", json!({})).unwrap();
    let r = regions(&a);
    let CalcState::Error(message) = r[0].1.state.clone() else { panic!("not an error: {:?}", r[0].1.state) };
    let mut h = timed(a);
    let first = on_screen(&mut h, &r[0].0);
    h.event(Event::PointerMoved(first));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_some(), "the error's message shows: {message}");
    assert!(h.query_by_label("Restore from calculation").is_none());
    let second = on_screen(&mut h, &r[1].0);
    h.event(Event::PointerMoved(second));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_none(), "the tooltip goes with the pointer");
    assert!(h.query_by_label("Restore from calculation").is_some(), "hovering brings up the bar");
    assert!(h.query_by_label("Keep Word's equation").is_some());
    // Out of both: the bar goes.
    h.event(Event::PointerMoved(Pos2::new(700.0, 850.0)));
    h.run_steps(4);
    assert!(h.query_by_label("Restore from calculation").is_none(), "leaving takes the bar away");
}

/// A refused resolve reaches the status bar.
#[test]
fn calc_resolve_refusal_shows_in_the_status_bar() {
    let mut a = app();
    calc_ui::resolve(&mut a, 9999, "source");
    let status = a.status.as_ref().map(|status| status.text.as_str()).unwrap_or_default();
    assert!(status.contains("no calculation 9999"), "{status:?}");
}

/// Go to existing finds a definition inside a table cell.
#[test]
fn calc_go_to_existing_finds_a_definition_in_a_table() {
    let mut a = app();
    a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf"})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    let (owner, region) = regions(&a).remove(0);
    assert!(owner.path.cell().is_some(), "the definition is in a cell");
    assert_eq!(calc_ui::region_pos(&a.session.doc, region.id), Some(owner.clone()));
    let mut h = ready(a);
    open_box(&mut h);
    type_text(&mut h, "P := 2 lbf");
    enter(&mut h);
    assert_eq!(calc_box(h.state()).and_then(|b| b.prompt).and_then(|p| p.existing), Some(region.id));
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Go to existing").click();
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none(), "the box closes");
    assert_eq!(h.state().session.sel.focus, owner, "the caret is at the owning region");
    assert_eq!(sources(h.state()), ["P := 1500 lbf"]);
}

/// Keyboard routes to the source box: Mod+Alt+= opens a new one, F2 the calculation at the caret,
/// and Enter a selected calculation; Enter beside one still starts a new paragraph.
#[test]
fn calc_keyboard_opens_the_source_box() {
    let mut a = app();
    a.run("insert.calculation", json!({"source": "P := 1500 lbf", "display": false})).unwrap();
    let mut h = ready(a);
    h.state_mut().canvas.focus_pending = true;
    h.run_steps(3);
    assert!(h.state().canvas.focused(&h.ctx), "the page has the keyboard");
    let id = regions(h.state())[0].1.id;
    // F2 with the caret just after the calculation.
    h.key_press(egui::Key::F2);
    h.run_steps(3);
    assert_eq!(calc_box(h.state()).and_then(|b| b.editing), Some(id), "F2 opens the box on the calculation");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none());
    // Enter with the calculation selected.
    let (at, _) = regions(h.state()).remove(0);
    let end = Pos { off: at.off + '\u{FFFC}'.len_utf8(), ..at.clone() };
    h.state_mut().run("select.range", json!({"anchor": at, "focus": end})).unwrap();
    h.state_mut().canvas.focus_pending = true;
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(calc_box(h.state()).and_then(|b| b.editing), Some(id), "Enter opens the box on a selected calculation");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // Enter with the caret beside it: a new paragraph, no box.
    h.state_mut().run("caret.set", json!({"pos": end})).unwrap();
    h.state_mut().canvas.focus_pending = true;
    h.run_steps(3);
    let paragraphs = h.state().session.doc.paragraph_count();
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none());
    assert_eq!(h.state().session.doc.paragraph_count(), paragraphs + 1);
    // Mod+Alt+= opens a new calculation.
    h.key_press_modifiers(egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), alt: true, ..Default::default() }, egui::Key::Equals);
    h.run_steps(3);
    assert_eq!(h.state().calc_dialog.as_ref().map(CalcDialog::name), Some("calculation"));
}

#[test]
fn right_click_menu_numbers_a_calculation() {
    let mut a = app();
    let _ = a.session.run("insert.calculation", &serde_json::json!({"source": "P := 1500 lbf =", "display": true}));
    let (at, _) = regions(&a).remove(0);
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    let pick = |a: &WordApp, label: &str| {
        calc_ui::menu_items(a).into_iter().find(|(l, _, _)| *l == label).unwrap_or_else(|| panic!("no {label} in the menu"))
    };
    let labels: Vec<&str> = calc_ui::menu_items(&a).iter().map(|(l, _, _)| *l).collect();
    assert_eq!(
        labels,
        ["Edit Calculation", "Result Format", "Change to Inline", "Align Left", "Align Center", "Align Right", "Add Equation Number", "Recalculate"]
    );
    let (_, id, params) = pick(&a, "Result Format");
    a.run(id, params).expect("opening the result format dialog from the menu works");
    assert_eq!(a.calc_dialog.take().as_ref().map(CalcDialog::name), Some("calcResultFormat"));
    let (_, id, params) = pick(&a, "Add Equation Number");
    a.session.run(id, &params).expect("numbering a calculation from the menu works");
    assert!(regions(&a)[0].1.numbered, "the calculation is numbered");
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    assert!(calc_ui::menu_items(&a).iter().any(|(l, _, _)| *l == "Remove Equation Number"));
    let (_, id, params) = pick(&a, "Align Left");
    a.session.run(id, &params).expect("aligning a calculation from the menu works");
    assert!(
        matches!(a.session.doc.para_at(&at).and_then(|p| p.object_at(at.off)), Some(InlineObject::Equation { math, .. }) if math.jc == wordcraft_doc::math::MathJc::Left)
    );
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at.clone() };
    let (_, id, params) = pick(&a, "Change to Inline");
    a.session.run(id, &params).expect("making a calculation inline from the menu works");
    a.session.sel = wordcraft_engine::Selection { anchor: at.clone(), focus: at };
    let labels: Vec<&str> = calc_ui::menu_items(&a).iter().map(|(l, _, _)| *l).collect();
    assert_eq!(
        labels,
        ["Edit Calculation", "Result Format", "Change to Display", "Add Equation Number", "Recalculate"],
        "inline: no alignment choices"
    );
    // Away from a calculation the menu adds nothing.
    assert!(calc_ui::menu_items(&app()).is_empty());
}

#[test]
fn right_clicking_a_calculation_offers_its_number() {
    let mut a = app();
    let _ = a.session.run("insert.calculation", &serde_json::json!({"source": "P := 1500 lbf =", "display": true}));
    let mut h = timed(a);
    let (at, _) = regions(h.state()).remove(0);
    let point = on_screen(&mut h, &at);
    h.event(Event::PointerMoved(point));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: point, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
    h.get_by_label_contains("Add Equation Number").click();
    h.run_steps(3);
    assert!(regions(h.state())[0].1.numbered, "the menu numbered the calculation");
}

#[test]
fn backslash_names_become_symbols() {
    // A space after the name turns it into the symbol and is used up, as in Word's math
    // autocorrect; any other character after it stays.
    assert_eq!(calc_ui::expand_shortcut("\\sigma ", 7), Some(("σ".to_string(), 1)));
    assert_eq!(calc_ui::expand_shortcut("x := \\theta+1", 12), Some(("x := θ+1".to_string(), 7)));
    assert_eq!(calc_ui::expand_shortcut("\\Delta ", 7), Some(("Δ".to_string(), 1)));
    assert_eq!(calc_ui::expand_shortcut("\\sqrt(x)", 6), Some(("√(x)".to_string(), 2)));
    // Unknown names, and names still being typed, are left alone.
    assert_eq!(calc_ui::expand_shortcut("\\sigm", 5), None);
    assert_eq!(calc_ui::expand_shortcut("\\nonsense ", 10), None);
    assert_eq!(calc_ui::expand_shortcut("plain text ", 11), None);
}

#[test]
fn condition_shortcuts_expand() {
    for (typed, symbol) in
        [("\\le ", "≤"), ("\\ge ", "≥"), ("\\ne ", "≠"), ("\\sum ", "∑"), ("\\wedge ", "\u{2227}"), ("\\vee ", "\u{2228}"), ("\\neg ", "¬")]
    {
        let n = typed.chars().count();
        assert_eq!(calc_ui::expand_shortcut(typed, n), Some((symbol.to_string(), 1)), "{typed}");
    }
    assert_eq!(calc_ui::expand_shortcut("\\nu ", 4), Some(("ν".to_string(), 1)), "Greek still wins its own names");
}

#[test]
fn symbol_menu_inserts_at_the_cursor() {
    let mut b = calc_ui::CalcBox::new(None, "x := 2", None);
    b.cursor = Some(1);
    b.insert_symbol("_0");
    assert_eq!(b.source, "x_0 := 2");
    assert_eq!(b.cursor, Some(3), "the cursor ends after what was inserted");
    let mut b = calc_ui::CalcBox::new(None, "A := ", None);
    b.cursor = None;
    b.insert_symbol("π");
    assert_eq!(b.source, "A := π", "with no cursor yet, at the end");
    assert!(calc_ui::SYMBOL_GROUPS.iter().any(|(_, s)| s.contains(&"σ")), "Greek letters are offered");
    assert!(calc_ui::SYMBOL_GROUPS.iter().any(|(_, s)| s.contains(&"g_0")), "the constants are offered");
}

#[test]
fn units_row_has_angstrom_and_percent() {
    let units = calc_ui::SYMBOL_GROUPS.iter().find(|(group, _)| *group == "Units").expect("a Units group").1;
    assert!(units.contains(&"Å"), "the angstrom is offered");
    assert!(units.contains(&"%"), "the percent is offered");
}

#[test]
fn insert_symbol_menu_works_in_the_box() {
    let mut h = ready(app());
    open_box(&mut h);
    type_text(&mut h, "\\sigma ");
    h.run_steps(3);
    assert_eq!(calc_box(h.state()).expect("open").source, "σ", "the shortcut became σ");
    type_text(&mut h, "_b := 2 kg·");
    h.run_steps(2);
    h.get_by_label("Insert Symbol").click();
    h.run_steps(3);
    h.get_by_label("g_0").click();
    h.run_steps(3);
    let source = calc_box(h.state()).expect("open").source;
    assert!(source.ends_with("2 kg·g_0"), "{source}");
    enter(&mut h);
    assert_eq!(sources(h.state()), ["σ_b := 2 kg·g_0"]);
}

// ---------------------------------------------------------------------------------------------
// Vectors and data tables

/// A table typed cell by cell (`insert.table`, then the cells' text with Tab between), with the
/// caret after it; answers its body block.
fn typed_table(a: &mut WordApp, rows: &[&[&str]]) -> usize {
    let r = a.run("insert.table", json!({"rows": rows.len(), "cols": rows[0].len()})).unwrap();
    let block = r["table"][0].as_u64().unwrap_or_else(|| panic!("no table in {r}")) as usize;
    for (k, text) in rows.iter().flat_map(|r| r.iter()).enumerate() {
        if k > 0 {
            a.run("text.tab", json!({})).unwrap();
        }
        a.run("text.insert", json!({"text": text})).unwrap();
    }
    a.run("caret.docEnd", json!({})).unwrap();
    block
}

/// The caret in cell (`row`, `col`) of the table at body block `block`.
fn cell_pos(block: usize, row: u32, col: u32) -> Pos {
    Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![block as u32, row, col, 0]), off: 0 }
}

fn table_calc(a: &WordApp, block: usize) -> Option<wordcraft_doc::calc::CalcTable> {
    match &*a.session.doc.body[block] {
        wordcraft_doc::Block::Table(t) => t.calc.as_deref().cloned(),
        wordcraft_doc::Block::Para(_) => panic!("block {block} isn't a table"),
    }
}

fn right_click(h: &mut egui_kittest::Harness<'static, WordApp>, at: &Pos) {
    let caret = h.state_mut().session.layout().caret(at).expect("the caret is laid out");
    let point = crate::canvas::page_to_screen(h.state_mut(), caret.page, caret.x + 2.0, caret.top + caret.height / 2.0).expect("on screen");
    h.state_mut().session.sel = wordcraft_engine::Selection::caret(at.clone());
    h.event(Event::PointerMoved(point));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: point, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.run_steps(3);
}

/// The open context menu's item whose name starts `label` (the name carries its shortcut too).
fn menu_item<'h>(h: &'h egui_kittest::Harness<'static, WordApp>, label: &'h str) -> Option<egui_kittest::Node<'h>> {
    use egui_kittest::kittest::NodeT;
    h.query_all(egui_kittest::kittest::By::new().label(label))
        .find(|n| matches!(n.accesskit_node().role(), egui::accesskit::Role::Button | egui::accesskit::Role::MenuItem))
}

/// Right-click in a table: Use as Calculation Data marks it; the menu then offers Stop Using as
/// Calculation Data. Only a table in the main text offers either.
#[test]
fn table_menu_marks_calculation_data() {
    let mut a = app();
    let t = typed_table(&mut a, &[&["D (in)", "qty"], &["0.5", "3"]]);
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let mut h = timed(a);
    right_click(&mut h, &cell_pos(t, 1, 0));
    assert!(menu_item(&h, "Stop Using as Calculation Data").is_none());
    menu_item(&h, "Use as Calculation Data").expect("the table's menu offers it").click();
    h.run_steps(3);
    assert!(table_calc(h.state(), t).is_some(), "the menu marked the table");
    right_click(&mut h, &cell_pos(t, 1, 1));
    assert!(menu_item(&h, "Use as Calculation Data").is_none(), "a marked table isn't offered marking again");
    menu_item(&h, "Stop Using as Calculation Data").expect("a marked table's menu offers it").click();
    h.run_steps(3);
    assert!(table_calc(h.state(), t).is_none(), "the menu unmarked the table");
    // What the menu offers, by where the caret is.
    let mut a = app();
    let t = typed_table(&mut a, &[&["D (in)"], &["0.5"]]);
    assert_eq!(calc_ui::data_table_item(&a), None, "the caret after the table");
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(t, 1, 0));
    assert_eq!(calc_ui::data_table_item(&a), Some(("Use as Calculation Data", json!({"on": true}))));
    assert!(crate::app::command_board::command_enabled(&a, "calc.dataTable"), "the item can be clicked");
    a.run("calc.dataTable", json!({"on": true})).unwrap();
    assert_eq!(calc_ui::data_table_item(&a), Some(("Stop Using as Calculation Data", json!({"on": false}))));
    // A table in a header: the command refuses there, so the menu doesn't offer it.
    a.run("insert.header", json!({"text": "Head"})).unwrap();
    a.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
    assert!(a.session.sel.focus.story != StoryRef::Body && a.session.sel.focus.path.cell().is_some(), "the caret is in a header table");
    assert_eq!(calc_ui::data_table_item(&a), None);
}

/// Calculation Options: Arrays count from 0 (Mathcad) or 1, sent as `origin`.
#[test]
fn options_choose_array_origin() {
    let mut a = app();
    a.run("calc.settings", json!({})).unwrap();
    let mut h = ready(a);
    assert!(h.query_by_label("Arrays count from").is_some(), "the choice is labelled");
    assert!(h.query_by_label("0 (Mathcad)").is_some());
    // Changing it renumbers what quoted elements show: the dialog says so.
    assert!(h.query_by_label("Changing this renumbers every index, including quoted elements.").is_some(), "the renumbering note is shown");
    h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "1").click();
    h.run_steps(2);
    let params = match h.state().calc_dialog.as_ref() {
        Some(d) => match d {
            CalcDialog::Options(f) => f.params(),
            _ => panic!("not the options"),
        },
        _ => panic!("no dialog"),
    };
    assert_eq!(params["origin"], json!(1), "{params}");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
    h.run_steps(3);
    assert!(h.state().calc_dialog.is_none());
    assert_eq!(h.state().session.doc.calc_settings.clone().map(|s| s.origin), Some(1));
    // Read back when the dialog opens again.
    assert_eq!(calc_ui::OptionsForm::read(&h.state().session.doc).origin, 1);
}

/// The fields' results in the body, in order.
fn field_results(a: &WordApp) -> Vec<String> {
    a.session
        .doc
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| a.session.doc.para(StoryRef::Body, p))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

fn value_form(a: &WordApp) -> calc_ui::ValueForm {
    match a.calc_dialog.as_ref() {
        Some(d) => match d {
            CalcDialog::Value(f) => f.clone(),
            _ => panic!("not the picker"),
        },
        _ => panic!("no dialog"),
    }
}

/// The form's Insert button (the ribbon's Insert tab, at the top, has the same name).
fn click_insert(h: &mut egui_kittest::Harness<'static, WordApp>) {
    let lowest = h.get_all_by_role_and_label(egui::accesskit::Role::Button, "Insert").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    lowest.expect("the form's Insert button").click();
    h.run_steps(3);
}

fn set_value_form(h: &mut egui_kittest::Harness<'static, WordApp>, change: impl FnOnce(&mut calc_ui::ValueForm)) {
    if let Some(d) = h.state_mut().calc_dialog.as_mut()
        && let CalcDialog::Value(f) = d
    {
        change(f);
    }
    h.run_steps(2);
}

/// The Calculated Value form's optional Element box: `1` sends `"index": [1]`, `1, 0` sends
/// `[1, 0]`; other text is refused in the form, and the command's refusals show there too.
#[test]
fn calculated_value_takes_an_index() {
    // What the box reads.
    for (text, index) in
        [("", None), ("  ", None), ("1", Some(vec![1])), (" 2 ", Some(vec![2])), ("1, 0", Some(vec![1, 0])), ("1 ,0", Some(vec![1, 0]))]
    {
        assert_eq!(calc_ui::parse_index(text), Ok(index), "{text:?}");
    }
    for bad in ["x", "-1", "1.5", "1,", ",1", "1, 2, 3", "1 0", "[1]", "4294967296"] {
        assert!(calc_ui::parse_index(bad).is_err(), "{bad:?} is refused");
    }
    let mut a = app();
    for s in ["D := [0.840, 1.050] in", "M := [1, 2; 3, 4]", "x := 2 in"] {
        a.run("insert.calculation", json!({"source": s})).unwrap();
        a.run("text.newParagraph", json!({})).unwrap();
    }
    a.run("insert.calcValue", json!({})).unwrap();
    let mut h = ready(a);
    assert!(h.query_by_label("Element").is_some(), "the Element box is drawn");
    let f = value_form(h.state());
    assert_eq!(f.index, "", "empty: the whole value");
    set_value_form(&mut h, |f| {
        f.name = "D".into();
        f.index = "1".into();
    });
    assert_eq!(value_form(h.state()).params(), json!({"name": "D", "index": [1]}));
    set_value_form(&mut h, |f| {
        f.name = "M".into();
        f.index = " 1 , 0 ".into();
    });
    assert_eq!(value_form(h.state()).params(), json!({"name": "M", "index": [1, 0]}));
    // Other text: refused in the form with a hint; nothing is sent.
    set_value_form(&mut h, |f| f.index = "first".into());
    click_insert(&mut h);
    assert!(h.state().calc_dialog.is_some(), "the form stays open");
    let message = value_form(h.state()).message;
    assert!(!message.is_empty(), "a hint says why");
    assert!(h.query_by_label(&message).is_some(), "the hint is visible: {message}");
    assert!(field_results(h.state()).is_empty(), "nothing inserted");
    // The command's refusal shows in the form.
    set_value_form(&mut h, |f| {
        f.name = "x".into();
        f.index = "0".into();
    });
    click_insert(&mut h);
    assert_eq!(value_form(h.state()).message, "x isn't a vector");
    assert!(h.query_all(egui_kittest::kittest::By::new().label("x isn't a vector")).next().is_some());
    assert!(field_results(h.state()).is_empty(), "nothing inserted");
    // An element of the matrix, typed into the box.
    set_value_form(&mut h, |f| {
        f.name = "M".into();
        f.index = String::new();
    });
    h.get_by_label("Element").click();
    h.run_steps(2);
    type_text(&mut h, "1, 0");
    assert_eq!(value_form(h.state()).index, "1, 0", "typed into the box");
    click_insert(&mut h);
    assert!(h.state().calc_dialog.is_none(), "inserted");
    assert_eq!(field_results(h.state()), ["3"]);
}

/// Every error of a data table, grouped by where its marker goes: one per column with an error
/// (all its messages, in order), and the first cell for an error about the whole table.
#[test]
fn table_errors_group_by_column() {
    use wordcraft_doc::calc::ColumnError;
    let e = |col: Option<u32>, m: &str| ColumnError { col, message: m.into() };
    let errors = [e(Some(1), "D_o row 2 isn't a number"), e(None, "merged"), e(Some(1), "D_o row 3 isn't a number"), e(Some(0), "bad name")];
    assert_eq!(
        calc_ui::error_markers(&errors),
        [(1, "D_o row 2 isn't a number\nD_o row 3 isn't a number".to_string()), (0, "merged\nbad name".to_string())]
    );
    assert!(calc_ui::error_markers(&[]).is_empty());
}

/// A data table with a column error: a red ⚠ at the top-right of that heading cell (the first
/// cell for an error about the whole table), its message on hover; the document is untouched.
#[test]
fn table_error_is_marked_on_screen() {
    let mut a = app();
    let t = typed_table(&mut a, &[&["D (in)", "D_o (in)"], &["0.5", "0.840"], &["0.75", "1.050"]]);
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(t, 0, 0));
    a.run("calc.dataTable", json!({"on": true})).unwrap();
    // A cell that isn't a number: an error for its column.
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(t, 2, 1));
    a.run("text.insert", json!({"text": "abc "})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    let errors = table_calc(&a, t).expect("marked").errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].col, Some(1), "{errors:?}");
    let message = errors[0].message.clone();
    let before = a.session.doc.clone();
    let mut h = timed(a);
    let font = egui::TextStyle::Body.resolve(&h.ctx.global_style());
    assert!(h.ctx.fonts_mut(|f| f.has_glyph(&font, '⚠')), "the interface can draw the marker");
    let marker = h.get_by_label("⚠");
    let bounds = marker.rect();
    // The heading cell of the column, on screen.
    let range =
        wordcraft_engine::CellRange { story: StoryRef::Body, table: wordcraft_doc::Path(vec![t as u32]), rows: (0, 0), cols: (1, 1), stream: false };
    let (page, cell) = {
        let app = h.state_mut();
        let layout = app.session.layout();
        wordcraft_engine::cmd::table_mouse::cell_rects(&layout, &app.session.doc, &range)[0]
    };
    let top_right = crate::canvas::page_to_screen(h.state_mut(), page, cell.x + cell.w, cell.y).expect("on screen");
    let top_left = crate::canvas::page_to_screen(h.state_mut(), page, cell.x, cell.y).expect("on screen");
    assert!(bounds.right() <= top_right.x + 1.0 && bounds.right() >= top_right.x - 24.0, "at the cell's right: {bounds:?} vs {top_right:?}");
    assert!(bounds.left() > (top_left.x + top_right.x) / 2.0, "in the right half of the cell: {bounds:?}");
    assert!((bounds.top() - top_right.y).abs() < 12.0, "at the cell's top: {bounds:?} vs {top_right:?}");
    assert!(h.query_by_label(&message).is_none(), "the message waits for the pointer");
    h.event(Event::PointerMoved(bounds.center()));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_some(), "hovering the marker shows the message: {message}");
    h.event(Event::PointerMoved(Pos2::new(700.0, 850.0)));
    h.run_steps(4);
    assert!(h.query_by_label(&message).is_none(), "the message goes with the pointer");
    assert!(h.state().session.doc.same_content(&before), "the marker is never written into the document");
    // An error about the whole table: the marker is on the first cell.
    let mut a = app();
    let t = typed_table(&mut a, &[&["D (in)", "qty"], &["0.5", "3"]]);
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(t, 0, 0));
    a.run("calc.dataTable", json!({"on": true})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    a.run("view.commentsPane", json!({"value": false})).unwrap();
    if let Some(c) = a.session.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(vec![t as u32])).ok().and_then(|t| t.calc.as_mut()) {
        c.errors = vec![wordcraft_doc::calc::ColumnError { col: None, message: "a calculation data table can't have merged cells".into() }];
    }
    let mut h = timed(a);
    let bounds = h.get_by_label("⚠").rect();
    let range =
        wordcraft_engine::CellRange { story: StoryRef::Body, table: wordcraft_doc::Path(vec![t as u32]), rows: (0, 0), cols: (0, 0), stream: false };
    let (page, cell) = {
        let app = h.state_mut();
        let layout = app.session.layout();
        wordcraft_engine::cmd::table_mouse::cell_rects(&layout, &app.session.doc, &range)[0]
    };
    let top_right = crate::canvas::page_to_screen(h.state_mut(), page, cell.x + cell.w, cell.y).expect("on screen");
    assert!(bounds.right() <= top_right.x + 1.0 && bounds.right() >= top_right.x - 24.0, "at the first cell's right: {bounds:?} vs {top_right:?}");
    assert!((bounds.top() - top_right.y).abs() < 12.0, "at the first cell's top: {bounds:?} vs {top_right:?}");
    h.event(Event::PointerMoved(bounds.center()));
    h.run_steps(4);
    assert!(h.query_by_label("a calculation data table can't have merged cells").is_some());
}

/// The walk behind the table markers: nothing while no table has an error (ordinary tables, data
/// tables without errors), and every erroring data table at any depth, by its path, in document
/// order.
#[test]
fn erroring_tables_are_found_at_any_depth() {
    let mut a = app();
    let outer = typed_table(&mut a, &[&["x", "y"], &["1", "2"]]);
    assert!(calc_ui::erroring_tables(&a.session.doc.body).is_empty(), "an ordinary table");
    // A data table inside a cell of the outer table, and one after it.
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(outer, 1, 1));
    let r = a.run("insert.table", json!({"rows": 2, "cols": 1})).unwrap();
    let inner: Vec<u32> = r["table"].as_array().expect("a path").iter().filter_map(|v| v.as_u64()).map(|v| v as u32).collect();
    assert_eq!(inner.len(), 4, "nested in a cell: {r}");
    a.session.sel =
        wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: wordcraft_doc::Path([inner.clone(), vec![0, 0, 0]].concat()), off: 0 });
    a.run("calc.dataTable", json!({"on": true})).unwrap();
    a.run("caret.docEnd", json!({})).unwrap();
    let after = typed_table(&mut a, &[&["D (in)"], &["0.5"]]);
    a.session.sel = wordcraft_engine::Selection::caret(cell_pos(after, 1, 0));
    a.run("calc.dataTable", json!({"on": true})).unwrap();
    let marked = |a: &WordApp, path: &[u32]| {
        a.session.doc.table(StoryRef::Body, &wordcraft_doc::Path(path.to_vec())).and_then(|t| t.calc.as_deref().cloned()).is_some()
    };
    assert!(marked(&a, &inner) && marked(&a, &[after as u32]), "both are data tables");
    let error = |col: Option<u32>, m: &str| wordcraft_doc::calc::ColumnError { col, message: m.into() };
    let set = |a: &mut WordApp, path: &[u32], errors: Vec<wordcraft_doc::calc::ColumnError>| {
        if let Some(c) = a.session.doc.table_mut(StoryRef::Body, &wordcraft_doc::Path(path.to_vec())).ok().and_then(|t| t.calc.as_mut()) {
            c.errors = errors;
        }
    };
    set(&mut a, &inner, Vec::new());
    set(&mut a, &[after as u32], Vec::new());
    assert!(calc_ui::erroring_tables(&a.session.doc.body).is_empty(), "data tables without errors");
    set(&mut a, &[after as u32], vec![error(Some(0), "D row 1 isn't a number")]);
    set(&mut a, &inner, vec![error(None, "a calculation data table needs a row of headings and at least one row of values")]);
    let found = calc_ui::erroring_tables(&a.session.doc.body);
    assert_eq!(
        found,
        [
            (wordcraft_doc::Path(inner), vec![(0, "a calculation data table needs a row of headings and at least one row of values".to_string())]),
            (wordcraft_doc::Path(vec![after as u32]), vec![(0, "D row 1 isn't a number".to_string())]),
        ]
    );
}

#[test]
fn worksheet_import_review_jumps_to_the_reported_region() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30"><regions>
        <region top="10"><text><p>Worksheet introduction</p></text></region>
        <region top="30" region-id="42"><component/></region>
    </regions></worksheet>"#;
    let mut h = ready(app());
    let opened =
        h.state_mut().run("file.open", json!({"path": "review.xmcd", "data": wordcraft_engine::cmd::insert::base64_encode(worksheet)})).unwrap();
    let pos: Pos = serde_json::from_value(opened["importReview"][0]["pos"].clone()).unwrap();
    h.run_steps(3);
    h.get_by_label("Import warning: [Mathcad region 42: external component was not imported]").click();
    h.run_steps(3);
    assert_eq!(h.state().session.sel.focus, pos);
    let document = h.state().session.document_id();
    h.state_mut().run("calc.importReview", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Import warning: [Mathcad region 42: external component was not imported]").click();
    h.run_steps(3);
    assert_eq!(h.state().session.document_id(), document);
    assert_eq!(h.state().session.sel.focus, pos);
}

#[test]
fn worksheet_import_review_expands_a_root_errors_dependents() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30"><regions>
        <region top="10"><math><m:define><m:id>RootFailure</m:id><m:id>MissingValue</m:id></m:define></math></region>
        <region top="30"><math><m:define><m:id>ChildFailure</m:id><m:id>RootFailure</m:id></m:define></math></region>
    </regions></worksheet>"#;
    let mut h = ready(app());
    h.state_mut().run("file.open", json!({"path": "groups.xmcd", "data": wordcraft_engine::cmd::insert::base64_encode(worksheet)})).unwrap();
    let child = "Calculation error ChildFailure := RootFailure: depends on a calculation with an error";
    h.run_steps(3);
    assert!(h.query_by_label(child).is_none());
    h.get_by_label("Show dependent calculations (1)").click();
    h.run_steps(3);
    assert!(h.query_by_label(child).is_some());
}
