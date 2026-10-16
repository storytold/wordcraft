//! File › Options › LawCraft: profiles list, edit, import/export, approve.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static LAW_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn unique_id(prefix: &str) -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{}-{}-{}", prefix, std::process::id(), LAW_TEST_ID.fetch_add(1, Ordering::Relaxed), n)
}

/// The law profile store is process-wide; tests in this file run one at a time.
static LAW_SETTINGS_LOCK: Mutex<()> = Mutex::new(());

fn serial() -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let guard = LAW_SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Keep the tests' profile store out of the person's real settings folder: point it at a fresh
    // temp folder for each test, so runs do not accumulate profiles or read another run's.
    let dir = std::env::temp_dir().join(format!("wc-ui-law-store-{}", unique_id("test")));
    let _ = std::fs::create_dir_all(&dir);
    wordcraft_law::set_profiles_dir(Some(dir.clone()));
    (guard, dir)
}
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_law::MODULE;
use wordcraft_law_ui::{LawAddIn, LawUi};
use wordcraft_ui_egui::control::{ControlRequest, Outcome, handle};
use wordcraft_ui_egui::file_dialogs::{FileDialogAnswer, FileDialogRequest};
use wordcraft_ui_egui::{Services, WordApp};

type Shown = Rc<RefCell<Vec<(FileDialogRequest, Sender<FileDialogAnswer>)>>>;

/// The page's state, which lives in the LawCraft add-in.
trait LawState {
    fn law(&mut self) -> &mut LawUi;
    fn law_ref(&self) -> &LawUi;
}

impl LawState for WordApp {
    fn law(&mut self) -> &mut LawUi {
        &mut self.addin_mut::<LawAddIn>(MODULE).unwrap().state
    }
    fn law_ref(&self) -> &LawUi {
        &self.addin::<LawAddIn>(MODULE).unwrap().state
    }
}

/// The app with the LawCraft add-in, as the desktop app sets it up.
fn with_addin(mut a: WordApp) -> WordApp {
    a.add_addin(Box::new(LawAddIn::default()));
    a
}

fn law_app() -> WordApp {
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    with_addin(WordApp::new(s, Services::default()))
}

fn plain_app() -> WordApp {
    with_addin(WordApp::new(Session::new(Document::new()), Services::default()))
}

fn async_app() -> (WordApp, Shown) {
    let shown: Shown = Rc::default();
    let log = shown.clone();
    let services = Services {
        file_dialog: Some(Box::new(move |req| {
            let (tx, rx) = channel();
            log.borrow_mut().push((req, tx));
            rx
        })),
        ..Default::default()
    };
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    (with_addin(WordApp::new(s, services)), shown)
}

fn answer(shown: &Shown, picked: Option<&str>) {
    let (_, tx) = shown.borrow_mut().pop().unwrap();
    tx.send(Ok(picked.map(str::to_string).into_iter().collect())).unwrap();
}

/// Make a profile file unwritable, so the next `law.profiles.update` fails (a forced save error).
/// A save writes a new file beside it and renames it over the old one, so on Unix the folder is
/// made read-only too.
fn make_read_only(path: &std::path::Path) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path, perms).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = path.parent().unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    }
}

/// Restore write access on every platform: on Unix `set_readonly(true)` clears the mode's write
/// bits, so clear `0o644` back; elsewhere `set_readonly(false)` is the matching inverse.
fn make_writable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = path.parent().unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o644);
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(false);
    }
    std::fs::set_permissions(path, perms).unwrap();
}

fn open_law_options(a: &mut WordApp) {
    a.run("file.options", json!({"pane": "addin:law"})).unwrap();
    assert!(a.ui.backstage);
}

