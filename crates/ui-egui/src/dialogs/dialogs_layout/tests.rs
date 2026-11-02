use serde_json::json;
use wordcraft_engine::Session;

use super::*;
use crate::dialogs::Dialog;
use crate::{Services, WordApp};

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::from_text("Dear Jo,\nThanks.")), Services::default())
}

fn form(a: &mut WordApp) -> LayoutDialog {
    let Some(Dialog::Layout(d)) = a.dialog.take() else { panic!("no dialog: {:?}", a.dialog) };
    *d
}

/// #407: Line Numbering Options… opens with the section's numbering and OK sets every
/// option; Hyphenation Options sets the document's hyphenation settings.
#[test]
fn line_numbers_and_hyphenation_options_apply() {
    let mut a = app();
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    let LayoutDialog::LineNumbers(mut f) = form(&mut a) else { panic!("line numbers") };
    assert!(!f.on && f.auto_distance && f.restart == "restartPage");
    f.on = true;
    f.start = 10;
    f.count_by = 5;
    f.restart = "continuous".into();
    a.run("layout.lineNumbers", f.params()).unwrap();
    let l = a.session.doc.last_section.line_numbers.clone().unwrap();
    assert_eq!((l.start, l.count_by, l.distance, l.restart), (10, 5, 0.0, LineNumberRestart::Continuous));
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    assert!(matches!(form(&mut a), LayoutDialog::LineNumbers(f) if f.on && f.start == 10 && f.restart == "continuous"));

    a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
    let LayoutDialog::Hyphenation(mut h) = form(&mut a) else { panic!("hyphenation") };
    assert!(!h.auto && h.caps && h.limit == 0);
    (h.auto, h.caps, h.limit) = (true, false, 2);
    a.run("layout.hyphenation", h.params()).unwrap();
    let s = &a.session.doc.settings;
    assert!(s.auto_hyphenation && !s.hyphenate_caps && s.consecutive_hyphen_limit == 2);
    assert!((s.hyphenation_zone - 18.0).abs() < 0.01, "zone untouched: {}", s.hyphenation_zone);
}

/// #1113: Format Page Numbers opens with the section's numbering, OK sets the format and
/// start, and "Continue from previous section" numbers on from the section before; a plain
/// command call without `start` keeps the section's start.
#[test]
fn page_numbers_dialog_sets_the_format_and_start() {
    use wordcraft_doc::section::NumFormat;
    let mut a = app();
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
    assert!(f.cont && f.format == "decimal", "{f:?}");
    f.cont = false;
    f.start = 5;
    f.format = "upperRoman".into();
    a.run("layout.pageNumberFormat", f.params()).unwrap();
    let s = a.session.doc.last_section.clone();
    assert_eq!((s.page_num_format, s.page_num_start), (NumFormat::UpperRoman, Some(5)));
    // The reopened dialog shows the section's numbering; continuing clears the start.
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
    assert!(!f.cont && f.start == 5 && f.format == "upperRoman", "{f:?}");
    f.cont = true;
    a.run("layout.pageNumberFormat", f.params()).unwrap();
    let s = a.session.doc.last_section.clone();
    assert_eq!((s.page_num_format, s.page_num_start), (NumFormat::UpperRoman, None), "continue numbers on from the section before");
    // A call with no `start` keeps it (the ribbon's old plain call cleared it).
    a.run("layout.pageNumberFormat", json!({"start": 7})).unwrap();
    a.run("layout.pageNumberFormat", json!({})).unwrap();
    assert_eq!(a.session.doc.last_section.page_num_start, Some(7));
    // A valid start above 32,767 is not lowered by opening the dialog and accepting it.
    a.run("layout.pageNumberFormat", json!({"start": 40_000})).unwrap();
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let LayoutDialog::PageNumbers(f) = form(&mut a) else { panic!("page numbers") };
    assert_eq!(f.start, 40_000);
    a.run("layout.pageNumberFormat", f.params()).unwrap();
    assert_eq!(a.session.doc.last_section.page_num_start, Some(40_000));
    // The dialog follows the caret: moved to another section, it shows that section's
    // numbering, so OK never writes the numbers read from the first one.
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let LayoutDialog::PageNumbers(mut f) = form(&mut a) else { panic!("page numbers") };
    assert_eq!(f.section, 0);
    a.run("caret.end", json!({})).unwrap();
    a.run("layout.break", json!({"kind": "continuous"})).unwrap();
    a.run("caret.end", json!({})).unwrap();
    assert_eq!(wordcraft_engine::cmd::page::section_index_of_focus(&a.session), 1, "the caret is in the new section");
    a.run("layout.pageNumberFormat", json!({"format": "lowerRoman", "start": 9})).unwrap();
    f.follow(&a);
    assert_eq!((f.section, f.format.as_str(), f.start), (1, "lowerRoman", 9));
}

