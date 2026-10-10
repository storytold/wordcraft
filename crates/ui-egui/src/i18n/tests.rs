//! Interface language tests (#8).

use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

fn lang(code: &str) -> Lang {
    Lang::from_code(code).unwrap_or_else(|| panic!("{code} registered"))
}

#[test]
fn system_locales_map_to_languages() {
    assert_eq!(lang_from_tag("zh_CN.UTF-8"), Some(lang("zh-hans")));
    assert_eq!(lang_from_tag("zh-CN"), Some(lang("zh-hans")));
    assert_eq!(lang_from_tag("zh"), Some(lang("zh-hans")));
    assert_eq!(lang_from_tag("zh-SG"), Some(lang("zh-hans")));
    assert_eq!(lang_from_tag("zh-Hans-HK"), Some(lang("zh-hans")), "a script beats the region");
    assert_eq!(lang_from_tag("zh_TW.UTF-8"), Some(lang("zh-hant")));
    assert_eq!(lang_from_tag("zh-HK"), Some(lang("zh-hant")));
    assert_eq!(lang_from_tag("zh-Hant"), Some(lang("zh-hant")));
    assert_eq!(lang_from_tag("ja-JP"), Some(lang("ja")));
    assert_eq!(lang_from_tag("ja_JP.eucJP@euro"), Some(lang("ja")));
    assert_eq!(lang_from_tag("nb_NO.UTF-8"), Some(lang("nb")));
    assert_eq!(lang_from_tag("nb-NO"), Some(lang("nb")));
    assert_eq!(lang_from_tag("no_NO.UTF-8"), Some(lang("nb")));
    assert_eq!(lang_from_tag("nn-NO"), Some(lang("nn")));
    assert_eq!(lang_from_tag("nn_NO.UTF-8"), Some(lang("nn")));
    assert_eq!(lang_from_tag("no-nn-NO"), Some(lang("nn")));
    assert_eq!(lang_from_tag("en-GB"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("fr-FR"), None);
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
}

#[test]
fn norwegian_numbers_and_measurements_round_trip() {
    let nb = lang("nb");
    assert_eq!(nb.measurement(72.0), "2,54 cm");
    assert_eq!(nb.number(12.5, 1), "12,5");
    assert_eq!(Lang::EN.measurement(72.0), "1\"");
    assert_eq!(parse_number(" 12,5 "), Some(12.5));
    assert_eq!(parse_number("12.5"), Some(12.5));
    assert_eq!(parse_number("NaN"), None);
    set_current(nb);
    assert_eq!(format_number(2.54, 0..=2), "2,54");
    assert_eq!(format_number(21.0, 0..=2), "21");
    assert_eq!(format_number(21.0, 2..=2), "21,00");
    assert_eq!(t("File"), "Fil");
    assert_eq!(location("Home › Font"), "Hjem › Skrift");
    set_current(Lang::EN);
}

#[test]
fn norwegian_preferences_affect_new_documents_only_and_persist() {
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_engine::Session;
    let mut app = crate::WordApp::new(Session::new(Document::new()), Default::default());
    app.run("ui.language", json!({"value": "NB"})).unwrap();
    assert_eq!(app.session.doc.last_section.page_w, 612.0, "changing UI language must preserve the existing document");
    let prefs: crate::UiState = serde_json::from_str(&serde_json::to_string(&app.prefs()).unwrap()).unwrap();
    assert_eq!(prefs.language, "nb");
    app.apply_prefs(prefs);
    for template in ["blank", "letter", "resume", "report", "sample"] {
        app.run("file.new", json!({"template": template})).unwrap();
        assert!((app.session.doc.last_section.page_w - 21.0 * wordcraft_geom::PT_PER_CM).abs() < 0.01);
        assert!((app.session.doc.last_section.page_h - 29.7 * wordcraft_geom::PT_PER_CM).abs() < 0.01);
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("nb-NO"));
        let bytes = wordcraft_engine::io::save_bytes("test.docx", &app.session.doc).unwrap();
        let reopened = wordcraft_engine::io::open_bytes("test.docx", &bytes).unwrap();
        assert!((reopened.last_section.page_w - app.session.doc.last_section.page_w).abs() < 0.1);
        assert_eq!(reopened.styles.default_chr.lang.as_deref(), Some("nb-NO"));
    }
    if let Some(crate::dialogs::Dialog::PageSetup { left, .. }) = crate::dialogs::Dialog::open("pageSetup", &mut app) {
        assert!((left - 2.54).abs() < 0.001);
    } else {
        panic!("page setup dialog");
    }
    app.run("file.new", json!({"locale": "en-US"})).unwrap();
    assert_eq!(app.session.doc.last_section.page_w, 612.0, "explicit command parameters take precedence");
    app.run("insert.dateTime", json!({})).unwrap();
    let date = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(date.chars().nth(2) == Some('.') && date.chars().nth(5) == Some('.'), "{date}");
    set_current(Lang::EN);
}