/// Click LawCraft's switch on the Add-ins pane (its options page row has the same name).
fn click_switch(h: &egui_kittest::Harness<'static, WordApp>) {
    h.query_all_by_label("LawCraft").find(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox).expect("LawCraft's switch").click();
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

#[test]
fn law_options_lists_bundled_profiles_with_verification_badges() {
    let (_lock, _) = serial();
    let mut a = law_app();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    assert!(h.query_by_label("Missouri appellate brief").is_some());
    assert!(h.query_by_label("Federal appellate brief").is_some());
    assert!(h.query_by_label("Missouri circuit court").is_some());
    assert!(h.query_all_by_label("Missouri").count() >= 2, "both Missouri profiles show their jurisdiction");
    assert!(h.query_all_by_label("Verified").next().is_some());
    assert!(h.query_by_label("missouri-appellate-brief").is_some());
}

#[test]
fn law_module_toggle_uses_file_options_and_shows_law_pane() {
    let (_lock, _) = serial();
    let mut a = plain_app();
    assert!(!a.session.module_enabled(MODULE));
    a.run("file.options", json!({"pane": "addins"})).unwrap();
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("Court profiles").is_none(), "LawCraft pane hidden until the module is on");
    click_switch(&h);
    h.run_steps(6);
    assert!(h.state().session.module_enabled(MODULE));
    assert_eq!(h.state().ui.options_pane, "addin:law");
    assert!(h.query_by_label("Court profiles").is_some());
    let opts = h.state_mut().run("file.options", json!({"modules": {"law": false}})).unwrap();
    assert!(opts["modules"].get("law").is_none(), "{opts}");
    h.run_steps(2);
    assert!(!h.state().session.module_enabled(MODULE));
}

#[test]
fn editing_a_profile_marks_it_unverified() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-edit-unverified");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Edit unverified test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{"name": "Normal"}],
                    "sections": [{"id": "cover", "name": "Cover"}, {"id": "body", "name": "Body"}],
                    "checks": [{"kind": "requiredSections", "ids": ["cover"], "source": "Test"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Edit unverified test").click();
    h.run_steps(6);
    h.get_by_label("Add check").click();
    h.run_steps(4);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["verified"], json!(false));
    assert!(after["docs"]["brief"]["checks"].as_array().is_some_and(|c| c.len() >= 2), "Add check saves through the editor");
}

#[test]
fn law_profile_import_and_export_through_file_dialogs() {
    let (_lock, _) = serial();
    let dir = std::env::temp_dir().join(format!("wc-ui-law-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("export.json");

    let (mut a, shown) = async_app();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Missouri appellate brief").click();
    h.run_steps(4);
    h.get_by_label("Export…").click();
    h.run_steps(2);
    assert!(matches!(shown.borrow().last().map(|(r, _)| r.clone()), Some(FileDialogRequest::Save { .. })));
    answer(&shown, Some(&path.to_string_lossy()));
    h.run_steps(4);
    assert!(path.is_file());

    h.get_by_label("Import…").click();
    h.run_steps(2);
    answer(&shown, Some(&path.to_string_lossy()));
    h.run_steps(4);
    let imported = h.state_mut().run("law.profiles.get", json!({"id": "missouri-appellate-brief"})).unwrap();
    assert_eq!(imported["name"], json!("Missouri appellate brief"));
    assert_eq!(h.state().law_ref().doc_type, "appendix", "import picks the profile's first document type");
    let section_count = imported["docs"]["appendix"]["sections"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(section_count > 0);
    h.run_steps(4);
    assert!(h.query_by_label("Sections").is_some(), "editor shows the selected document type's sections");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editing_a_check_source_across_frames_saves_on_blur() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-source-edit");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Source edit test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "Old"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Source edit test").click();
    h.run_steps(6);

    // Type one character per frame: egui rebuilds the widget buffer every frame, so the draft has
    // to be kept between frames or the characters would be lost before the field can save.
    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in "New".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Source:").value().unwrap_or_default();
    assert!(shown.ends_with("New"), "the field keeps every character typed, not just the last: {shown:?}");

    // Blurring commits through law.profiles.update.
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let saved = after["docs"]["brief"]["checks"][0]["source"].as_str().unwrap_or("");
    assert_ne!(saved, "Old", "the source edit is saved, not discarded");
    assert!(saved.ends_with("New"), "saved source {saved:?}");
}

#[test]
fn editing_a_section_name_across_frames_saves_on_blur() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-section-name");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Section edit test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Section edit test").click();
    h.run_steps(6);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["sections"][0]["name"], json!("Cover Page"));
    assert_eq!(after["verified"], json!(false), "editing the profile makes it unverified again");
}

