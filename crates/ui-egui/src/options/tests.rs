use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use wordcraft_geom::Unit;

use super::{decimal, length, parse_length, unit};

use crate::theme::Appearance;
use crate::{Services, UiState, WordApp};

#[test]
fn unknown_settings_keys_load_in_linear_time() {
    let mut map = serde_json::Map::new();
    for i in 0..20_000 {
        map.insert(format!("unknownKey{i}"), json!(i));
    }
    map.insert("miniToolbar".into(), json!(false));
    let bytes = serde_json::to_vec(&serde_json::Value::Object(map)).unwrap();
    let state = UiState::from_json(&bytes).unwrap();
    assert!(!state.mini_toolbar);
}

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

fn nav_harness(language: &str, percent: u16) -> egui_kittest::Harness<'static, WordApp> {
    let mut a = app();
    a.run("ui.language", json!({"value": language})).unwrap();
    a.run("ui.interfaceSize", json!({"percent": percent})).unwrap();
    a.run("file.options", json!({"pane": "general"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h
}

/// File › Options must not offer a way around the protected document's author check.
#[test]
fn user_name_field_disables_when_protection_starts() {
    let mut h = nav_harness("en", 100);
    let user_name = || egui_kittest::kittest::By::new().role(egui::accesskit::Role::TextInput).value("WordCraft User");
    assert!(!h.get(user_name()).accesskit_node().is_disabled());
    h.state_mut().run("review.restrict", json!({"mode": "readOnly"})).unwrap();
    h.run_steps(4);
    assert!(h.get(user_name()).accesskit_node().is_disabled());
}

/// #1266: check the actual painted text and renderer clip, including the bold selected
/// label. AccessKit must still expose the full name and the row must keep its fixed size.
#[test]
fn options_nav_labels_stay_inside_their_rows() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    for language in ["en", "ru"] {
        for percent in [100, 125, 150] {
            let mut h = nav_harness(language, percent);
            // An interface size is absolute (#822): `percent` points per 100 screen pixels,
            // whatever the display's own scale (2.0 here).
            assert_eq!(h.ctx.pixels_per_point(), f32::from(percent) / 100.0);
            for selected in ["general", "accessibility"] {
                h.state_mut().run("file.options", json!({"pane": selected})).unwrap();
                h.run_steps(4);
                for id in wordcraft_engine::cmd::options::PANES {
                    let label = crate::i18n::tr(crate::i18n::Lang::from_pref(language), super::pane_label(id));
                    let node =
                        h.query_all_by_label(label).find(|n| n.accesskit_node().toggled().is_some()).expect("pane has its full accessible name");
                    assert_eq!(
                        node.accesskit_node().toggled(),
                        Some(if id == selected { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False })
                    );
                    let row = node.rect();
                    assert_eq!(row.size(), egui::vec2(170.0, 30.0));
                    let (clip, text) = h
                        .output()
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(text) if text.galley.text() == label && row.contains(text.pos) => Some((s.clip_rect, text)),
                            _ => None,
                        })
                        .expect("navigation label is painted");
                    let bounds = text.visual_bounding_rect();
                    assert!(row.contains_rect(bounds.intersect(clip)), "{language} {percent}% {selected}: {label} paints outside {row:?}");
                    if language == "en" && ["general", "display", "save"].contains(&id) {
                        assert!(clip.contains_rect(bounds), "short labels must remain fully visible");
                    }
                }
            }
        }
    }
}

/// Clipping only the painter must leave the entire row clickable, its hover/selection
/// backgrounds intact, and the selected pane's settings visible at every interface size.
#[test]
fn options_nav_rows_keep_hover_and_selection() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    for language in ["en", "ru"] {
        for percent in [100, 125, 150] {
            let mut h = nav_harness(language, percent);
            for (id, setting) in [("display", "Spaces"), ("save", "Save documents"), ("accessibility", "Visual effects"), ("general", "Interface")] {
                let lang = crate::i18n::Lang::from_pref(language);
                let row = h
                    .query_all_by_label(crate::i18n::tr(lang, super::pane_label(id)))
                    .find(|n| n.accesskit_node().toggled().is_some())
                    .expect("pane has its full accessible name")
                    .rect();
                // Both ends of the row, including its empty left inset, stay clickable.
                for x in [row.left() + 2.0, row.right() - 2.0] {
                    h.state_mut().run("file.options", json!({"pane": "agents"})).unwrap();
                    h.hover_at(egui::pos2(x, row.center().y));
                    h.run_steps(4);
                    let t = crate::theme::Tokens::get(&h.ctx);
                    let background = |h: &egui_kittest::Harness<'_, WordApp>, color| {
                        h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == row && r.fill == color))
                    };
                    assert!(background(&h, t.hover), "{language} {percent}%: hover covers the row");
                    for pressed in [true, false] {
                        h.input_mut().events.push(egui::Event::PointerButton {
                            pos: egui::pos2(x, row.center().y),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        });
                        h.run_steps(1);
                    }
                    h.run_steps(4);
                    assert_eq!(h.state().ui.options_pane, id);
                    assert!(background(&h, t.checked), "{language} {percent}%: selection covers the row");
                    assert!(h.query_by_label(crate::i18n::tr(lang, setting)).is_some(), "{id} settings are shown");
                }
            }
        }
    }
}

