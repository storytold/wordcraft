use serde_json::{Value, json};
use wordcraft_doc::calc::{CalcRegion, CalcState};
use wordcraft_doc::math::Math;
use wordcraft_doc::{Block, Document, InlineObject, Pos};

use crate::{Session, catalog};

thread_local! {
    /// Makes the pass panic on its own stack (see `on_pass_stack`), to test that the panic is
    /// contained.
    static INJECT_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the pass should panic (read on the caller's thread: the pass panics on its own).
pub(super) fn injecting_panic() -> bool {
    INJECT_PANIC.with(std::cell::Cell::get)
}

fn session() -> Session {
    let mut s = Session::new(Document::new());
    s.run("document.setText", &json!({"text": ""})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Every equation and field in the body, tables included, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    fn walk(blocks: &[std::sync::Arc<Block>], out: &mut Vec<InlineObject>) {
        for b in blocks {
            match &**b {
                Block::Para(p) => out.extend(p.objects.iter().cloned()),
                Block::Table(t) => {
                    for row in &t.rows {
                        for cell in &row.cells {
                            walk(&cell.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}

/// (region, linear) of every calculation region in the body.
fn regions(s: &Session) -> Vec<(CalcRegion, String)> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: Some(c), .. } => Some((*c, linear)),
            _ => None,
        })
        .collect()
}

fn region(s: &Session, id: u64) -> (CalcRegion, String) {
    regions(s).into_iter().find(|(r, _)| r.id == id).unwrap_or_else(|| panic!("no region {id}"))
}

fn field_results(s: &Session) -> Vec<String> {
    objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

/// Inserts a calculation on a line of its own; returns its id.
fn calc(s: &mut Session, source: &str) -> u64 {
    let r = run(s, "insert.calculation", json!({"source": source, "display": true}));
    run(s, "text.newParagraph", json!({}));
    r["id"].as_u64().unwrap_or_else(|| panic!("no id in {r}"))
}

/// `P`, `A_b` and `σ := P/A_b =`.
fn sample(s: &mut Session) -> (u64, u64, u64) {
    (calc(s, "P := 1500 lbf"), calc(s, "A_b := 2.5 in^2"), calc(s, "σ := P/A_b ="))
}

#[test]
fn insert_and_recalculate() {
    let mut s = session();
    let (p, a, sigma) = sample(&mut s);
    assert!(p < a && a < sigma, "fresh ids {p} {a} {sigma}");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=600 \"psi\""), "{linear}");
}

#[test]
fn fresh_ids_never_reuse_one() {
    let mut s = session();
    let (p, a, _) = sample(&mut s);
    let ids: Vec<u64> = regions(&s).iter().map(|(r, _)| r.id).collect();
    let max = *ids.iter().max().unwrap();
    let next = calc(&mut s, "x := 1");
    assert_eq!(next, max + 1);
    assert_ne!(next, p);
    assert_ne!(next, a);
}

#[test]
fn editing_input_updates_result() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
}

#[test]
fn set_source_on_an_unknown_id_is_refused() {
    let mut s = session();
    sample(&mut s);
    let before = s.doc.clone();
    assert!(s.run("calc.setSource", &json!({"id": 999, "source": "q := 1"})).is_err());
    assert!(s.doc.same_content(&before));
}

#[test]
fn duplicate_insert_is_refused() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    let before = s.doc.clone();
    let e = s.run("insert.calculation", &json!({"source": "P := 2 lbf"})).unwrap_err();
    assert!(e.to_string().contains("already defined"), "{e}");
    assert!(s.doc.same_content(&before), "the document is unchanged");
    // A source that doesn't parse is still inserted: it shows its error at the region.
    let bad = calc(&mut s, "q := (");
    assert!(matches!(region(&s, bad).0.state, CalcState::Error(_)));
    // Renaming a region to a name another owns is refused too.
    let e = s.run("calc.setSource", &json!({"id": bad, "source": "A_b := 1 in^2"})).unwrap_err();
    assert!(e.to_string().contains("already defined"), "{e}");
    // Its own name is fine.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 1500 lbf"}));

    let r = run(&mut s, "calc.checkName", json!({"name": "P"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert_eq!(r["reason"], json!("duplicate"), "{r}");
    assert_eq!(r["existing"], json!(p), "{r}");
    assert!(r["message"].as_str().unwrap().contains("already defined"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "m"}));
    assert_eq!(r["ok"], json!(false), "{r}");
    assert_eq!(r["reason"], json!("reserved"), "{r}");
    assert!(r["message"].as_str().unwrap().contains("unit, function or constant"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "a_"}));
    assert_eq!(r["reason"], json!("reserved"), "{r}");
    let r = run(&mut s, "calc.checkName", json!({"name": "P", "id": p}));
    assert_eq!(r, json!({"ok": true}));
    let r = run(&mut s, "calc.checkName", json!({"name": "brand_new"}));
    assert_eq!(r, json!({"ok": true}));
}

#[test]
fn duplicate_message_names_page() {
    let mut s = session();
    sample(&mut s);
    // A pasted copy of P's definition (the paste bypasses the name check).
    copy_object(&mut s, 0);
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let (r, _) = regions(&s).pop().unwrap();
    assert_eq!(r.source, "P := 1500 lbf");
    match r.state {
        CalcState::Error(m) => assert!(m.contains("already defined on page 1"), "{m}"),
        other => panic!("{other:?}"),
    }
}

/// Each name of `[a; b] := …` is checked as a single definition's is.
#[test]
fn a_definition_of_several_names_is_checked_name_by_name() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    let before = s.doc.clone();
    for source in ["[P; q] := [1; 2]", "[q; P] := [1; 2]", "[q; m] := [1; 2]"] {
        let e = s.run("insert.calculation", &json!({"source": source})).unwrap_err();
        let want = if source.contains('m') { "unit, function or constant" } else { "P is already defined" };
        assert!(e.to_string().contains(want), "{source}: {e}");
        assert!(s.doc.same_content(&before), "{source}: the document is unchanged");
    }
    let many = calc(&mut s, "[q; r] := [1; 2]");
    // Renaming a region so one of its names is another's is refused; its own names are fine.
    let e = s.run("calc.setSource", &json!({"id": many, "source": "[q; A_b] := [1; 2]"})).unwrap_err();
    assert!(e.to_string().contains("A_b is already defined"), "{e}");
    run(&mut s, "calc.setSource", json!({"id": many, "source": "[r; q] := [3; 4]"}));
    let e = s.run("calc.setSource", &json!({"id": p, "source": "q := 1"})).unwrap_err();
    assert!(e.to_string().contains("q is already defined"), "{e}");
}

/// A pasted copy of `[a; b] := …` names the page of the first, as a single definition's does.
#[test]
fn a_copied_definition_of_several_names_names_the_page() {
    let mut s = session();
    calc(&mut s, "[q; r] := [1; 2]");
    sample(&mut s);
    copy_object(&mut s, 0);
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let (r, _) = regions(&s).pop().unwrap();
    assert_eq!(r.source, "[q; r] := [1; 2]");
    match r.state {
        CalcState::Error(m) => assert!(m.contains("q is already defined on page 1"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn undo_restores_results() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(region(&s, p).0.source, "P := 1500 lbf");
    assert!(region(&s, sigma).1.ends_with("=600 \"psi\""), "{}", region(&s, sigma).1);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
}

#[test]
fn manual_mode_marks_stale() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    // Switching to manual changes no result: they are still current.
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh);
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(linear.contains('≈'), "{linear}");
    let report = run(&mut s, "calc.recalculate", json!({}));
    assert_eq!(report["regions"], json!(3), "{report}");
    assert_eq!(report["errors"], json!(0), "{report}");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Fresh);
    assert!(linear.ends_with("=1200 \"psi\""), "{linear}");
    // Undo puts the stale results back exactly; it doesn't recalculate.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    // Back to automatic recalculates.
    run(&mut s, "calc.settings", json!({"manual": false}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh);
}

#[test]
fn a_command_that_changes_nothing_stays_a_no_op_in_manual_mode() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "caret.docStart", json!({}));
    let undo = s.undo_depth();
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(s.undo_depth(), undo, "Backspace at the start adds no undo step");
    assert!(regions(&s).iter().all(|(r, _)| r.state == CalcState::Fresh));
}

#[test]
fn settings_change_the_display() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    let r = run(&mut s, "calc.settings", json!({"system": "si", "precision": 2}));
    assert_eq!(r["system"], json!("si"), "{r}");
    assert_eq!(r["precision"], json!(2), "{r}");
    assert_eq!(r["manual"], json!(false), "{r}");
    let linear = region(&s, sigma).1;
    assert!(!linear.contains("psi"), "{linear}");
    assert!(s.run("calc.settings", &json!({"precision": 16})).is_err());
    assert!(s.run("calc.settings", &json!({"system": "metric"})).is_err());
}

#[test]
fn resolve_edited_keep_word() {
    let mut s = session();
    let (_, a, sigma) = sample(&mut s);
    let names = run(&mut s, "calc.names", json!({}));
    assert!(names.to_string().contains("\"A_b\""), "{names}");
    run(&mut s, "calc.resolveEdited", json!({"id": a, "keep": "word"}));
    assert!(regions(&s).iter().all(|(r, _)| r.id != a), "A_b is a plain equation now");
    let plain = objects(&s.doc).into_iter().filter(|o| matches!(o, InlineObject::Equation { calc: None, .. })).count();
    assert_eq!(plain, 1);
    let names = run(&mut s, "calc.names", json!({}));
    assert!(!names.to_string().contains("\"A_b\""), "{names}");
    // σ depends on a name that is gone now.
    assert!(matches!(region(&s, sigma).0.state, CalcState::Error(_)));
}

#[test]
fn resolve_edited_keep_source() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    let fresh = region(&s, p).1;
    // Something other than the calculator rewrote P's equation.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Body)[0].clone();
    let para = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &path).unwrap();
    if let Some(InlineObject::Equation { linear, calc: Some(c), .. }) = para.objects.get_mut(0) {
        *linear = "P=1".into();
        c.state = CalcState::EditedOutside;
    }
    run(&mut s, "calc.resolveEdited", json!({"id": p, "keep": "source"}));
    let (r, linear) = region(&s, p);
    assert_eq!(r.state, CalcState::Fresh);
    assert_eq!(linear, fresh);
    assert!(s.run("calc.resolveEdited", &json!({"id": p, "keep": "both"})).is_err());
}

#[test]
fn calc_value_quotes_a_variable() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "ksi", "precision": 2}));
    assert_eq!(field_results(&s), ["0.6 ksi"]);
    let field = objects(&s.doc).into_iter().find(|o| matches!(o, InlineObject::Field { .. })).unwrap();
    assert!(matches!(field, InlineObject::Field { locked: true, ref instr, .. } if instr == "WCCALC σ \\u ksi \\p 2"), "{field:?}");
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(field_results(&s), ["1.2 ksi"]);
    // Not a variable, a precision over 15: refused, nothing inserted.
    let e = s.run("insert.calcValue", &json!({"name": "nope"})).unwrap_err();
    assert!(e.to_string().contains("nope isn't a defined variable"), "{e}");
    assert!(s.run("insert.calcValue", &json!({"name": "P", "precision": 16})).is_err());
    assert_eq!(field_results(&s).len(), 1);
}

#[test]
fn calc_value_outside_the_body_is_refused() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.editHeader", json!({}));
    let e = s.run("insert.calcValue", &json!({"name": "P"})).unwrap_err();
    assert!(e.to_string().contains("body"), "{e}");
}

#[test]
fn equation_commands_refuse_a_calculation() {
    let mut s = session();
    run(&mut s, "insert.equation", json!({"linear": "x^2", "edit": false}));
    run(&mut s, "text.newParagraph", json!({}));
    let p = calc(&mut s, "P := 1500 lbf");
    let before = region(&s, p);
    // Caret just after P's equation.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Body)[1].clone();
    s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off: wordcraft_doc::para::OBJ.len_utf8() });
    for (id, v) in [("equation.set", json!({"linear": "y"})), ("equation.edit", json!({}))] {
        let e = s.run(id, &v).unwrap_err();
        assert!(e.to_string().contains("calculation source"), "{id}: {e}");
    }
    assert_eq!(region(&s, p), before);
    // Convert All converts the plain equation and leaves the calculation alone.
    let r = run(&mut s, "equation.convert", json!({"to": "linear", "all": true}));
    assert_eq!(r["converted"], json!(1), "{r}");
    assert_eq!(region(&s, p), before);
}

#[test]
fn plain_documents_pay_nothing() {
    let mut s = session();
    run(&mut s, "text.insert", json!({"text": "hello"}));
    assert!(!super::has_calc(&s.doc));
    calc(&mut s, "x := 1");
    assert!(super::has_calc(&s.doc));
}

#[test]
fn hostile_parameters_are_refused() {
    let mut s = session();
    let huge = "x := ".to_string() + &"1+".repeat(20_000) + "1";
    assert!(s.run("insert.calculation", &json!({"source": huge})).is_err());
    assert!(s.run("insert.calculation", &json!({})).is_err());
    assert!(s.run("calc.setSource", &json!({"id": "x", "source": "y := 1"})).is_err());
    assert!(s.run("calc.checkName", &json!({})).is_err());
    assert!(s.run("calc.resolveEdited", &json!({"id": 1})).is_err());
}

#[test]
fn catalog_has_calc_commands() {
    let reg = crate::cmd::registry();
    let e = catalog::entries();
    for id in ["insert.calculation", "insert.calcValue", "calc.recalculate", "calc.settings"] {
        assert!(e.iter().any(|x| x.3 == id), "{id} catalogued");
        assert!(reg.get(id).is_some(), "{id} registered");
    }
    for id in ["calc.setSource", "calc.names", "calc.checkName", "calc.resolveEdited", "calc.dataTable"] {
        assert!(reg.get(id).is_some(), "{id} registered");
    }
}

/// The region `id`'s equation in the body, to change behind the calculator's back.
fn edit_outside(s: &mut Session, id: u64, shown: &str) {
    for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
        let para = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &path).unwrap();
        for o in para.objects.iter_mut() {
            if let InlineObject::Equation { linear, calc: Some(c), .. } = o
                && c.id == id
            {
                *linear = shown.into();
                c.state = CalcState::EditedOutside;
            }
        }
    }
}

