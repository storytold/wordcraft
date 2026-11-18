use super::*;

/// File › Options › Advanced › Editing: alignment guides are a UI command, on by default and
/// kept between runs.
#[test]
fn alignment_guides_are_a_ui_command_and_survive_a_restart() {
    let mut a = app();
    assert_eq!(a.run("ui.alignmentGuides", json!({})).unwrap(), json!({"alignmentGuides": true}), "on by default");
    assert_eq!(a.run("ui.alignmentGuides", json!({"value": false})).unwrap(), json!({"alignmentGuides": false}));
    for junk in [json!({"value": "no"}), json!({"value": 0}), json!({"value": null})] {
        assert!(a.run("ui.alignmentGuides", junk).is_err());
    }
    assert!(!a.ui.alignment_guides, "a rejected value keeps the setting");
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert!(!b.ui.alignment_guides);
    let old: UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
    assert!(old.alignment_guides, "older settings leave it on");
}

/// Animation effects: "Use system setting" (the default) follows what the host reports and is
/// on when it reports nothing; On and Off ignore the system. The choice is kept between runs.
#[test]
fn animation_effects_setting_follows_the_system_and_survives_a_restart() {
    use theme::Animations;
    for (setting, system, on) in [
        (Animations::System, None, true),
        (Animations::System, Some(true), true),
        (Animations::System, Some(false), false),
        (Animations::On, Some(false), true),
        (Animations::Off, Some(true), false),
        (Animations::Off, None, false),
    ] {
        assert_eq!(setting.enabled(system), on, "{setting:?} with the system at {system:?}");
    }
    // No host hook (tests, headless screenshots): nothing is reported, so they are on.
    let mut a = app();
    assert_eq!(a.run("ui.animations", json!({})).unwrap(), json!({"animations": "system", "enabled": true, "system": null}));
    // The system has them off, as Windows reports with Animation effects switched off.
    let system = std::rc::Rc::new(std::cell::Cell::new(Some(false)));
    let mut a = app_with_system_animations(&system);
    assert_eq!(a.run("ui.animations", json!({})).unwrap(), json!({"animations": "system", "enabled": false, "system": false}));
    assert_eq!(a.run("ui.animations", json!({"value": "On"})).unwrap()["enabled"], true);
    system.set(Some(true));
    assert_eq!(a.run("ui.animations", json!({"value": "off"})).unwrap(), json!({"animations": "off", "enabled": false, "system": true}));
    assert!(a.run("ui.animations", json!({"value": "sometimes"})).is_err());
    assert_eq!(a.ui.animations, Animations::Off, "a rejected value keeps the setting");
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""animations":"off""#), "{saved}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.animations, Animations::Off);
    assert!(!b.animations_on());
    // Preferences from before the setting, or with a value it doesn't know, follow the system.
    for raw in [r#"{"tab": "Insert"}"#, r#"{"animations": "wobbly", "tab": "Insert"}"#, r#"{"animations": 3, "tab": "Insert"}"#] {
        let ui = UiState::from_json(raw.as_bytes()).unwrap();
        assert_eq!((ui.animations, ui.tab.as_str()), (Animations::System, "Insert"), "{raw}");
    }
}