/// File › Options settings come back after a restart, and a `ui.json` with a wrong value in
/// one setting keeps every other setting (#485).
#[test]
fn options_survive_a_restart_and_junk_in_ui_json() {
    let mut a = app();
    a.run(
        "file.options",
        json!({"initials": "VK", "units": "centimeters", "typingReplacesSelection": false, "marks": {"tabs": true}, "ignoreNumbers": false, "addWords": ["zqxoptionsword"]}),
    )
    .unwrap();
    a.ui.recent_count = 30;
    a.ui.print_backgrounds = true;
    let saved = serde_json::to_vec(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(UiState::from_json(&saved).unwrap());
    assert_eq!(b.session.prefs, a.session.prefs);
    assert_eq!(b.session.prefs.units, Some(Unit::Centimeters));
    assert_eq!(b.session.initials, "VK");
    assert_eq!((b.ui.recent_count, b.ui.print_backgrounds), (30, true));
    assert!(wordcraft_engine::proof::user_dictionary().contains(&"zqxoptionsword".to_string()));

    let junk = br#"{"theme": "dark", "recentCount": "lots", "printBackgrounds": 7, "optionsPane": ["x"], "dark": true,
            "editing": {"measurementUnits": "furlongs", "typingReplacesSelection": false, "marks": {"tabs": "yes", "spaces": true}}}"#;
    let ui = UiState::from_json(junk).unwrap();
    assert_eq!(ui.theme, Appearance::Dark);
    assert_eq!((ui.recent_count, ui.print_backgrounds, ui.options_pane.as_str()), (12, false, "general"));
    assert_eq!(ui.legacy_dark, Some(true));
    assert_eq!(ui.editing.units, None);
    assert!(!ui.editing.typing_replaces_selection);
    assert!(ui.editing.marks.spaces && !ui.editing.marks.tabs);
    assert!(UiState::from_json(b"[1, 2]").is_none() && UiState::from_json(b"not json").is_none());
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
    b.apply_prefs(UiState::from_json(&saved).unwrap());
    assert_eq!(b.session.prefs.dictionaries_folder.as_deref(), Some(dir.to_string_lossy().as_ref()));
    assert_eq!(wordcraft_engine::proof::hunspell::search_dirs(None).first(), Some(&dir), "searched first");
}

/// #1792: Check spelling as you type is saved with the other proofing choices, so after a
/// restart the box comes back the way the user left it.
#[test]
fn check_spelling_survives_a_restart() {
    let mut a = app();
    a.run("file.options", json!({"checkSpelling": false})).unwrap();
    assert!(!a.session.view.proofing, "off right away");
    let saved = serde_json::to_value(a.prefs()).unwrap();
    assert_eq!(saved["editing"]["checkSpelling"], false, "saved in ui.json");
    let mut b = app();
    b.apply_prefs(UiState::from_json(&serde_json::to_vec(&saved).unwrap()).unwrap());
    assert!(!b.session.view.proofing, "spelling stays off after a restart");
}