/// #755: the Line Numbers "From text" distance and the Hyphenation zone follow the
/// File › Options unit (centimeters here); `params` sends points whatever the unit.
#[test]
fn line_numbers_and_hyphenation_measure_in_the_options_unit() {
    let mut a = app();
    a.run("file.options", json!({"units": "centimeters"})).unwrap();
    a.run("layout.lineNumbers", json!({"value": "restartPage", "start": 1, "countBy": 1, "distance": 18.0})).unwrap();
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    let LayoutDialog::LineNumbers(f) = form(&mut a) else { panic!("line numbers") };
    assert!(!f.auto_distance);
    let cm = 18.0 / wordcraft_geom::PT_PER_CM;
    assert!((f.distance - cm).abs() < 0.01, "distance was {} cm", f.distance);
    let sent = f.params()["distance"].as_f64().unwrap_or_default();
    assert!((sent - 18.0).abs() < 0.05, "params sent {sent} pt");

    a.run("layout.hyphenation", json!({"value": true, "zone": 36.0})).unwrap();
    a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
    let LayoutDialog::Hyphenation(h) = form(&mut a) else { panic!("hyphenation") };
    assert!((h.zone - 36.0 / wordcraft_geom::PT_PER_CM).abs() < 0.01, "zone was {} cm", h.zone);
    let sent = h.params()["zone"].as_f64().unwrap_or_default();
    assert!((sent - 36.0).abs() < 0.05, "params sent {sent} pt");
}

/// #407: the Language button opens the dialog (scripts with a language never see it); OK
/// marks the selection and "do not check", Set As Default changes the document default.
#[test]
fn language_dialog_marks_the_selection() {
    let mut a = app();
    a.run("select.all", json!({})).unwrap();
    assert_eq!(a.run("review.language", json!({})).unwrap(), json!({"pending": "language"}));
    let LayoutDialog::Language(mut f) = form(&mut a) else { panic!("language") };
    assert_eq!(f.lang, "en-US");
    f.lang = "sr-Latn-RS".into();
    f.no_proof = true;
    a.run("review.language", f.params()).unwrap();
    let c = a.session.typing_props();
    assert_eq!((c.lang.as_deref(), c.no_proof), (Some("sr-Latn-RS"), Some(true)));
    a.run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
    assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));
    assert!(a.dialog.is_none());
    assert_eq!(language_name("xx-YY"), "xx-YY", "an unknown tag shows as itself");
}

/// #2717: language and no-proof can come from defaults or either kind of style.
#[test]
fn language_dialog_reads_inherited_proofing() {
    for source in ["default", "paragraph", "character"] {
        let mut a = app();
        a.run("select.all", json!({})).unwrap();
        let proofing = wordcraft_doc::CharProps { lang: Some("en-GB".into()), no_proof: Some(true), ..Default::default() };
        match source {
            "default" => a.session.doc.styles.default_chr.overlay(&proofing),
            "paragraph" => a.session.doc.styles.styles.iter_mut().find(|s| s.id == "Normal").unwrap().chr.overlay(&proofing),
            _ => {
                a.session.doc.styles.styles.push(wordcraft_doc::Style {
                    id: "ProofingCharacter".into(),
                    kind: wordcraft_doc::StyleKind::Character,
                    chr: proofing,
                    ..Default::default()
                });
                let (from, to) = a.session.sel.ordered();
                let (from, to) = (from.clone(), to.clone());
                a.session.doc.format_range(&from, &to, &|c| c.style = Some("ProofingCharacter".into())).unwrap();
            }
        }
        let f = LanguageForm::read(&a);
        assert_eq!((f.lang.as_str(), f.no_proof), ("en-GB", true), "{source}");
        a.run("review.language", json!({"lang": "fr-FR", "noProof": false})).unwrap();
        let f = LanguageForm::read(&a);
        assert_eq!((f.lang.as_str(), f.no_proof), ("fr-FR", false), "direct formatting overrides {source}");
    }
}