#[test]
fn nynorsk_ui_formats_and_document_language_are_distinct() {
    use serde_json::json;
    let nn = lang("nn");
    assert_eq!(tr(nn, "Home"), "Heim");
    assert_eq!(tr(nn, "Insert"), "Set inn");
    assert_eq!(tr(nn, "Save"), "Lagre");
    assert_eq!(tr(nn, "Columns"), "Kolonnar");
    assert_eq!(tr(nn, "Open"), "Opne");
    assert_eq!(tr(nn, "Font Size"), "Skriftstorleik");
    assert_eq!(tr(nn, "Norwegian Nynorsk"), "Norsk nynorsk");
    assert_eq!(nn.measurement(72.0), "2,54 cm");
    assert_eq!(nn.number(12.5, 1), "12,5");
    assert_eq!(nn.document_locale(), "nn-NO");
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.run("ui.language", json!({"value": "NN"})).unwrap();
    assert_eq!(app.session.doc.last_section.page_w, 612.0);
    let prefs: crate::UiState = serde_json::from_str(&serde_json::to_string(&app.prefs()).unwrap()).unwrap();
    assert_eq!(prefs.language, "nn");
    app.apply_prefs(prefs);
    for template in ["blank", "letter", "resume", "report", "sample"] {
        app.run("file.new", json!({"template": template})).unwrap();
        assert!((app.session.doc.last_section.page_w - 21.0 * wordcraft_geom::PT_PER_CM).abs() < 0.01);
        assert!((app.session.doc.last_section.page_h - 29.7 * wordcraft_geom::PT_PER_CM).abs() < 0.01);
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("nn-NO"));
    }
    app.run("file.new", json!({})).unwrap();
    app.run("insert.dateTime", json!({})).unwrap();
    let date = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(date.chars().nth(2) == Some('.') && date.chars().nth(5) == Some('.'), "{date}");
    app.run("review.language", json!({})).unwrap();
    assert_eq!(app.session.typing_props().lang.as_deref(), Some("nn-NO"));
    app.run("file.new", json!({"locale": "nb-NO"})).unwrap();
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("nb-NO"));
    set_current(Lang::EN);
}

#[test]
fn the_first_supported_preferred_language_wins() {
    // Windows and macOS list several preferred languages in order.
    assert_eq!(first_supported(["fr-FR", "zh-Hant-TW", "en-US"]), Some(lang("zh-hant")));
    assert_eq!(first_supported(["en-US", "ja-JP"]), Some(Lang::EN));
    assert_eq!(first_supported(["fr-FR", "de-DE"]), None);
    assert_eq!(first_supported([]), None);
}

#[test]
fn settings_resolve_with_fallback() {
    assert_eq!(Lang::from_pref("ja"), lang("ja"));
    assert_eq!(Lang::from_pref("ZH-HANS"), lang("zh-hans"));
    assert_eq!(Lang::from_pref("en"), Lang::EN);
    // `auto` and unknown codes follow the system (English under test).
    assert_eq!(Lang::from_pref(AUTO), Lang::EN);
    assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
    assert_eq!(normalize_pref("Zh-Hant"), Some("zh-hant"));
    assert_eq!(normalize_pref("Auto"), Some(AUTO));
    assert_eq!(normalize_pref("zh"), None, "only exact codes are settings");
    assert_eq!(normalize_pref("xx"), None);
}

#[test]
fn lookups_fall_back_to_english() {
    let zh = lang("zh-hans");
    assert_eq!(tr(zh, "File"), "文件");
    assert_eq!(tr(lang("zh-hant"), "File"), "檔案");
    assert_eq!(tr(lang("ja"), "File"), "ファイル");
    assert_eq!(tr(zh, "no such label"), "no such label");
    assert_eq!(tr(Lang::EN, "File"), "File");
    assert!(has(zh, "Save") && !has(Lang::EN, "Save"));
}

