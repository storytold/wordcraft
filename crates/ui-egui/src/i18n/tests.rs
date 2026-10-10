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
    assert_eq!(lang_from_tag("en-GB"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("fr-FR"), None);
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
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

#[test]
fn ukrainian_locales_and_saved_preference_work_without_changing_the_document() {
    let uk = lang("uk");
    for tag in ["uk", "uk-UA", "UK_ua.UTF-8", "uk-Cyrl-UA", "uk_UA.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(uk), "{tag}");
    }
    assert_eq!(first_supported(["fr-FR", "uk-UA", "en-US"]), Some(uk));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "UK"})).unwrap();
    assert_eq!(result["effective"], "uk");
    assert_eq!(app.ui.language, "uk");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), uk);
    assert_eq!(uk.name(), "Українська");
}

#[test]
fn ukrainian_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    assert_eq!(keys(lang("uk").0.source), keys(lang("zh-hans").0.source));
    let uk = lang("uk");
    assert_eq!(tr(uk, "Home"), "Основне");
    assert_eq!(tr(uk, "Font"), "Шрифт");
    assert_eq!(tr(uk, "Review"), "Рецензування");
    assert_eq!(tr(uk, "Save"), "Зберегти");
    assert_eq!(tr(uk, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(uk, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Експортовано draft-{words}.docx");
    // Count-neutral wording is grammatical for every Ukrainian integer category, including teens.
    for count in [0, 1, 2, 5, 11, 12, 21, 22, 25, 111, 121] {
        let words = count.to_string();
        assert_eq!(fmt(tr(uk, "{words} words"), &[("words", &words)]), format!("Слів: {count}"));
        assert_eq!(fmt(tr(uk, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Слів: {count}; виділено: 1"));
    }
}

#[test]
fn bundled_interface_fonts_cover_ukrainian_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("uk").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| ('\u{0400}'..='\u{04ff}').contains(c)).chain("ҐґЄєІіЇї".chars()).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn brazilian_portuguese_locales_and_saved_preference_work_without_changing_the_document() {
    let pt = lang("pt-br");
    for tag in ["pt-BR", "pt_BR.UTF-8", "PT-BR", "pt-BR-latn", "pt_BR.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(pt), "{tag}");
    }
    // European Portuguese and a bare `pt` have no catalog yet, so they follow the fallback.
    assert_eq!(lang_from_tag("pt-PT"), None);
    assert_eq!(lang_from_tag("pt"), None);
    assert_eq!(first_supported(["fr-FR", "pt-BR", "en-US"]), Some(pt));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "PT-BR"})).unwrap();
    assert_eq!(result["effective"], "pt-br");
    assert_eq!(app.ui.language, "pt-br");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), pt);
    assert_eq!(pt.name(), "Português (Brasil)");
}

#[test]
fn brazilian_portuguese_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    assert_eq!(keys(lang("pt-br").0.source), keys(lang("zh-hans").0.source));
    let pt = lang("pt-br");
    assert_eq!(tr(pt, "Home"), "Início");
    assert_eq!(tr(pt, "Font"), "Fonte");
    assert_eq!(tr(pt, "Review"), "Revisão");
    assert_eq!(tr(pt, "Save"), "Salvar");
    assert_eq!(tr(pt, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(pt, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Exportado draft-{words}.docx");
    // Count-neutral wording is grammatical for every count: `Palavras: 21`, `Palavras: 21; seleção: 1`.
    for count in [0, 1, 2, 5, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(pt, "{words} words"), &[("words", &words)]), format!("Palavras: {count}"));
        assert_eq!(fmt(tr(pt, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Palavras: {count}; seleção: 1"));
    }
}

#[test]
fn bundled_interface_fonts_cover_brazilian_portuguese_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("pt-br").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        // The accented Latin letters Portuguese adds on top of ASCII; symbols such as ¶, ⌘ or 📂
        // come from the English sources and render as they already do.
        .filter(|c| ('\u{00C0}'..='\u{017F}').contains(c))
        .chain("ÀàÁáÂâÃãÇçÉéÊêÍíÓóÔôÕõÚúÜü".chars())
        .collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn chinese_interface_fonts_load_and_cover_simplified_hanzi() {
    // #241: without an embedded Chinese face the interface adds an installed CJK font; egui must
    // accept it (it panics on font data it can't parse) and draw simplified-only hanzi with it.
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::font_definitions(true, true));
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let covered = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), "删除页选"));
    let embedded = !wordcraft_fonts::ui_needs_system_cjk(true, &wordcraft_fonts::ui_cjk_fonts(true));
    if embedded || wordcraft_fonts::system_cjk_ui_font(true).is_some() {
        assert!(covered, "a Chinese face is available but the interface lacks simplified hanzi");
    }
}

#[test]
fn only_cjk_languages_ask_for_an_installed_cjk_font() {
    // #241 follow-up: an English interface must not read a large system CJK font on startup.
    let cjk: Vec<&str> = Lang::all().filter(|l| l.uses_cjk()).map(Lang::code).collect();
    assert_eq!(cjk, ["zh-hans", "zh-hant", "ja"]);
}
