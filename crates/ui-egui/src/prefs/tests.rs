use crate::prefs::{Prefs, import_legacy_window, legacy_ui_json};
use crate::{MAX_FONT_NAME_BYTES, MAX_RECENT_FONTS, Services, StatusCount, WordApp, push_recent_font};
use serde_json::json;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

fn store_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wc-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn open(dir: &std::path::Path) -> craft::settings::SharedSettings<Prefs> {
    let (store, _) = craft::settings::SettingsStore::<Prefs>::open_at(dir);
    craft::settings::SharedSettings::new(store)
}

/// An old `ui.json` is imported once: good values are kept, junk reads as the default and is
/// reported with its path, unknown keys are dropped, and old shapes migrate.
#[test]
fn a_legacy_ui_json_is_imported_with_its_junk_reset_to_defaults() {
    let junk = br#"{"theme": "dark", "recentCount": "lots", "printBackgrounds": 7, "optionsPane": ["x"], "dark": true,
            "statusCount": "paragraphs", "recentFonts": ["Arial", 3, null, "Georgia"],
            "editing": {"measurementUnits": "furlongs", "typingReplacesSelection": false, "marks": {"tabs": "yes", "spaces": true}}}"#;
    let (ui, errors) = legacy_ui_json(junk).unwrap();
    assert_eq!(ui.theme, crate::theme::Appearance::Dark);
    assert_eq!((ui.recent_count, ui.print_backgrounds, ui.options_pane.as_str()), (12, false, "general"));
    assert_eq!(ui.status_count, StatusCount::Words);
    assert_eq!(ui.legacy_dark, Some(true));
    assert_eq!(ui.editing.units, None);
    assert!(!ui.editing.typing_replaces_selection);
    assert!(ui.editing.marks.spaces && !ui.editing.marks.tabs);
    assert_eq!(ui.font_picker.recent, ["Arial", "Georgia"], "the old recent fonts join the picker's list");
    let sections: Vec<&str> = errors.iter().map(|e| e.section.as_str()).collect();
    // `statusCount` is not among them: `StatusCount` reads any unknown code as Words (#2797), so
    // that value is never a read error (its default is asserted above).
    for wrong in ["recentCount", "printBackgrounds", "optionsPane"] {
        assert!(sections.contains(&wrong), "{wrong} is reported: {sections:?}");
    }
    assert!(legacy_ui_json(b"[1, 2]").is_err() && legacy_ui_json(b"not json").is_err());

    // Unknown keys are dropped, and read in linear time.
    let mut map = serde_json::Map::new();
    for i in 0..20_000 {
        map.insert(format!("unknownKey{i}"), json!(i));
    }
    map.insert("miniToolbar".into(), json!(false));
    let (ui, errors) = legacy_ui_json(&serde_json::to_vec(&serde_json::Value::Object(map)).unwrap()).unwrap();
    assert!(!ui.mini_toolbar && errors.is_empty());

    // The unit saved before #838 under `editing.units` carries over unless it is inches.
    let unit = |text: &str| legacy_ui_json(format!(r#"{{"editing": {text}}}"#).as_bytes()).unwrap().0.editing.units;
    assert_eq!(unit(r#"{"units": "centimeters"}"#), Some(wordcraft_geom::Unit::Centimeters));
    assert_eq!(unit(r#"{"units": "inches"}"#), None);
    assert_eq!(unit(r#"{"units": "centimeters", "measurementUnits": null}"#), None);

    // A saved size reads, junk reads as Auto, out-of-range clamps; the old size key is kept for the migration.
    for (raw, want) in [("1000", Some(300)), ("10", Some(50)), (r#""big""#, None), ("null", None)] {
        let (ui, _) = legacy_ui_json(format!(r#"{{"uiScale": {raw}, "tab": "Insert"}}"#).as_bytes()).unwrap();
        assert_eq!((ui.interface_size.percent(), ui.tab.as_str()), (want, "Insert"), "{raw}");
    }
    let (ui, _) = legacy_ui_json(br#"{"interfaceSize": 125}"#).unwrap();
    assert!(ui.legacy_interface_size.is_some());

    // The old shortcut map imports through the registry; junk is reported.
    let (ui, errors) = legacy_ui_json(br#"{"keyboard": {"assigned": {}, "removed": []}}"#).unwrap();
    assert!(ui.keyboard.0.is_empty() && errors.is_empty());
    let (_, errors) = legacy_ui_json(br#"{"keyboard": 7}"#).unwrap();
    assert!(errors.iter().any(|e| e.section == "keyboard"));
}

/// The old `window` object reaches the window store with its size and position; junk is dropped.
#[test]
fn the_old_window_entry_reaches_the_restore_store() {
    use craft::window::restore::{MAIN, Store};
    let mut store = Store::in_memory();
    import_legacy_window(br#"{"window": {"position": [100, 50], "width": 900, "height": 700, "maximized": true}}"#, &mut store);
    let placement = store.get(MAIN).expect("the main entry is recorded");
    assert_eq!(placement.normal.size, [900.0, 700.0]);
    assert_eq!(placement.position_px, Some([100, 50]));
    assert!(placement.maximized);
    for junk in [r#"{"window": {"width": "wide"}}"#, r#"{"window": {"width": 1e12, "height": 5}}"#, r#"{"window": 5}"#, "not json"] {
        let mut store = Store::in_memory();
        import_legacy_window(junk.as_bytes(), &mut store);
        assert!(store.get(MAIN).is_none(), "{junk}");
    }
}

/// A setting changed by command is saved through the store and read back by the next run; a
/// rejected value changes nothing.
#[test]
fn a_setting_changed_by_command_survives_a_restart() {
    type Check = fn(&WordApp) -> bool;
    let rows: [(&str, serde_json::Value, Check); 4] = [
        ("ui.alignmentGuides", json!({"value": false}), |a| !a.ui.alignment_guides),
        ("ui.interfaceSize", json!({"percent": 150}), |a| a.ui.interface_size.percent() == Some(150)),
        ("ui.theme", json!({"value": "Dark"}), |a| a.ui.theme == crate::theme::Appearance::Dark),
        ("file.options", json!({"initials": "VK"}), |a| a.session.initials == "VK"),
    ];
    for (n, (command, params, check)) in rows.into_iter().enumerate() {
        let dir = store_dir(&format!("row{n}"));
        let mut a = app().with_settings(open(&dir));
        a.run(command, params).unwrap();
        assert!(matches!(a.flush_prefs(), Some(Ok(()))), "{command} saves");
        let b = app().with_settings(open(&dir));
        assert!(check(&b), "{command} survives a restart");
        let _ = std::fs::remove_dir_all(&dir);
    }
    let mut a = app();
    for junk in [json!({"value": "no"}), json!({"value": 0}), json!({"value": null})] {
        assert!(a.run("ui.alignmentGuides", junk).is_err());
    }
    assert!(a.ui.alignment_guides, "a rejected value keeps the setting");
}

/// #4368: a saved empty user name stays empty; one of only spaces keeps the default.
#[test]
fn a_blank_user_name_survives_a_restart() {
    let mut first = app();
    first.session.author.clear();
    let saved = serde_json::to_vec(&first.prefs()).unwrap();
    let mut second = app();
    assert!(!second.session.author.is_empty());
    second.apply_prefs(serde_json::from_slice(&saved).unwrap());
    assert_eq!(second.session.author, "");
    let default = app().session.author;
    let mut third = app();
    third.apply_prefs(serde_json::from_str(r#"{"author": "   "}"#).unwrap());
    assert_eq!(third.session.author, default);
}

/// #2068: the recent fonts keep their limits on push and on load.
#[test]
fn recently_used_fonts_refuse_overlong_names_and_huge_lists() {
    let long = "N".repeat(MAX_FONT_NAME_BYTES + 1);
    let mut list = vec!["Arial".to_string()];
    push_recent_font(&mut list, &long);
    push_recent_font(&mut list, &"M".repeat(MAX_FONT_NAME_BYTES));
    assert_eq!(list.len(), 2, "an overlong name is refused, the longest allowed is kept");
    let mut saved: Vec<String> = vec![long.clone(), "x".repeat(4 << 20)];
    saved.extend((0..200_000).map(|i| format!("Family {i}")));
    let raw = json!({ "recentFonts": saved }).to_string();
    let (ui, _) = legacy_ui_json(raw.as_bytes()).unwrap();
    assert_eq!(ui.font_picker.recent.len(), MAX_RECENT_FONTS);
    assert_eq!(ui.font_picker.recent.first().map(String::as_str), Some("Family 0"));
    let mut b = app();
    let mut prefs = Prefs::default();
    prefs.font_picker.recent = vec![long, " Arial ".into(), "ARIAL".into()];
    b.apply_prefs(prefs);
    assert_eq!(b.ui.font_picker.recent, ["Arial"]);
}

/// #115: the old dark-mode switch migrates to Light/Dark when applied.
#[test]
fn interface_theme_migrates_and_survives_a_restart() {
    use crate::theme::Appearance;
    for (saved, want) in [(r#"{"dark": false}"#, Appearance::Light), (r#"{"dark": true}"#, Appearance::Dark), ("{}", Appearance::System)] {
        let mut a = app();
        a.apply_prefs(legacy_ui_json(saved.as_bytes()).unwrap().0);
        assert_eq!(a.ui.theme, want, "{saved}");
    }
    let mut a = app();
    a.run("ui.theme", json!({"value": "System"})).unwrap();
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""theme":"system""#) && !saved.contains(r#""dark":"#), "{saved}");
}

/// #822: a `ui.json` from before saved `interfaceSize` on top of the display scale; it becomes
/// absolute once the first frame says the display's scale.
#[test]
fn interface_size_from_before_822_migrates() {
    // The migration runs in `frame_input` (what eframe's `raw_input_hook` asks for before each
    // frame), which reads the display's scale the previous pass reported: two frames.
    let frame = |a: &mut WordApp, native: f32| {
        let ctx = crate::test_harness::context();
        a.ctx = Some(ctx.clone());
        for _ in 0..2 {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            craft::ui::shell::embed::before_input(&ctx, |out| a.frame_input(out));
            ctx.run_ui(input, |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
        }
    };
    for (json, native, want) in [
        (r#"{"interfaceSize": 100}"#, 2.0, None),
        (r#"{"interfaceSize": 125}"#, 2.0, Some(250)),
        (r#"{"interfaceSize": 80}"#, 0.5, Some(50)),
        (r#"{"interfaceSize": 150, "uiScale": 90}"#, 2.0, Some(90)),
    ] {
        let mut a = app();
        a.apply_prefs(legacy_ui_json(json.as_bytes()).unwrap().0);
        frame(&mut a, native);
        assert_eq!(a.ui.interface_size.percent(), want, "{json} on a {native}× display");
        assert!(!serde_json::to_string(&a.prefs()).unwrap().contains("interfaceSize"), "{json}");
    }
}

/// #2716: taking documents off the recent list is final and goes to the store at once.
#[test]
fn recent_list_edits_are_final_and_persist() {
    let dir = store_dir("recent");
    let mut a = app().with_settings(open(&dir));
    a.ui.recent = vec!["/tmp/alpha.docx".into(), "/tmp/beta.docx".into()];
    a.session.path = Some(std::path::PathBuf::from("/tmp/beta.docx"));
    let r = a.run("ui.recentRemove", json!({"path": "/tmp/beta.docx"})).unwrap();
    assert_eq!((r["removed"].clone(), r["saved"].clone()), (json!(1), json!(true)));
    assert_eq!(a.ui.recent, ["/tmp/alpha.docx".to_string()], "the open document stays off the list");
    assert!(a.run("ui.recentRemove", json!({"path": "/tmp/gamma.docx"})).is_err());
    for junk in [json!({}), json!({"path": 7}), json!({"path": null})] {
        assert!(a.run("ui.recentRemove", junk).is_err());
    }
    let b = app().with_settings(open(&dir));
    assert_eq!(b.ui.recent, ["/tmp/alpha.docx".to_string()], "the removal is in the store");
    let r = a.run("ui.recentClear", json!({})).unwrap();
    assert_eq!(r["removed"], 1);
    let c = app().with_settings(open(&dir));
    assert!(c.ui.recent.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