/// #2717: opening and accepting the dialog must not reset inherited proofing.
#[test]
fn language_dialog_ok_keeps_inherited_proofing() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.run("ui.language", json!({"value": "en"})).unwrap();
    let normal = a.session.doc.styles.styles.iter_mut().find(|s| s.id == "Normal").unwrap();
    normal.chr.lang = Some("en-GB".into());
    normal.chr.no_proof = Some(true);
    a.run("select.all", json!({})).unwrap();
    let before = a.session.doc.clone();
    a.run("review.language", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.doc, before, "cancel must not format the selection");
    h.state_mut().run("review.language", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    let state = h.state_mut().run("format.state", json!({})).unwrap();
    assert_eq!(state["lang"], "en-GB");
    assert_eq!(state["noProof"], true);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.doc, before, "one undo restores the inherited formatting");
}

/// Set As Default and OK persist the chosen proofing language in a fresh app.
#[test]
fn language_dialog_default_survives_a_restart() {
    use egui_kittest::kittest::Queryable;
    let mut a = app();
    a.ui.language = "en".into();
    a.dialog = Some(Dialog::Layout(Box::new(LayoutDialog::Language(LanguageForm { lang: "fr-FR".into(), no_proof: false, detect: false }))));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        a,
    );
    h.run_steps(3);
    h.get_by_label("Set As Default").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let saved = serde_json::to_vec(&h.state().prefs()).unwrap();
    let mut b = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    b.apply_prefs(serde_json::from_slice(&saved).unwrap());
    b.logic(&egui::Context::default());
    assert_eq!(LanguageForm::read(&b).lang, "fr-FR");
    assert!(!b.session.prefs.detect_language);
    assert!(!b.session.dirty && !b.session.can_undo());
    b.run("file.new", json!({})).unwrap();
    assert_eq!(LanguageForm::read(&b).lang, "fr-FR");
}

/// #407: Envelopes starts from the selected address; Add to Document puts an envelope
/// section before the letter without asking to save (nothing is replaced).
#[test]
fn envelope_dialog_adds_an_envelope_section() {
    let mut a = app();
    a.run("select.paragraph", json!({})).unwrap();
    a.session.dirty = true;
    assert_eq!(a.run("mailings.envelopes", json!({})).unwrap(), json!({"pending": "envelopes"}));
    let LayoutDialog::Envelopes(mut f) = form(&mut a) else { panic!("envelopes") };
    assert_eq!(f.delivery, "Dear Jo,");
    f.delivery = "Jo Doe\n1 Main St".into();
    f.ret = "Me".into();
    f.size = ENVELOPE_SIZES.iter().position(|s| s.0 == "Envelope DL").unwrap();
    let document = a.session.document_id();
    a.run("mailings.envelopes", f.params(true)).unwrap();
    assert!(a.dialog.is_none(), "no save prompt");
    assert_eq!(a.session.document_id(), document);
    let secs = a.session.doc.sections();
    assert_eq!(secs.len(), 2);
    assert!((secs[0].1.page_w - 623.6).abs() < 0.1);
    assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Thanks."));
}

/// A table `cols` wide, typed cell by cell (empty cells are skipped over); the caret ends in
/// the last cell.
fn table_app(cols: usize, cells: &[&str]) -> WordApp {
    let mut a = app();
    a.run("insert.table", json!({"rows": cells.len() / cols, "cols": cols})).unwrap();
    for (k, c) in cells.iter().enumerate() {
        if k > 0 {
            a.run("text.tab", json!({})).unwrap();
        }
        if !c.is_empty() {
            a.run("text.insert", json!({"text": c})).unwrap();
        }
    }
    a
}

fn fields(a: &WordApp) -> Vec<(String, String)> {
    let mut out = Vec::new();
    a.session.doc.objects_in_reading_order(&mut |o| {
        if let wordcraft_doc::InlineObject::Field { instr, result, .. } = o {
            out.push((instr.clone(), result.clone()));
        }
    });
    out
}

/// #1201: Table Layout › Formula opens its dialog instead of inserting `=SUM(ABOVE)` at once.
/// The formula is suggested from the numbers next to the cell (above, else to the left), Paste
/// function adds a function to it, and OK inserts the field of the formula as edited.
#[test]
fn table_formula_opens_its_dialog() {
    let mut a = table_app(2, &["Item", "Cost", "Tea", "3", "Jam", "4", "Total", ""]);
    assert_eq!(a.run("table.formula", json!({})).unwrap(), json!({"pending": "tableFormula"}));
    assert!(fields(&a).is_empty(), "nothing inserted yet");
    let LayoutDialog::Formula(mut f) = form(&mut a) else { panic!("formula") };
    assert_eq!(f.formula, "=SUM(ABOVE)");
    f.formula = "=".into();
    f.paste("MAX");
    assert_eq!(f.formula, "=MAX()");
    f.formula = "=MAX(ABOVE)".into();
    a.run("table.formula", f.params()).unwrap();
    assert_eq!(fields(&a), [("=MAX(ABOVE)".to_string(), "4".to_string())]);
    assert!(a.dialog.is_none());

    let mut a = table_app(3, &["Tea", "3", "", "Jam", "4", ""]);
    a.run("table.formula", json!({})).unwrap();
    let LayoutDialog::Formula(f) = form(&mut a) else { panic!("formula") };
    assert_eq!(f.formula, "=SUM(LEFT)", "nothing above, numbers to the left");

    let mut a = app();
    a.run("ui.dialog", json!({"name": "tableFormula"})).unwrap();
    assert!(a.dialog.is_none(), "no dialog outside a table");
}

