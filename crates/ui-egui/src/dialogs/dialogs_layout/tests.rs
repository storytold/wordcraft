use std::sync::Arc;

use serde_json::json;
use wordcraft_engine::Session;

use wordcraft_doc::section::LineNumberRestart;
use wordcraft_engine::cmd::mailings::{ENVELOPE_SIZES, LABEL_PRODUCTS};

use super::*;
use crate::dialogs::host::{Filed, FormHost, WordForm, open_form};
use crate::{Services, WordApp};

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::from_text("Dear Jo,\nThanks.")), Services::default())
}

/// The form of the dialog the host has open.
fn form<F: WordForm>(a: &mut WordApp) -> F {
    a.dialog_host.get::<FormHost<F>>().expect("the dialog is open").form().clone()
}

/// #407: Line Numbering Options… opens with the section's numbering and OK sets every
/// option; Hyphenation Options sets the document's hyphenation settings.
#[test]
fn line_numbers_and_hyphenation_options_apply() {
    let mut a = app();
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    let mut f: LineNumbersForm = form(&mut a);
    assert!(!f.on && f.auto_distance && f.restart == "restartPage");
    f.on = true;
    f.start = 10;
    f.count_by = 5;
    f.restart = "continuous".into();
    a.run("layout.lineNumbers", f.params()).unwrap();
    let l = a.session.doc.last_section.line_numbers.clone().unwrap();
    assert_eq!((l.start, l.count_by, l.distance, l.restart), (10, 5, 0.0, LineNumberRestart::Continuous));
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    let f: LineNumbersForm = form(&mut a);
    assert!(f.on && f.start == 10 && f.restart == "continuous");

    a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
    let mut h: HyphenationForm = form(&mut a);
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
    let mut f: PageNumbersForm = form(&mut a);
    assert!(f.cont && f.format == "decimal", "{f:?}");
    f.cont = false;
    f.start = 5;
    f.format = "upperRoman".into();
    a.run("layout.pageNumberFormat", f.params()).unwrap();
    let s = a.session.doc.last_section.clone();
    assert_eq!((s.page_num_format, s.page_num_start), (NumFormat::UpperRoman, Some(5)));
    // The reopened dialog shows the section's numbering; continuing clears the start.
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let mut f: PageNumbersForm = form(&mut a);
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
    let f: PageNumbersForm = form(&mut a);
    assert_eq!(f.start, 40_000);
    a.run("layout.pageNumberFormat", f.params()).unwrap();
    assert_eq!(a.session.doc.last_section.page_num_start, Some(40_000));
    // The dialog follows the caret: moved to another section, it shows that section's
    // numbering, so OK never writes the numbers read from the first one.
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let mut f: PageNumbersForm = form(&mut a);
    assert_eq!(f.section, 0);
    a.run("caret.end", json!({})).unwrap();
    a.run("layout.break", json!({"kind": "continuous"})).unwrap();
    a.run("caret.end", json!({})).unwrap();
    assert_eq!(wordcraft_engine::cmd::page::section_index_of_focus(&a.session), 1, "the caret is in the new section");
    a.run("layout.pageNumberFormat", json!({"format": "lowerRoman", "start": 9})).unwrap();
    f.follow(&a);
    assert_eq!((f.section, f.format.as_str(), f.start), (1, "lowerRoman", 9));
}

/// #4911: zero is an explicit start, distinct from continuing the previous section.
#[test]
fn page_numbers_dialog_preserves_zero_and_continue() {
    let mut a = app();
    a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
    let f: PageNumbersForm = form(&mut a);
    assert!(f.cont);
    assert_eq!(f.start, 1, "unspecified numbering still defaults to one");
    for start in [0, 1, 100_000] {
        a.run("layout.pageNumberFormat", json!({"start": start})).unwrap();
        assert_eq!(a.session.doc.last_section.page_num_start, Some(start));
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let f: PageNumbersForm = form(&mut a);
        assert!(!f.cont);
        assert_eq!(f.start, start);
        assert_eq!(f.params()["start"], json!(start));
        a.run("layout.pageNumberFormat", f.params()).unwrap();
        assert_eq!(a.session.doc.last_section.page_num_start, Some(start));
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let mut f: PageNumbersForm = form(&mut a);
        assert!(!f.cont);
        assert_eq!(f.start, start, "reopening preserves the explicit start");
        f.cont = true;
        assert_eq!(f.params()["start"], serde_json::Value::Null);
        a.run("layout.pageNumberFormat", f.params()).unwrap();
        assert_eq!(a.session.doc.last_section.page_num_start, None);
        a.run("ui.dialog", json!({"name": "pageNumbers"})).unwrap();
        let f: PageNumbersForm = form(&mut a);
        assert!(f.cont);
        assert_eq!(f.start, 1, "continuing does not become an explicit zero");
    }
}