#[test]
fn editing_a_style_font_across_frames_keeps_spaces() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-style-font");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Style font test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{"name": "Norm1"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Norm1").click();
    h.run_steps(2);

    // One character per frame, spaces included: the untrimmed draft must survive between frames.
    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for ch in "Times New Roman".chars() {
        h.get_by_label("Font:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Font:").value().unwrap_or_default();
    assert_eq!(shown, "Times New Roman", "the style font field keeps the spaces typed: {shown:?}");

    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["styles"][0]["chr"]["font"], json!("Times New Roman"));
    assert_eq!(after["verified"], json!(false), "editing a style makes the profile unverified");
}

#[test]
fn clearing_a_style_font_unsets_it_on_blur() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-style-font-clear");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Style font clear test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{"name": "Norm1", "chr": {"font": "Old"}}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(6);
    h.get_by_label("Norm1").click();
    h.run_steps(2);

    // Erase the existing font and blur. The erase is an edit like any other: it must be committed
    // (as the supported "no font" representation, the key absent) instead of being silently
    // dropped while the draft is cleared.
    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for _ in 0..8 {
        h.key_press(egui::Key::Backspace);
        h.run_steps(1);
    }
    assert_eq!(h.get_by_label("Font:").value().unwrap_or_default(), "", "the field is cleared");
    h.get_by_label("Add check").focus();
    h.run_steps(2);

    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let style = &after["docs"]["brief"]["styles"][0];
    assert!(style["chr"].get("font").is_none(), "clearing the font unsets it, not drops the edit: {style}");
    assert_eq!(after["verified"], json!(false), "clearing a style makes the profile unverified");
    assert!(!h.state().law_ref().style_font_drafts.contains_key(&format!("{id}\0brief\0style\00")), "the draft is cleared once the erase saves");
}

#[test]
fn editing_a_check_font_across_frames_keeps_spaces() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-check-font");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Check font test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "minFontSize", "pt": 13.0, "font": "", "source": "Test"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(6);

    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for ch in "Times New Roman".chars() {
        h.get_by_label("Font:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let shown = h.get_by_label("Font:").value().unwrap_or_default();
    assert_eq!(shown, "Times New Roman", "the check font field keeps the spaces typed: {shown:?}");

    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(after["docs"]["brief"]["checks"][0]["font"], json!("Times New Roman"));
    assert_eq!(after["verified"], json!(false));
}

#[test]
fn editing_source_and_exclusion_together_keeps_both() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-check-merge");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Check merge test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "s1", "name": "Section One"}, {"id": "s2", "name": "Section Two"}],
                    "checks": [{"kind": "wordLimit", "limit": 10000, "excludeSections": [], "source": "Old"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in "New".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    // Clicking the exclusion checkbox also blurs Source in the same frame: both edits must survive.
    h.get_by_label("s1").click();
    h.run_steps(2);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let check = &after["docs"]["brief"]["checks"][0];
    assert!(check["source"].as_str().unwrap_or("").ends_with("New"), "the source edit is saved: {check}");
    assert_eq!(check["excludeSections"], json!(["s1"]), "the exclusion survives the source commit: {check}");
    assert_eq!(after["verified"], json!(false));
}

#[test]
fn renaming_and_reordering_a_section_keeps_both_identities() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-section-reorder");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Section reorder test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [
                        {"id": "first", "name": "First"},
                        {"id": "second", "name": "Second"}
                    ],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    // Rename the first section, then click its down arrow in the same frame: the blur commits the
    // name while the arrow reorders the list.
    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " X".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }
    let down = h.query_all_by_label("↓").find(|n| !n.accesskit_node().is_disabled()).expect("an enabled down arrow for the first section");
    down.click();
    h.run_steps(2);

    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let sections = after["docs"]["brief"]["sections"].as_array().cloned().unwrap_or_default();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0]["id"], json!("second"), "the renamed section moved down one place");
    assert_eq!(sections[1]["id"], json!("first"));
    let renamed = sections.iter().find(|s| s["id"] == json!("first")).unwrap();
    let neighbour = sections.iter().find(|s| s["id"] == json!("second")).unwrap();
    assert_eq!(renamed["name"], json!("First X"), "the rename follows the section's id, not its old index");
    assert_eq!(neighbour["name"], json!("Second"), "the neighbouring section is not renamed");
}