fn first_column(a: &WordApp) -> Vec<String> {
    let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    t.rows.iter().map(|r| r.cells[0].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()).collect()
}

/// #1200: Table Layout › Sort opens its dialog instead of sorting by the first column at once;
/// with a header row the columns are named by it, and OK sorts the rows under it by the
/// chosen column (as numbers here: 2, 7, 10).
#[test]
fn table_sort_opens_its_dialog() {
    let mut a = table_app(2, &["Name", "Score", "Bea", "10", "Al", "2", "Cy", "7"]);
    assert_eq!(a.run("table.sort", json!({})).unwrap(), json!({"pending": "tableSort"}));
    assert_eq!(first_column(&a), ["Name", "Bea", "Al", "Cy"], "nothing sorted yet");
    let LayoutDialog::Sort(mut f) = form(&mut a) else { panic!("sort") };
    assert!(!f.header && f.column == 0 && !f.descending);
    assert_eq!(f.column_label(1), "Column 2");
    f.header = true;
    assert_eq!(f.column_label(1), "Score");
    f.column = 1;
    a.run("table.sort", f.params()).unwrap();
    assert_eq!(first_column(&a), ["Name", "Al", "Cy", "Bea"]);
    assert!(a.dialog.is_none());
}

fn table_shape(a: &WordApp) -> Vec<usize> {
    a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows.iter().map(|r| r.cells.len()).collect()
}

/// #2592: Split Cells opens its dialog from the ribbon's command (scripts with settings never
/// see it); OK splits the cell into the entered columns and rows. Merge cells before split is
/// offered, and on, when several cells are selected.
#[test]
fn split_cells_opens_its_dialog() {
    let mut a = table_app(2, &["A", "B", "C", "D"]);
    assert_eq!(a.run("table.split", json!({})).unwrap(), json!({"pending": "splitCells"}));
    assert_eq!(table_shape(&a), [2, 2], "nothing split yet");
    let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
    assert_eq!((f.columns, f.rows, f.several), (2, 1, false));
    assert_eq!(f.params()["mergeFirst"], false);
    (f.columns, f.rows) = (3, 2);
    a.run("table.split", f.params()).unwrap();
    assert_eq!(table_shape(&a), [2, 4, 4]);
    assert!(a.dialog.is_none());

    let mut a = table_app(2, &["A", "B", "C", "D"]);
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 1)})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
    assert!(f.several && f.merge_first);
    (f.columns, f.rows) = (2, 1);
    a.run("table.split", f.params()).unwrap();
    // The merged cell spans both rows; its two parts do too.
    assert_eq!(table_shape(&a), [2, 2], "merged into one cell, then split in two");
    let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert!(t.rows[1].cells.iter().all(|c| c.props.vmerge == wordcraft_doc::props::VMerge::Continue));

    let mut a = app();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    assert!(a.dialog.is_none(), "no dialog outside a table");
}

/// #2592, as Word does: for a merged cell the dialog's message says the row count must
/// divide its height (or lie in 1 to the height); the engine refuses the same counts and the
/// document stays as it was.
#[test]
fn split_cells_dialog_checks_a_merged_cells_row_count() {
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(2, 0)})).unwrap();
    a.run("table.merge", json!({})).unwrap();
    a.run("caret.set", json!({"pos": cell(0, 0)})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
    assert_eq!(f.height, 3);
    for (rows, ok) in [(1, true), (2, false), (3, true), (4, false)] {
        f.rows = rows;
        assert_eq!(f.check().is_none(), ok, "rows {rows}");
    }
    f.rows = 2;
    assert!(f.check().is_some_and(|m| m.contains("divisor of 3")));
    let (depth, shape) = (a.session.undo_depth(), table_shape(&a));
    assert!(a.run("table.split", f.params()).is_err());
    assert_eq!((a.session.undo_depth(), table_shape(&a)), (depth, shape));
}