/// Settings saved before #838 kept the unit under `editing.units` (default inches): a unit
/// the user chose carries over to `measurementUnits`; inches, junk or a newer value don't.
#[test]
fn a_unit_chosen_before_regions_survives_the_rename() {
    let editing = |json: &str| UiState::from_json(format!(r#"{{"editing": {json}}}"#).as_bytes()).unwrap().editing.units;
    assert_eq!(editing(r#"{"units": "centimeters"}"#), Some(Unit::Centimeters));
    assert_eq!(editing(r#"{"units": "picas", "overtype": true}"#), Some(Unit::Picas));
    assert_eq!(editing(r#"{"units": "inches"}"#), None);
    assert_eq!(editing(r#"{"units": "furlongs"}"#), None);
    assert_eq!(editing(r#"{"units": 7}"#), None);
    assert_eq!(editing(r#"{"units": "millimeters", "measurementUnits": "points"}"#), Some(Unit::Points));
    assert_eq!(editing(r#"{"units": "millimeters", "measurementUnits": null}"#), None);
    assert_eq!(editing("{}"), None);
    let mut a = app();
    a.apply_prefs(UiState::from_json(br#"{"editing": {"units": "millimeters"}}"#).unwrap());
    assert_eq!(a.session.units(), Unit::Millimeters);
    // Saved again, it's under the new key.
    let saved = serde_json::to_value(a.prefs()).unwrap();
    assert_eq!(saved["editing"]["measurementUnits"], "millimeters");
}

/// `file.options` opens the page at a pane; the pane list switches panes, and their
/// checkboxes change the settings (#485).
#[test]
fn options_panes_open_and_change_settings() {
    let mut a = app();
    a.run("file.options", json!({"pane": "display"})).unwrap();
    assert!(a.ui.backstage);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h.get_by_label("Spaces").click();
    h.run_steps(2);
    assert!(h.state().session.prefs.marks.spaces, "Display › Spaces");
    h.get_by_label("Proofing").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.options_pane, "proofing");
    h.get_by_label("Ignore words in capitals").click();
    h.run_steps(2);
    assert!(!h.state().session.prefs.ignore_uppercase);
    // The AutoCorrect Options dialog opens from this pane.
    assert!(h.state().dialog.is_none());
    h.get_by_label("AutoCorrect Options").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("autoCorrect"));
    h.state_mut().dialog = None;
    h.run_steps(2);
    // #1063: the dictionaries folder and the dictionary each language found.
    assert!(h.query_by_label("Dictionaries folder:").is_some());
    assert!(h.query_by_label_contains("Polish: ").is_some(), "the Polish dictionary's line");
    h.get_by_label("Advanced").click();
    h.run_steps(4);
    h.get_by_label("Typing replaces selected text").click();
    h.run_steps(2);
    assert!(!h.state().session.prefs.typing_replaces_selection);
    assert!(h.query_by_label("Measurement units:").is_some());
}

/// File › Options › Save › Privacy (#1062): the checkboxes are the document's
/// `file.privacyOptions` settings, and an undoable change like any other.
#[test]
fn save_privacy_checkboxes_set_the_privacy_options() {
    let mut a = app();
    a.run("file.options", json!({"pane": "save"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert!(h.query_by_label("Privacy for this document").is_some(), "the section is there");
    h.get_by_label("Remove my name from comments, changes and file properties when saving").click();
    h.run_steps(4);
    assert!(h.state().session.doc.settings.remove_personal_info);
    assert!(!h.state().session.doc.settings.remove_date_time);
    h.get_by_label("Leave out the date and time of comments and tracked changes when saving").click();
    h.run_steps(4);
    let privacy = h.state_mut().session.run("file.privacy", &json!({})).unwrap();
    assert_eq!(privacy, json!({"removePersonalInfo": true, "removeDateTime": true}));
    h.get_by_label("Remove my name from comments, changes and file properties when saving").click();
    h.run_steps(4);
    assert!(!h.state().session.doc.settings.remove_personal_info, "and off again");
}

/// File › Options › Accessibility › Animation effects: Use system setting (the default), On
/// or Off, with a line saying what the system's own setting is while it is followed.
#[test]
fn animation_effects_is_an_accessibility_option() {
    use crate::theme::Animations;
    let services = Services { animations_enabled: Some(Box::new(|| Some(false))), ..Default::default() };
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
    a.run("file.options", json!({"pane": "accessibility"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert_eq!(h.state().ui.animations, Animations::System);
    assert!(h.query_by_label("Animation effects:").is_some());
    assert!(h.query_by_label("Your system has animation effects turned off.").is_some());
    assert!(!h.state().animations_on());
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Animation effects:").click();
    h.run_steps(4);
    h.get_by_label("On").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.animations, Animations::On);
    assert!(h.state().animations_on());
    assert!(h.query_by_label("Your system has animation effects turned off.").is_none(), "only said while the system is followed");
}

/// File › Options › General › Start-up (#766): the checkbox is the `ui.startScreen` setting.
#[test]
fn general_start_up_turns_the_start_screen_off_and_on() {
    let mut a = app();
    a.run("file.options", json!({"pane": "general"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert!(h.query_by_label("Start-up").is_some(), "the section is there");
    h.get_by_label("Show the Start screen when WordCraft starts").click();
    h.run_steps(4);
    assert!(!h.state().ui.start_screen, "the checkbox turns it off");
    h.get_by_label("Show the Start screen when WordCraft starts").click();
    h.run_steps(4);
    assert!(h.state().ui.start_screen, "and on again");
}

/// File › Options › General › Never change the document page color (#1722): the checkbox
/// is the `ui.neverChangePage` setting and decides whether the page follows the theme.
#[test]
fn general_never_change_page_checkbox_pins_the_page() {
    let mut a = app();
    a.run("ui.theme", json!({"value": "dark"})).unwrap();
    a.run("file.options", json!({"pane": "general"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let label = "Never change the document page color";
    assert!(h.state().page_is_dark(), "off: the dark theme gives a dark page");
    h.get_by_label(label).click();
    h.run_steps(4);
    assert_eq!(h.state().ui.never_change_page, Some(true));
    assert!(!h.state().page_is_dark(), "on: the document's own light page");
    h.get_by_label(label).click();
    h.run_steps(4);
    assert_eq!(h.state().ui.never_change_page, Some(false));
    assert!(h.state().page_is_dark(), "off again: the page follows the theme");
}

/// File › Options › Advanced › Editing: the alignment guides checkbox is the
/// `ui.alignmentGuides` setting.
#[test]
fn advanced_alignment_guides_checkbox_turns_snapping_off_and_on() {
    let mut a = app();
    a.run("file.options", json!({"pane": "advanced"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    let label = "Show alignment guides when moving pictures and shapes";
    h.get_by_label(label).click();
    h.run_steps(4);
    assert!(!h.state().ui.alignment_guides, "the checkbox turns them off");
    h.get_by_label(label).click();
    h.run_steps(4);
    assert!(h.state().ui.alignment_guides, "and on again");
}

/// File › Options › Advanced › Animation: the checkbox is the `ui.caretAnimation` setting,
/// and the section is translated.
#[test]
fn advanced_animation_turns_the_caret_animation_off_and_on() {
    let mut a = app();
    a.run("file.options", json!({"pane": "advanced"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    assert!(h.query_by_label("Animation").is_some(), "the section is there");
    h.get_by_label("Animate the caret (it glides and fades)").click();
    h.run_steps(4);
    assert!(!h.state().ui.caret_animation, "the checkbox turns it off");
    h.get_by_label("Animate the caret (it glides and fades)").click();
    h.run_steps(4);
    assert!(h.state().ui.caret_animation, "and on again");
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    h.state_mut().run("ui.language", json!({"value": "es"})).unwrap();
    h.run_steps(4);
    assert!(h.query_by_label("Animación").is_some() && h.query_by_label("Animation").is_none());
}

/// #838: in a Polish region the Page Setup margins read `2,54 cm` (the region's unit and
/// decimal comma), a typed `3,5` or `1 in` is understood, and the Options page shows the
/// region's unit until the user picks another.
#[test]
fn length_fields_follow_the_region() {
    let mut a = app();
    a.run("file.options", json!({"region": "pl-PL"})).unwrap();
    a.run("file.new", json!({})).unwrap();
    assert_eq!(wordcraft_geom::paper_name(a.session.doc.last_section.page_w, a.session.doc.last_section.page_h), Some("A4"));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(2);
    // The dialog reads the unit when it opens, after a frame has picked up the region.
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(4);
    assert_eq!((unit(), decimal()), (Unit::Centimeters, ','));
    assert_eq!(length(72.0), "2,54 cm");
    assert_eq!(h.query_all_by_value("2,54 cm").count(), 4, "four margins in centimetres with a decimal comma");

    h.state_mut().run("file.options", json!({"units": "inches"})).unwrap();
    h.run_steps(4);
    assert_eq!(length(108.0), "1,5\"", "the user's unit, still the region's decimal comma");
    h.state_mut().run("file.options", json!({"units": "auto", "region": "en-US"})).unwrap();
    h.run_steps(4);
    assert_eq!(length(72.0), "1\"");
}

/// #1734: a Russian interface shows centimetres as `см`, as Word does, and a length field
/// takes the unit typed either way.
#[test]
fn length_units_follow_the_interface_language() {
    if !crate::i18n::catalogs_embedded() {
        return;
    }
    let mut a = app();
    a.run("ui.language", json!({"value": "ru"})).unwrap();
    a.run("file.options", json!({"region": "ru-RU"})).unwrap();
    a.run("file.new", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(2);
    h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
    h.run_steps(4);
    assert_eq!((unit(), decimal()), (Unit::Centimeters, ','));
    assert_eq!(length(72.0), "2,54 см");
    assert_eq!(h.query_all_by_value("2,54 см").count(), 4, "four margins in centimetres, named in Russian");
    assert_eq!(h.query_all_by_value("2,54 cm").count(), 0);
    assert_eq!(parse_length("3 см", Unit::Centimeters), Some(3.0));
    assert_eq!(parse_length("3cm", Unit::Centimeters), Some(3.0));
    assert_eq!(parse_length("10 мм", Unit::Centimeters).map(|v| (v * 100.0).round() / 100.0), Some(1.0));
    assert_eq!(parse_length("72 пт", Unit::Centimeters).map(|v| (v * 100.0).round() / 100.0), Some(2.54));

    h.state_mut().run("file.options", json!({"units": "millimeters"})).unwrap();
    h.run_steps(4);
    assert_eq!(length(72.0), "25,4 мм");
    h.state_mut().run("ui.language", json!({"value": "en"})).unwrap();
    h.run_steps(4);
    assert_eq!(length(72.0), "25,4 mm", "English keeps the English names");
}