#[test]
fn calculation_outside_the_body_is_refused() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.editHeader", json!({}));
    let before = s.doc.clone();
    let e = s.run("insert.calculation", &json!({"source": "h := 1 in"})).unwrap_err();
    assert!(e.to_string().contains("body"), "{e}");
    assert!(s.doc.same_content(&before));
}

#[test]
fn a_panic_in_the_pass_is_contained() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    INJECT_PANIC.with(|c| c.set(true));
    let r = s.run("calc.setSource", &json!({"id": p, "source": "P := 3000 lbf"}));
    INJECT_PANIC.with(|c| c.set(false));
    // The edit stands, no result looks current, and the status line says why.
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(region(&s, p).0.source, "P := 3000 lbf");
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(linear.contains('≈'), "{linear}");
    assert!(s.status.contains("injected failure"), "{}", s.status);
    // The next edit recalculates.
    run(&mut s, "text.insert", json!({"text": "x"}));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
}

#[test]
fn set_source_drops_a_result_typed_outside() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    edit_outside(&mut s, sigma, "σ=999 \"psi\"");
    run(&mut s, "calc.setSource", json!({"id": sigma, "source": "σ := P/A_b = ? ksi"}));
    let (r, linear) = region(&s, sigma);
    assert_eq!(r.state, CalcState::Stale);
    assert!(!linear.contains("999"), "{linear}");
}

#[test]
fn check_name_wants_one_plain_name() {
    let mut s = session();
    sample(&mut s);
    for name in ["f(x)", "a b", "", "  ", "x := 2"] {
        let r = run(&mut s, "calc.checkName", json!({"name": name}));
        assert_eq!(r["ok"], json!(false), "{name}: {r}");
        assert_eq!(r["reason"], json!("reserved"), "{name}: {r}");
    }
    assert_eq!(run(&mut s, "calc.checkName", json!({"name": " q "})), json!({"ok": true}));
}

#[test]
fn a_pasted_copy_is_found_by_its_recalculated_id_in_manual_mode() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    // A pasted copy of σ's equation, renamed, keeping σ's id (manual mode doesn't renumber it).
    let end = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    let copy = InlineObject::Equation {
        linear: String::new(),
        display: true,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion {
            result_format: None,
            worksheet_order: None,
            id: sigma,
            source: "τ := 2*P/A_b =".into(),
            unit: None,
            precision: None,
            numbered: false,
            state: CalcState::Fresh,
        })),
    };
    s.doc.insert_object(&end, copy, &Default::default()).unwrap();
    run(&mut s, "text.insert", json!({"text": "x"}));
    let names = run(&mut s, "calc.names", json!({}));
    let tau = names["names"].as_array().unwrap().iter().find(|n| n["name"] == "τ").unwrap()["id"].as_u64().unwrap();
    assert_ne!(tau, sigma);
    run(&mut s, "calc.setSource", json!({"id": tau, "source": "τ := 3*P/A_b ="}));
    let sources: Vec<String> = regions(&s).into_iter().map(|(r, _)| r.source).collect();
    assert!(sources.contains(&"σ := P/A_b =".to_string()), "{sources:?}");
    assert!(sources.contains(&"τ := 3*P/A_b =".to_string()), "{sources:?}");
    // The copy's own name is its own; σ's isn't.
    assert_eq!(run(&mut s, "calc.checkName", json!({"name": "τ", "id": tau})), json!({"ok": true}));
    assert!(s.run("calc.setSource", &json!({"id": tau, "source": "σ := 1"})).is_err());
    // A new region's id is no region's id now.
    let next = calc(&mut s, "z := 1");
    assert!(next > tau, "{next} {tau}");
}

#[test]
fn a_tracked_deletion_is_no_region() {
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert!(s.run("calc.setSource", &json!({"id": p, "source": "P := 1 lbf"})).is_err(), "P is deleted");
}

#[test]
fn wrong_typed_optional_parameters_are_refused() {
    let mut s = session();
    sample(&mut s);
    assert!(s.run("insert.calculation", &json!({"source": "q := 1", "display": "yes"})).is_err());
    assert!(s.run("insert.calcValue", &json!({"name": "P", "unit": 5})).is_err());
    assert!(s.run("insert.calcValue", &json!({"name": "P", "precision": "2"})).is_err());
    assert!(s.run("calc.checkName", &json!({"name": "P", "id": "x"})).is_err());
}

#[test]
fn only_a_wccalc_field_counts() {
    let mut s = session();
    let field = |instr: &str| InlineObject::Field {
        instr: instr.into(),
        result: String::new(),
        locked: true,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let at = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    s.doc.insert_object(&at, field("WCCALCX y"), &Default::default()).unwrap();
    assert!(!super::has_calc(&s.doc));
    let at = s.doc.end_of(wordcraft_doc::StoryRef::Body);
    s.doc.insert_object(&at, field(" wccalc y"), &Default::default()).unwrap();
    assert!(super::has_calc(&s.doc));
}

// ---------------------------------------------------------------------------------------------
// Opening and saving
// ---------------------------------------------------------------------------------------------

/// A session on `name` reopened from what `s` saves under that name.
fn reopen(s: &Session, name: &str) -> Session {
    let bytes = crate::io::save_bytes(name, &s.doc).unwrap();
    Session::new(crate::io::open_bytes(name, &bytes).unwrap())
}

#[test]
fn docx_save_and_open_keeps_calculations_live() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "psi"}));
    let before = s.doc.clone();
    let back = reopen(&s, "report.docx");
    assert_eq!(s.doc, before, "saving leaves the live document alone");
    assert_eq!(regions(&back).len(), 3);
    for (r, linear) in regions(&back) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    assert_eq!(region(&back, p).0.source, "P := 1500 lbf");
    assert_eq!(region(&back, sigma).1, region(&s, sigma).1);
    assert_eq!(field_results(&back), field_results(&s));
    // Flat XML too.
    assert_eq!(regions(&reopen(&s, "report.xml")).len(), 3);
}

#[test]
fn opening_recalculates_stale_results() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "insert.calculation", json!({"source": "Q := 2 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    // Native JSON keeps the stale state; opening makes the results current.
    let back = reopen(&s, "report.json");
    for (r, linear) in regions(&back) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    // And through .docx, which shows the stale `≈` in Word.
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let xml = {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
        xml
    };
    assert!(xml.contains('≈'), "Word sees what WordCraft showed");
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    assert!(regions(&back).iter().all(|(r, _)| r.state == CalcState::Fresh));
}

#[test]
fn opening_a_plain_document_changes_nothing() {
    let mut s = session();
    run(&mut s, "text.insert", json!({"text": "Plain text"}));
    run(&mut s, "insert.equation", json!({"linear": "x^2", "edit": false}));
    for name in ["plain.docx", "plain.json"] {
        let bytes = crate::io::save_bytes(name, &s.doc).unwrap();
        let mut read = if name.ends_with("docx") { wordcraft_docx::read(&bytes).unwrap() } else { serde_json::from_slice(&bytes).unwrap() };
        read.ensure_nonempty();
        let opened = crate::io::open_bytes(name, &bytes).unwrap();
        assert!(opened.same_content(&read), "{name}");
    }
    let mut doc = s.doc.clone();
    super::after_open(&mut doc);
    assert_eq!(doc, s.doc);
}

#[test]
fn a_panicking_open_pass_leaves_nothing_looking_current() {
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    let bytes = crate::io::save_bytes("report.json", &s.doc).unwrap();
    INJECT_PANIC.with(|p| p.set(true));
    let opened = crate::io::open_bytes("report.json", &bytes);
    INJECT_PANIC.with(|p| p.set(false));
    let back = Session::new(opened.unwrap());
    assert_eq!(region(&back, sigma).0.state, CalcState::Stale);
    assert!(super::take_open_notice().is_some_and(|n| n.contains("out of date")));

    // Through file.open, from a .docx: the status line says what happened.
    let docx = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut t = session();
    INJECT_PANIC.with(|p| p.set(true));
    let opened = t.run("file.open", &json!({"path": "report.docx", "data": crate::cmd::insert::base64_encode(&docx)}));
    INJECT_PANIC.with(|p| p.set(false));
    opened.unwrap();
    assert!(t.status.contains("opened as plain equations"), "{}", t.status);
    assert!(regions(&t).is_empty());
}

#[test]
fn a_tracked_deleted_calculation_survives_docx_and_comes_back_on_reject() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Error("P isn't defined".into()));
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    // The control is outside the deletion, which marks the equation inside it.
    let sdt = &xml[xml.find("<w:sdt>").unwrap()..xml.find("</w:sdt>").unwrap()];
    assert!(sdt.contains("wordcraft-calc:1") && sdt.contains("<w:del ") && sdt.contains("P≔1500"), "{sdt}");
    let mut back = reopen(&s, "report.docx");
    assert_eq!(region(&back, p).0.source, "P := 1500 lbf", "the deleted calculation keeps its source");
    assert!(matches!(region(&back, sigma).0.state, CalcState::Error(_)), "still deleted, so still undefined");
    run(&mut back, "review.rejectAll", json!({}));
    assert_eq!(region(&back, p).0.state, CalcState::Fresh);
    let (sig, linear) = region(&back, sigma);
    assert_eq!(sig.state, CalcState::Fresh, "{linear}");
    assert_eq!(
        linear,
        region(
            &{
                let mut fresh = session();
                sample(&mut fresh);
                fresh
            },
            sigma
        )
        .1
    );
}

/// `bytes` (a `.docx`) with `word/document.xml` rewritten by `f`: an edit made in Word.
fn edit_in_word(bytes: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut part = z.by_index(i).unwrap();
        let name = part.name().to_string();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            data = f(String::from_utf8(data).unwrap()).into_bytes();
        }
        w.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn a_calculation_edited_in_word_stops_its_dependents_and_says_so() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "insert.calcValue", json!({"name": "σ"}));
    assert_eq!(field_results(&s), ["600 psi"]);
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let edited = edit_in_word(&bytes, |xml| {
        assert!(xml.contains(">P≔1500<"), "the saved equation shows P≔1500");
        xml.replace(">P≔1500<", ">P≔2000<")
    });
    let mut back = Session::new(Document::new());
    run(&mut back, "file.open", json!({"path": "report.docx", "data": crate::cmd::insert::base64_encode(&edited)}));
    // P keeps Word's equation, marked; nothing shows a number from either version of P.
    let (r, linear) = region(&back, p);
    assert_eq!(r.state, CalcState::EditedOutside);
    assert!(linear.contains("2000") && linear.ends_with(" ⚠"), "{linear}");
    let (r, linear) = region(&back, sigma);
    assert_eq!(r.state, CalcState::Error("depends on a calculation edited in Word — resolve it".into()));
    assert!(!linear.contains("psi") && !linear.contains("600") && !linear.contains("800"), "{linear}");
    assert_eq!(field_results(&back), ["?"]);
    assert!(back.status.contains("1 calculation was edited in Word"), "{}", back.status);
    // Resolving it from the calculation brings everything back.
    run(&mut back, "calc.resolveEdited", json!({"id": p, "keep": "source"}));
    assert_eq!(region(&back, sigma).0.state, CalcState::Fresh);
    assert_eq!(field_results(&back), ["600 psi"]);
}

#[test]
fn keeping_words_equation_drops_the_mark() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    edit_outside(&mut s, p, "P≔2000 \"lbf\"");
    run(&mut s, "calc.recalculate", json!({}));
    assert!(region(&s, p).1.ends_with(" ⚠"));
    run(&mut s, "calc.resolveEdited", json!({"id": p, "keep": "word"}));
    let plain: Vec<String> = objects(&s.doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: None, math, .. } => {
                assert!(!wordcraft_calc::render::has_edited_mark(&math));
                Some(linear)
            }
            _ => None,
        })
        .collect();
    assert_eq!(plain, ["P≔2000 \"lbf\""]);
    assert_eq!(region(&s, sigma).0.state, CalcState::Error("P isn't defined".into()));
}

/// Every equation and field in the stories other than the body.
fn objects_outside_body(doc: &Document) -> Vec<InlineObject> {
    let mut out = Vec::new();
    for k in doc.parts.keys() {
        let story = wordcraft_doc::StoryRef::Part(*k);
        for path in doc.para_paths(story) {
            if let Some(p) = doc.para(story, &path) {
                out.extend(p.objects.iter().cloned());
            }
        }
    }
    out
}

/// Copies the one object of body paragraph `block` (a calculation or a quoted value).
fn copy_object(s: &mut Session, block: usize) {
    run(s, "select.range", json!({"anchor": {"block": block, "off": 0}, "focus": {"block": block, "off": 3}}));
    run(s, "edit.copy", json!({}));
}

/// Nothing outside the body shows a number: no calculation, and every quoted value `?`.
fn assert_nothing_live_outside_body(s: &Session, what: &str) {
    let outside = objects_outside_body(&s.doc);
    let mut seen = 0;
    for o in &outside {
        match o {
            InlineObject::Equation { calc, linear, .. } => {
                assert!(calc.is_none(), "{what}: a calculation outside the body: {linear}");
                if linear.contains("not calculated here") {
                    assert!(!linear.contains("psi") && !linear.contains('='), "{what}: {linear}");
                    seen += 1;
                }
            }
            InlineObject::Field { result, instr, .. } if instr.starts_with("WCCALC") => {
                assert_eq!(result, "?", "{what}");
                seen += 1;
            }
            _ => {}
        }
    }
    assert!(seen >= 2, "{what}: the pasted calculation and value are there: {outside:?}");
}