/// #2592, as Word does: Split Cells starts with twice the selected columns and the selected
/// rows; Merge is off and greyed for one cell. Unticking it splits each cell on its own: one
/// row and two columns again.
#[test]
fn split_cells_defaults_follow_the_selection() {
    let cell = |r: u32, c: u32| json!({"story": "body", "path": [0, r, c, 0], "off": 0});
    let defaults = |anchor: Value, focus: Value| {
        let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
        a.run("select.range", json!({"anchor": anchor, "focus": focus})).unwrap();
        a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
        let LayoutDialog::SplitCells(f) = form(&mut a) else { panic!("split cells") };
        f
    };
    let f = defaults(cell(1, 1), cell(1, 1));
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, false, false), "one cell");
    let f = defaults(cell(0, 0), cell(1, 0));
    assert_eq!((f.columns, f.rows, f.merge_first), (2, 2, true), "two cells in a column");
    let f = defaults(cell(0, 0), cell(0, 2));
    assert_eq!((f.columns, f.rows, f.merge_first), (6, 1, true), "a row of three");
    let mut f = defaults(cell(0, 0), cell(1, 2));
    assert_eq!((f.columns, f.rows, f.merge_first, f.max_rows), (6, 2, true, 2), "two rows of three");
    f.set_merge(false);
    assert_eq!((f.columns, f.rows), (2, 1), "each cell on its own");
    assert_eq!(f.params()["mergeFirst"], false);

    // A merged cell is one cell, wherever in it the selection is.
    let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)})).unwrap();
    a.run("table.merge", json!({})).unwrap();
    a.run("select.range", json!({"anchor": cell(0, 0), "focus": cell(1, 0)})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, false, false), "one merged cell");
    // Word allows 1 to its height rows, with its message, and keeps the dialog open.
    f.rows = 3;
    assert_eq!(f.check().as_deref(), Some("The number must be between 1 and 2."));
    f.rows = 2;
    assert_eq!(f.check(), None);

    // The merged cell and the two cells to its right: Merge ticked, 4 × 2.
    let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    a.run("select.range", json!({"anchor": cell(0, 1), "focus": cell(1, 1)})).unwrap();
    a.run("table.merge", json!({})).unwrap();
    a.run("select.range", json!({"anchor": cell(0, 1), "focus": cell(1, 2)})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let LayoutDialog::SplitCells(mut f) = form(&mut a) else { panic!("split cells") };
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (4, 2, true, true), "merged cell and neighbours");
    f.rows = 3;
    assert_eq!(f.check().as_deref(), Some("The number must be between 1 and 2."));

    // One cell merged across two columns: Merge offered and ticked, 2 × 1.
    let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    a.run("select.range", json!({"anchor": cell(2, 0), "focus": cell(2, 1)})).unwrap();
    a.run("table.merge", json!({})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let LayoutDialog::SplitCells(f) = form(&mut a) else { panic!("split cells") };
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, true, true), "a wide cell");
    assert_eq!(f.check(), None);
}

/// #2592: Convert Text to Table opens its dialog; the separator and column count are guessed
/// from the text, the row count follows the columns, and OK converts with the settings.
#[test]
fn text_to_table_opens_its_dialog() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("a\tb\tc\nd")), Services::default());
    a.run("select.all", json!({})).unwrap();
    assert_eq!(a.run("table.fromText", json!({})).unwrap(), json!({"pending": "textToTable"}));
    assert!(a.session.doc.body.iter().all(|b| b.as_table().is_none()), "nothing converted yet");
    let LayoutDialog::TextToTable(mut f) = form(&mut a) else { panic!("text to table") };
    assert_eq!((f.separator.as_str(), f.columns, f.rows, f.autofit.as_str(), f.width.as_str()), ("tab", 3, 2, "initial", "Auto"));
    assert!(f.params().get("width").is_none(), "Auto");
    f.width = "1 in".into();
    assert_eq!((f.check(), f.params()["width"].as_f64()), (None, Some(72.0)));
    f.width = "50 in".into();
    assert!(f.check().is_some_and(|e| e.starts_with("The measurement must be between 0")), "{:?}", f.check());
    f.width = "0".into();
    assert_eq!((f.check(), f.params().get("width")), (None, None), "0 is Auto");
    f.width = "Auto".into();
    f.separator = "paragraph".into();
    f.guess_columns();
    assert_eq!((f.columns, f.rows), (1, 2));
    f.separator = "tab".into();
    f.columns = 2;
    f.count_rows();
    assert_eq!(f.rows, 3, "a b / c / d");
    a.run("table.fromText", f.params()).unwrap();
    assert!(a.dialog.is_none());
    let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert!(!t.props.fixed, "the table still resizes to fit its contents");

    let mut a = app();
    a.run("select.all", json!({})).unwrap();
    a.run("ui.dialog", json!({"name": "textToTable"})).unwrap();
    let LayoutDialog::TextToTable(f) = form(&mut a) else { panic!("text to table") };
    assert_eq!((f.separator.as_str(), f.columns, f.rows), ("paragraph", 1, 2), "a comma in one paragraph only");

    // Word opens comma text on Paragraphs too; Commas counts the columns, short lines pad.
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("a,b,c\nd,e,f\ng,h\ni")), Services::default());
    a.run("select.all", json!({})).unwrap();
    a.run("table.fromText", json!({})).unwrap();
    let LayoutDialog::TextToTable(mut f) = form(&mut a) else { panic!("text to table") };
    assert_eq!((f.separator.as_str(), f.columns, f.rows, f.other_char.as_str()), ("paragraph", 1, 4, "-"));
    f.separator = "comma".into();
    f.guess_columns();
    assert_eq!((f.columns, f.rows), (3, 4));
    a.run("table.fromText", f.params()).unwrap();
    let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(), [3, 3, 3, 3]);
}