#[test]
fn editing_style_font_and_line_spacing_together_keeps_both() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-style-merge");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Style merge test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [{
                        "name": "Norm1",
                        "chr": {"font": "Old", "size": 12.0},
                        "para": {"lineSpacing": {"rule": "multiple", "value": 1.0}}
                    }],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);
    h.get_by_label("Norm1").click();
    h.run_steps(4);

    h.get_by_label("Font:").focus();
    h.run_steps(2);
    for ch in "Times New Roman".chars() {
        h.get_by_label("Font:").type_text(&ch.to_string());
        h.run_steps(1);
    }

    // Change the size and the line spacing in one frame. The size commits the font through the
    // same handler; the spacing save must merge into the just-saved font, not the frame's starting
    // styles snapshot.
    let (size, spacing) = {
        let spins: Vec<_> = h.query_all_by_role(egui::accesskit::Role::SpinButton).collect();
        assert!(spins.len() >= 2, "expected size and line-spacing spin buttons, found {}", spins.len());
        (spins[0].accesskit_node().locate(), spins[1].accesskit_node().locate())
    };
    // Both actions go into the same frame: `Harness::event` would give each its own frame, and
    // the bug only appears when the two saves happen together.
    h.input_mut().events.push(egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        target_node: size.0,
        target_tree: size.1,
        action: egui::accesskit::Action::Increment,
        data: None,
    }));
    h.input_mut().events.push(egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        target_node: spacing.0,
        target_tree: spacing.1,
        action: egui::accesskit::Action::Increment,
        data: None,
    }));
    h.run_steps(2);

    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let style = &after["docs"]["brief"]["styles"][0];
    let font = style["chr"]["font"].as_str().unwrap_or("");
    assert!(font.ends_with("Times New Roman"), "the font edit survives the spacing save: {style}");
    assert!(style["chr"]["size"].as_f64().unwrap_or(0.0) > 12.0, "the size edit survives the spacing save: {style}");
    assert!(style["para"]["lineSpacing"]["value"].as_f64().unwrap_or(0.0) > 1.0, "the line-spacing edit is saved: {style}");
    assert_eq!(after["verified"], json!(false));
}

#[test]
fn switching_profiles_commits_a_field_that_never_blurred() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let first = unique_id("ui-law-switch-a");
    let second = unique_id("ui-law-switch-b");
    for (id, name) in [(&first, "Switch A"), (&second, "Switch B")] {
        a.run(
            "law.profiles.create",
            json!({
                "schema": 1,
                "id": id,
                "name": name,
                "jurisdiction": "Test",
                "verified": true,
                "docs": {
                    "brief": {
                        "styles": [],
                        "sections": [{"id": "cover", "name": "Cover"}],
                        "source": "Test"
                    }
                }
            }),
        )
        .unwrap();
    }
    a.run("law.profiles.approve", json!({"id": first})).unwrap();
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(8);

    // Select A, type into its section name, then click B without blurring the field first.
    h.get_by_label("Switch A").click();
    h.run_steps(6);
    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }
    h.get_by_label("Switch B").click();
    h.run_steps(6);
    assert_eq!(h.state().law_ref().selected.as_deref(), Some(second.as_str()), "the click still switches profiles");

    // The edit made before switching is committed, not discarded, and the profile is unverified.
    let committed = h.state_mut().run("law.profiles.get", json!({"id": first})).unwrap();
    assert_eq!(committed["docs"]["brief"]["sections"][0]["name"], json!("Cover Page"), "the typed name is saved before the switch");
    assert_eq!(committed["verified"], json!(false), "committing an edit unverifies the profile");

    // Returning to A shows the saved name, not a lost draft.
    h.get_by_label("Switch A").click();
    h.run_steps(6);
    assert_eq!(h.state().law_ref().selected.as_deref(), Some(first.as_str()));
    let shown = h.get_by_label("1.").value().unwrap_or_default();
    assert_eq!(shown, "Cover Page");
}

#[test]
fn toggling_required_marks_the_section_required() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-required");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Required toggle test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.approve", json!({"id": id})).unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Required").click();
    h.run_steps(4);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let checks = after["docs"]["brief"]["checks"].as_array().cloned().unwrap_or_default();
    let req = checks.iter().find(|c| c["kind"] == json!("requiredSections")).expect("a requiredSections check is saved");
    assert_eq!(req["ids"], json!(["cover"]), "the section id becomes required");
    assert_eq!(after["verified"], json!(false), "editing the profile makes it unverified");
}