/// #4368: a user name cleared in File › Options stays empty after a restart.
#[test]
fn blank_user_name_survives_restart() {
    let mut first = app();
    first.session.author.clear();
    let saved = serde_json::to_vec(&first.prefs()).unwrap();

    let mut second = app();
    assert!(!second.session.author.is_empty());
    second.apply_prefs(serde_json::from_slice(&saved).unwrap());
    assert_eq!(second.session.author, "");

    // A name of only spaces is not a name: the default stays.
    let default = app().session.author;
    let mut third = app();
    third.apply_prefs(serde_json::from_str(r#"{"author": "   "}"#).unwrap());
    assert_eq!(third.session.author, default);
}

/// #2068: fonts applied from the font box or the mini toolbar (`format.font`) and from the
/// Font dialog (`format.set`) join Recently Used Fonts, most recent first, once each, ten at
/// most; the list is kept in `ui.json` and read back tidied.
#[test]
fn applied_fonts_join_recently_used_fonts() {
    let mut a = typed();
    assert!(a.ui.recent_fonts.is_empty());
    a.run("format.font", json!({"name": "Courier New"})).unwrap();
    a.run("format.font", json!({"name": "Georgia"})).unwrap();
    assert_eq!(a.ui.recent_fonts, ["Georgia", "Courier New"]);
    // Applied again: to the front, once, as typed.
    a.run("format.font", json!({"name": "courier new"})).unwrap();
    assert_eq!(a.ui.recent_fonts, ["courier new", "Georgia"]);
    // The Font dialog applies through format.set.
    a.run("format.set", json!({"props": {"font": "Verdana", "bold": true}})).unwrap();
    assert_eq!(a.ui.recent_fonts, ["Verdana", "courier new", "Georgia"]);
    // A refused command, or one applying no Latin font, adds nothing.
    assert!(a.run("format.font", json!({"name": "  "})).is_err());
    a.run("format.font", json!({"eastAsia": "SimSun"})).unwrap();
    a.run("format.set", json!({"props": {"italic": true}})).unwrap();
    assert_eq!(a.ui.recent_fonts.len(), 3);
    for i in 0..12 {
        a.run("format.font", json!({"name": format!("Font {i}")})).unwrap();
    }
    assert_eq!(a.ui.recent_fonts.len(), MAX_RECENT_FONTS);
    assert_eq!(a.ui.recent_fonts.first().map(String::as_str), Some("Font 11"));
    assert_eq!(a.ui.recent_fonts.last().map(String::as_str), Some("Font 2"));
    // Kept between runs.
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""recentFonts":["Font 11","Font 10""#), "{saved}");
    let mut b = app();
    b.apply_prefs(UiState::from_json(saved.as_bytes()).unwrap());
    assert_eq!(b.ui.recent_fonts, a.ui.recent_fonts);
    // A ui.json from before the list has nothing recent; junk in the list is dropped and the
    // rest kept once each, trimmed, ten at most; a value of the wrong type keeps the default.
    assert!(UiState::from_json(br#"{"tab": "Insert"}"#).unwrap().recent_fonts.is_empty());
    let mut list: Vec<String> = ["", "  Arial ", "ARIAL", "Georgia"].map(String::from).to_vec();
    list.extend((0..12).map(|i| format!("F{i}")));
    let raw = json!({ "recentFonts": list });
    let tidy = UiState::from_json(raw.to_string().as_bytes()).unwrap().recent_fonts;
    assert_eq!(tidy.len(), MAX_RECENT_FONTS);
    assert_eq!(&tidy[..3], ["Arial", "Georgia", "F0"]);
    assert!(UiState::from_json(br#"{"recentFonts": "Arial"}"#).unwrap().recent_fonts.is_empty());
    // One entry that isn't a name drops that entry, not the list.
    let mixed = UiState::from_json(br#"{"recentFonts": ["Arial", 3, null, {"a": 1}, "Georgia"]}"#).unwrap();
    assert_eq!(mixed.recent_fonts, ["Arial", "Georgia"]);
}

/// #2068 review: a hand-edited `ui.json` with a huge list or huge names read back slowly and
/// kept a multi-megabyte name, and the Font dialog applies names of any length; neither is
/// kept. Reading stops at ten names.
#[test]
fn recently_used_fonts_refuse_overlong_names_and_huge_lists() {
    let long = "N".repeat(MAX_FONT_NAME_BYTES + 1);
    let mut a = typed();
    a.run("format.font", json!({"name": "Georgia"})).unwrap();
    a.run("format.set", json!({"props": {"font": long}})).unwrap();
    assert_eq!(a.ui.recent_fonts, ["Georgia"], "the dialog's overlong name is applied but not kept");
    let mut list = vec!["Arial".to_string()];
    push_recent_font(&mut list, &long);
    push_recent_font(&mut list, &"M".repeat(MAX_FONT_NAME_BYTES));
    assert_eq!(list.len(), 2, "a name of the longest length is kept");
    // A huge saved list: overlong names skipped, the first ten others kept, quickly.
    let mut saved: Vec<String> = vec![long.clone(), "x".repeat(4 << 20)];
    saved.extend((0..200_000).map(|i| format!("Family {i}")));
    let raw = json!({ "recentFonts": saved }).to_string();
    let started = std::time::Instant::now();
    let ui = UiState::from_json(raw.as_bytes()).unwrap();
    assert_eq!(ui.recent_fonts.len(), MAX_RECENT_FONTS);
    assert_eq!(ui.recent_fonts.first().map(String::as_str), Some("Family 0"));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "read in {:?}", started.elapsed());
    let mut b = app();
    b.apply_prefs(UiState { recent_fonts: vec![long, " Arial ".into(), "ARIAL".into()], ..UiState::default() });
    assert_eq!(b.ui.recent_fonts, ["Arial"]);
}

/// #115: the old dark-mode switch migrates to Light/Dark; a saved System stays System, and the
/// old key is never written back.
#[test]
fn interface_theme_migrates_and_survives_a_restart() {
    use theme::Appearance;
    for (saved, want) in [(r#"{"dark": false}"#, Appearance::Light), (r#"{"dark": true}"#, Appearance::Dark), ("{}", Appearance::System)] {
        let mut a = app();
        a.apply_prefs(serde_json::from_str(saved).unwrap());
        assert_eq!(a.ui.theme, want, "{saved}");
    }
    let mut a = app();
    a.run("ui.theme", json!({"value": "System"})).unwrap();
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""theme":"system""#) && !saved.contains(r#""dark":"#), "{saved}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.theme, Appearance::System);
    // An unknown saved value keeps the rest of the preferences.
    let odd: UiState = serde_json::from_str(r#"{"theme": "sepia", "tab": "Insert"}"#).unwrap();
    assert_eq!((odd.theme, odd.tab.as_str()), (Appearance::System, "Insert"));
}

/// #475, #822: the interface size is kept between runs; junk saved values read as Auto and
/// out-of-range ones are clamped, without losing the other preferences.
#[test]
fn interface_size_survives_a_restart_and_clamps_junk() {
    let mut a = app();
    assert_eq!(a.run("ui.interfaceSize", json!({})).unwrap(), json!({"auto": true, "percent": 100}));
    assert_eq!(a.run("ui.interfaceSize", json!({"percent": 150})).unwrap(), json!({"auto": false, "percent": 150}));
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""uiScale":150"#) && !saved.contains("interfaceSize"), "{saved}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved).unwrap());
    assert_eq!(b.ui.interface_size.percent(), Some(150));
    for (raw, want) in [
        ("1000", Some(300)),
        ("10", Some(50)),
        ("-5", Some(50)),
        ("124.6", Some(125)),
        (r#""auto""#, None),
        (r#""big""#, None),
        ("null", None),
        ("[1]", None),
    ] {
        let ui: UiState = serde_json::from_str(&format!(r#"{{"uiScale": {raw}, "tab": "Insert"}}"#)).unwrap();
        assert_eq!((ui.interface_size.percent(), ui.tab.as_str()), (want, "Insert"), "{raw}");
    }
    assert_eq!(a.run("ui.interfaceSize", json!({"percent": 500})).unwrap()["percent"], 300);
    assert!(a.run("ui.interfaceSize", json!({"percent": "huge"})).is_err());
    assert!(a.run("ui.interfaceSize", json!({"percent": null})).is_err());
    assert_eq!(a.ui.interface_size.percent(), Some(300), "a rejected value keeps the setting");
    assert_eq!(a.run("ui.interfaceSize", json!({"value": "125%"})).unwrap()["percent"], 125);
    assert_eq!(a.run("ui.interfaceSize", json!({"value": "Auto"})).unwrap()["auto"], true);
    let saved = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved.contains(r#""uiScale":"auto""#), "{saved}");
}

/// #822: a `ui.json` from before saved `interfaceSize` on top of the display scale. Its
/// default, 100, becomes Auto (the same size as before); a size picked on purpose becomes the
/// absolute size it drew at, once the first frame says the display's scale: 125 % on a 2×
/// display drew at 250 % and stays there. A valid `uiScale` beside it wins.
#[test]
fn interface_size_from_before_822_migrates() {
    let frame = |a: &mut WordApp, native: f32| {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
        ctx.run_ui(input, |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
    };
    for (json, native, want) in [
        (r#"{"interfaceSize": 100}"#, 2.0, None),
        (r#"{"interfaceSize": 100.2}"#, 2.0, None),
        (r#"{"interfaceSize": "big"}"#, 2.0, None),
        (r#"{"interfaceSize": 125}"#, 2.0, Some(250)),
        (r#"{"interfaceSize": 150}"#, 1.0, Some(150)),
        (r#"{"interfaceSize": 80}"#, 1.5, Some(120)),
        (r#"{"interfaceSize": 200}"#, 2.0, Some(300)),
        (r#"{"interfaceSize": 80}"#, 0.5, Some(50)),
        (r#"{"interfaceSize": 150, "uiScale": 90}"#, 2.0, Some(90)),
        (r#"{"interfaceSize": 150, "uiScale": "junk"}"#, 2.0, Some(300)),
    ] {
        let mut a = app();
        a.apply_prefs(serde_json::from_str(json).unwrap());
        frame(&mut a, native);
        assert_eq!(a.ui.interface_size.percent(), want, "{json} on a {native}× display");
        assert!(!serde_json::to_string(&a.prefs()).unwrap().contains("interfaceSize"), "{json}");
    }
    // Converted once: a later move to another display keeps the absolute size.
    let mut a = app();
    a.apply_prefs(serde_json::from_str(r#"{"interfaceSize": 125}"#).unwrap());
    frame(&mut a, 2.0);
    frame(&mut a, 1.0);
    assert_eq!(a.ui.interface_size.percent(), Some(250));
    // A size the user picks before the first frame isn't overwritten by the old one.
    let mut a = app();
    a.apply_prefs(serde_json::from_str(r#"{"interfaceSize": 125}"#).unwrap());
    a.run("ui.interfaceSize", json!({"percent": 100})).unwrap();
    frame(&mut a, 2.0);
    assert_eq!(a.ui.interface_size.percent(), Some(100));
}

/// Issue #2716: taking documents off the recent list is final — the open document must not be
/// pushed back by the after-command re-add — and the change goes to the settings at once, so
/// it survives the app being killed. Junk params return errors, never panics.
#[test]
fn recent_list_edits_are_final_and_persist() {
    let saved = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let saved_hook = saved.clone();
    let mut a = app();
    a.services.save_prefs = Some(Box::new(move |app: &WordApp| {
        saved_hook.set(saved_hook.get() + 1);
        let _ = serde_json::to_string(&app.prefs()).unwrap();
        Ok(true)
    }));
    a.ui.recent = vec!["/tmp/alpha.docx".into(), "/tmp/beta.docx".into()];
    // The open document is on the list, at the back.
    a.session.path = Some(std::path::PathBuf::from("/tmp/beta.docx"));

    // Removing the open document sticks: `after_command` must not re-add it.
    let r = a.run("ui.recentRemove", json!({"path": "/tmp/beta.docx"})).unwrap();
    assert_eq!(r["removed"], 1);
    assert_eq!(r["recent"], json!(["/tmp/alpha.docx"]));
    assert_eq!(r["saved"], true);
    assert_eq!(a.ui.recent, ["/tmp/alpha.docx".to_string()], "the open document stays off the list");
    assert_eq!(saved.get(), 1, "the change is written out at once");

    // Removing an entry that isn't there is an error, not a silent success, and saves nothing.
    assert!(a.run("ui.recentRemove", json!({"path": "/tmp/gamma.docx"})).is_err());
    assert_eq!(saved.get(), 1, "a rejected command writes nothing");
    // Hostile params: missing path, junk types.
    for junk in [json!({}), json!({"path": 7}), json!({"path": null}), json!({"path": ["x"]})] {
        assert!(a.run("ui.recentRemove", junk).is_err());
    }
    assert_eq!(a.ui.recent, ["/tmp/alpha.docx".to_string()], "junk params change nothing");

    // A normal command re-adds the open document, as before.
    a.run("text.insert", json!({"text": "x"})).unwrap();
    assert_eq!(a.ui.recent.first(), Some(&"/tmp/beta.docx".to_string()));

    // Clearing empties the list (the re-added open document is back, so both entries go) and
    // keeps it empty through the next command.
    let r = a.run("ui.recentClear", json!({})).unwrap();
    assert_eq!(r["removed"], 2);
    assert!(a.ui.recent.is_empty());
    assert_eq!(saved.get(), 2, "the clear is written out at once");
    a.run("text.insert", json!({"text": "y"})).unwrap();
    assert_eq!(a.ui.recent.first(), Some(&"/tmp/beta.docx".to_string()), "the open document comes back after clearing");

    // Clearing an empty list reports zero removed and is not an error.
    a.ui.recent.clear();
    assert_eq!(a.run("ui.recentClear", json!({})).unwrap()["removed"], 0);

    // The list lives in the prefs, so a restart keeps the removal.
    a.ui.recent = vec!["/tmp/alpha.docx".into(), "/tmp/beta.docx".into()];
    a.run("ui.recentRemove", json!({"path": "/tmp/alpha.docx"})).unwrap();
    let saved_prefs = serde_json::to_string(&a.prefs()).unwrap();
    assert!(saved_prefs.contains(r#""recent":["/tmp/beta.docx"]"#), "{saved_prefs}");
    let mut b = app();
    b.apply_prefs(serde_json::from_str(&saved_prefs).unwrap());
    assert_eq!(b.ui.recent, ["/tmp/beta.docx".to_string()]);
}
