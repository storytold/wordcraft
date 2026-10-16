//! The New Pleading window: Create waits for the required fields, makes the document blank first
//! (File › New, with its Save Changes prompt), and writes the pleading.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;

use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;
use wordcraft_law::MODULE;
use wordcraft_law_ui::LawAddIn;
use wordcraft_law_ui::pleading::{PleadingForm, firm_from};
use wordcraft_ui_egui::dialogs::Dialog;
use wordcraft_ui_egui::{Services, WordApp};

/// The firm details file sits beside the process-wide profile store; one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("wc-ui-law-pleadings-{}-{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    wordcraft_law::set_profiles_dir(Some(dir));
    guard
}

fn law_app() -> WordApp {
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    let mut a = WordApp::new(s, Services::default());
    a.add_addin(Box::new(LawAddIn::default()));
    a
}

fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

fn form(a: &mut WordApp) -> &mut PleadingForm {
    a.addin_mut::<LawAddIn>(MODULE).unwrap().state.pleading.as_mut().expect("the New Pleading window is open")
}

fn is_open(a: &WordApp) -> bool {
    a.addin::<LawAddIn>(MODULE).unwrap().state.pleading.is_some()
}

fn fill(f: &mut PleadingForm) {
    f.county = "ADAIR".into();
    f.case_number = "26AR-DR00012".into();
    f.first_names = "JANE DOE".into();
    f.second_names = "JOHN DOE".into();
    f.filing_date = "2026-10-15".into();
    f.firm = firm_from(&json!({
        "firmName": "Nolan Law Firm LLC", "address1": "101 Main St", "city": "Kirksville", "state": "MO", "zip": "63501",
        "phone": "660-555-0100", "email": "office@example.com", "attorneyName": "Pat Nolan", "barNumber": "12345",
    }));
}

fn create_disabled(h: &egui_kittest::Harness<'static, WordApp>) -> bool {
    h.get_by_label("Create").accesskit_node().is_disabled()
}

fn body(a: &WordApp) -> String {
    a.session.doc.plain_text(StoryRef::Body)
}

/// The caret sits on the first placeholder, collapsed.
fn caret_on_placeholder(a: &WordApp) -> bool {
    let sel = &a.session.sel;
    sel.anchor == sel.focus
        && a.session.doc.para(sel.focus.story, &sel.focus.path).and_then(|p| p.text.get(sel.focus.off..)).is_some_and(|t| t.starts_with('['))
}

#[test]
fn create_stays_disabled_until_the_required_fields_are_set() {
    let _lock = serial();
    let mut a = law_app();
    // What `law.pleadings.new` with no params asks of the interface.
    a.session.ui_requests.push(json!({"addin": MODULE, "open": "newPleading"}));
    a.run("select.all", json!({})).unwrap();
    assert!(is_open(&a));
    assert_eq!(form(&mut a).title, "Motion", "the title starts as the type's");
    let today = form(&mut a).filing_date.clone();
    assert_eq!(today, wordcraft_law_ui::pleading::today(), "the filing date starts as today");
    assert!(today.len() == 10 && today.as_bytes().get(4) == Some(&b'-') && today.as_bytes().get(7) == Some(&b'-'), "{today}");
    assert_eq!(form(&mut a).params()["filingDate"], today.as_str(), "the date always goes to the command");
    let mut h = harness(a);
    h.run_steps(4);
    assert!(create_disabled(&h));
    assert!(h.query_by_label("Fill in County to create.").is_some(), "the reason is shown");
    fill(form(h.state_mut()));
    h.run_steps(2);
    assert!(!create_disabled(&h));
    form(h.state_mut()).usps = true;
    h.run_steps(2);
    assert!(create_disabled(&h), "USPS service needs the party");
    assert!(h.query_by_label("Fill in Party name to create.").is_some());
    let f = form(h.state_mut());
    f.usps = false;
    f.filing_date.clear();
    h.run_steps(2);
    assert!(create_disabled(&h), "the filing date is required");
    assert!(h.query_by_label("Fill in Filing date to create.").is_some());
    let f = form(h.state_mut());
    f.filing_date = "2026-10-15".into();
    f.firm = Default::default();
    h.run_steps(2);
    assert!(create_disabled(&h), "the firm details are required");
    assert!(h.query_by_label("Fill in Firm name to create.").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(!is_open(h.state()));
}

#[test]
fn create_on_a_clean_document_with_text_runs_file_new_and_writes_the_pleading() {
    let _lock = serial();
    let mut a = law_app();
    a.run("text.insert", json!({"text": "Old notes"})).unwrap();
    a.session.dirty = false;
    let before = a.session.document_id();
    let depth = a.session.undo_depth();
    a.run("law.pleadings.new", json!({})).unwrap();
    assert!(is_open(&a), "no params opens the window");
    assert!(!a.session.dirty && a.session.undo_depth() == depth, "opening it changes nothing");
    fill(form(&mut a));
    let mut h = harness(a);
    h.run_steps(4);
    h.get_by_label("Create").click();
    h.run_steps(4);
    let a = h.state_mut();
    assert!(!is_open(a), "{:?}", a.addin::<LawAddIn>(MODULE).unwrap().state.pleading.as_ref().map(|f| &f.error));
    assert_ne!(a.session.document_id(), before, "a new document");
    let text = body(a);
    assert!(text.contains("IN THE CIRCUIT COURT OF ADAIR COUNTY"), "{text}");
    assert!(!text.contains("Old notes"));
    assert!(caret_on_placeholder(a));
    let firm = a.run("law.firm.get", json!({})).unwrap();
    assert_eq!(firm["firmName"], "Nolan Law Firm LLC", "Remember firm details saved them");
}

#[test]
fn cancelling_the_save_prompt_creates_nothing_and_keeps_the_window() {
    let _lock = serial();
    let mut a = law_app();
    a.run("text.insert", json!({"text": "Unsaved work"})).unwrap();
    assert!(a.session.dirty);
    let before = a.session.document_id();
    a.run("law.pleadings.new", json!({})).unwrap();
    fill(form(&mut a));
    let mut h = harness(a);
    h.run_steps(4);
    h.get_by_label("Create").click();
    h.run_steps(2);
    assert!(matches!(h.state().dialog, Some(Dialog::SaveChanges { .. })), "File › New asks first");
    h.state_mut().run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    h.run_steps(4);
    let a = h.state_mut();
    assert!(is_open(a), "the window stays");
    assert_eq!(form(a).pending, None);
    assert_eq!(a.session.document_id(), before);
    assert_eq!(body(a).trim(), "Unsaved work", "nothing created");
    // Another try, this time not saving: the pleading goes into the new document.
    h.get_by_label("Create").click();
    h.run_steps(2);
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    h.run_steps(4);
    let a = h.state_mut();
    assert!(!is_open(a));
    assert_ne!(a.session.document_id(), before);
    assert!(body(a).contains("STATE OF MISSOURI"));
    assert!(caret_on_placeholder(a));
}