#[test]
fn calculations_pasted_outside_the_body_never_look_live() {
    for target in ["textBox", "footnote"] {
        let mut s = session();
        let p = calc(&mut s, "P := 1500 lbf");
        let sigma = calc(&mut s, "σ := P/(2 in^2) =");
        run(&mut s, "insert.calcValue", json!({"name": "σ"}));
        assert_eq!(field_results(&s), ["750 psi"]);
        // The σ region, then the quoted value, into a text box or a footnote.
        copy_object(&mut s, 1);
        run(&mut s, "caret.docEnd", json!({}));
        match target {
            "textBox" => {
                run(&mut s, "insert.textBox", json!({}));
            }
            _ => {
                run(&mut s, "references.footnote", json!({}));
            }
        }
        run(&mut s, "edit.paste", json!({}));
        assert!(s.sel.focus.story != wordcraft_doc::StoryRef::Body, "{target}: pasting outside the body");
        let story = s.sel.focus.story;
        // Back to the body to copy the quoted value, then into the same story again.
        let field_block = s.doc.body.len() - 1;
        let at_story = s.sel.focus.clone();
        copy_object(&mut s, field_block);
        run(&mut s, "caret.set", json!({"pos": at_story}));
        assert_eq!(s.sel.focus.story, story);
        run(&mut s, "edit.paste", json!({}));
        assert_nothing_live_outside_body(&s, target);
        // The input changes: the body follows, the copies outside still show no number.
        run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
        assert!(region(&s, sigma).1.ends_with("=1500 \"psi\""), "{}", region(&s, sigma).1);
        assert_eq!(field_results(&s), ["1500 psi"]);
        assert_nothing_live_outside_body(&s, target);
    }
}

#[test]
fn calculations_outside_the_body_are_frozen_on_open() {
    let mut s = session();
    sample(&mut s);
    run(&mut s, "insert.textBox", json!({}));
    let part = match s.sel.focus.story {
        wordcraft_doc::StoryRef::Part(k) => k,
        other => panic!("not in a text box: {other:?}"),
    };
    // A hostile native file: a live-looking calculation and quoted value in a text box.
    let path = s.doc.para_paths(wordcraft_doc::StoryRef::Part(part))[0].clone();
    let para = s.doc.para_mut(wordcraft_doc::StoryRef::Part(part), &path).unwrap();
    let eq = InlineObject::Equation {
        linear: "y≔2=2".into(),
        display: false,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion {
            result_format: None,
            worksheet_order: None,
            id: 99,
            source: "y := 2 =".into(),
            unit: None,
            precision: None,
            numbered: false,
            state: CalcState::Fresh,
        })),
    };
    let field =
        InlineObject::Field { instr: "WCCALC σ".into(), result: "600 psi".into(), locked: true, dirty: false, code: false, form: String::new() };
    para.insert_object(0, eq, &Default::default()).unwrap();
    para.insert_object(0, field, &Default::default()).unwrap();
    let back = reopen(&s, "report.json");
    assert_nothing_live_outside_body(&back, "opened");
}

#[test]
fn editing_a_pasted_copy_in_manual_mode_leaves_the_original() {
    let mut s = session();
    run(&mut s, "calc.settings", json!({"manual": true}));
    let a = calc(&mut s, "a := 1 =");
    copy_object(&mut s, 0);
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    // Every stored id is unique once the paste is done, in manual mode too.
    let ids: Vec<u64> = regions(&s).into_iter().map(|(r, _)| r.id).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "the copy has its own id");
    assert_eq!(ids[0], a);
    // The interface edits the copy by the id stored on it.
    run(&mut s, "calc.setSource", json!({"id": ids[1], "source": "b := 2 ="}));
    let sources: Vec<String> = regions(&s).into_iter().map(|(r, _)| r.source).collect();
    assert_eq!(sources, ["a := 1 =", "b := 2 ="]);
}

#[test]
fn typing_text_in_manual_mode_leaves_results_current() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "text.insert", json!({"text": "The stress follows."}));
    run(&mut s, "text.newParagraph", json!({}));
    assert!(regions(&s).iter().all(|(r, _)| r.state == CalcState::Fresh), "typing text changes no input");
    // Editing a source still marks what follows from it out of date.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
}

#[test]
fn a_quoted_value_unit_is_checked() {
    let mut s = session();
    sample(&mut s);
    calc(&mut s, "T_1 := 70 °F");
    calc(&mut s, "T_2 := 100 °F");
    calc(&mut s, "dT := T_2 - T_1");
    for (name, unit, why) in [
        ("σ", "furlongs", "furlongs"),
        ("σ", "in", "σ can't be shown in in"),
        ("dT", "°C", "°C shows an absolute temperature; use Δ°C for a difference"),
    ] {
        let e = s.run("insert.calcValue", &json!({"name": name, "unit": unit})).unwrap_err();
        assert!(e.to_string().contains(why), "{name} in {unit}: {e}");
    }
    assert!(field_results(&s).is_empty(), "nothing inserted");
    run(&mut s, "insert.calcValue", json!({"name": "σ", "unit": "ksi"}));
    run(&mut s, "insert.calcValue", json!({"name": "T_1", "unit": "°C"}));
    run(&mut s, "insert.calcValue", json!({"name": "dT", "unit": "Δ°C"}));
    assert_eq!(field_results(&s), ["0.6 ksi", "21.111 °C", "16.667 Δ°C"]);
}

/// The caret just after the region `id`'s equation.
fn caret_after(s: &mut Session, id: u64) {
    for path in s.doc.para_paths(wordcraft_doc::StoryRef::Body) {
        let para = s.doc.para(wordcraft_doc::StoryRef::Body, &path).unwrap();
        if let Some((off, _)) = para.objects_with_offsets().find(|(_, o)| matches!(o, InlineObject::Equation { calc: Some(c), .. } if c.id == id)) {
            s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off: off + wordcraft_doc::para::OBJ.len_utf8() });
            return;
        }
    }
    panic!("no region {id}");
}

/// Whether the region `id`'s equation is numbered the way a plain one is: one row ending in `#`.
fn numbered_eq(doc: &Document, id: u64) -> bool {
    let mut found = None;
    for o in objects(doc) {
        if let InlineObject::Equation { math, display, calc: Some(c), .. } = o
            && c.id == id
        {
            found = Some(
                display
                    && matches!(math.nodes.as_slice(), [wordcraft_doc::math::MNode::EqArr { rows }] if rows.len() == 1 && wordcraft_doc::math::has_number_mark(&rows[0])),
            );
        }
    }
    found.unwrap_or_else(|| panic!("no region {id}"))
}

#[test]
fn a_calculation_can_be_numbered() {
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({}));
    assert!(region(&s, sigma).0.numbered);
    assert!(numbered_eq(&s.doc, sigma), "drawn with an automatic number");
    let (_, linear) = region(&s, sigma);
    assert!(linear.ends_with("=600 \"psi\"#"), "{linear}");
    // The number stays through a recalculation, and in manual mode through going stale.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert!(numbered_eq(&s.doc, sigma));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\"#"), "{}", region(&s, sigma).1);
    run(&mut s, "calc.settings", json!({"manual": true}));
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 1500 lbf"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Stale);
    assert!(numbered_eq(&s.doc, sigma));
    assert!(region(&s, sigma).1.contains("≈1200"), "the stale result is kept: {}", region(&s, sigma).1);
    run(&mut s, "calc.recalculate", json!({}));
    // Through .docx: Word gets a numbered equation, and it comes back live and numbered.
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let mut xml = String::new();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains("<m:eqArr>") && xml.contains("#"), "Word sees a numbered equation");
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    let (r, linear) = region(&back, sigma);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(r.numbered && numbered_eq(&back.doc, sigma), "{linear}");
    assert_eq!(linear, region(&s, sigma).1);
    assert!(regions(&back).iter().all(|(r, _)| r.state == CalcState::Fresh));
    // And the number can be taken away again.
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({"value": false}));
    assert!(!region(&s, sigma).0.numbered && !numbered_eq(&s.doc, sigma));
    assert!(!region(&s, sigma).1.ends_with('#'));
}

/// A pasted copy of P's definition at the end of the body.
fn paste_copy_of_p(s: &mut Session) -> CalcRegion {
    copy_object(s, 0);
    run(s, "caret.docEnd", json!({}));
    run(s, "edit.paste", json!({}));
    regions(s).pop().unwrap().0
}

#[test]
fn duplicate_pages_are_looked_up_only_in_print_layout_and_once_per_revision() {
    let mut s = session();
    sample(&mut s);
    s.view.mode = wordcraft_layout::ViewMode::Draft;
    let copy = paste_copy_of_p(&mut s);
    // Draft view has no pages: no layout, and the message says so plainly.
    assert_eq!(copy.state, CalcState::Error("P is already defined elsewhere in the document — choose a unique name".into()));
    assert!(s.calc_pages.is_none(), "no page lookup outside Print Layout");
    s.view.mode = wordcraft_layout::ViewMode::Print;
    run(&mut s, "calc.recalculate", json!({}));
    assert!(matches!(regions(&s).pop().unwrap().0.state, CalcState::Error(m) if m.contains("on page 1")));
    // The same document at the same revision reuses the pages found.
    let first = super::region_pages(&mut s);
    let again = super::region_pages(&mut s);
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    // Any edit looks again.
    run(&mut s, "text.insert", json!({"text": "x"}));
    let after = super::region_pages(&mut s);
    assert!(!std::sync::Arc::ptr_eq(&first, &after));
}

#[test]
fn calculation_commands_have_keys() {
    let reg = crate::cmd::registry();
    assert_eq!(reg.by_shortcut("Mod+Alt+=").map(|c| c.id), Some("insert.calculation"));
    assert_eq!(reg.by_shortcut("F2").map(|c| c.id), Some("calc.edit"));
    // calc.edit names the calculation at the caret (on it, just after it, or selected) and is
    // greyed out anywhere else.
    let mut s = session();
    let (p, _, _) = sample(&mut s);
    run(&mut s, "caret.docEnd", json!({}));
    assert!(s.why_disabled("calc.edit").is_some());
    caret_after(&mut s, p);
    assert_eq!(run(&mut s, "calc.edit", json!({})), json!({"id": p, "source": "P := 1500 lbf"}));
    copy_object(&mut s, 0);
    assert_eq!(run(&mut s, "calc.edit", json!({}))["id"], json!(p));
}

#[test]
fn a_numbered_calculation_is_cross_referenced_like_a_plain_equation() {
    // Equation cross-references go to Equation captions, for a plain equation and a calculation
    // alike; the `#` number marks neither as a target on its own.
    let targets = |s: &mut Session| crate::cmd::citations::ref_targets(s, "equation").into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    let mut plain = session();
    run(&mut plain, "insert.equation", json!({"linear": "x=1", "edit": false}));
    run(&mut plain, "equation.number", json!({}));
    let mut s = session();
    let (_, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.number", json!({}));
    assert_eq!(targets(&mut s), targets(&mut plain));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "references.caption", json!({"label": "Equation", "text": "Bolt stress"}));
    let listed = targets(&mut s);
    assert!(listed.iter().any(|t| t.contains("Bolt stress")), "{listed:?}");
    assert!(region(&s, sigma).0.numbered, "the caption leaves the calculation as it was");
}

/// The display flag and justification of the region `id`'s equation.
fn layout_of(doc: &Document, id: u64) -> (bool, wordcraft_doc::math::MathJc) {
    let mut found = None;
    for o in objects(doc) {
        if let InlineObject::Equation { display, math, calc: Some(c), .. } = o
            && c.id == id
        {
            found = Some((display, math.jc));
        }
    }
    found.unwrap_or_else(|| panic!("no region {id}"))
}

#[test]
fn a_calculation_can_be_aligned_and_made_inline() {
    use wordcraft_doc::math::MathJc;
    let mut s = session();
    let (p, _, sigma) = sample(&mut s);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.justify", json!({"jc": "left"}));
    assert_eq!(layout_of(&s.doc, sigma), (true, MathJc::Left));
    // Alignment survives a recalculation and the .docx round trip.
    run(&mut s, "calc.setSource", json!({"id": p, "source": "P := 3000 lbf"}));
    assert_eq!(layout_of(&s.doc, sigma), (true, MathJc::Left));
    assert!(region(&s, sigma).1.ends_with("=1200 \"psi\""), "{}", region(&s, sigma).1);
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    assert_eq!(layout_of(&back.doc, sigma), (true, MathJc::Left));
    assert_eq!(region(&back, sigma).0.state, CalcState::Fresh);
    caret_after(&mut s, sigma);
    run(&mut s, "equation.justify", json!({"jc": "right"}));
    assert_eq!(layout_of(&s.doc, sigma).1, MathJc::Right);
    // Inline, so text can follow it on the same line; a number needs display, so it goes.
    run(&mut s, "equation.number", json!({"value": true}));
    caret_after(&mut s, sigma);
    run(&mut s, "equation.display", json!({"value": false}));
    assert!(!layout_of(&s.doc, sigma).0, "inline now");
    assert!(!region(&s, sigma).0.numbered, "an inline calculation isn't numbered");
    caret_after(&mut s, sigma);
    run(&mut s, "text.insert", json!({"text": "\tbearing stress, see ref. 3"}));
    assert_eq!(region(&s, sigma).0.state, CalcState::Fresh, "text after it leaves it live");
    caret_after(&mut s, sigma);
    run(&mut s, "equation.display", json!({"value": true}));
    assert!(layout_of(&s.doc, sigma).0, "display again");
    // Undo restores the inline state.
    run(&mut s, "edit.undo", json!({}));
    assert!(!layout_of(&s.doc, sigma).0);
}

#[test]
fn g_setting_switches_between_gravity_and_gram() {
    let mut s = session();
    let w = calc(&mut s, "F_w := 5 kg·g = ? N");
    assert!(region(&s, w).1.ends_with("=49.033 \"N\""), "Mathcad's g by default: {}", region(&s, w).1);
    let r = run(&mut s, "calc.settings", json!({"g": "gram"}));
    assert_eq!(r["g"], "gram");
    assert!(matches!(region(&s, w).0.state, CalcState::Error(_)), "g is the gram now: kg·g isn't a force");
    // It's saved with the document, .docx included.
    let bytes = crate::io::save_bytes("report.docx", &s.doc).unwrap();
    let back = Session::new(crate::io::open_bytes("report.docx", &bytes).unwrap());
    assert_eq!(back.doc.calc_settings.clone().unwrap_or_default().g, wordcraft_doc::calc::GMeans::Gram);
    run(&mut s, "calc.settings", json!({"g": "gravity"}));
    assert!(region(&s, w).1.ends_with("=49.033 \"N\""));
    let bad = s.run("calc.settings", &json!({"g": "grams"}));
    assert!(bad.is_err());
}