/// Convert to Text opens its dialog on Tabs; OK converts with the separator chosen.
#[test]
fn table_to_text_opens_its_dialog() {
    let mut a = table_app(2, &["a", "b", "c", "d"]);
    assert_eq!(a.run("table.toText", json!({})).unwrap(), json!({"pending": "tableToText"}));
    assert!(a.session.doc.body.iter().any(|b| b.as_table().is_some()), "nothing converted yet");
    let LayoutDialog::TableToText(mut f) = form(&mut a) else { panic!("table to text") };
    assert_eq!((f.separator.as_str(), f.other_char.as_str()), ("tab", "-"));
    assert_eq!(f.params(), json!({"separator": "tab"}));
    f.separator = "other".into();
    f.other_char = "/".into();
    assert_eq!(f.params(), json!({"separator": "other", "char": "/"}));
    a.run("table.toText", f.params()).unwrap();
    assert!(a.dialog.is_none());
    let texts: Vec<&str> = a.session.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect();
    assert!(texts.contains(&"a/b") && texts.contains(&"c/d"), "{texts:?}");
    assert!(a.session.doc.body.iter().all(|b| b.as_table().is_none()));

    // Outside a table there is nothing to convert.
    let mut a = app();
    assert_eq!(a.run("ui.dialog", json!({"name": "tableToText"})).unwrap(), json!({"dialog": null}));
}

/// The envelope sizes go through the interface language's catalog and use the region's
/// decimal separator (#1128; the untranslated-text audit found `#10   4.13 × 9.5 in` drawn in
/// English).
#[test]
fn envelope_sizes_follow_the_interface_language_and_region() {
    let size = |name: &str| ENVELOPE_SIZES.iter().find(|s| s.0 == name).map(|(n, w, h)| envelope_label(n, *w, *h)).unwrap();
    assert_eq!(size("Envelope #10"), "#10   4.13 × 9.5 in");
    assert_eq!(size("Envelope DL"), "DL   110 × 220 mm");
    let mut a = app();
    a.run("file.options", json!({"region": "it-IT"})).unwrap();
    crate::options::sync(&a);
    assert_eq!(size("Envelope #10"), "#10   4,13 × 9,5 in");
    crate::i18n::pseudo::set(true);
    assert_eq!(size("Envelope #10"), "⟦#10   4,13 × 9,5 in⟧");
    assert_eq!(size("Envelope DL"), "⟦DL   110 × 220 mm⟧");
    crate::i18n::pseudo::set(false);
    a.run("file.options", json!({"region": "en-US"})).unwrap();
    crate::options::sync(&a);
}

/// #407: Labels builds a new document of the chosen sheet, or a single label.
#[test]
fn labels_dialog_builds_a_sheet() {
    let mut a = app();
    assert_eq!(a.run("mailings.labels", json!({})).unwrap(), json!({"pending": "labels"}));
    let LayoutDialog::Labels(mut f) = form(&mut a) else { panic!("labels") };
    f.text = "Ada Lovelace\nLondon".into();
    f.product = LABEL_PRODUCTS.iter().position(|p| p.id == "a4-3x8").unwrap();
    let r = a.session.run("mailings.labels", &f.params()).unwrap();
    assert_eq!(r["labels"], 24);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).matches("Ada Lovelace").count(), 24);
    f.single = true;
    (f.row, f.col) = (3, 2);
    assert_eq!(a.session.run("mailings.labels", &f.params()).unwrap()["filled"], 1);
    assert!(product_label(f.product).contains("63.5 × 33.9 mm (A4)"));
}