#[test]
fn validation_errors_are_shown_inline() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-validation");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Validation test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "OK"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);
    let mut p = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    p["docs"]["brief"]["checks"][0]["source"] = json!("x".repeat(201));
    let expect = h.state_mut().run("law.profiles.validate", p).unwrap_err();
    assert!(expect.contains("not valid"), "{expect}");
    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for _ in 0..10 {
        h.get_by_label("Source:").type_text("\u{8}");
        h.run_steps(1);
    }
    h.get_by_label("Source:").type_text(&"x".repeat(201));
    h.run_steps(2);
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    let err = h.state().law_ref().validation.as_deref().unwrap_or("");
    assert!(err.contains("not valid"), "law.profiles.validate reported the problem: {err:?}");
    assert!(h.query_by_label(err).is_some(), "the validation error is shown inline");
    let still = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(still["docs"]["brief"]["checks"][0]["source"], json!("OK"), "a failed validate does not write the profile");
}

#[test]
fn switching_document_type_commits_or_keeps_check_source_draft() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-doc-type-switch");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Doc type switch test",
            "jurisdiction": "Test",
            "verified": true,
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "Brief source"}],
                    "source": "Test"
                },
                "motion": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "Motion source"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in " edited".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    // Switch document type before blur: pending edits must commit first, not be discarded.
    h.get_by_label("motion").click();
    h.run_steps(4);
    assert_eq!(h.state().law_ref().doc_type, "motion");

    let brief = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let saved = brief["docs"]["brief"]["checks"][0]["source"].as_str().unwrap_or("");
    assert!(saved.contains("edited"), "the brief check source was committed before the switch: {saved:?}");

    h.get_by_label("brief").click();
    h.run_steps(4);
    let shown = h.get_by_label("Source:").value().unwrap_or_default();
    assert!(shown.contains("edited"), "returning to brief still shows the saved source: {shown:?}");
}

#[test]
fn switching_document_type_commits_drafts_for_every_profile() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let a_id = unique_id("ui-law-multi-doctype-a");
    let b_id = unique_id("ui-law-multi-doctype-b");
    for (id, name) in [(&a_id, "Multi doctype A"), (&b_id, "Multi doctype B")] {
        a.run(
            "law.profiles.create",
            json!({
                "schema": 1,
                "id": id,
                "name": name,
                "jurisdiction": "Test",
                "docs": {
                    "brief": {
                        "styles": [],
                        "sections": [{"id": "cover", "name": "Cover"}],
                        "checks": [],
                        "source": "Test"
                    },
                    "motion": {
                        "styles": [],
                        "sections": [{"id": "cover", "name": "Cover"}],
                        "checks": [],
                        "source": "Test"
                    }
                }
            }),
        )
        .unwrap();
    }
    open_law_options(&mut a);
    a.law().selected = Some(a_id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    // A pending edit on profile B, which is not the selected profile.
    let b_draft = format!("{b_id}\0brief\0section\0cover");
    h.state_mut().law().section_name_drafts.insert(b_draft.clone(), "B edited".into());

    // B cannot be saved: switching A's document type is a navigation that must commit every draft
    // for the whole page, so it is cancelled and B's draft is not dropped.
    let b_file = store_dir.join(format!("{}.json", b_id));
    make_read_only(&b_file);
    h.get_by_label("motion").click();
    h.run_steps(4);
    assert_eq!(h.state().law_ref().doc_type, "brief", "the switch is cancelled when another profile's draft fails");
    assert!(h.state().law_ref().validation.is_some(), "the failing draft's error shows inline");
    assert!(h.state().law_ref().section_name_drafts.contains_key(&b_draft), "the failing draft is kept for a retry");

    // Once B is writable the same click commits every draft first, then navigates.
    make_writable(&b_file);
    h.get_by_label("motion").click();
    h.run_steps(6);
    assert_eq!(h.state().law_ref().doc_type, "motion", "the switch succeeds once every draft saves");
    let b = h.state_mut().run("law.profiles.get", json!({"id": b_id})).unwrap();
    assert_eq!(b["docs"]["brief"]["sections"][0]["name"], json!("B edited"), "the other profile's draft was committed");
    assert!(!h.state().law_ref().section_name_drafts.contains_key(&b_draft), "the draft is cleared after it saves");
}