// ---------------------------------------------------------------------------------------------
// Data tables, the origin setting and quoted elements
// ---------------------------------------------------------------------------------------------

/// Inserts a table holding `rows` (one text per cell, typed with Tab between cells) and leaves
/// the caret at the end of the document, after it; returns the table's body block index.
fn typed_table(s: &mut Session, rows: &[&[&str]]) -> usize {
    let r = run(s, "insert.table", json!({"rows": rows.len(), "cols": rows[0].len()}));
    let block = r["table"][0].as_u64().unwrap_or_else(|| panic!("no table in {r}")) as usize;
    for (k, text) in rows.iter().flat_map(|r| r.iter()).enumerate() {
        if k > 0 {
            run(s, "text.tab", json!({}));
        }
        run(s, "text.insert", json!({"text": text}));
    }
    run(s, "caret.docEnd", json!({}));
    block
}

/// Puts the caret in cell (`row`, `col`) of the table at body block `block`, `off` bytes in.
fn caret_in_cell(s: &mut Session, block: usize, row: u32, col: u32, off: usize) {
    let path = wordcraft_doc::Path(vec![block as u32, row, col, 0]);
    s.sel = crate::Selection::caret(Pos { story: wordcraft_doc::StoryRef::Body, path, off });
}

/// The calculation data mark of the table at body block `block`.
fn table_mark(s: &Session, block: usize) -> Option<wordcraft_doc::calc::CalcTable> {
    match &*s.doc.body[block] {
        Block::Table(t) => t.calc.as_deref().cloned(),
        Block::Para(_) => panic!("block {block} isn't a table"),
    }
}

fn shows(s: &Session, id: u64, tail: &str) {
    let (r, linear) = region(s, id);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(linear.ends_with(tail), "{linear} should end with {tail}");
}

#[test]
fn mark_table_defines_columns() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["D (in)", "qty"], &["1/2", "3"]]);
    caret_in_cell(&mut s, t, 0, 0, 0);
    let r = run(&mut s, "calc.dataTable", json!({"on": true}));
    let id = r["id"].as_u64().unwrap_or_else(|| panic!("no id in {r}"));
    assert_eq!(table_mark(&s, t).map(|c| (c.id, c.errors)), Some((id, Vec::new())));
    // Marking a marked table changes nothing and answers its id.
    let undo = s.undo_depth();
    assert_eq!(run(&mut s, "calc.dataTable", json!({"on": true})), json!({"id": id}));
    assert_eq!(s.undo_depth(), undo, "no undo step");
    run(&mut s, "caret.docEnd", json!({}));
    let d = calc(&mut s, "D =");
    let (r, linear) = region(&s, d);
    assert_eq!(r.state, CalcState::Fresh, "{linear}");
    assert!(linear.contains("0.5") && linear.ends_with("\"in\""), "{linear}");
    // Another table gets an id of its own.
    let u = typed_table(&mut s, &[&["E (in)"], &["1"]]);
    caret_in_cell(&mut s, u, 1, 0, 0);
    let other = run(&mut s, "calc.dataTable", json!({"on": true}))["id"].as_u64().unwrap();
    assert_ne!(other, id);
    // Unmarking: the column is gone.
    caret_in_cell(&mut s, t, 1, 1, 0);
    assert_eq!(run(&mut s, "calc.dataTable", json!({"on": false})), json!({"on": false}));
    assert_eq!(table_mark(&s, t), None);
    assert_eq!(region(&s, d).0.state, CalcState::Error("D isn't defined".into()));
    // Outside a table, or with parameters that aren't a bool: refused, nothing changes.
    run(&mut s, "caret.docEnd", json!({}));
    let before = s.doc.clone();
    let e = s.run("calc.dataTable", &json!({"on": true})).unwrap_err();
    assert_eq!(e.to_string(), "Put the caret in a table in the main text");
    caret_in_cell(&mut s, t, 0, 0, 0);
    for bad in [json!({}), json!({"on": "yes"}), json!({"on": 1})] {
        assert!(matches!(s.run("calc.dataTable", &bad), Err(crate::CmdError::Params(_))), "{bad}");
    }
    assert!(s.doc.same_content(&before));
}

#[test]
fn data_table_in_a_header_is_refused() {
    let mut s = session();
    run(&mut s, "insert.editHeader", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 1}));
    let e = s.run("calc.dataTable", &json!({"on": true})).unwrap_err();
    assert_eq!(e.to_string(), "Put the caret in a table in the main text");
}

#[test]
fn undo_unmark_restores() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["D (in)"], &["1/2"]]);
    let d = calc(&mut s, "D =");
    assert_eq!(region(&s, d).0.state, CalcState::Error("D isn't defined".into()));
    caret_in_cell(&mut s, t, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    shows(&s, d, "\"in\"");
    let marked = region(&s, d);
    run(&mut s, "calc.dataTable", json!({"on": false}));
    assert_eq!(region(&s, d).0.state, CalcState::Error("D isn't defined".into()));
    // Undoing the unmark brings the results back; undoing the mark leaves D undefined.
    run(&mut s, "edit.undo", json!({}));
    assert!(table_mark(&s, t).is_some());
    assert_eq!(region(&s, d), marked);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(table_mark(&s, t), None);
    assert_eq!(region(&s, d).0.state, CalcState::Error("D isn't defined".into()));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(region(&s, d), marked);
}

#[test]
fn origin_setting_reindexes() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["D (in)"], &["1/2"], &["3/4"]]);
    caret_in_cell(&mut s, t, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    run(&mut s, "caret.docEnd", json!({}));
    let x = calc(&mut s, "x := D[1] =");
    shows(&s, x, "=0.75 \"in\"");
    let r = run(&mut s, "calc.settings", json!({"origin": 1}));
    assert_eq!(r["origin"], json!(1), "{r}");
    shows(&s, x, "=0.5 \"in\"");
    for bad in [json!(2), json!(-1), json!("1"), json!(true), json!(0.5)] {
        let e = s.run("calc.settings", &json!({"origin": bad})).unwrap_err();
        assert_eq!(e, crate::CmdError::Params("`origin` is 0 or 1".into()), "{bad}");
    }
    assert_eq!(run(&mut s, "calc.settings", json!({}))["origin"], json!(1));
    // In manual mode the switch marks results stale; recalculating makes them current.
    run(&mut s, "calc.settings", json!({"manual": true}));
    assert_eq!(region(&s, x).0.state, CalcState::Fresh);
    run(&mut s, "calc.settings", json!({"origin": 0}));
    assert_eq!(region(&s, x).0.state, CalcState::Stale);
    run(&mut s, "calc.recalculate", json!({}));
    shows(&s, x, "=0.75 \"in\"");
}

#[test]
fn quoting_an_element() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["D (in)"], &["1/2"], &["3/4"]]);
    caret_in_cell(&mut s, t, 0, 0, 0);
    let id = run(&mut s, "calc.dataTable", json!({"on": true}))["id"].clone();
    run(&mut s, "caret.docEnd", json!({}));
    calc(&mut s, "x := 2");
    // Table columns are names a quoted value can show, known by their table's id.
    let names = run(&mut s, "calc.names", json!({}));
    let list = names["names"].as_array().unwrap();
    let col = list.iter().find(|n| n["name"] == "D").unwrap_or_else(|| panic!("D in {names}"));
    assert_eq!(col["table"], json!(true), "{col}");
    assert_eq!(col["tableId"], id, "{col}");
    assert!(col.get("id").is_none(), "a table id is no region id: {col}");
    assert!(list.iter().any(|n| n["name"] == "x" && n.get("id").is_some()), "{names}");
    let check = run(&mut s, "calc.checkName", json!({"name": "D"}));
    assert_eq!(check["reason"], json!("duplicate"), "{check}");
    assert_eq!(check["existingTable"], id, "{check}");
    assert!(check.get("existing").is_none(), "a table id is no region id: {check}");
    run(&mut s, "insert.calcValue", json!({"name": "D", "index": [0]}));
    assert_eq!(field_results(&s), ["0.5 in"]);
    let field = objects(&s.doc).into_iter().find(|o| matches!(o, InlineObject::Field { .. })).unwrap();
    assert!(matches!(field, InlineObject::Field { ref instr, .. } if instr.contains("\\i 0")), "{field:?}");
    run(&mut s, "insert.calcValue", json!({"name": "D"}));
    assert_eq!(field_results(&s), ["0.5 in", "0.5, 0.75 in"]);
    // An index on a single value, outside the vector, or not one or two whole numbers: refused,
    // nothing inserted.
    let before = s.doc.clone();
    let e = s.run("insert.calcValue", &json!({"name": "x", "index": [0]})).unwrap_err();
    assert_eq!(e.to_string(), "x isn't a vector");
    let e = s.run("insert.calcValue", &json!({"name": "D", "index": [5]})).unwrap_err();
    assert!(e.to_string().contains("outside D"), "{e}");
    for bad in [json!([]), json!([1, 2, 3]), json!([-1]), json!([4_294_967_296u64]), json!("0"), json!([0.5]), json!(0)] {
        let e = s.run("insert.calcValue", &json!({"name": "D", "index": bad})).unwrap_err();
        assert!(matches!(e, crate::CmdError::Params(_)), "{bad}: {e}");
    }
    assert!(s.doc.same_content(&before));
}

#[test]
fn has_calc_counts_data_tables() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["D (in)"], &["1/2"]]);
    assert!(!super::has_calc(&s.doc));
    caret_in_cell(&mut s, t, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    assert!(super::has_calc(&s.doc));
    assert_eq!(table_mark(&s, t).map(|c| c.errors), Some(Vec::new()));
    // A cell edit recalculates: the table's errors are written.
    caret_in_cell(&mut s, t, 1, 0, 0);
    run(&mut s, "text.insert", json!({"text": "abc "}));
    let errors = table_mark(&s, t).map(|c| c.errors).unwrap_or_default();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message, "D row 1 isn't a number", "{errors:?}");
    // Saving takes the calculations' path to Word now; it still saves.
    assert!(crate::io::save_bytes("report.docx", &s.doc).is_ok());
}

