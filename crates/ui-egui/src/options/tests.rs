use serde_json::json;
use wordcraft_geom::Unit;

use craft::ui::patterns::preferences::{PrefValue, PreferencesModel};

use super::{OptionsModel, length, unit_suffix};
use crate::prefs::{Prefs, legacy_ui_json};
use crate::{Services, StatusCount, WordApp};

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

/// Runs `f` over a model of `app` inside one frame of a headless harness.
fn with_model<R: 'static>(app: &mut WordApp, motion_full: bool, f: impl FnOnce(&mut OptionsModel<'_>) -> R + 'static) -> R {
    struct State<R> {
        app: WordApp,
        job: Option<Box<dyn FnOnce(&mut WordApp, bool) -> R>>,
        out: Option<R>,
    }
    let job: Box<dyn FnOnce(&mut WordApp, bool) -> R> = Box::new(move |app, full| f(&mut OptionsModel::new(app, 1.0, full)));
    let state = State { app: std::mem::replace(app, self::app()), job: Some(job), out: None };
    let mut h = crate::test_harness::builder().build_ui_state(
        move |_ui, st: &mut State<R>| {
            if let Some(job) = st.job.take() {
                st.out = Some(job(&mut st.app, motion_full));
            }
        },
        state,
    );
    h.run_steps(1);
    let st = h.state_mut();
    *app = std::mem::replace(&mut st.app, self::app());
    st.out.take().expect("the frame ran")
}

fn row_enabled(model: &OptionsModel<'_>, key: &str) -> bool {
    model.pages().iter().flat_map(|p| p.groups.iter()).flat_map(|g| g.rows.iter()).find(|r| r.key == key).map(|r| r.enabled).expect("row exists")
}

/// The app showing File › Options on the Accessibility page.
/// The runtime is installed over `system`, the reader it resolves the system's motion from.
fn options_harness(mut a: WordApp, system: craft::platform::appearance::SystemReader) -> egui_kittest::Harness<'static, WordApp> {
    a.run("ui.backstage", json!({"value": true, "page": "options", "pane": "accessibility"})).unwrap();
    let mut h = crate::test_harness::builder_with(move || crate::test_harness::install_spec().system(system)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    options_frames(&mut h, 3);
    h
}

/// `n` frames, each given the input the app hands the runtime.
fn options_frames(h: &mut egui_kittest::Harness<'static, WordApp>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
        h.step();
    }
}

/// A reader of the system that answers `reduced` to "reduce motion".
fn system_reducing_motion(reduced: bool) -> craft::platform::appearance::SystemReader {
    let reader = craft::platform::appearance::SystemReader::unwatched();
    reader.report_reduced_motion(reduced);
    reader
}

/// The help line the Animation effects row shows while the system's setting is followed.
fn system_help(h: &egui_kittest::Harness<'static, WordApp>) -> Option<String> {
    use egui_kittest::kittest::{NodeT, Queryable};
    h.query_by_label_contains("Your system has animation effects").and_then(|n| n.accesskit_node().label()).map(|s| s.to_string())
}

/// File › Options must not offer a way around the protected document's author check.
#[test]
fn user_name_field_disables_when_protection_starts() {
    let mut a = app();
    assert!(with_model(&mut a, true, |m| row_enabled(m, "general.userName")));
    a.run("review.restrict", json!({"mode": "readOnly"})).unwrap();
    assert!(!with_model(&mut a, true, |m| row_enabled(m, "general.userName")));
}

/// The dictionaries folder (#1063) is saved with the settings and searched again after a
/// restart, before any document is checked.
#[test]
fn the_dictionaries_folder_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("wc-ui-dictionaries-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app();
    a.run("file.options", json!({"dictionariesFolder": dir.to_string_lossy()})).unwrap();
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    wordcraft_engine::proof::hunspell::set_dictionary_dir(None);
    let mut b = app();
    b.apply_prefs(serde_json::from_slice::<Prefs>(&saved).unwrap());
    assert_eq!(b.session.prefs.dictionaries_folder.as_deref(), Some(dir.to_string_lossy().as_ref()));
    assert_eq!(wordcraft_engine::proof::hunspell::search_dirs(None).first(), Some(&dir), "searched first");
}