#[test]
fn switching_profile_commits_drafts_for_every_profile() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let a_id = unique_id("ui-law-multi-switch-a");
    let b_id = unique_id("ui-law-multi-switch-b");
    for (id, name) in [(&a_id, "Multi switch A"), (&b_id, "Multi switch B")] {
        a.run(
            "law.profiles.create",
            json!({
                "schema": 1,
                "id": id,
                "name": name,
                "jurisdiction": "Test",
                "docs": {
                    "brief": {
                        "styles": [],
                        "sections": [{"id": "cover", "name": "Cover"}],
                        "checks": [],
                        "source": "Test"
                    }
                }
            }),
        )
        .unwrap();
    }
    open_law_options(&mut a);
    a.law().selected = Some(a_id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    let b_draft = format!("{b_id}\0brief\0section\0cover");
    h.state_mut().law().section_name_drafts.insert(b_draft.clone(), "B edited".into());

    let b_file = store_dir.join(format!("{}.json", b_id));
    make_read_only(&b_file);
    h.get_by_label("Multi switch B").click();
    h.run_steps(4);
    assert_eq!(h.state().law_ref().selected.as_deref(), Some(a_id.as_str()), "the profile switch is cancelled when another profile's draft fails");
    assert!(h.state().law_ref().validation.is_some(), "the error shows inline");
    assert!(h.state().law_ref().section_name_drafts.contains_key(&b_draft), "the failing draft is kept");

    make_writable(&b_file);
    h.get_by_label("Multi switch B").click();
    h.run_steps(6);
    assert_eq!(h.state().law_ref().selected.as_deref(), Some(b_id.as_str()), "the switch succeeds once every draft saves");
    let b = h.state_mut().run("law.profiles.get", json!({"id": b_id})).unwrap();
    assert_eq!(b["docs"]["brief"]["sections"][0]["name"], json!("B edited"), "the other profile's draft was committed");
    assert!(!h.state().law_ref().section_name_drafts.contains_key(&b_draft), "the draft is cleared after it saves");
}

#[test]
fn failed_section_name_update_keeps_draft_for_retry() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-failed-retry");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Failed retry test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "OK"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for _ in 0..10 {
        h.get_by_label("Source:").type_text("\u{8}");
        h.run_steps(1);
    }
    h.get_by_label("Source:").type_text(&"x".repeat(201));
    h.run_steps(2);
    h.get_by_label("Add check").focus();
    h.run_steps(2);
    assert!(h.state().law_ref().validation.is_some(), "a failed validate/update surfaces an error");
    let shown = h.get_by_label("Source:").value().unwrap_or_default();
    assert!(shown.chars().count() > 200, "the failed save keeps the draft in the field: {shown:?}");

    let before = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(before["docs"]["brief"]["checks"][0]["source"], json!("OK"), "the invalid draft was not saved");

    let draft_key = format!("{id}\0brief\00");
    h.state_mut().law().check_source_drafts.insert(draft_key.clone(), "Saved after retry".into());

    let profile_file = store_dir.join(format!("{}.json", id));
    make_read_only(&profile_file);

    h.get_by_label("Add check").click();
    h.run_steps(4);
    assert!(h.state().law_ref().validation.is_some(), "a failed flush keeps the error visible");
    make_writable(&profile_file);

    h.get_by_label("General").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.options_pane, "general", "retry commits drafts before leaving LawCraft");
    assert!(h.state().law_ref().validation.is_none(), "a successful retry clears the inline error: {:?}", h.state().law_ref().validation);
    let after = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let saved = after["docs"]["brief"]["checks"][0]["source"].as_str().unwrap_or("");
    assert!(saved.contains("Saved after retry"), "the retried edit persisted: {saved:?}");
    assert!(!h.state().law_ref().check_source_drafts.contains_key(&draft_key), "draft removed after success");
}

#[test]
fn add_check_after_editing_source_keeps_flushed_source() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-add-check-source");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Add check source test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "Before"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("Source:").focus();
    h.run_steps(2);
    for ch in "After flush".chars() {
        h.get_by_label("Source:").type_text(&ch.to_string());
        h.run_steps(1);
    }
    h.get_by_label("Add check").click();
    h.run_steps(6);

    let p = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    let source = p["docs"]["brief"]["checks"][0]["source"].as_str().unwrap_or("");
    assert!(source.contains("After flush"), "Add check kept the flushed source edit: {source:?}");
    assert_eq!(p["docs"]["brief"]["checks"].as_array().map(|a| a.len()), Some(2));
}