#[test]
fn edited_table_notice() {
    let mut s = session();
    let pipes: &[&[&str]] =
        &[&["NPS (in)", "D_o (in)", "t_nom (in)"], &["1/2", "0.840", "0.109"], &["3/4", "1.050", "0.113"], &["1", "1.315", "0.133"]];
    let t = typed_table(&mut s, pipes);
    caret_in_cell(&mut s, t, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    run(&mut s, "caret.docEnd", json!({}));
    let p = calc(&mut s, "P := 1000 psi");
    for source in ["S := 20000 psi", "E := 1", "Y := 0.4", "t_min := P·D_o/(2·(S·E + P·Y))", "margin := t_nom - t_min"] {
        calc(&mut s, source);
    }
    let min = calc(&mut s, "min(margin) =");
    shows(&s, min, "=0.087 \"in\"");
    let other = typed_table(&mut s, &[&["L (ft)"], &["12"]]);
    caret_in_cell(&mut s, other, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    let bytes = crate::io::save_bytes("pipes.docx", &s.doc).unwrap();
    let open = |edited: &[u8]| {
        let mut back = Session::new(Document::new());
        run(&mut back, "file.open", json!({"path": "pipes.docx", "data": crate::cmd::insert::base64_encode(edited)}));
        back
    };
    // Untouched: nothing to say.
    let back = open(&bytes);
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    // One table edited: its new value is used, and the status line says so.
    let back = open(&edit_in_word(&bytes, |xml| xml.replace(">0.113<", ">0.120<")));
    assert_eq!(back.status, "Calculation data was edited in Word in 1 table; results were recalculated");
    shows(&back, min, "=0.088 \"in\"");
    // Two tables.
    let back = open(&edit_in_word(&bytes, |xml| xml.replace(">0.113<", ">0.120<").replace(">12<", ">14<")));
    assert_eq!(back.status, "Calculation data was edited in Word in 2 tables; results were recalculated");
    // An equation too: the table notice follows the equations one.
    let back = open(&edit_in_word(&bytes, |xml| {
        assert!(xml.contains(">P≔1000<"));
        xml.replace(">0.113<", ">0.120<").replace(">P≔1000<", ">P≔1100<")
    }));
    assert_eq!(region(&back, p).0.state, CalcState::EditedOutside);
    assert_eq!(
        back.status,
        "1 calculation was edited in Word: it is marked ⚠, and calculations that use it show an error until you resolve it. \
         Calculation data was edited in Word in 1 table; results were recalculated"
    );
}

/// A text cell changed in Word: the table's fingerprint sees it, the status line says so, and a
/// lookup of the old text fails while one of an untouched row still works.
#[test]
fn edited_text_column_notice() {
    let mut s = session();
    let t = typed_table(&mut s, &[&["Material (text)", "S (ksi)"], &["A106 B", "20"], &["A312 TP316", "17"]]);
    caret_in_cell(&mut s, t, 0, 0, 0);
    run(&mut s, "calc.dataTable", json!({"on": true}));
    run(&mut s, "caret.docEnd", json!({}));
    let old = calc(&mut s, "lookup(Material, S, \"A106 B\") = ? ksi");
    let kept = calc(&mut s, "lookup(Material, S, \"A312 TP316\") = ? ksi");
    shows(&s, old, "=20 \"ksi\"");
    shows(&s, kept, "=17 \"ksi\"");
    let bytes = crate::io::save_bytes("materials.docx", &s.doc).unwrap();
    let open = |edited: &[u8]| {
        let mut back = Session::new(Document::new());
        run(&mut back, "file.open", json!({"path": "materials.docx", "data": crate::cmd::insert::base64_encode(edited)}));
        back
    };
    let back = open(&bytes);
    assert!(!back.status.contains("edited in Word"), "{}", back.status);
    shows(&back, old, "=20 \"ksi\"");
    let back = open(&edit_in_word(&bytes, |xml| {
        assert!(xml.contains(">A106 B<"));
        xml.replace(">A106 B<", ">A106 C<")
    }));
    assert_eq!(back.status, "Calculation data was edited in Word in 1 table; results were recalculated");
    assert_eq!(table_mark(&back, t).map(|c| c.errors), Some(Vec::new()));
    assert_eq!(region(&back, old).0.state, CalcState::Error("\"A106 B\" isn't in Material".into()));
    shows(&back, kept, "=17 \"ksi\"");
}

/// Calculations nested as deep as the evaluator allows (32 calls of your own functions, each
/// through 60 `abs`) recalculate on a 1 MB stack, the size of the Windows main thread's, however
/// the pass is reached: an edit, Recalculate, quoting a value, changing the options, or opening
/// a document. A stack overflow aborts the app, and no `catch_unwind` can contain it.
#[test]
fn deep_calculations_recalculate_on_a_small_stack() {
    let small = std::thread::Builder::new().stack_size(1 << 20).spawn(|| {
        let mut s = session();
        let nest = |inner: String| (0..60).fold(inner, |e, _| format!("abs({e})"));
        calc(&mut s, &format!("h0(x) := {}", nest("x".to_string())));
        for k in 1..32 {
            calc(&mut s, &format!("h{k}(x) := {}", nest(format!("h{}(x)", k - 1))));
        }
        let y = calc(&mut s, "y := h31(-2) =");
        // Evaluated all the way down, not stopped at a limit.
        assert!(region(&s, y).1.ends_with("=2"), "{}", region(&s, y).1);
        run(&mut s, "calc.recalculate", json!({}));
        // Quoting an element in a unit checks both against the evaluated document.
        calc(&mut s, "len := [h31(-2), 3] m");
        run(&mut s, "insert.calcValue", json!({"name": "len", "index": [0], "unit": "mm"}));
        assert!(field_results(&s).iter().any(|r| r == "2000 mm"), "{:?}", field_results(&s));
        run(&mut s, "calc.settings", json!({"precision": 4}));
        assert!(region(&s, y).1.ends_with("=2"), "{}", region(&s, y).1);
        // Opening: the pass every opened document gets. (Called on the document itself: neither
        // saved format reads back an equation nested this deep, but a crafted .docx can carry
        // these sources with shallow equations.)
        let mut opened = s.doc.clone();
        wordcraft_calc::mark_stale(&mut opened);
        super::after_open(&mut opened);
        let back = Session::new(opened);
        assert!(region(&back, y).1.ends_with("=2"), "{}", region(&back, y).1);
        assert_eq!(super::take_open_notice(), None);
    });
    small.expect("a thread to run on").join().expect("recalculated without failing");
}
#[test]
fn mcdx_imports_packaged_text_units_and_solve_blocks_as_editable_document() {
    use craft_zip::{Compression::Stored, Writer};

    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet50" xmlns:m="http://schemas.mathsoft.com/math50"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:p="clr-namespace:Ptc.Wpf;assembly=Ptc.Core">
        <regions>
            <region top="0" left="0" actualWidth="170"><math><m:define>
                <m:id><x:Span>T<p:Subscript>H2O</p:Subscript></x:Span></m:id>
                <m:eval><m:apply><m:scale/><m:real>40</m:real><m:id>&#176;F</m:id></m:apply>
                <m:unitOverride><m:id>&#176;R</m:id></m:unitOverride></m:eval>
            </m:define></math></region>
            <region top="0" left="300" width="180"><text item-idref="text"/></region>
            <region top="40" left="0"><math><m:define><m:id>guess</m:id><m:real>1</m:real></m:define></math></region>
            <region top="60" left="0"><solveblock><regions>
                <region top="0" solve-block-category="constraint"><math><m:apply><m:equal/>
                    <m:apply><m:nthRoot/><m:placeholder/><m:id>guess</m:id></m:apply><m:real>2</m:real>
                </m:apply></math></region>
                <region top="30" solve-block-category="solver"><math><m:define><m:id>answer</m:id>
                    <m:eval><m:apply><m:id>Find</m:id><m:id>guess</m:id></m:apply></m:eval>
                </m:define></math></region>
            </regions></solveblock></region>
            <region top="140"><math><m:eval><m:apply><m:vectorize/>
                <m:apply><m:nthRoot/><m:placeholder/><m:matrix rows="2" cols="1"><m:real>4</m:real><m:real>9</m:real></m:matrix></m:apply>
            </m:apply></m:eval></math></region>
            <region top="160"><math><m:eval><m:apply><m:mult/><m:real>2</m:real><m:id labels="UNIT">lb</m:id></m:apply>
                <m:unitOverride><m:id>lb</m:id></m:unitOverride></m:eval></math></region>
            <region top="180"><pageBreak/></region>
            <region top="200"><math><m:eval><m:id>answer</m:id><m:result><m:real>4</m:real></m:result></m:eval></math></region>
        </regions>
    </worksheet>"#;
    let mut flow = Writer::new();
    flow.add(
        "Xaml/Document.xaml",
        br#"<Section xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" TextAlignment="Right">
        <Paragraph><Run FontWeight="Bold">Water temperature</Run></Paragraph>
    </Section>"#,
        Stored,
    )
    .unwrap();
    let mut zip = Writer::new();
    zip.add("mathcad/worksheet.xml", worksheet, Stored).unwrap();
    zip.add("mathcad/xaml/Text.XamlPackage", &flow.finish().unwrap(), Stored).unwrap();
    zip.add(
        "mathcad/_rels/worksheet.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
        <Relationship Id="text" Target="/mathcad/xaml/Text.XamlPackage"/>
    </Relationships>"#,
        Stored,
    )
    .unwrap();
    let bytes = zip.finish().unwrap();
    let mut session = Session::new(Document::default());
    let opened = run(&mut session, "file.open", json!({"path":"prime.MCDX", "data":crate::cmd::insert::base64_encode(&bytes)}));
    let review = opened["importReview"].as_array().unwrap();
    assert!(review.iter().any(|entry| entry["kind"] == "savedResult" && entry["saved"] == 4.0 && entry["displayed"] == 4.0));
    assert!(session.path.is_none() && session.dirty);
    let bytes = crate::io::save_bytes("prime.docx", &session.doc).unwrap();
    let reopened = crate::io::open_bytes("prime.docx", &bytes).unwrap();
    let Block::Table(table) = reopened.body[0].as_ref() else { panic!("expected an aligned layout table") };
    let note = table.rows[0].cells.last().unwrap().blocks[0].as_para().unwrap();
    assert_eq!(note.text, "Water temperature");
    assert_eq!(note.props.align, Some(wordcraft_doc::props::Align::Right));
    assert_eq!(note.props_of_char(0).bold, Some(true));
    assert_eq!(reopened.body.last().unwrap().as_para().unwrap().props.page_break_before, Some(true));
    let live: Vec<_> = objects(&reopened)
        .into_iter()
        .filter_map(|object| match object {
            InlineObject::Equation { calc: Some(calc), linear, .. } => Some((calc, linear)),
            _ => None,
        })
        .collect();
    assert!(live.iter().all(|(calc, _)| calc.state == CalcState::Fresh));
    assert!(live[0].0.source.starts_with("T_H2O :=") && live[0].1.contains("499.67"));
    assert!(live.iter().any(|(calc, _)| calc.source == "Given"));
    assert!(live.iter().any(|(calc, linear)| calc.source.starts_with("sqrt(") && linear.ends_with("=[■(2@3)]")));
    assert!(live.iter().any(|(calc, _)| calc.source == "(2 * lbm) = ? lbm"));
    assert!(live.last().unwrap().1.ends_with("=4"));
    assert!(crate::io::open_bytes("broken.mcdx", b"not a ZIP").is_err());
}

#[test]
fn xmcd_opens_as_an_unsaved_document_with_live_calculations() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <settings><calculation><builtInVariables array-origin="1"/></calculation></settings>
        <regions>
            <region top="40"><math><m:eval>
                <m:provenance><m:id>value</m:id></m:provenance><m:result><m:real>999</m:real></m:result>
            </m:eval></math></region>
            <region top="10"><text><p>Test <b>worksheet</b> &amp; notes</p></text></region>
            <region top="20"><math><m:provenance><m:define><m:id>value</m:id>
                <m:apply><m:pow/><m:real>-2</m:real><m:real>2</m:real></m:apply>
            </m:define></m:provenance></math></region>
            <region top="30"><area><region top="1"><math><m:define><m:id>value</m:id>
                <m:apply><m:plus/><m:id>value</m:id><m:real>3</m:real></m:apply>
            </m:define></math></region></area></region>
            <region top="50" region-id="42"><component/></region>
            <region top="60"><math><m:eval><m:id>missing_value</m:id></m:eval></math></region>
            <region top="70"><text><p>[Mathcad region 43: this is user text and was not imported]</p></text></region>
        </regions>
    </worksheet>"#;
    let mut session = Session::new(Document::default());
    let opened = run(&mut session, "file.open", json!({"path":"test.XMCD", "data":crate::cmd::insert::base64_encode(worksheet)}));
    let review = opened["importReview"].as_array().unwrap();
    assert!(review.iter().any(|entry| entry["kind"] == "savedResult" && entry["saved"] == 999.0 && entry["displayed"] == 7.0));
    assert_eq!(review.len(), 3, "worksheet text resembling a diagnostic is ordinary content");
    for (kind, block) in [("unsupported", 4), ("calculationError", 5)] {
        let entry = review.iter().find(|entry| entry["kind"] == kind).unwrap();
        assert_eq!(serde_json::from_value::<wordcraft_doc::Pos>(entry["pos"].clone()).unwrap(), wordcraft_doc::Pos::body(block, 0));
    }
    assert!(session.path.is_none());
    assert!(session.dirty);
    assert_eq!(session.doc.calc_settings.as_ref().unwrap().origin, 1);
    let paragraphs: Vec<_> = session.doc.body.iter().filter_map(|block| block.as_para()).collect();
    assert_eq!(paragraphs[0].text, "Test worksheet & notes");
    assert_eq!(paragraphs[4].text, "[Mathcad region 42: external component was not imported]");
    let InlineObject::Equation { linear, calc: Some(calc), .. } = &paragraphs[3].objects[0] else {
        panic!("expected a live calculation");
    };
    assert_eq!(calc.source, "valueb =");
    assert!(linear.contains('7'), "{linear}");
    assert!(!linear.contains("999"), "{linear}");
    let bytes = crate::io::save_bytes("imported.docx", &session.doc).unwrap();
    let reopened = crate::io::open_bytes("imported.docx", &bytes).unwrap();
    assert_eq!(reopened.calc_settings.as_ref().unwrap().origin, 1);
    let paragraph = reopened.body[3].as_para().unwrap();
    assert!(matches!(&paragraph.objects[0], InlineObject::Equation { calc: Some(calc), .. } if calc.source == "valueb ="));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn xmcd_open_relinks_a_numeric_workbook_beside_the_worksheet() {
    use std::io::{Cursor, Write};

    let mut record = vec![0; 66];
    record[..4].copy_from_slice(&[5, 0, 13, 0]);
    for field in [r"X:\Archive\Data\Samples.xls", "", "", ""] {
        let units: Vec<_> = field.encode_utf16().collect();
        record.extend([0xff, 0xfe, 0xff, units.len() as u8]);
        record.extend(units.into_iter().flat_map(u16::to_le_bytes));
    }
    let mut storage = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    storage.create_stream("/Contents").unwrap().write_all(&record).unwrap();
    storage.flush().unwrap();
    let data = crate::cmd::insert::base64_encode(&storage.into_inner().into_inner());
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region top="10"><component clsid-buddy="01350083-1122-11DB-9380-000D56C6051A" item-idref="1"><outputs><m:id>Samples</m:id></outputs></component></region>
            <region top="20"><math><m:eval><m:apply><m:id>mean</m:id><m:id>Samples</m:id></m:apply></m:eval></math></region>
        </regions><binaryContent><item item-id="1">{data}</item></binaryContent>
    </worksheet>"#
    );
    let dir = std::env::temp_dir().join(format!("wordcraft-xmcd-relink-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Data")).unwrap();
    let source = dir.join("sheet.xmcd");
    std::fs::write(&source, xml.as_bytes()).unwrap();
    let xls = wordcraft_formats::sheet::xls_fixture(&[("Inputs", vec![vec!["2".into()], vec!["4".into()]])]).unwrap();
    std::fs::write(dir.join("Data/Samples.xls"), xls).unwrap();
    let mut session = Session::new(Document::default());
    run(&mut session, "file.open", json!({"path": source}));
    assert!(session.path.is_none());
    assert!(session.doc.body[0].as_para().unwrap().text.contains("Data/Samples.xls"));
    let equations = objects(&session.doc);
    assert!(matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.state == CalcState::Fresh && linear.ends_with("=3")));
    let bare = crate::io::open_bytes("sheet.xmcd", xml.as_bytes()).unwrap();
    assert!(bare.body[0].as_para().unwrap().text.contains("was not imported"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn xmcd_program_finds_a_table_row_and_returns_nested_coefficients() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <settings><calculation><builtInVariables array-origin="1"/></calculation></settings>
      <regions>
        <region top="10"><math><m:define><m:id>Coefficients</m:id><m:matrix rows="3" cols="2">
          <m:str>set</m:str><m:real>0</m:real><m:str>marker</m:str><m:real>100</m:real><m:str>a</m:str><m:real>26.5</m:real>
        </m:matrix></m:define></math></region>
        <region top="20"><math><m:define>
          <m:function><m:id>FindRow</m:id><m:boundVars><m:id>Data</m:id></m:boundVars></m:function>
          <m:program>
            <m:localDefine><m:id>row</m:id><m:real>0</m:real></m:localDefine>
            <m:for><m:id>i</m:id><m:range><m:real>1</m:real><m:apply><m:id>rows</m:id><m:id>Data</m:id></m:apply></m:range>
              <m:ifThen><m:apply><m:equal/><m:apply><m:indexer/><m:id>Data</m:id><m:sequence><m:id>i</m:id><m:real>1</m:real></m:sequence></m:apply><m:str>marker</m:str></m:apply>
                <m:program><m:localDefine><m:id>row</m:id><m:id>i</m:id></m:localDefine><m:break/></m:program>
              </m:ifThen>
            </m:for>
            <m:return><m:matrix rows="2" cols="1"><m:str>found</m:str>
              <m:apply><m:id>submatrix</m:id><m:sequence><m:id>Data</m:id><m:id>row</m:id><m:real>3</m:real><m:real>2</m:real><m:real>2</m:real></m:sequence></m:apply>
            </m:matrix></m:return>
            <m:real>999</m:real>
          </m:program>
        </m:define></math></region>
        <region top="30"><math><m:define><m:matrix rows="2" cols="1"><m:id>Label</m:id><m:id>Values</m:id></m:matrix>
          <m:apply><m:id>FindRow</m:id><m:id>Coefficients</m:id></m:apply>
        </m:define></math></region>
        <region top="40"><math><m:eval><m:apply><m:indexer/><m:id>Values</m:id><m:real>2</m:real></m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("program.xmcd", worksheet).unwrap();
    let reopened = crate::io::open_bytes("program.docx", &crate::io::save_bytes("program.docx", &doc).unwrap()).unwrap();
    for doc in [&doc, &reopened] {
        let equations = objects(doc);
        assert!(
            matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), linear, .. }
            if calc.state == CalcState::Fresh && linear.ends_with("=26.5")),
            "{:?}",
            equations.last()
        );
    }
}