#[test]
fn current_language_is_per_thread() {
    set_current(lang("ja"));
    assert_eq!(t("Insert"), "挿入");
    assert_eq!(location("Home › Font"), "ホーム › フォント");
    assert_eq!(prefixed("Undo", "Bold"), "元に戻す: 太字");
    assert_eq!(prefixed("Undo", ""), "元に戻す");
    std::thread::spawn(|| assert_eq!(current(), Lang::EN)).join().unwrap();
    set_current(Lang::EN);
    assert_eq!(t("Insert"), "Insert");
    assert_eq!(prefixed("Undo", "Typing"), "Undo Typing");
}

#[test]
fn placeholders_fill_without_reinterpreting_values() {
    let args = [("path", "a-{pages}.docx"), ("pages", "2")];
    assert_eq!(fmt("{pages}: {path}; {unknown}", &args), "2: a-{pages}.docx; {unknown}");
    assert_eq!(fmt("text {unfinished", &args), "text {unfinished");
    assert_eq!(fmt(tr(lang("zh-hans"), "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "第 3 页，共 9 页");
}

#[test]
fn malformed_catalog_lines_are_skipped_not_fatal() {
    let (c, errors) =
        Catalog::parse("# c\n\tHello\t你好\nctx\tA\tB\n\tOpen…\t打开\n\tPage {n}\t第 {m} 页\nno tabs\n\tHello\t重复\n\tLine\\nTwo\t一\\n二\n");
    assert_eq!(errors.len(), 5, "{errors:?}");
    assert_eq!(c.plain("Hello"), Some("你好"), "the first entry wins");
    assert_eq!(c.plain("Line\nTwo"), Some("一\n二"));
    assert_eq!(c.plain("Open…"), None);
}

/// Every bundled catalog parses without errors, and its entries keep the English placeholders.
#[test]
fn bundled_catalogs_are_well_formed() {
    let mut codes = std::collections::HashSet::new();
    for l in &LANGUAGES {
        assert!(codes.insert(l.code), "duplicate code {}", l.code);
        assert_eq!(l.code, l.code.to_ascii_lowercase());
        let (entries, errors) = parse_entries(l.source);
        assert!(errors.is_empty(), "{}: {errors:?}", l.code);
        for e in &entries {
            assert_eq!(placeholders(&e.source), placeholders(&e.translation), "{}: {:?}", l.code, e.source);
            assert_eq!(e.translation.trim(), e.translation, "{}: stray whitespace in {:?}", l.code, e.translation);
            assert!(!e.translation.contains("..."), "{}: use … rather than three dots: {:?}", l.code, e.translation);
        }
        assert!(l.code == "en" || entries.len() > 700, "{} has {} entries", l.code, entries.len());
    }
}

/// The ribbon tabs and every registered command's label and ribbon location are translated, in
/// every language, so menus, tooltips and command search never fall back to English.
#[test]
fn every_tab_and_command_is_translated() {
    let session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
    // Labels that read the same in every language.
    let universal = |s: &str| !s.chars().any(char::is_alphabetic);
    let mut missing = Vec::new();
    for l in Lang::all().filter(|l| *l != Lang::EN) {
        let tabs = crate::ribbon::TABS.iter().chain(&["Table Design", "Table Layout"]).copied();
        let commands = session.registry.all().iter().flat_map(|spec| std::iter::once(spec.label).chain(spec.location.split(" › ")));
        for s in tabs.chain(commands).filter(|s| !s.is_empty() && !universal(s)) {
            if !has(l, s) {
                missing.push(format!("{}: {s:?}", l.code()));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "untranslated: {missing:#?}");
}

#[test]
fn the_language_setting_is_a_ui_command_and_persists() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    assert_eq!(app.ui.language, AUTO);
    let r = app.run("ui.language", serde_json::json!({"value": "ZH-HANT"})).unwrap();
    assert_eq!(app.ui.language, "zh-hant");
    assert_eq!(r["effective"], "zh-hant");
    assert!(r["available"].as_array().is_some_and(|a| a.len() == LANGUAGES.len()));
    assert!(app.run("ui.language", serde_json::json!({"value": "xx"})).is_err());
    assert_eq!(app.ui.language, "zh-hant", "an unknown code keeps the setting");
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.language, "zh-hant");
    // Settings saved before this option existed follow the system.
    let legacy: crate::UiState = serde_json::from_str(r#"{"tab":"Home"}"#).unwrap();
    assert_eq!(legacy.language, AUTO);
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
    // Reading without a value reports the current setting.
    assert_eq!(app.run("ui.language", serde_json::json!({})).unwrap()["language"], AUTO);
}