#[test]
fn leaving_law_options_cancels_when_pending_drafts_fail_to_commit() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-leave-options");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Leave options test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "OK"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }

    let profile_file = store_dir.join(format!("{}.json", id));
    make_read_only(&profile_file);

    h.get_by_label("General").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.options_pane, "addin:law", "navigation away from LawCraft is cancelled when flush fails");
    assert!(h.state().law_ref().validation.is_some(), "the flush error is shown inline");

    make_writable(&profile_file);
    h.get_by_label("General").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.options_pane, "general", "navigation succeeds after the draft saves");
}

#[test]
fn import_is_cancelled_when_pending_drafts_fail_to_commit() {
    let (_lock, store_dir) = serial();
    let dir = std::env::temp_dir().join(format!("wc-ui-law-import-draft-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let import_path = dir.join("import.json");

    let (mut a, shown) = async_app();
    let id = unique_id("ui-law-import-draft");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Import draft test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    a.run("law.profiles.export", json!({"id": "missouri-appellate-brief", "path": import_path.to_string_lossy()})).unwrap();

    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }

    let profile_file = store_dir.join(format!("{}.json", id));
    make_read_only(&profile_file);

    h.get_by_label("Import…").click();
    h.run_steps(2);
    assert!(shown.borrow().is_empty(), "import is cancelled before the file dialog when flush fails");

    let still = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(still["docs"]["brief"]["sections"][0]["name"], json!("Cover"), "the unsaved draft was not written");

    make_writable(&profile_file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_completion_commits_all_drafts_and_cancels_on_another_profiles_failure() {
    let (_lock, store_dir) = serial();
    let dir = std::env::temp_dir().join(format!("wc-ui-law-import-complete-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let import_path = dir.join("import.json");

    let (mut a, shown) = async_app();
    let a_id = unique_id("ui-law-import-complete-a");
    let b_id = unique_id("ui-law-import-complete-b");
    for (id, name) in [(&a_id, "Import complete A"), (&b_id, "Import complete B")] {
        a.run(
            "law.profiles.create",
            json!({
                "schema": 1,
                "id": id,
                "name": name,
                "jurisdiction": "Test",
                "docs": {
                    "brief": {
                        "styles": [],
                        "sections": [{"id": "cover", "name": "Cover"}],
                        "source": "Test"
                    }
                }
            }),
        )
        .unwrap();
    }
    a.run("law.profiles.export", json!({"id": "missouri-appellate-brief", "path": import_path.to_string_lossy()})).unwrap();

    open_law_options(&mut a);
    a.law().selected = Some(a_id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    // Open the import dialog while there are no drafts.
    h.get_by_label("Import…").click();
    h.run_steps(2);
    assert!(matches!(shown.borrow().last().map(|(r, _)| r.clone()), Some(FileDialogRequest::Open { .. })));

    // A pending edit appears after the dialog opened and its profile cannot be saved. Finishing the
    // pick must commit every page draft first and cancel when one fails, not import over the top.
    let b_file = store_dir.join(format!("{}.json", b_id));
    let b_draft = format!("{b_id}\0brief\0section\0cover");
    h.state_mut().law().section_name_drafts.insert(b_draft.clone(), "B edited".into());
    make_read_only(&b_file);
    answer(&shown, Some(&import_path.to_string_lossy()));
    h.run_steps(4);
    assert!(h.state().law_ref().validation.is_some(), "the failed completion shows the error inline");
    assert_eq!(h.state().law_ref().selected.as_deref(), Some(a_id.as_str()), "the import is cancelled, not applied");
    assert!(h.state().law_ref().section_name_drafts.contains_key(&b_draft), "the failing draft is kept");

    make_writable(&b_file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approve_profile_in_ui_and_block_automation() {
    let (_lock, _) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-approve-test");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Approve UI test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [{"name": "Normal"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(a.run("law.profiles.get", json!({"id": id})).unwrap()["verified"], json!(false));
    open_law_options(&mut a);
    let mut h = harness(a);
    h.run_steps(8);
    h.get_by_label("Approve UI test").click();
    h.run_steps(4);
    let before = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(before["verified"], json!(false));
    h.get_by_label("Approve profile…").click();
    h.run_steps(4);
    assert!(h.state().law_ref().confirm_approve, "Approve profile opens the confirmation step");
    h.get_by_label("Approve").click();
    h.run_steps(4);
    let p = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(p["verified"], json!(true));

    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new("engine.execute", json!({"command": "law.profiles.approve", "params": {"id": id}}));
    let Outcome::Done(v) = handle(h.state_mut(), &ctx, &req) else { panic!("screenshot") };
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap_or("").contains("outside automation"));
}

#[test]
fn turning_lawcraft_off_from_addins_commits_drafts_first_and_cancels_on_failure() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-disable-addins");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Disable addins test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "OK"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }

    // Stand on the Add-ins pane (where the module switch lives) and make the profile unwritable.
    h.state_mut().ui.options_pane = "addins".into();
    h.run_steps(4);
    let profile_file = store_dir.join(format!("{}.json", id));
    make_read_only(&profile_file);
    click_switch(&h);
    h.run_steps(4);

    assert!(h.state().session.module_enabled(MODULE), "a failed save leaves the module on");
    assert!(h.state().law_ref().validation.is_some(), "the save error is shown inline");
    let err = h.state().law_ref().validation.clone().unwrap_or_default();
    assert!(
        h.query_by_label("LawCraft: pending changes on its options page could not be saved").is_some(),
        "the Add-ins pane shows why the add-in stayed on: {err:?}"
    );

    make_writable(&profile_file);
    click_switch(&h);
    h.run_steps(4);
    assert!(!h.state().session.module_enabled(MODULE), "the module turns off once the draft saves");
    assert!(h.state().law_ref().validation.is_none(), "a successful toggle clears the stale error");
}

#[test]
fn file_options_disabling_lawcraft_commits_drafts_first_and_cancels_on_failure() {
    let (_lock, store_dir) = serial();
    let mut a = law_app();
    let id = unique_id("ui-law-disable-direct");
    a.run(
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Disable direct test",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "checks": [{"kind": "noHyperlinks", "source": "OK"}],
                    "source": "Test"
                }
            }
        }),
    )
    .unwrap();
    open_law_options(&mut a);
    a.law().selected = Some(id.clone());
    a.law().doc_type = "brief".into();
    let mut h = harness(a);
    h.run_steps(8);

    h.get_by_label("1.").focus();
    h.run_steps(2);
    for ch in " Page".chars() {
        h.get_by_label("1.").type_text(&ch.to_string());
        h.run_steps(1);
    }

    let profile_file = store_dir.join(format!("{}.json", id));
    make_read_only(&profile_file);
    let r = h.state_mut().run("file.options", json!({"modules": {"law": false}}));
    h.run_steps(4);
    assert!(r.is_err(), "disabling is cancelled when the pending draft cannot be saved: {r:?}");
    assert!(h.state().session.module_enabled(MODULE), "the module stays on");
    assert_eq!(h.state().ui.options_pane, "addin:law", "the LawCraft pane stays visible");
    assert!(h.state().law_ref().validation.is_some(), "the save error is shown inline");

    let shown = h.get_by_label("1.").value().unwrap_or_default();
    assert_eq!(shown, "Cover Page", "the failed disable keeps the draft in the field");
    let still = h.state_mut().run("law.profiles.get", json!({"id": id})).unwrap();
    assert_eq!(still["docs"]["brief"]["sections"][0]["name"], json!("Cover"), "the unsaved draft was not written");

    make_writable(&profile_file);
    h.state_mut().run("file.options", json!({"modules": {"law": false}})).unwrap();
    h.run_steps(4);
    assert!(!h.state().session.module_enabled(MODULE), "the module turns off once the draft saves");
    assert!(h.state().session.registry.get("law.profiles.get").is_none(), "its commands go with it");
    // Read the saved profile from the store itself: LawCraft's commands are off now.
    let saved = serde_json::to_value(wordcraft_law::store().get(&id).expect("the profile").clone()).unwrap();
    assert_eq!(saved["docs"]["brief"]["sections"][0]["name"], json!("Cover Page"), "the retried draft persisted");
}