#[test]
fn xmcd_mixed_shape_lookup_uses_the_worksheet_origin_and_survives_docx() {
    let xml = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <settings><calculation><builtInVariables array-origin="1"/></calculation></settings>
      <regions>
        <region top="10"><math><m:define><m:id>ORIGIN</m:id><m:real>0</m:real></m:define></math></region>
        <region top="20"><math><m:define><m:id>Shapes</m:id><m:matrix rows="2" cols="2">
          <m:str>001</m:str><m:real>12.8</m:real><m:str>002</m:str><m:real>9</m:real>
        </m:matrix></m:define></math></region>
        <region top="30"><math><m:define><m:id>Labels</m:id><m:apply><m:matcol/><m:id>Shapes</m:id><m:real>0</m:real></m:apply></m:define></math></region>
        <region top="40"><math><m:define><m:id>Rowids</m:id><m:range><m:real>0</m:real><m:real>1</m:real></m:range></m:define></math></region>
        <region top="50"><math><m:define><m:apply><m:indexer/><m:id>Rows</m:id><m:sequence><m:id>Rowids</m:id><m:real>0</m:real></m:sequence></m:apply><m:id>Rowids</m:id></m:define></math></region>
        <region top="60"><math><m:define><m:id>Keys</m:id><m:apply><m:id>augment</m:id><m:sequence><m:id>Labels</m:id><m:id>Rows</m:id></m:sequence></m:apply></m:define></math></region>
        <region top="70"><math><m:define><m:id>Matchrow</m:id><m:apply><m:id>vlookup</m:id><m:sequence><m:str>001</m:str><m:id>Keys</m:id><m:real>1</m:real></m:sequence></m:apply></m:define></math></region>
        <region top="80"><math><m:define><m:id>Rowindex</m:id><m:apply><m:indexer/><m:id>Matchrow</m:id><m:real>0</m:real></m:apply></m:define></math></region>
        <region top="90"><math><m:eval><m:apply><m:indexer/><m:id>Shapes</m:id><m:sequence><m:id>Rowindex</m:id><m:real>1</m:real></m:sequence></m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("shapes.xmcd", xml).unwrap();
    let reopened = crate::io::open_bytes("shapes.docx", &crate::io::save_bytes("shapes.docx", &doc).unwrap()).unwrap();
    for doc in [&doc, &reopened] {
        let equations = objects(doc);
        assert!(
            matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), linear, .. }
            if calc.state == CalcState::Fresh && linear.ends_with("=12.8")),
            "{:?}",
            equations.last()
        );
    }
}

#[test]
fn xmcd_recovers_a_direct_component_snapshot_and_imports_legacy_conditionals() {
    let xml = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region region-id="1" top="10"><component><outputs><m:id>ExternalTable</m:id></outputs></component></region>
            <region region-id="2" top="20"><math><m:eval><m:id>ExternalTable</m:id><m:result>
                <m:matrix rows="2" cols="1"><m:real>2</m:real><m:real>4</m:real></m:matrix>
            </m:result></m:eval></math></region>
            <region region-id="3" top="30"><math><m:eval><m:program>
                <m:ifThen><m:apply><m:equal/><m:real>1</m:real><m:real>1</m:real></m:apply>
                    <m:localDefine><m:id>choice</m:id><m:apply><m:id>mean</m:id><m:id>ExternalTable</m:id></m:apply></m:localDefine>
                </m:ifThen>
                <m:otherwise><m:localDefine><m:id>choice</m:id><m:id>unavailable</m:id></m:localDefine></m:otherwise>
            </m:program></m:eval></math></region>
            <region region-id="5" top="40"><component><outputs><m:id>MixedTable</m:id></outputs></component></region>
            <region region-id="6" top="50"><math><m:eval><m:id>MixedTable</m:id><m:result>
                <m:matrix rows="2" cols="2"><m:str>First label</m:str><m:real>2</m:real><m:str>Second label</m:str><m:real>4</m:real></m:matrix>
            </m:result></m:eval></math></region>
            <region region-id="4" top="60"><component><outputs><m:id>OtherTable</m:id></outputs></component></region>
            <region region-id="7" top="70"><math><m:eval><m:apply><m:indexer/><m:id>MixedTable</m:id>
                <m:sequence><m:real>1</m:real><m:real>0</m:real></m:sequence>
            </m:apply></m:eval></math></region>
        </regions>
    </worksheet>"#;
    let mut session = Session::new(Document::default());
    let opened = run(&mut session, "file.open", json!({"path":"component.xmcd", "data":crate::cmd::insert::base64_encode(xml)}));
    let review = opened["importReview"].as_array().unwrap();
    assert_eq!(review.iter().filter(|entry| entry["kind"] == "savedSnapshot").count(), 2);
    let doc = &session.doc;
    assert!(doc.body[0].as_para().unwrap().text.contains("imported from saved worksheet values"));
    assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("external data for OtherTable was not imported"));
    let equations = objects(doc);
    assert!(matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.state == CalcState::Fresh && linear.ends_with("=2")));
    assert!(matches!(&equations[0], InlineObject::Equation { calc: Some(calc), .. } if calc.source == "ExternalTable := [2; 4]"));
    assert!(equations.iter().any(|object| matches!(object, InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source.starts_with("if(") && calc.state == CalcState::Fresh && linear.ends_with("=3"))));
    let table = doc
        .body
        .iter()
        .find_map(|block| match block.as_ref() {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells[1].blocks[0].as_para().unwrap().text, "Second label");
    assert_eq!(table.rows[1].cells[0].blocks[0].as_para().unwrap().text, "2");
}

#[test]
fn xmcd_rejects_malformed_xml_and_dtds() {
    for bytes in [
        b"<worksheet xmlns='http://schemas.mathsoft.com/worksheet30'><regions>".as_slice(),
        b"<!DOCTYPE worksheet><worksheet xmlns='http://schemas.mathsoft.com/worksheet30'><regions/></worksheet>".as_slice(),
    ] {
        assert!(crate::io::open_bytes("test.xmcd", bytes).is_err());
    }
}

#[test]
fn xmcd_pairs_aligned_regions_in_borderless_tables_with_live_calculations() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <settings><presentation>
            <pageModel page-width="612" page-height="792"><margins left="36" right="54" top="54" bottom="36"/></pageModel>
            <textRendering><textStyles>
                <textStyle name="Normal"><inlineAttr font-family="Arial" font-size="10"/></textStyle>
                <textStyle name="Note" base-style="Normal"><blockAttr text-align="right" margin-left="12" text-indent="-12"/></textStyle>
            </textStyles></textRendering>
            <mathRendering><mathStyles><mathStyle name="Variables" font-size="10"/></mathStyles></mathRendering>
        </presentation></settings>
        <regions>
            <region left="0" width="350" align-y="10"><text><p>Inputs</p></text></region>
            <region left="0" width="50" align-y="42"><math><m:define><m:id>flow</m:id><m:real>2</m:real></m:define></math></region>
            <region left="70" width="55" align-y="42"><math><m:eval><m:id>flow</m:id></m:eval></math></region>
            <region left="150" width="150" align-y="40"><text><p style="Note" text-align="inherit">Measured</p></text></region>
            <region left="0" width="125" align-y="72"><text><p>First note</p></text></region>
            <region left="150" width="150" align-y="72"><text><p text-align="justify">Second note</p></text></region>
        </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("aligned.xmcd", worksheet).unwrap();
    assert_eq!(doc.body[0].as_para().unwrap().text, "Inputs");
    let Block::Table(table) = doc.body[1].as_ref() else { panic!("expected a layout table") };
    assert_eq!(table.rows.len(), 2);
    assert!(table.props.borders.as_ref().is_some_and(|borders| !borders.any_visible()));
    assert_eq!(table.rows[0].cells.len(), 3);
    assert!(table.rows[1].cells[0].span() > 1);
    let measured = table.rows[0].cells.last().unwrap().blocks[0].as_para().unwrap();
    assert_eq!(measured.text, "Measured");
    assert_eq!(measured.props.align, Some(wordcraft_doc::props::Align::Right));
    assert_eq!(measured.props.indent_first, Some(-12.0));
    let second = table.rows[1].cells.last().unwrap().blocks[0].as_para().unwrap();
    assert_eq!(second.props.align, Some(wordcraft_doc::props::Align::Justify));
    let bytes = crate::io::save_bytes("aligned.docx", &doc).unwrap();
    let reopened = crate::io::open_bytes("aligned.docx", &bytes).unwrap();
    assert_eq!(reopened.last_section.margin_left, 36.0);
    let Block::Table(table) = reopened.body[1].as_ref() else { panic!("expected a layout table after reopening") };
    let equation = table.rows[0].cells[0].blocks[0].as_para().unwrap();
    assert_eq!(reopened.styles.resolve_char(equation.props.style.as_deref(), equation.props_of_char(0)).size, 10.0);
    let note = table.rows[0].cells.last().unwrap().blocks[0].as_para().unwrap();
    assert_eq!(note.props_of_char(0).font.as_deref(), Some("Arial"));
    assert_eq!(note.props_of_char(0).size, Some(10.0));
    let regions: Vec<_> = table.rows[0]
        .cells
        .iter()
        .flat_map(|cell| &cell.blocks)
        .filter_map(|block| block.as_para())
        .flat_map(|paragraph| &paragraph.objects)
        .collect();
    assert!(matches!(regions.as_slice(), [InlineObject::Equation { calc: Some(_), .. }, InlineObject::Equation { calc: Some(calc), linear, .. }]
        if calc.state == CalcState::Fresh && linear.ends_with("=2")));
}

#[test]
fn find_replace_renames_live_calculation_identifiers_in_imported_tables() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region left="0" width="70" align-y="15"><math><m:define><m:id>Lb</m:id><m:real>2</m:real></m:define></math></region>
            <region left="120" width="140" align-y="15"><text><p>Pipe length</p></text></region>
            <region left="0" width="140" align-y="45"><math><m:eval><m:apply><m:plus/><m:id>Lb</m:id><m:id>Lb</m:id></m:apply></m:eval></math></region>
            <region left="0" width="120" align-y="75"><math><m:define><m:id>Lb_extra</m:id><m:real>8</m:real></m:define></math></region>
        </regions>
    </worksheet>"#;
    let mut s = Session::new(crate::io::open_bytes("length.xmcd", worksheet).unwrap());
    let original = objects(&s.doc);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Lb"}))["count"], 2);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Lb", "includeCalculations": false}))["count"], 0);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "Lb", "with": "L_u"}))["replaced"], 0);
    assert_eq!(objects(&s.doc), original);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Lb", "includeCalculations": true}))["count"], 2);
    assert!(s.run("edit.replaceAll", &json!({"text": "Lb", "with": "Lb_extra"})).is_err());
    assert_eq!(objects(&s.doc), original);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "Lb", "with": "L_u"}))["replaced"], 3);
    let edited = objects(&s.doc);
    assert!(matches!(&edited[0], InlineObject::Equation { calc: Some(calc), .. } if calc.source == "L_u := 2"));
    assert!(matches!(&edited[1], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source.matches("L_u").count() == 2 && calc.state == CalcState::Fresh && linear.ends_with("=4")));
    assert_eq!(edited[2], original[2]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(objects(&s.doc), original);
}

#[test]
fn xmcd_imports_disabled_math_as_static_objects_without_defining_names() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region top="10"><text><p>Equation:
                <region width="60" height="20"><math disable-calc="true">
                    <m:define><m:id>coef</m:id><m:real>12</m:real></m:define>
                </math><rendering item-idref="9"/></region>; explanatory text</p></text></region>
            <region top="40"><math disable-calc="true">
                <m:define><m:id>coef</m:id><m:apply><m:div/><m:real>1</m:real><m:real>2</m:real></m:apply></m:define>
            </math></region>
            <region top="60"><math><m:define><m:id>coef</m:id><m:real>3</m:real></m:define></math></region>
            <region top="80"><math><m:eval><m:id>coef</m:id></m:eval></math></region>
        </regions>
        <binaryContent><item item-id="9">
            iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jLfcAAAAASUVORK5CYII=
        </item></binaryContent>
    </worksheet>"#;
    let doc = crate::io::open_bytes("disabled.xmcd", worksheet).unwrap();
    let first = doc.body[0].as_para().unwrap();
    assert!(first.text.ends_with("; explanatory text") && !first.text.contains("coef"));
    let equations = objects(&doc);
    assert!(matches!(&equations[0], InlineObject::Equation { calc: None, display: false, linear, math }
        if linear == "coef==12" && math.nodes.iter().any(|node| matches!(node, wordcraft_doc::math::MNode::Run(run)
            if run.text == "=" && run.sty == Some(wordcraft_doc::math::MSty::Bold)))));
    assert!(matches!(&equations[1], InlineObject::Equation { calc: None, display: true, linear, math }
        if linear.starts_with("coef==") && matches!(math.nodes.as_slice(),
            [wordcraft_doc::math::MNode::Run(lhs), wordcraft_doc::math::MNode::Run(equal), wordcraft_doc::math::MNode::Frac { .. }]
            if lhs.text == "coef" && equal.text == "=" && equal.sty == Some(wordcraft_doc::math::MSty::Bold))));
    assert!(matches!(&equations[2], InlineObject::Equation { calc: Some(calc), .. } if calc.source == "coef := 3"));
    assert!(matches!(&equations[3], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.state == CalcState::Fresh && linear.ends_with("=3")));
}