/// #1792: Check spelling as you type is saved with the other proofing choices.
#[test]
fn check_spelling_survives_a_restart() {
    let mut a = app();
    a.run("file.options", json!({"checkSpelling": false})).unwrap();
    assert!(!a.session.view.proofing, "off right away");
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_slice::<Prefs>(&saved).unwrap());
    assert!(!b.session.view.proofing, "spelling stays off after a restart");
}

/// Settings saved before #838 kept the unit under `editing.units` (default inches): a unit
/// the user chose carries over to `measurementUnits`; inches, junk or a newer value don't.
#[test]
fn a_unit_chosen_before_regions_survives_the_rename() {
    let units = |json: &str| {
        let mut a = app();
        let (prefs, _) = legacy_ui_json(format!(r#"{{"editing": {json}}}"#).as_bytes()).unwrap();
        a.apply_prefs(prefs);
        a.session.prefs.units
    };
    assert_eq!(units(r#"{"units": "centimeters"}"#), Some(Unit::Centimeters));
    assert_eq!(units(r#"{"units": "picas", "overtype": true}"#), Some(Unit::Picas));
    assert_eq!(units(r#"{"units": "inches"}"#), None);
    assert_eq!(units(r#"{"units": "furlongs"}"#), None);
    assert_eq!(units(r#"{"units": "millimeters", "measurementUnits": "points"}"#), Some(Unit::Points));
    assert_eq!(units(r#"{"units": "millimeters", "measurementUnits": null}"#), None);
    assert_eq!(units("{}"), None);
    let mut a = app();
    a.apply_prefs(legacy_ui_json(br#"{"editing": {"units": "millimeters"}}"#).unwrap().0);
    assert_eq!(a.session.units(), Unit::Millimeters);
}

/// One table over the Bool rows: `set` runs the row's command or writes its setting, and the
/// effect follows, on and off.
#[test]
fn options_panes_open_and_change_settings() {
    type Read = fn(&WordApp) -> bool;
    let table: [(&str, Read); 14] = [
        ("display.spaces", |a| a.session.prefs.marks.spaces),
        ("display.tabs", |a| a.session.prefs.marks.tabs),
        ("display.allMarks", |a| a.session.view.marks),
        ("display.rulers", |a| a.session.view.ruler),
        ("display.printBackgrounds", |a| a.ui.print_backgrounds),
        ("general.startScreen", |a| a.ui.start_screen),
        ("general.miniToolbar", |a| a.ui.mini_toolbar),
        ("general.darkPage", |a| a.session.view.dark_mode),
        ("proofing.ignoreUppercase", |a| a.session.prefs.ignore_uppercase),
        ("advanced.typingReplacesSelection", |a| a.session.prefs.typing_replaces_selection),
        ("advanced.alignmentGuides", |a| a.ui.alignment_guides),
        ("advanced.caretAnimation", |a| a.ui.caret_animation),
        ("accessibility.skipCitations", |a| a.session.read_aloud.skip_citations),
        ("advanced.keytips", |a| a.ui.keytips_enabled),
    ];
    for (key, read) in table {
        for on in [true, false] {
            let mut a = app();
            with_model(&mut a, true, move |m| m.set(key, PrefValue::Bool(on)));
            assert_eq!(read(&a), on, "{key} set to {on}");
        }
    }
}

/// One table over the Choice rows and the document's privacy options.
#[test]
fn save_privacy_checkboxes_set_the_privacy_options() {
    let mut a = app();
    with_model(&mut a, true, |m| m.set("save.removePersonalInfo", PrefValue::Bool(true)));
    assert!(a.session.doc.settings.remove_personal_info);
    assert!(!a.session.doc.settings.remove_date_time);
    with_model(&mut a, true, |m| m.set("save.removeDateTime", PrefValue::Bool(true)));
    let privacy = a.session.run("file.privacy", &json!({})).unwrap();
    assert_eq!(privacy, json!({"removePersonalInfo": true, "removeDateTime": true}));
    with_model(&mut a, true, |m| m.set("save.removePersonalInfo", PrefValue::Bool(false)));
    assert!(!a.session.doc.settings.remove_personal_info, "and off again");

    with_model(&mut a, true, |m| m.set("display.statusCount", PrefValue::Choice("characters".into())));
    assert_eq!(a.ui.status_count, StatusCount::Characters);
    with_model(&mut a, true, |m| m.set("advanced.units", PrefValue::Choice("millimeters".into())));
    assert_eq!(a.session.units(), Unit::Millimeters);
    with_model(&mut a, true, |m| m.set("advanced.units", PrefValue::Choice("auto".into())));
    assert_eq!(a.session.prefs.units, None);
    with_model(&mut a, true, |m| m.set("advanced.region", PrefValue::Choice("pl-PL".into())));
    assert_eq!(a.session.prefs.region.as_deref(), Some("pl-PL"));
}

/// File › Options › Accessibility › Animation effects: the help line follows the runtime's resolved
/// motion while System is chosen, and an explicit choice overrides it.
#[test]
fn animation_effects_is_an_accessibility_option() {
    use crate::theme::Animations;
    // The runtime holds its reader, so each answer of the system is its own install.
    let reduced = options_harness(app(), system_reducing_motion(true));
    assert_eq!(reduced.state().ui.animations, Animations::System);
    let full = options_harness(app(), system_reducing_motion(false));
    let (reduced_help, full_help) = (system_help(&reduced), system_help(&full));
    assert!(reduced_help.is_some() && full_help.is_some(), "the page says what the system answered while it is followed");
    assert_ne!(reduced_help, full_help, "the line follows the system's answer");
    let mut h = reduced;
    h.state_mut().run("ui.animations", json!({"value": "on"})).unwrap();
    assert_eq!(h.state().ui.animations, Animations::On);
    options_frames(&mut h, 3);
    assert_eq!(system_help(&h), None, "only said while the system is followed");
}

/// #838: in a Polish region lengths read `2,54 cm` (the region's unit and decimal comma).
#[test]
fn length_fields_follow_the_region() {
    let mut a = app();
    a.run("file.options", json!({"region": "pl-PL"})).unwrap();
    assert_eq!((a.session.units(), a.session.region().decimal()), (Unit::Centimeters, ','));
    let fmt = craft::i18n::Format::from_tag("pl-PL");
    assert_eq!(length(72.0, Unit::Centimeters, &fmt), "2,54 cm");
    assert_eq!(length(108.0, Unit::Inches, &fmt), "1,5\"", "the user's unit, still the region's decimal comma");
    let us = craft::i18n::Format::from_tag("en-US");
    assert_eq!(length(72.0, Unit::Inches, &us), "1\"");
}

/// #1734: a Russian interface shows centimetres as `см`, as Word does.
#[test]
fn length_units_follow_the_interface_language() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    let mut a = app();
    a.run("ui.language", json!({"value": "ru"})).unwrap();
    let mut h = crate::test_harness::builder().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    // The frames bind the interface language through the input the app hands the runtime.
    crate::test_harness::frames(&mut h, 2);
    assert_eq!(unit_suffix(Unit::Centimeters), " см");
    assert_eq!(length(72.0, Unit::Centimeters, &craft::i18n::Format::from_tag("ru-RU")), "2,54 см");
    h.state_mut().run("ui.language", json!({"value": "en"})).unwrap();
    crate::test_harness::frames(&mut h, 2);
    assert_eq!(unit_suffix(Unit::Millimeters), " mm", "English keeps the English names");
}

#[test]
fn screen_paper_color_options_round_trip_and_work_without_a_document() {
    use wordcraft_engine::cmd::view::PaperColor;
    let mut a = app();
    a.start_screen();
    a.run("view.paperColor", json!({"value":"green"})).unwrap();
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_slice::<Prefs>(&saved).unwrap());
    assert_eq!(b.session.prefs.paper_color, PaperColor::Green);
    with_model(&mut b, true, |m| m.set("general.paperColor", PrefValue::Choice("cream".into())));
    assert_eq!(b.session.prefs.paper_color, PaperColor::Cream);
}

/// #5840: File › Options › Proofing has the Dictionary file setting.
#[test]
fn proofing_custom_dictionary_file_setting_is_visible() {
    let mut a = app();
    let found = with_model(&mut a, true, |m| {
        m.pages().iter().flat_map(|p| p.groups.iter()).flat_map(|g| g.rows.iter()).any(|r| r.key == "proofing.customDictionaryFile")
    });
    assert!(found);
}

/// #5840: File › Options › Proofing sets, clears and validates the custom dictionary path.
#[test]
fn proofing_custom_dictionary_interaction() {
    let dir = std::env::temp_dir().join(format!("wc-ui-dict-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let valid_path = dir.join("ui_custom.dic");
    std::fs::write(&valid_path, "uiword\n").unwrap();
    let valid_path_str = valid_path.to_string_lossy().into_owned();
    let mut a = app();

    // An invalid path is refused and not applied to the preferences.
    with_model(&mut a, true, |m| m.set("proofing.customDictionaryFile", PrefValue::Text("/invalid/path/nonexistent.dic".into())));
    assert_eq!(a.session.prefs.custom_dictionary, None, "invalid path was not applied to prefs");

    let path = valid_path_str.clone();
    with_model(&mut a, true, move |m| m.set("proofing.customDictionaryFile", PrefValue::Text(path)));
    assert_eq!(a.session.prefs.custom_dictionary.as_deref(), Some(&valid_path_str[..]));

    // Empty reverts to the default dictionary.
    with_model(&mut a, true, |m| m.set("proofing.customDictionaryFile", PrefValue::Text(String::new())));
    assert_eq!(a.session.prefs.custom_dictionary, None, "empty reverts custom dictionary to default");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #5840: a bound custom dictionary file is the source of truth. The saved word copy must not
/// be restored first, or words from a missing file would reach the default dictionary on the next
/// Add to Dictionary.
#[test]
fn a_bound_custom_dictionary_does_not_restore_saved_backup_words() {
    let saved = Prefs {
        dictionary: vec!["zqxbackupword".to_string()],
        editing: wordcraft_engine::Prefs { custom_dictionary: Some("/nonexistent/custom.dic".to_string()), ..Default::default() },
        ..Default::default()
    };

    let mut a = app();
    a.apply_prefs(saved);
    assert!(!wordcraft_engine::proof::user_dictionary().contains(&"zqxbackupword".to_string()), "backup words stay out of memory");
    assert_eq!(a.session.prefs.custom_dictionary.as_deref(), Some("/nonexistent/custom.dic"), "the file setting is kept");
}

/// #5840: the Dictionary file field commits the path on Enter, not on every keystroke. Typing
/// `notes.dic` passes through `notes`, a file that exists, which must not be bound on the way.
#[test]
fn typing_the_custom_dictionary_path_binds_it_only_on_enter() {
    use egui_kittest::kittest::Queryable;
    let dir = std::env::temp_dir().join(format!("wc-ui-dict-typing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes"), "partway\n").unwrap();
    let target = dir.join("notes.dic");
    std::fs::write(&target, "uiword\n").unwrap();
    let target_str = target.to_string_lossy().into_owned();

    let mut a = app();
    a.run("ui.backstage", json!({"value": true, "page": "options", "pane": "proofing"})).unwrap();
    let mut h = crate::test_harness::builder().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    options_frames(&mut h, 3);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Dictionary file:").focus();
    options_frames(&mut h, 2);
    for c in target_str.chars() {
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Dictionary file:").type_text(&c.to_string());
        options_frames(&mut h, 2);
        assert_eq!(h.state().session.prefs.custom_dictionary, None, "typing {c:?} bound a path");
    }
    h.key_press(egui::Key::Enter);
    options_frames(&mut h, 3);
    assert_eq!(h.state().session.prefs.custom_dictionary.as_deref(), Some(target_str.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A harness over the real Options page, Proofing pane, with two dictionary files on disk.
fn dictionary_harness(tag: &str) -> (egui_kittest::Harness<'static, WordApp>, std::path::PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("wc-ui-dict-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let one = dir.join("one.dic");
    let two = dir.join("two.dic");
    std::fs::write(&one, "oneword\n").unwrap();
    std::fs::write(&two, "twoword\n").unwrap();
    let mut a = app();
    a.run("ui.backstage", json!({"value": true, "page": "options", "pane": "proofing"})).unwrap();
    let mut h = crate::test_harness::builder().build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    options_frames(&mut h, 3);
    (h, dir, one.to_string_lossy().into_owned(), two.to_string_lossy().into_owned())
}

fn dictionary_field_text(h: &egui_kittest::Harness<'static, WordApp>) -> String {
    use egui_kittest::kittest::Queryable;
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Dictionary file:").value().unwrap_or_default()
}

fn type_into_dictionary_field(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
    use egui_kittest::kittest::Queryable;
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Dictionary file:").focus();
    options_frames(h, 2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Dictionary file:").type_text(text);
    options_frames(h, 2);
}

/// #5899: a path typed but not committed yields to a saved path an agent sets meanwhile: the field
/// shows the agent's value (the framework ends the edit when the value changes outside it), and a
/// later Enter does not write the abandoned draft over it.
#[test]
fn an_outside_change_replaces_a_path_being_typed() {
    let (mut h, dir, one, two) = dictionary_harness("outside");
    type_into_dictionary_field(&mut h, &one);
    assert_eq!(h.state().session.prefs.custom_dictionary, None, "typing binds nothing");
    h.state_mut().run("file.options", json!({"customDictionary": two})).unwrap();
    options_frames(&mut h, 3);
    assert_eq!(h.state().session.prefs.custom_dictionary.as_deref(), Some(two.as_str()));
    assert_eq!(dictionary_field_text(&h), two, "the field shows the agent's path");
    h.key_press(egui::Key::Enter);
    options_frames(&mut h, 3);
    assert_eq!(h.state().session.prefs.custom_dictionary.as_deref(), Some(two.as_str()), "Enter did not overwrite it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #5899: an uncommitted draft does not outlive its pane: back on Proofing the field shows the saved path.
#[test]
fn leaving_the_pane_drops_an_uncommitted_path() {
    let (mut h, dir, one, two) = dictionary_harness("leave");
    h.state_mut().run("file.options", json!({"customDictionary": two})).unwrap();
    options_frames(&mut h, 3);
    type_into_dictionary_field(&mut h, &one);
    h.state_mut().run("ui.backstage", json!({"value": false})).unwrap();
    options_frames(&mut h, 3);
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "options", "pane": "proofing"})).unwrap();
    options_frames(&mut h, 3);
    assert_eq!(dictionary_field_text(&h), two, "the field shows the saved path");
    assert_eq!(h.state().session.prefs.custom_dictionary.as_deref(), Some(two.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #5899: focus leaving the field commits the typed path.
#[test]
fn focus_loss_commits_the_custom_dictionary_path() {
    use egui_kittest::kittest::Queryable;
    let (mut h, dir, one, _two) = dictionary_harness("blur");
    type_into_dictionary_field(&mut h, &one);
    assert_eq!(h.state().session.prefs.custom_dictionary, None);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "New word").focus();
    options_frames(&mut h, 3);
    assert_eq!(h.state().session.prefs.custom_dictionary.as_deref(), Some(one.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #5899: Escape abandons the typed path: nothing binds, and the field shows the saved path again.
#[test]
fn escape_does_not_commit_the_custom_dictionary_path() {
    let (mut h, dir, one, _two) = dictionary_harness("escape");
    type_into_dictionary_field(&mut h, &one);
    h.key_press(egui::Key::Escape);
    options_frames(&mut h, 3);
    assert_eq!(h.state().session.prefs.custom_dictionary, None);
    assert_eq!(dictionary_field_text(&h), "", "the field shows the saved (empty) path");
    let _ = std::fs::remove_dir_all(&dir);
}