/// A `w`×`h` pixel PNG, base64 for `insert.picture`.
fn png_data(w: u32, h: u32) -> String {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    wordcraft_engine::cmd::insert::base64_encode(&png)
}

/// An app with a `w`×`h` pixel picture, inserted and selected as by Insert › Pictures.
fn picture_app(w: u32, h: u32) -> WordApp {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("insert.picture", json!({"data": png_data(w, h)})).unwrap();
    a
}

fn selected_size(a: &WordApp) -> (f32, f32) {
    let (_, o) = wordcraft_engine::cmd::objects::selected(&a.session).unwrap();
    o.frame().map(|(w, h, _)| (w, h)).unwrap()
}

/// #2573: Picture Format › Size's launcher (`picture.size` without a size) opens the Size
/// dialog on the selected picture: its size, the scale of its original size, Lock aspect ratio
/// and Reset; OK resizes it in one undo step.
#[test]
fn size_dialog_scales_locks_and_resets_the_picture() {
    let mut a = picture_app(40, 20);
    assert_eq!(selected_size(&a), (30.0, 15.0), "40×20 px at 96 ppi");
    a.run("picture.size", json!({"width": 60.0, "height": 15.0, "lockAspect": false})).unwrap();
    assert_eq!(a.run("picture.size", json!({})).unwrap(), json!({"pending": "pictureSize"}));
    assert_eq!(selected_size(&a), (60.0, 15.0), "opening the dialog changes nothing");
    let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
    assert!(f.lock && f.relative, "a picture opens locked, scaled from its original size");
    assert_eq!((f.original, f.uncropped, f.pixels), (Some((30.0, 15.0)), Some((30.0, 15.0)), Some((40, 20))));
    assert_eq!(f.scale(), (200.0, 100.0));
    // Locked, the other side follows in proportion, in size and in scale.
    f.set_scale_height(200.0);
    assert_eq!((f.width, f.height, f.scale()), (120.0, 30.0, (400.0, 200.0)));
    f.set_width(60.0);
    assert_eq!((f.width, f.height), (60.0, 15.0));
    f.lock = false;
    f.set_height(45.0);
    assert_eq!((f.width, f.height), (60.0, 45.0), "unlocked, one side alone");
    f.reset();
    assert_eq!((f.width, f.height, f.scale()), (30.0, 15.0, (100.0, 100.0)), "Reset: the original size");
    f.relative = false;
    assert_eq!(f.scale(), (50.0, 100.0), "from the size the dialog opened with");
    f.set_scale_width(150.0);
    assert_eq!((f.width, f.height), (90.0, 15.0));
    a.run("picture.size", f.params()).unwrap();
    assert_eq!(selected_size(&a), (90.0, 15.0));
    a.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected_size(&a), (60.0, 15.0), "one undo step");
}

/// #2573: a cropped picture's Reset is what the crop leaves of the original; a shape opens the
/// dialog unlocked with no original size; with nothing selected nothing opens.
#[test]
fn size_dialog_follows_the_crop_and_shapes_have_no_original() {
    let mut a = picture_app(40, 20);
    a.run("picture.crop", json!({"left": 0.25, "right": 0.25})).unwrap();
    a.run("picture.size", json!({})).unwrap();
    let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
    assert_eq!((f.original, f.uncropped), (Some((15.0, 15.0)), Some((30.0, 15.0))));
    f.reset();
    assert_eq!((f.width, f.height), (15.0, 15.0));

    let mut a = app();
    assert!(!crate::widgets::enabled(&a, "picture.size"), "nothing to size");
    let _ = a.run("ui.dialog", json!({"name": "pictureSize"}));
    assert!(a.dialog.is_none());
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("picture.size", json!({})).unwrap();
    let LayoutDialog::Size(f) = form(&mut a) else { panic!("size") };
    assert!(!f.lock && !f.relative && f.original.is_none() && f.uncropped.is_none());
    assert_eq!(f.scale(), (100.0, 100.0));
}