#[test]
fn xmcd_preserves_a_centered_cover_and_ignores_xml_indentation() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30">
        <settings><presentation><pageModel page-height="792"><margins top="72" bottom="36"/></pageModel></presentation></settings>
        <regions>
            <region left="0" top="90" height="15" width="450"><text><p text-align="center">
                <f size="14"><b>Appendix B</b></f><sp/>Cover</p></text></region>
            <region left="6" top="720" height="15" width="450"><text><p>
                <b>Part 1</b> - Pipe Heat Transfer</p></text></region>
            <region left="100" top="760" height="20" width="20">
                <ole item-idref="1"/><rendering item-idref="2"/>
            </region>
        </regions>
        <binaryContent><item item-id="2">
            iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jLfcAAAAASUVORK5CYII=
        </item></binaryContent>
    </worksheet>"#;
    let doc = crate::io::open_bytes("cover.xmcd", worksheet).unwrap();
    assert_eq!(doc.body.len(), 3);
    let cover = doc.body[0].as_para().unwrap();
    assert_eq!(cover.text, "Appendix B Cover");
    assert_eq!(cover.props.align, Some(wordcraft_doc::props::Align::Center));
    let heading = doc.body[1].as_para().unwrap();
    assert_eq!(heading.text, "Part 1 - Pipe Heat Transfer");
    assert_eq!(heading.props.page_break_before, Some(true));
    assert!(matches!(doc.body[2].as_para().unwrap().objects.as_slice(), [InlineObject::Image { .. }]));
}

#[test]
fn xmcd_imports_pointwise_vectorization_and_keeps_trailing_subscripts_distinct() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region><math><m:define><m:id>flow</m:id><m:matrix rows="2" cols="1"><m:real>2</m:real><m:real>3</m:real></m:matrix></m:define></math></region>
            <region><math><m:define><m:id subscript="dA_">C</m:id>
                <m:apply><m:vectorize/><m:parens><m:apply><m:mult/><m:id>flow</m:id><m:id>flow</m:id></m:apply></m:parens></m:apply>
            </m:define></math></region>
            <region><math><m:define><m:id subscript="dA">C</m:id><m:eval>
                <m:apply><m:id>mean</m:id><m:id subscript="dA_">C</m:id></m:apply>
            </m:eval></m:define></math></region>
            <region region-id="4"><math><m:eval>
                <m:apply><m:vectorize/><m:apply><m:id>mean</m:id><m:id>flow</m:id></m:apply></m:apply>
            </m:eval></math></region>
        </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("vectorized.xmcd", worksheet).unwrap();
    let paragraph = doc.body[2].as_para().unwrap();
    assert!(matches!(&paragraph.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "C_dA := mean(C_dA_2) =" && linear.ends_with("=6.5")));
    assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("[Mathcad region 4: unsupported calculation was not imported]"));
}

#[test]
fn xmcd_uses_user_defined_unit_scales_in_results() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region><math><m:eval><m:apply><m:mult/><m:real>2</m:real><m:id>psi</m:id></m:apply>
                <m:unitOverride><m:id>inWG</m:id></m:unitOverride>
            </m:eval></math></region>
            <region><math><m:define><m:id>inWG</m:id>
                <m:apply><m:mult/><m:real>0.5</m:real><m:id>psi</m:id></m:apply>
            </m:define></math></region>
        </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("pressure.xmcd", worksheet).unwrap();
    let paragraph = doc.body[0].as_para().unwrap();
    assert!(matches!(&paragraph.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.state == CalcState::Fresh && linear.ends_with("=4 \"inWG\"")));
}

#[test]
fn xmcd_displays_a_powered_result_unit_with_a_superscript() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions><region><math><m:eval>
            <m:apply><m:mult/><m:real>22</m:real><m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply></m:apply>
            <m:unitOverride><m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply></m:unitOverride>
        </m:eval></math></region></regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("volume.xmcd", worksheet).unwrap();
    let paragraph = doc.body[0].as_para().unwrap();
    assert!(matches!(&paragraph.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source.ends_with("= ? ft³") && linear.ends_with("=22 \"ft³\"")));
}

#[test]
fn xmcd_displays_compound_result_units_as_stacked_fractions() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions><region><math><m:eval>
            <m:apply><m:div/><m:id>lbm</m:id><m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply></m:apply>
            <m:unitOverride><m:apply><m:div/><m:id>lbm</m:id>
                <m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply>
            </m:apply></m:unitOverride>
        </m:eval></math></region></regions>
    </worksheet>"#;
    use wordcraft_doc::math::{FracKind, MNode, ScriptKind};
    let doc = crate::io::open_bytes("density.xmcd", worksheet).unwrap();
    let paragraph = doc.body[0].as_para().unwrap();
    let InlineObject::Equation { calc: Some(calc), linear, math, .. } = &paragraph.objects[0] else {
        panic!("expected a live density calculation");
    };
    assert!(calc.source.ends_with("= ? lbm/ft³"));
    assert!(linear.ends_with("=1 \"lbm/ft³\""));
    assert!(
        matches!(math.nodes.last(), Some(MNode::Frac { kind: FracKind::Bar, num, den })
        if matches!(num.as_slice(), [MNode::Run(run)] if run.text == "lbm")
            && matches!(den.as_slice(), [MNode::Script { kind: ScriptKind::Sup, .. }])),
        "{:?}",
        math.nodes
    );
}

#[test]
fn xmcd_preserves_saved_derivations_and_blank_inline_comparisons() {
    let xml = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region region-id="1" top="10" width="100" height="25">
          <math><m:apply><m:equal/><m:apply><m:equal/><m:id>x</m:id><m:id>y</m:id></m:apply><m:id> </m:id></m:apply></math>
          <rendering item-idref="1"/>
        </region>
        <region region-id="2" top="30"><text><p>Limit <region><math><m:apply><m:lessOrEqual/>
          <m:id> </m:id><m:id> </m:id></m:apply></math></region></p></text></region>
      </regions>
      <binaryContent><item item-id="1">
        iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jLfcAAAAASUVORK5CYII=
      </item></binaryContent>
    </worksheet>"#;
    let doc = crate::io::open_bytes("derivation.xmcd", xml).unwrap();
    let first = doc.body[0].as_para().unwrap();
    assert!(matches!(first.objects.as_slice(), [InlineObject::Image { .. }]));
    assert!(first.worksheet_notes.iter().any(|note| matches!(note.kind, wordcraft_doc::para::WorksheetNoteKind::SavedSnapshot)));
    let second = doc.body[1].as_para().unwrap();
    assert!(matches!(second.objects.as_slice(), [InlineObject::Equation { linear, calc: None, .. }] if linear == "□≤□"));
}

#[test]
fn worksheet_number_formats_override_defaults_and_survive_docx() {
    let mut xml =
        String::from(r#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30"><regions>"#);
    let examples = [
        ("1200", r#"<general precision="1" exponential-threshold="3" show-trailing-zeros="true"/>"#),
        ("12", r#"<decimal precision="2" show-trailing-zeros="true"/>"#),
        ("1200", r#"<scientific precision="2" show-trailing-zeros="true" use-e-notation="true"/>"#),
        ("12345", r#"<engineering precision="1" use-e-notation="true"/>"#),
        ("-2.25", r#"<fraction precision="12" use-mixed-numbers="true"/>"#),
        ("12", ""),
    ];
    for (i, (value, format)) in examples.into_iter().enumerate() {
        xml.push_str(&format!(
            r#"<region top="{}"><math><m:eval><m:real>{value}</m:real></m:eval><resultFormat>{format}</resultFormat></math></region>"#,
            i * 10
        ));
    }
    xml.push_str("</regions></worksheet>");
    let mut session = Session::new(crate::io::open_bytes("formats.xmcd", xml.as_bytes()).unwrap());
    run(&mut session, "calc.settings", json!({"resultFormat":{"style":"decimal","decimals":4,"trailingZeros":true}}));
    let output = crate::io::save_bytes("formats.docx", &session.doc).unwrap();
    let reopened = crate::io::open_bytes("formats.docx", &output).unwrap();
    let expected = ["=1.2×〖10〗^3", "=12.00", "=1.20E+003", "=12.3E+003", "=-2 1/4", "=12.0000"];
    for doc in [&session.doc, &reopened] {
        for (object, suffix) in objects(doc).iter().zip(expected) {
            assert!(
                matches!(object, InlineObject::Equation { linear, calc: Some(c), .. } if c.state == CalcState::Fresh && linear.ends_with(suffix)),
                "{object:?}, expected {suffix}"
            );
        }
    }

    let id = wordcraft_calc::body_regions(&session.doc)[1].2.id;
    run(&mut session, "calc.resultFormat", json!({"id": id, "resultFormat": null}));
    assert!(matches!(&objects(&session.doc)[1], InlineObject::Equation { linear, .. } if linear.ends_with("=12.0000")));
}

#[test]
fn function_plots_sample_live_variables_and_keep_their_sources_in_docx() {
    let mut s = Session::new(Document::new());
    run(&mut s, "insert.calculation", json!({"source":"gain := 2"}));
    let gain = wordcraft_calc::body_regions(&s.doc)[0].2.id;
    run(&mut s, "caret.docEnd", json!({}));
    let chart = run(&mut s, "insert.functionPlot", json!({"variable":"t","expression":"gain*t","range":"0..3"}));
    assert_eq!(chart["chart"]["series"][0]["values"], json!([0.0, 2.0, 4.0, 6.0]));
    run(&mut s, "calc.setSource", json!({"id":gain,"source":"gain := 3"}));
    let read_plot = |doc: &Document| {
        objects(doc)
            .into_iter()
            .find_map(|o| match o {
                InlineObject::Graphic { graphic, .. } => graphic.chart.clone(),
                _ => None,
            })
            .expect("editable chart")
    };
    assert_eq!(read_plot(&s.doc).series[0].values.last().copied().flatten(), Some(9.0));
    let bytes = crate::io::save_bytes("plot.docx", &s.doc).unwrap();
    let doc = crate::io::open_bytes("plot.docx", &bytes).unwrap();
    let plot = read_plot(&doc);
    assert_eq!(plot.plot.as_ref().unwrap().expression, "gain*t");
    assert_eq!(plot.plot.as_ref().unwrap().range, "0..3");
    assert_eq!(plot.series[0].values.last().copied().flatten(), Some(9.0));
}

#[test]
fn xmcd_imports_global_definitions_as_live() {
    let xml = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:eval><m:id>coefficient</m:id></m:eval></math></region>
        <region top="15"><math><m:define><m:id>points</m:id><m:range>
          <m:sequence><m:real>21</m:real><m:real>22</m:real></m:sequence><m:real>30</m:real>
        </m:range></m:define></math></region>
        <region top="20"><math><m:globalDefine><m:id>coefficient</m:id><m:real>42</m:real></m:globalDefine></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("plot.xmcd", xml).unwrap();
    let sampled = wordcraft_calc::sample_plots(
        &doc,
        &[wordcraft_doc::chart::PlotSpec { variable: "z".into(), expression: "z * coefficient".into(), range: "points".into() }],
    );
    assert_eq!(sampled[0].as_ref().unwrap().last(), Some(&(30.0, 1260.0)));
    let objects = objects(&doc);
    assert!(matches!(&objects[0], InlineObject::Equation { linear, calc: Some(c), .. } if c.state == CalcState::Fresh && linear.ends_with("=42")));
    assert!(matches!(&objects[2], InlineObject::Equation { calc: Some(c), .. } if c.source == "coefficient := 42" && c.state == CalcState::Fresh));
}

#[test]
fn xmcd_formats_feet_and_fractional_inches() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:eval><m:apply><m:id>/FIF</m:id>
          <m:apply><m:mult/><m:real>76</m:real><m:id>in</m:id></m:apply>
        </m:apply></m:eval></math></region>
        <region top="20"><math><m:eval><m:apply><m:id>/FIF</m:id>
          <m:apply><m:mult/><m:real>-16.375</m:real><m:id>in</m:id></m:apply>
        </m:apply></m:eval></math></region>
        <region top="30"><math><m:eval><m:apply><m:id>/FIF</m:id>
          <m:apply><m:mult/><m:real>11.99</m:real><m:id>in</m:id></m:apply>
        </m:apply></m:eval></math></region>
        <region top="40"><math><m:eval><m:apply><m:id>/FIF</m:id><m:real>1</m:real></m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("feet-and-inches.xmcd", worksheet).unwrap();
    let equations = objects(&doc);
    for (index, expected) in [r#"6' 4""#, r#"-1' 4 3/8""#, r#"1' 0""#].into_iter().enumerate() {
        assert!(
            matches!(&equations[index], InlineObject::Equation { linear, calc: Some(calc), .. }
            if calc.state == CalcState::Fresh && linear.contains(expected)),
            "{:?}",
            equations[index]
        );
    }
    assert!(matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), .. } if matches!(calc.state, CalcState::Error(_))));
}

#[test]
fn xmcd_preserves_legacy_units_and_affine_temperature_calculations() {
    let xml = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:eval>
          <m:apply><m:mult/><m:real>62.4</m:real><m:apply><m:div/><m:id>lb</m:id><m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply></m:apply></m:apply>
          <m:unitOverride><m:apply><m:div/><m:id>lb</m:id><m:apply><m:pow/><m:id>ft</m:id><m:real>3</m:real></m:apply></m:apply></m:unitOverride>
        </m:eval></math></region>
        <region top="20"><math><m:eval><m:apply><m:mult/><m:real>0.2</m:real><m:id>g</m:id></m:apply><m:unitOverride><m:id>g</m:id></m:unitOverride></m:eval></math></region>
        <region top="30"><math><m:eval>
          <m:apply><m:minus/><m:apply fixity="postfix"><m:id>&#176;F</m:id><m:real>115</m:real></m:apply><m:apply fixity="postfix"><m:id>&#176;F</m:id><m:real>40</m:real></m:apply></m:apply>
          <m:unitOverride><m:id>&#916;&#176;F</m:id></m:unitOverride>
        </m:eval></math></region>
        <region top="40"><math><m:eval>
          <m:apply><m:id>linterp</m:id><m:sequence>
            <m:apply fixity="postfix"><m:id>&#176;F</m:id><m:matrix rows="2" cols="1"><m:real>100</m:real><m:real>200</m:real></m:matrix></m:apply>
            <m:apply><m:mult/><m:matrix rows="2" cols="1"><m:real>16</m:real><m:real>13.3</m:real></m:matrix><m:id>ksi</m:id></m:apply>
            <m:apply fixity="postfix"><m:id>&#176;F</m:id><m:real>140</m:real></m:apply>
          </m:sequence></m:apply><m:unitOverride><m:id>ksi</m:id></m:unitOverride>
        </m:eval></math></region>
        <region top="50"><math><m:eval>
          <m:apply><m:id>max</m:id><m:sequence><m:real>0</m:real><m:apply><m:mult/><m:real>0.0625</m:real><m:id>in</m:id></m:apply></m:sequence></m:apply>
          <m:unitOverride><m:id>in</m:id></m:unitOverride>
        </m:eval></math></region>
        <region top="60"><math><m:eval>
          <m:apply><m:id>max</m:id><m:sequence><m:real>0</m:real><m:apply><m:mult/><m:real>1</m:real><m:id>in</m:id></m:apply><m:apply><m:mult/><m:real>1</m:real><m:id>psi</m:id></m:apply></m:sequence></m:apply>
        </m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("legacy-units.xmcd", xml).unwrap();
    let equations = objects(&doc);
    for (index, expected) in ["=62.4", "=0.2", "=75", "=14.92", "=0.063"].into_iter().enumerate() {
        assert!(
            matches!(&equations[index], InlineObject::Equation { calc: Some(calc), linear, .. }
            if calc.state == CalcState::Fresh && linear.rsplit_once('=').is_some_and(|(_, value)| value.starts_with(&expected[1..]))),
            "{:?}",
            equations[index]
        );
    }
    assert!(matches!(equations.last().unwrap(), InlineObject::Equation { calc: Some(calc), .. } if matches!(calc.state, CalcState::Error(_))));
}

#[test]
fn xmcd_imports_postfix_temperature_literals_with_a_display_unit() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions><region><math><m:define><m:id subscript="H2O">T</m:id><m:eval>
            <m:apply fixity="postfix"><m:id>&#176;F</m:id><m:real>40</m:real></m:apply>
            <m:unitOverride><m:id>R</m:id></m:unitOverride>
        </m:eval></m:define></math></region>
        <region><math><m:eval><m:apply><m:mult/><m:real>500</m:real><m:id>R</m:id></m:apply>
            <m:unitOverride><m:id>&#176;R</m:id></m:unitOverride>
        </m:eval></math></region>
        <region><math><m:define><m:id>R</m:id><m:real>2</m:real></m:define></math></region>
        <region><math><m:eval><m:id>R</m:id></m:eval></math></region></regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("temperature.xmcd", worksheet).unwrap();
    let paragraph = doc.body[0].as_para().unwrap();
    assert!(
        matches!(&paragraph.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "T_H2O := (40 °F) = ? °R" && linear.contains("499.67")),
        "{:?}",
        paragraph.objects
    );
    let literal = doc.body[1].as_para().unwrap();
    assert!(matches!(&literal.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "(500 * °R) = ? °R" && linear.contains("500")));
    let variable = doc.body[3].as_para().unwrap();
    assert!(matches!(&variable.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "R =" && linear.ends_with('2')));
}

#[test]
fn xmcd_normalizes_dotted_names_without_merging_distinct_variables() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region><math><m:define><m:id subscript="100.gage">P</m:id>
                <m:apply><m:mult/><m:real>100</m:real><m:id>psi</m:id></m:apply>
            </m:define></math></region>
            <region><math><m:eval><m:id subscript="100.gage">P</m:id>
                <m:unitOverride><m:id>psi</m:id></m:unitOverride>
            </m:eval></math></region>
            <region><math><m:define><m:id>tank.max.gage</m:id><m:real>7</m:real></m:define></math></region>
            <region><math><m:define><m:id>tank_max_gage</m:id><m:real>2</m:real></m:define></math></region>
            <region><math><m:eval><m:apply><m:minus/><m:id>tank.max.gage</m:id><m:id>tank_max_gage</m:id></m:apply></m:eval></math></region>
        </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("pressure.xmcd", worksheet).unwrap();
    let pressure = doc.body[1].as_para().unwrap();
    assert!(matches!(&pressure.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "P_100_gage = ? psi" && linear.contains("100")));
    let last = doc.body.last().unwrap().as_para().unwrap();
    assert!(
        matches!(&last.objects[0], InlineObject::Equation { calc: Some(calc), linear, .. }
        if calc.source == "(tank_max_gage_2 - tank_max_gage) =" && linear.ends_with('5')),
        "{:?}",
        last.objects
    );
}