/// #755: the forms hold points whatever the File › Options unit (centimeters here); the field converts
/// for display, so `params` sends what the form holds.
#[test]
fn line_numbers_and_hyphenation_measure_in_the_options_unit() {
    let mut a = app();
    a.run("file.options", json!({"units": "centimeters"})).unwrap();
    a.run("layout.lineNumbers", json!({"value": "restartPage", "start": 1, "countBy": 1, "distance": 18.0})).unwrap();
    a.run("ui.dialog", json!({"name": "lineNumbers"})).unwrap();
    let f: LineNumbersForm = form(&mut a);
    assert!(!f.auto_distance);
    assert_eq!(f.distance, 18.0);

    assert_eq!(f.params()["distance"].as_f64(), Some(18.0));

    a.run("layout.hyphenation", json!({"value": true, "zone": 36.0})).unwrap();
    a.run("ui.dialog", json!({"name": "hyphenation"})).unwrap();
    let h: HyphenationForm = form(&mut a);
    assert_eq!(h.zone, 36.0);
    assert_eq!(h.params()["zone"].as_f64(), Some(36.0));
}

/// #407: the Language button opens the dialog (scripts with a language never see it); OK
/// marks the selection and "do not check", Set As Default changes the document default.
#[test]
fn language_dialog_marks_the_selection() {
    let mut a = app();
    a.run("select.all", json!({})).unwrap();
    assert_eq!(a.run("review.language", json!({})).unwrap(), json!({"pending": "language"}));
    let mut f: LanguageForm = form(&mut a);
    assert_eq!(f.lang, "en-US");
    f.lang = "sr-Latn-RS".into();
    f.no_proof = true;
    a.run("review.language", f.params()).unwrap();
    let c = a.session.typing_props();
    assert_eq!((c.lang.as_deref(), c.no_proof), (Some("sr-Latn-RS"), Some(true)));
    a.run("review.language", json!({"lang": "fr-FR", "default": true})).unwrap();
    assert_eq!(a.session.doc.styles.default_chr.lang.as_deref(), Some("fr-FR"));
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
            "default" => Arc::make_mut(&mut a.session.doc.styles).default_chr.overlay(&proofing),
            "paragraph" => Arc::make_mut(&mut a.session.doc.styles).styles.iter_mut().find(|s| s.id == "Normal").unwrap().chr.overlay(&proofing),
            _ => {
                Arc::make_mut(&mut a.session.doc.styles).styles.push(wordcraft_doc::Style {
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
    let normal = Arc::make_mut(&mut a.session.doc.styles).styles.iter_mut().find(|s| s.id == "Normal").unwrap();
    normal.chr.lang = Some("en-GB".into());
    normal.chr.no_proof = Some(true);
    a.run("select.all", json!({})).unwrap();
    let before = a.session.doc.clone();
    a.run("review.language", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
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
    assert!(h.state().dialog_host.open_dialogs().is_empty());
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
    let language = LanguageForm {
        lang: "fr-FR".into(),
        no_proof: false,
        detect: false,
        whole_document: false,
        initial: ("fr-FR".into(), false),
        proof_touched: false,
    };
    open_form(&mut a, None, language);
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
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

/// #3518: ticking "Apply to the whole document" makes OK send `scope: "document"`; unticked sends none.
#[test]
fn language_dialog_whole_document_checkbox_sends_scope() {
    use egui_kittest::kittest::Queryable;
    let f = LanguageForm {
        lang: "fr-FR".into(),
        no_proof: false,
        detect: false,
        whole_document: false,
        initial: ("fr-FR".into(), false),
        proof_touched: false,
    };
    assert!(f.params().get("scope").is_none());
    assert_eq!(LanguageForm { whole_document: true, ..f.clone() }.params()["scope"], "document");
    let mut a = app();
    a.ui.language = "en".into();
    let para = wordcraft_doc::Paragraph::with_text("Header", Default::default());
    let header = a.session.doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(wordcraft_doc::Block::Para(para))]);
    open_form(&mut a, None, f);
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        a,
    );
    h.run_steps(3);
    h.get_by_label("Apply to the whole document").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    // The header, which the caret isn't in, took the language.
    let doc = &h.state().session.doc;
    let hp = doc.para(wordcraft_doc::StoryRef::Part(header), &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(hp.runs.first().and_then(|r| r.props.lang.as_deref()), Some("fr-FR"));
}

/// "Plain text, then a Do-not-check run", the caret in the plain text.
fn proofing_mix_app() -> WordApp {
    let mut a = app();
    a.run("text.insert", json!({"text": "Plain code"})).unwrap();
    a.run("select.text", json!({"text": "code"})).unwrap();
    a.run("review.language", json!({"lang": "de-DE", "noProof": true})).unwrap();
    a.run("select.text", json!({"text": "Plain"})).unwrap();
    a.run("review.language", json!({"lang": "en-US", "noProof": false})).unwrap();
    a.run("select.text", json!({"text": "Plain"})).unwrap();
    a
}

fn run_proofing(a: &mut WordApp, text: &str) -> (Option<String>, Option<bool>) {
    a.run("select.text", json!({"text": text})).unwrap();
    let st = a.run("format.state", json!({})).unwrap();
    (st["lang"].as_str().map(String::from), st["noProof"].as_bool())
}

/// #3518: whole-document OK sends only what the user changed.
#[test]
fn language_dialog_whole_document_sends_only_changes() {
    // Changing only the language leaves the Do-not-check run alone.
    let mut a = proofing_mix_app();
    let mut f = LanguageForm::read(&a);
    f.whole_document = true;
    f.lang = "en-GB".into();
    let p = f.params();
    assert!(p.get("noProof").is_none());
    a.run("review.language", p).unwrap();
    assert_eq!(run_proofing(&mut a, "code"), (Some("en-GB".into()), Some(true)));
    assert_eq!(run_proofing(&mut a, "Plain"), (Some("en-GB".into()), Some(false)));

    // Changing only Do-not-check leaves each run's language alone.
    let mut a = proofing_mix_app();
    let mut f = LanguageForm::read(&a);
    f.whole_document = true;
    f.no_proof = true;
    f.proof_touched = true;
    let p = f.params();
    assert!(p.get("lang").is_none());
    a.run("review.language", p).unwrap();
    assert_eq!(run_proofing(&mut a, "code"), (Some("de-DE".into()), Some(true)));
    assert_eq!(run_proofing(&mut a, "Plain"), (Some("en-US".into()), Some(true)));

    // No edits: the caret's language, no noProof (never the engine's en-US default).
    let mut a = proofing_mix_app();
    let mut f = LanguageForm::read(&a);
    f.lang = "fr-FR".into();
    f.initial.0 = "fr-FR".into();
    f.whole_document = true;
    let p = f.params();
    assert_eq!(p["lang"], "fr-FR");
    assert!(p.get("noProof").is_none());
    a.run("review.language", p).unwrap();
    assert_eq!(run_proofing(&mut a, "code"), (Some("fr-FR".into()), Some(true)));

    // Selection mode is unchanged: both are sent.
    f.whole_document = false;
    assert_eq!(f.params()["noProof"], false);
}

/// #3518: a ticked Do-not-check box the user did not change (the caret sits in a no-proof run)
/// is not sent, so a language-only whole-document apply leaves other runs' proofing state alone.
#[test]
fn language_dialog_whole_document_unchanged_no_proof_is_not_sent() {
    let mut a = proofing_mix_app();
    a.run("select.text", json!({"text": "code"})).unwrap();
    let mut f = LanguageForm::read(&a);
    assert!(f.no_proof && f.initial.1);
    f.whole_document = true;
    f.lang = "en-GB".into();
    let p = f.params();
    assert!(p.get("noProof").is_none(), "{p}");
    a.run("review.language", p).unwrap();
    assert_eq!(run_proofing(&mut a, "code"), (Some("en-GB".into()), Some(true)));
    assert_eq!(run_proofing(&mut a, "Plain"), (Some("en-GB".into()), Some(false)));
}

/// #3518: with the caret in a no-proof run, ticking the whole-document box and clicking Do not
/// check off and on again (ending ticked) still marks the whole document no-proof.
#[test]
fn language_dialog_whole_document_retoggled_no_proof_is_sent() {
    let mut a = proofing_mix_app();
    a.run("select.text", json!({"text": "code"})).unwrap();
    let mut f = LanguageForm::read(&a);
    assert!(f.no_proof && f.initial.1 && !f.proof_touched);
    f.whole_document = true;
    for _ in 0..2 {
        f.no_proof = !f.no_proof;
        f.proof_touched = true;
    }
    assert!(f.no_proof);
    let p = f.params();
    assert_eq!(p["noProof"], true, "{p}");
    assert!(p.get("lang").is_none(), "{p}");
    a.run("review.language", p).unwrap();
    assert_eq!(run_proofing(&mut a, "Plain"), (Some("en-US".into()), Some(true)));
    assert_eq!(run_proofing(&mut a, "code"), (Some("de-DE".into()), Some(true)));
}

/// #407: Envelopes starts from the selected address; Add to Document puts an envelope
/// section before the letter without asking to save (nothing is replaced).
#[test]
fn envelope_dialog_adds_an_envelope_section() {
    let mut a = app();
    a.run("select.paragraph", json!({})).unwrap();
    a.session.dirty = true;
    assert_eq!(a.run("mailings.envelopes", json!({})).unwrap(), json!({"pending": "envelopes"}));
    let mut f: EnvelopeForm = form(&mut a);
    assert_eq!(f.delivery, "Dear Jo,");
    f.delivery = "Jo Doe\n1 Main St".into();
    f.ret = "Me".into();
    f.size = ENVELOPE_SIZES.iter().position(|s| s.0 == "Envelope DL").unwrap();
    let document = a.session.document_id();
    a.run("mailings.envelopes", f.params(true)).unwrap();
    assert_eq!(a.session.document_id(), document, "no save prompt: nothing is replaced");
    // PLAN: assumed the save prompt's dialog key is `saveChanges` (shell-w8, dialogs-w13).
    assert!(a.dialog_host.open_dialogs().iter().all(|d| d.key.as_deref() != Some("saveChanges")), "no save prompt is open");
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

fn table_shape(a: &WordApp) -> Vec<usize> {
    a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows.iter().map(|r| r.cells.len()).collect()
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
    let mut f: SplitCellsForm = form(&mut a);
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
        let f: SplitCellsForm = form(&mut a);
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
    let mut f: SplitCellsForm = form(&mut a);
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
    let mut f: SplitCellsForm = form(&mut a);
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (4, 2, true, true), "merged cell and neighbours");
    f.rows = 3;
    assert_eq!(f.check().as_deref(), Some("The number must be between 1 and 2."));

    // One cell merged across two columns: Merge offered and ticked, 2 × 1.
    let mut a = table_app(3, &["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    a.run("select.range", json!({"anchor": cell(2, 0), "focus": cell(2, 1)})).unwrap();
    a.run("table.merge", json!({})).unwrap();
    a.run("ui.dialog", json!({"name": "splitCells"})).unwrap();
    let f: SplitCellsForm = form(&mut a);
    assert_eq!((f.columns, f.rows, f.several, f.merge_first), (2, 1, true, true), "a wide cell");
    assert_eq!(f.check(), None);
}

/// The envelope sizes go through the interface language's catalog and use the region's
/// decimal separator (#1128; the untranslated-text audit found `#10   4.13 × 9.5 in` drawn in
/// English).
#[test]
fn envelope_sizes_follow_the_interface_language_and_region() {
    // PLAN: assumed `envelope_label(name, w, h, region)` takes the region's format (dialogs-w27).
    let size = |name: &str, region: &_| ENVELOPE_SIZES.iter().find(|s| s.0 == name).map(|(n, w, h)| envelope_label(n, *w, *h, region)).unwrap();
    let mut a = app();
    assert_eq!(size("Envelope #10", &a.session.region()), "#10   4.13 × 9.5 in");
    assert_eq!(size("Envelope DL", &a.session.region()), "DL   110 × 220 mm");
    a.run("file.options", json!({"region": "it-IT"})).unwrap();
    assert_eq!(size("Envelope #10", &a.session.region()), "#10   4,13 × 9,5 in");
    crate::i18n::apply_language(crate::i18n::Lang::PSEUDO);
    assert_eq!(size("Envelope #10", &a.session.region()), "⟦#10   4,13 × 9,5 in⟧");
    assert_eq!(size("Envelope DL", &a.session.region()), "⟦DL   110 × 220 mm⟧");
    crate::i18n::apply_language(crate::i18n::Lang::EN);
}

/// #407: Labels builds a new document of the chosen sheet, or a single label.
#[test]
fn labels_dialog_builds_a_sheet() {
    let mut a = app();
    assert_eq!(a.run("mailings.labels", json!({})).unwrap(), json!({"pending": "labels"}));
    let mut f: LabelsForm = form(&mut a);
    f.text = "Ada Lovelace\nLondon".into();
    f.product = LABEL_PRODUCTS.iter().position(|p| p.id == "a4-3x8").unwrap();
    let r = a.session.run("mailings.labels", &f.params()).unwrap();
    assert_eq!(r["labels"], 24);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).matches("Ada Lovelace").count(), 24);
    f.single = true;
    (f.row, f.col) = (3, 2);
    assert_eq!(a.session.run("mailings.labels", &f.params()).unwrap()["filled"], 1);
    assert!(product_label(f.product, &a.session.region()).contains("63.5 × 33.9 mm (A4)"));
}

/// A `w`×`h` pixel PNG, base64 for `insert.picture`.
fn png_data(w: u32, h: u32) -> String {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
    let png = wordcraft_geom::picture::encode_format(&image::DynamicImage::from(img.clone()), image::ImageFormat::Png).unwrap();
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
    let mut f: SizeForm = form(&mut a);
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
    let mut f: SizeForm = form(&mut a);
    assert_eq!((f.original, f.uncropped), (Some((15.0, 15.0)), Some((30.0, 15.0))));
    f.reset();
    assert_eq!((f.width, f.height), (15.0, 15.0));

    let mut a = app();
    assert!(!crate::app::command_board::command_enabled(&a, "picture.size"), "nothing to size");
    let _ = a.run("ui.dialog", json!({"name": "pictureSize"}));
    assert!(a.dialog_host.open_dialogs().is_empty());
    a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
    a.run("picture.size", json!({})).unwrap();
    let f: SizeForm = form(&mut a);
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
    let mut f: SizeForm = form(&mut a);
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
    let mut f: SizeForm = form(&mut a);
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

/// Types `text` over the Size section's Height (the Scale section has one too, after it) as a user does: a click
/// puts the caret in the field where it lands, as in a text box (egui's drag value selected all of it on the click,
/// the framework's number field does not), so the field's text is selected first and the typing replaces it.
fn type_height(h: &mut egui_kittest::Harness<'_, WordApp>, text: &str) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("the height field").click();
    crate::test_harness::frames(h, 2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    crate::test_harness::frames(h, 1);
    h.get_all_by_role_and_label(Role::SpinButton, "Height:").next().expect("editing").type_text(text);
}

/// #2573 review: the Size dialog end to end. Its launcher (`picture.size` without a size)
/// opens it; a height typed in pixels changes nothing until OK, which sizes the picture in
/// proportion as one undo step and closes the dialog.
#[test]
fn size_dialog_typed_height_applies_on_ok_in_proportion() {
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    // Frames as the app's driver runs them (`frames` hands the runtime the app's input first): a focus the click
    // asks for, and so the typing, never lands on a harness stepped with `run_steps`.
    crate::test_harness::frames(&mut h, 4);
    // The Size section's Height (the Scale section has one too, after it).
    type_height(&mut h, "60 px");
    crate::test_harness::frames(&mut h, 2);
    assert!(!h.state().dialog_host.open_dialogs().is_empty());
    assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applies before OK");
    // In the framework's form dialog Enter in a single-line field is the dialog's OK (not only the field's
    // commit), so it files the typed height and closes the dialog as the OK button does.
    h.key_press(egui::Key::Enter);
    crate::test_harness::frames(&mut h, 3);
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
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
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let f: SizeForm = form(h.state_mut());
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
        crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
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
    crate::test_harness::frames(&mut h, 4);
    type_height(&mut h, "60 px");
    crate::test_harness::frames(&mut h, 2);
    let ok = h.get_by_role_and_label(Role::Button, "OK").rect().center();
    h.event(egui::Event::PointerMoved(ok));
    h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    crate::test_harness::frames(&mut h, 2);
    h.event(egui::Event::PointerButton { pos: ok, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    crate::test_harness::frames(&mut h, 2);
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
    assert_eq!(selected_size(h.state()), (90.0, 45.0), "the typed height, in proportion");

    // Esc throws the typing away and, as Cancel, closes the dialog: nothing applied.
    let mut a = picture_app(40, 20);
    a.run("picture.size", json!({})).unwrap();
    let mut h = harness(a);
    crate::test_harness::frames(&mut h, 4);
    type_height(&mut h, "60 px");
    // The framework gives Escape to the component that holds a draft first (`overlay/dismissal.rs`): the first
    // throws the field's typing away, the next is the dialog's Cancel.
    h.key_press(egui::Key::Escape);
    crate::test_harness::frames(&mut h, 2);
    h.key_press(egui::Key::Escape);
    crate::test_harness::frames(&mut h, 2);
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "Esc cancels the dialog");
    assert_eq!(selected_size(h.state()), (30.0, 15.0), "nothing applied");
}

/// Presses Yes (`extra` 0) or No (`extra` 1) as the host does: the extra's commands run, then the form steps on.
fn press(f: &mut ManualForm, a: &mut WordApp, extra: usize) {
    for (id, params) in f.extra(extra) {
        a.run(&id, params).unwrap();
    }
    f.applied(a, Filed::Extra(extra), Ok(&[]));
}

/// Yes and No walk the words by byte offsets: a multi-byte letter before the break moves the char
/// choice to a different byte, and the next word starts after the word plus the inserted soft hyphen.
#[test]
fn manual_hyphenation_yes_and_no_walk_the_words_by_byte_offsets() {
    let text = "überraschenderweise überwältigenderweise ".repeat(40);
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text(&text)), Services::default());
    let mut f = ManualForm::start(&mut a);
    if f.done {
        // No hyphenation data in this build, as the engine's own test skips.
        return;
    }
    let (pos, word, choice) = (f.pos.clone(), f.word.clone(), f.choice);
    let off = pos["off"].as_u64().unwrap_or(0) as usize;
    let in_word = word.char_indices().nth(choice).map_or(word.len(), |(i, _)| i);
    assert!(in_word > choice, "the multi-byte letter inside the word makes byte {in_word} differ from char {choice}");
    let expected = off + in_word;
    press(&mut f, &mut a, 0);
    let body = a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert_eq!(body.find('\u{ad}'), Some(expected), "the hyphen goes at the chosen point");
    assert!(f.done || f.pos != pos, "Yes moves on to the next word");

    let mut g = ManualForm::start(&mut a);
    if !g.done {
        let before = a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        let at = g.pos.clone();
        press(&mut g, &mut a, 1);
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), before, "No leaves the text alone");
        assert!(g.done || g.pos != at, "No moves on to the next word");
    }
    for _ in 0..500 {
        if g.done {
            break;
        }
        press(&mut g, &mut a, 1);
    }
    assert!(g.done, "the walk ends");
}