/// #2573 review: locked, a side past 4000 pt shrinks the other with it instead of flattening
/// the picture, and so does Reset for a photo larger than 4000 pt; under 1 pt likewise.
#[test]
fn size_dialog_keeps_the_proportions_at_the_limits() {
    let close = |(w, h): (f32, f32), (ew, eh): (f32, f32)| (w - ew).abs() < 0.01 && (h - eh).abs() < 0.01;
    let mut a = picture_app(6000, 30);
    a.run("picture.size", json!({})).unwrap();
    let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
    assert_eq!(f.original, Some((4500.0, 22.5)), "6000×30 px at 96 ppi");
    f.reset();
    assert!(close((f.width, f.height), (4000.0, 20.0)), "Reset: {}×{}", f.width, f.height);
    a.run("picture.size", f.params()).unwrap();
    assert!(close(selected_size(&a), (4000.0, 20.0)), "applied: {:?}", selected_size(&a));
    (f.width, f.height) = (15.0, 30.0);
    f.set_width(4000.0);
    assert_eq!((f.width, f.height), (2000.0, 4000.0));
    f.set_height(0.5);
    assert_eq!((f.width, f.height), (1.0, 2.0));
    f.lock = false;
    f.set_width(9000.0);
    assert_eq!((f.width, f.height), (4000.0, 2.0), "unlocked, one side alone");
}

/// #2573 review: dialogs aren't modal. Cropping the picture or picking another one while the
/// Size dialog is open starts the form over, so OK never sizes a picture with stale numbers;
/// with nothing left to size the dialog closes.
#[test]
fn size_dialog_follows_the_selection() {
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let LayoutDialog::Size(mut f) = form(&mut a) else { panic!("size") };
    f.set_width(300.0);
    assert!(f.follow(&a) && f.width == 300.0, "the same picture keeps the edits");
    // Cropped from the ribbon meanwhile: read again, Reset is what the new crop leaves.
    a.run("picture.crop", json!({"left": 0.25, "right": 0.25})).unwrap();
    assert!(f.follow(&a));
    assert_eq!((f.width, f.original), (30.0, Some((15.0, 15.0))));
    a.run("select.collapse", json!({"end": true})).unwrap();
    a.run("insert.picture", json!({"data": png_data(20, 40)})).unwrap();
    assert!(f.follow(&a), "another picture selected");
    assert_eq!((f.width, f.height, f.original), (15.0, 30.0, Some((15.0, 30.0))));
    a.run("select.collapse", json!({"end": true})).unwrap();
    a.run("text.insert", json!({"text": " caption"})).unwrap();
    assert!(!f.follow(&a), "nothing to size");
}

/// #2573 review: the Size dialog end to end. Its launcher (`picture.size` without a size)
/// opens it; a height typed in pixels changes nothing until OK, which sizes the picture in
/// proportion as one undo step and closes the dialog.
#[test]
fn size_dialog_typed_height_applies_on_ok_in_proportion() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    // The Size section's Height (the Scale section has one too, after it).
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
    h.run_steps(2);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_some());
    assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applies before OK");
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    assert_eq!(selected_size(h.state()), (90.0, 45.0), "60 px = 45 pt high, the width follows");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(selected_size(h.state()), (30.0, 15.0), "one undo step");
}

/// #2573 review: a value outside a field's range (a 1 px picture enlarged past 10000%) is shown
/// as it is. egui would otherwise clamp it on the first frame and report a change, which the
/// form would take as an edit and OK would apply unasked.
#[test]
fn size_dialog_leaves_out_of_range_values_alone() {
    let mut a = picture_app(1, 1);
    a.run("picture.size", json!({"width": 100.0, "height": 100.0, "lockAspect": false})).unwrap();
    a.run("picture.size", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let Some(Dialog::Layout(d)) = &h.state().dialog else { panic!("the Size dialog is open") };
    let LayoutDialog::Size(f) = d.as_ref() else { panic!("size") };
    assert!((f.scale().0 - 100.0 / 0.75 * 100.0).abs() < 1.0, "13333%: {:?}", f.scale());
    assert_eq!((f.width, f.height), (100.0, 100.0), "the form keeps the size");
    assert_eq!(selected_size(h.state()), (100.0, 100.0));
}

/// #2573 review: a typed size is kept when the field loses focus, as text fields do: typing a
/// height and going straight for OK (a real click: press, then release frames later) applies
/// it. Taking it only on Enter would throw the typed size away. Esc cancels, applying nothing.
#[test]
fn size_dialog_typed_height_then_ok_without_enter_applies_it() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let harness = |a: WordApp| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    };
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
    h.run_steps(2);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
    h.run_steps(2);
    let ok = h.get_by_role_and_label(Role::Button, "OK").rect().center();
    h.event(egui::Event::PointerMoved(ok));
    h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    assert_eq!(selected_size(h.state()), (90.0, 45.0), "the typed height, in proportion");

    // Esc throws the typing away and, as Cancel, closes the dialog: nothing applied.
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
    h.run_steps(2);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text("60 px");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "Esc cancels the dialog");
    assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applied");
}