#[test]
fn xmcd_uses_guesses_between_given_and_the_first_equation() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:id>Given</m:id></math></region>
        <region top="20"><math><m:define><m:id>guess</m:id><m:apply><m:mult/><m:real>-1</m:real><m:id>ft</m:id></m:apply></m:define></math></region>
        <region top="30"><math><m:apply><m:equal/>
          <m:apply><m:pow/><m:apply><m:div/><m:id>guess</m:id><m:id>ft</m:id></m:apply><m:real>2</m:real></m:apply>
          <m:real>4</m:real>
        </m:apply></math></region>
        <region top="40"><math><m:define><m:id>answer</m:id><m:eval>
          <m:apply><m:Find/><m:id>guess</m:id></m:apply>
        </m:eval></m:define></math></region>
        <region top="50"><math><m:eval><m:apply><m:div/><m:id>answer</m:id><m:id>ft</m:id></m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("guess.xmcd", worksheet).unwrap();
    for doc in [doc.clone(), crate::io::open_bytes("guess.docx", &crate::io::save_bytes("guess.docx", &doc).unwrap()).unwrap()] {
        let equations = objects(&doc);
        assert!(equations.iter().all(|object| matches!(object, InlineObject::Equation { calc: Some(calc), .. } if calc.state == CalcState::Fresh)));
        assert!(matches!(equations.last().unwrap(), InlineObject::Equation { linear, .. } if linear.ends_with("=-2")));
    }
}

#[test]
fn xmcd_moves_hidden_content_to_a_print_separable_live_appendix() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <settings><presentation>
        <pageModel page-width="612" page-height="792" print-single-page-width="true">
          <margins left="36" right="36" top="36" bottom="36"/>
        </pageModel>
      </presentation></settings>
      <regions>
        <region top="5"><text><p>Report</p></text></region>
        <region top="10"><math><m:id>Given</m:id></math></region>
        <region top="20" left="550"><math><m:define><m:id>guess</m:id><m:real>-1</m:real></m:define></math></region>
        <region top="30"><math><m:apply><m:equal/>
          <m:apply><m:pow/><m:id>guess</m:id><m:real>2</m:real></m:apply><m:real>4</m:real>
        </m:apply></math></region>
        <region top="40"><math><m:define><m:id>answer</m:id><m:eval>
          <m:apply><m:Find/><m:id>guess</m:id></m:apply>
        </m:eval></m:define></math></region>
        <region top="50"><area is-collapsed="true"><regions>
          <region top="1"><area is-collapsed="false">
            <region top="1"><text><p>Collapsed notes</p></text></region>
            <region top="2"><math><m:define><m:id>factor</m:id><m:real>3</m:real></m:define></math></region>
          </area></region>
        </regions></area></region>
        <region top="60"><math><m:eval><m:apply><m:mult/><m:id>answer</m:id><m:id>factor</m:id></m:apply></m:eval></math></region>
        <region top="70" left="600"><text><p>Off-margin notes</p></text></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("appendix.xmcd", worksheet).unwrap();
    let reopened = crate::io::open_bytes("appendix.docx", &crate::io::save_bytes("appendix.docx", &doc).unwrap()).unwrap();
    for (pass, doc) in [doc, reopened].into_iter().enumerate() {
        let index = doc
            .body
            .iter()
            .position(|block| block.as_para().is_some_and(|p| p.plain_text() == "Worksheet Appendix"))
            .unwrap_or_else(|| panic!("pass {pass}: {:?}", doc.body.iter().map(|b| b.as_para().map(|p| p.text.clone())).collect::<Vec<_>>()));
        let heading = doc.body[index].as_para().unwrap();
        assert_eq!(heading.props.page_break_before, Some(true));
        assert!(doc.body[..index].iter().any(|block| {
            block
                .as_para()
                .is_some_and(|p| p.objects.iter().any(|object| matches!(object, InlineObject::Equation { linear, .. } if linear.ends_with("=-6"))))
        }));
        let notes: Vec<_> =
            doc.body[index + 1..].iter().filter_map(|block| block.as_para()).filter(|p| p.objects.is_empty()).map(|p| p.text.as_str()).collect();
        assert_eq!(notes, ["Collapsed notes", "Off-margin notes"]);
        let equations = objects(&doc);
        assert!(equations.iter().all(|object| matches!(object, InlineObject::Equation { calc: Some(calc), .. } if calc.state == CalcState::Fresh)));
    }
}

#[test]
fn xmcd_review_groups_only_errors_with_a_single_root() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:define><m:id>RootFailure</m:id><m:id>MissingValue</m:id></m:define></math></region>
        <region top="20"><math><m:define><m:id>ChildFailure</m:id><m:id>RootFailure</m:id></m:define></math></region>
        <region top="30"><math><m:define><m:id>GrandchildFailure</m:id><m:id>ChildFailure</m:id></m:define></math></region>
        <region top="40"><math><m:define><m:id>OtherFailure</m:id><m:id>OtherMissingValue</m:id></m:define></math></region>
        <region top="50"><math><m:eval><m:apply><m:plus/><m:id>GrandchildFailure</m:id><m:id>OtherFailure</m:id></m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let mut session = Session::new(Document::default());
    let opened = run(&mut session, "file.open", json!({"path":"dependencies.xmcd", "data":crate::cmd::insert::base64_encode(worksheet)}));
    let review = opened["importReview"].as_array().unwrap();
    assert_eq!(review.len(), 5);
    assert_eq!(review[1]["upstreamId"], review[0]["id"]);
    assert_eq!(review[2]["upstreamId"], review[0]["id"]);
    assert!(review[4].get("upstreamId").is_none());
}

#[test]
fn xmcd_updates_fixed_matrix_cells_in_order() {
    let worksheet = br#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
      <regions>
        <region top="10"><math><m:define><m:apply><m:indexer/><m:id>Rows</m:id><m:real>0</m:real></m:apply>
          <m:matrix rows="1" cols="2"><m:str>first&quot;</m:str><m:real>4</m:real></m:matrix>
        </m:define></math></region>
        <region top="20"><math><m:define><m:apply><m:indexer/><m:id>Rows</m:id><m:real>1</m:real></m:apply>
          <m:matrix rows="1" cols="2"><m:str>second</m:str><m:real>9</m:real></m:matrix>
        </m:define></math></region>
        <region top="30"><math><m:eval><m:apply><m:plus/>
          <m:apply><m:indexer/><m:apply><m:indexer/><m:id>Rows</m:id><m:real>0</m:real></m:apply>
            <m:sequence><m:real>0</m:real><m:real>1</m:real></m:sequence></m:apply>
          <m:apply><m:indexer/><m:apply><m:indexer/><m:id>Rows</m:id><m:real>1</m:real></m:apply>
            <m:sequence><m:real>0</m:real><m:real>1</m:real></m:sequence></m:apply>
        </m:apply></m:eval></math></region>
      </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("results.xmcd", worksheet).unwrap();
    assert!(matches!(objects(&doc).last().unwrap(), InlineObject::Equation { linear, calc: Some(calc), .. }
        if calc.state == CalcState::Fresh && linear.ends_with("=13")));
}

#[test]
fn xmcd_solves_complete_blocks_only() {
    let template = r#"<worksheet xmlns="http://schemas.mathsoft.com/worksheet30" xmlns:m="http://schemas.mathsoft.com/math30">
        <regions>
            <region><math><m:apply><m:equal/><m:id>unbound</m:id><m:real>4</m:real></m:apply></math></region>
            <region><math><m:define><m:id>in</m:id><m:real>1</m:real></m:define></math></region>
            <region region-id="2"><math><m:id>Given</m:id></math></region>
            <region><math><m:apply><m:equal/><m:id>in</m:id><m:real>4</m:real></m:apply></math></region>
            EXTRA
            <region><math><m:define><m:id>in</m:id>
                <m:eval><m:parens><m:apply><m:Find auto-method="true"/><m:id>in</m:id></m:apply></m:parens></m:eval>
            </m:define></math></region>
        </regions>
    </worksheet>"#;
    let doc = crate::io::open_bytes("solver.xmcd", template.replace("EXTRA", "").as_bytes()).unwrap();
    let standalone = doc.body[0].as_para().unwrap();
    assert!(
        matches!(&standalone.objects[0], InlineObject::Equation { linear, calc: None, .. } if linear.contains("unbound") && !linear.contains('⚠'))
    );
    let given = doc.body[2].as_para().unwrap();
    assert!(matches!(&given.objects[0], InlineObject::Equation { calc: Some(calc), .. } if calc.source == "Given"));
    let last = doc.body.last().unwrap().as_para().unwrap();
    assert!(
        matches!(&last.objects[0], InlineObject::Equation { linear, calc: Some(calc), .. }
        if calc.source == "inc := (Find(inb)) =" && linear.ends_with('4')),
        "{:?}",
        last.objects
    );

    let unsupported = template.replace("EXTRA", "<region><math><m:program/></math></region>");
    let doc = crate::io::open_bytes("solver.xmcd", unsupported.as_bytes()).unwrap();
    assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("solve block with an unsupported expression"));
    let last = doc.body.last().unwrap().as_para().unwrap();
    assert!(matches!(&last.objects[0], InlineObject::Equation { linear, .. } if !linear.contains('4')));
}
