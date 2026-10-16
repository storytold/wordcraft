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
    assert_eq!(lang_from_tag("ko-KR"), Some(lang("ko")));
    assert_eq!(lang_from_tag("ko_KR.UTF-8"), Some(lang("ko")));
    assert_eq!(lang_from_tag("en-GB"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag("fr-FR"), Some(lang("fr")));
    assert_eq!(lang_from_tag(UNSUPPORTED), None);
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
    assert_eq!(lang_from_tag("sr"), Some(lang("sr")), "Cyrillic is the default script");
    assert_eq!(lang_from_tag("sr-RS"), Some(lang("sr")));
    assert_eq!(lang_from_tag("sr-Cyrl-RS"), Some(lang("sr")));
    assert_eq!(lang_from_tag("sr-Latn-RS"), Some(lang("sr-latn")));
    assert_eq!(lang_from_tag("sr-Latn"), Some(lang("sr-latn")));
    assert_eq!(lang_from_tag("de"), Some(lang("de")));
    assert_eq!(lang_from_tag("de-DE"), Some(lang("de")));
    assert_eq!(lang_from_tag("de_AT.UTF-8"), Some(lang("de")));
    assert_eq!(lang_from_tag("de-CH"), Some(lang("de")));
    assert_eq!(lang_from_tag("DE_de.UTF-8@euro"), Some(lang("de")));
}

/// A language tag no catalog will ever have: `qaa`–`qtz` are reserved for local use (ISO 639-2),
/// so these tests keep working as real languages land.
const UNSUPPORTED: &str = "qaa-QM";

#[test]
fn the_first_supported_preferred_language_wins() {
    // Windows and macOS list several preferred languages in order.
    assert_eq!(first_supported([UNSUPPORTED, "zh-Hant-TW", "en-US"]), Some(lang("zh-hant")));
    assert_eq!(first_supported(["en-US", "ja-JP"]), Some(Lang::EN));
    assert_eq!(first_supported([UNSUPPORTED, "qtz"]), None);
    assert_eq!(first_supported(["fr-FR", "de-DE"]), Some(lang("fr")));
    assert_eq!(first_supported(["nl-NL", "de-DE"]), Some(lang("de")));
    assert_eq!(first_supported(["nl-NL", UNSUPPORTED]), None);
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
fn korean_print_settings_and_printer_feedback_are_translated() {
    let korean = lang("ko");
    for source in [
        "Printer",
        "Select a printer",
        "Refresh printers",
        "Copies",
        "Two-sided printing",
        "Printer default",
        "One-sided",
        "Two-sided: long edge",
        "Two-sided: short edge",
        "Grayscale",
        "Finding printers…",
        "No printers found. Configure a system printer or export a PDF.",
        "Printer discovery failed.",
        "Submitting print job…",
        "Print job submitted.",
        "Print job submitted: {receipt}",
        "Print failed: the printing worker stopped.",
        "Print failed: {error}",
    ] {
        assert!(has(korean, source), "missing Korean print translation: {source}");
    }
    assert_eq!(korean.catalog().plain("Printer"), Some("프린터"));
    assert_eq!(korean.catalog().plain("Copies"), Some("매수"));
    assert_eq!(korean.catalog().plain("Two-sided printing"), Some("양면 인쇄"));
    assert_eq!(
        korean.catalog().plain("No printers found. Configure a system printer or export a PDF."),
        Some("프린터를 찾을 수 없습니다. 시스템 프린터를 설정하거나 PDF로 내보내세요.")
    );
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
    assert_eq!(first_supported([UNSUPPORTED, "uk-UA", "en-US"]), Some(uk));
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
fn serbian_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let reference = keys(lang("zh-hans").0.source);
    assert_eq!(keys(lang("sr").0.source), reference);
    assert_eq!(keys(lang("sr-latn").0.source), reference);
    let (cyr, lat) = (lang("sr"), lang("sr-latn"));
    assert_eq!(tr(cyr, "Home"), "Почетак");
    assert_eq!(tr(lat, "Home"), "Početak");
    assert_eq!(tr(cyr, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(lat, "Exported {path}"), &[("path", "draft-1.docx")]), "Izvezeno: draft-1.docx");
}

#[test]
fn bundled_interface_fonts_cover_serbian_cyrillic_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("sr").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| ('\u{0400}'..='\u{04ff}').contains(c)).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn bundled_interface_fonts_cover_serbian_latin_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("sr-latn").0.source);
    assert!(errors.is_empty());
    // Serbian Latin's letters beyond plain ASCII: š č ć ž đ (and their capitals).
    let extra = ['š', 'č', 'ć', 'ž', 'đ', 'Š', 'Č', 'Ć', 'Ž', 'Đ'];
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| extra.contains(c)).chain(extra).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn serbian_locales_resolve_by_script() {
    let cyr = lang("sr");
    let lat = lang("sr-latn");
    for tag in ["sr", "sr-RS", "sr_RS.UTF-8", "SR", "sr-Cyrl", "sr-Cyrl-RS"] {
        assert_eq!(lang_from_tag(tag), Some(cyr), "{tag}");
    }
    for tag in ["sr-Latn", "sr-Latn-RS", "sr-latn-me"] {
        assert_eq!(lang_from_tag(tag), Some(lat), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "sr-Latn-RS", "en-US"]), Some(lat));
    assert_eq!(tr(cyr, "File"), "Фајл");
    assert_eq!(tr(lat, "File"), "Fajl");
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
    assert_eq!(first_supported([UNSUPPORTED, "pt-BR", "en-US"]), Some(pt));
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
fn czech_locales_and_saved_preference_work_without_changing_the_document() {
    let cs = lang("cs");
    for tag in ["cs", "cs-CZ", "cs_CZ.UTF-8", "CS", "cs-Latn-CZ", "cs_CZ.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(cs), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "cs-CZ", "en-US"]), Some(cs));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "CS"})).unwrap();
    assert_eq!(result["effective"], "cs");
    assert_eq!(app.ui.language, "cs");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), cs);
    assert_eq!(cs.name(), "Čeština");
}

#[test]
fn czech_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Keep the original Chinese coverage and include the complete styles, equation gallery,
    // paper sizes and watermarks from the Russian and Spanish catalogs.
    assert!(keys(lang("cs").0.source).is_superset(&keys(lang("zh-hans").0.source)));
    assert!(keys(lang("cs").0.source).is_superset(&keys(lang("ru").0.source)));
    assert!(keys(lang("cs").0.source).is_superset(&keys(lang("es").0.source)));
    let cs = lang("cs");
    assert_eq!(tr(cs, "Home"), "Domů");
    assert_eq!(tr(cs, "Font"), "Písmo");
    assert_eq!(tr(cs, "Review"), "Revize");
    assert_eq!(tr(cs, "Save"), "Uložit");
    assert_eq!(tr(cs, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(cs, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Exportováno: draft-{words}.docx");
    // Count-neutral wording is grammatical across all Czech plural categories (1, 2–4, 0/5+).
    for count in [0, 1, 2, 3, 4, 5, 11, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(cs, "{words} words"), &[("words", &words)]), format!("Počet slov: {count}"));
        assert_eq!(fmt(tr(cs, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Počet slov: {words}; vybráno: 1"));
    }
    set_current(cs);
    // Built-in names and numbered style families use Czech; user-defined names stay as written.
    for (style, expected) in [("Normal", "Normální"), ("No Spacing", "Bez mezer"), ("Heading 2", "Nadpis 2"), ("TOC 3", "Obsah 3")] {
        assert_eq!(builtin_style_name(style), expected);
    }
    assert_eq!(builtin_style_name("My Own Style"), "My Own Style");
    set_current(Lang::EN);
}

#[test]
fn bundled_interface_fonts_cover_czech_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("cs").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        .filter(|c| ('\u{00C0}'..='\u{017F}').contains(c))
        .chain("ÁáČčĎďÉéĚěÍíŇňÓóŘřŠšŤťÚúŮůÝýŽž".chars())
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
    let (catalogs, _) = Lang::cjk_font_text(true);
    let embedded = !wordcraft_fonts::ui_needs_system_cjk(&catalogs, &wordcraft_fonts::ui_cjk_fonts(true));
    if embedded || wordcraft_fonts::system_cjk_ui_font(true, &['页']).is_some() {
        assert!(covered, "a Chinese face is available but the interface lacks simplified hanzi");
    }
}

#[test]
fn korean_interface_fonts_load_and_cover_hangul() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::font_definitions_language(false, true, true));
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let covered = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), "한글"));
    let (catalogs, names) = Lang::cjk_font_text_for(false, true);
    let mut texts = catalogs;
    texts.push(&names);
    let embedded = !wordcraft_fonts::ui_needs_system_cjk(&texts, &wordcraft_fonts::ui_cjk_fonts_for(false, true));
    if embedded || wordcraft_fonts::system_cjk_ui_font_for(false, true, &['한']).is_some() {
        assert!(covered, "a Korean face is available but the interface lacks Hangul glyphs");
    }
}

#[test]
fn cjk_catalogs_are_covered_by_the_embedded_interface_faces() {
    // #486: every CJK character of the Chinese, Japanese and Korean catalogs and language names
    // has a glyph in the interface font chain built from the embedded faces alone, in every
    // interface weight; `?` and boxes showed where the Japanese face was the only CJK one. Needs
    // a craft-fonts build (CRAFT_FONTS_DIR); without one there is nothing embedded to check, and
    // the interface reads an installed CJK font instead.
    if wordcraft_fonts::ui_cjk_fonts(false).is_empty() {
        eprintln!("skipped: built without craft-fonts (CRAFT_FONTS_DIR)");
        return;
    }
    let names: String = Lang::all().map(Lang::name).collect();
    for code in ["zh-hans", "zh-hant", "ja", "ko"] {
        let l = lang(code);
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions_language(l.prefers_hans(), l.prefers_korean(), false));
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let (entries, errors) = parse_entries(l.0.source);
        assert!(errors.is_empty(), "{code}: {errors:?}");
        let chars: std::collections::BTreeSet<char> =
            entries.iter().flat_map(|e| e.translation.chars()).chain(names.chars()).filter(|c| wordcraft_fonts::is_cjk(*c)).collect();
        assert!(chars.len() > 300, "{code}: {} CJK characters", chars.len());
        for font in [crate::theme::regular(14.0), crate::theme::medium(14.0), crate::theme::semibold(14.0)] {
            let missing: String = ctx.fonts_mut(|f| chars.iter().filter(|c| !f.has_glyph(&font, **c)).collect());
            assert!(missing.is_empty(), "{code}: no glyph in {font:?} for {missing}");
        }
        // So the interface reads no installed CJK font for these languages.
        assert_eq!(
            wordcraft_fonts::ui_uncovered_cjk(&[l.0.source, &names], &wordcraft_fonts::ui_cjk_fonts_for(l.prefers_hans(), l.prefers_korean())),
            Vec::<char>::new()
        );
    }
}

#[test]
fn only_cjk_languages_ask_for_an_installed_cjk_font() {
    // #241 follow-up: an English interface must not read a large system CJK font on startup.
    let cjk: Vec<&str> = Lang::all().filter(|l| l.uses_cjk()).map(Lang::code).collect();
    assert_eq!(cjk, ["zh-hans", "zh-hant", "ja", "ko"]);
}

#[test]
fn russian_locales_and_saved_preference_work_without_changing_the_document() {
    let ru = lang("ru");
    for tag in ["ru", "ru-RU", "ru_RU.UTF-8", "RU-ru", "ru_UA.UTF-8", "ru-Cyrl-RU"] {
        assert_eq!(lang_from_tag(tag), Some(ru), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "ru-RU", "en-US"]), Some(ru));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "RU"})).unwrap();
    assert_eq!(result["effective"], "ru");
    assert_eq!(app.ui.language, "ru");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), ru);
    assert_eq!(ru.name(), "Русский");
}

#[test]
fn russian_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Everything the other catalogs translate, plus the preview samples below.
    assert!(keys(lang("ru").0.source).is_superset(&keys(lang("zh-hans").0.source)));
    assert!(parse_entries(lang("ru").0.source).1.is_empty());
    let ru = lang("ru");
    // Style gallery and Design tab previews show Cyrillic sample text.
    assert_eq!(tr(ru, "AaBbCcDd"), "АаБбВвГг");
    assert_eq!(tr(ru, "Title"), "Заголовок");
    assert_eq!(tr(ru, "Heading 1"), "Заголовок 1");
    assert!(tr(ru, "Body text in a short paragraph to show spacing.").starts_with("Основной текст"));
    assert_eq!(tr(ru, "Home"), "Главная");
    assert_eq!(tr(ru, "Font"), "Шрифт");
    assert_eq!(tr(ru, "Review"), "Рецензирование");
    assert_eq!(tr(ru, "Save"), "Сохранить");
    assert_eq!(tr(ru, "unknown future label"), "unknown future label");
    // The search hint's example must find the command it names.
    assert_eq!(tr(ru, "Insert Table"), "Вставить таблицу");
    assert!(tr(ru, "Type a command, e.g. \"insert table\"").contains("вставить таблицу"));
    // Count-neutral wording is grammatical for every count: `Слов: 21`, `Слов: 1 из 21`.
    for count in [0, 1, 2, 5, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(ru, "{words} words"), &[("words", &words)]), format!("Слов: {count}"));
        assert_eq!(fmt(tr(ru, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Слов: 1 из {count}"));
    }
}

#[test]
fn bundled_interface_fonts_cover_russian_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("ru").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        // Cyrillic letters plus the Russian quotes and dash; symbols such as ¶, ⌘ or 📂 come from
        // the English sources and render as they already do.
        .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c) || matches!(c, '«' | '»' | '—'))
        .chain("АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя".chars())
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
fn built_in_style_names_are_translated_for_display_and_user_styles_are_not() {
    let sheet = wordcraft_doc::StyleSheet::builtin();
    let name = |id: &str| style_name(sheet.get(id).unwrap());
    set_current(lang("ru"));
    assert_eq!(name("Normal"), "Обычный");
    assert_eq!(name("Title"), "Заголовок");
    assert_eq!(name("Heading1"), "Заголовок 1");
    // Numbered families beyond the catalog's explicit entries go through one template.
    assert_eq!(name("Heading7"), "Заголовок 7");
    assert_eq!(name("TOC9"), "Оглавление 9");
    // A style the author made keeps its name, even when it matches a built-in one.
    let mine = wordcraft_doc::Style { name: "Normal".into(), builtin: false, ..Default::default() };
    assert_eq!(style_name(&mine), "Normal");
    // A built-in name without a translation stays English.
    assert_eq!(builtin_style_name("Some Future Style"), "Some Future Style");
    assert_eq!(builtin_style_name("Heading x"), "Heading x");
    // Every built-in paragraph and character style shows in Russian. Table styles (`Grid Table 4 –
    // Blue`) appear only as tooltips under Table Design and stay English for now.
    let untranslated: Vec<&str> = sheet
        .styles
        .iter()
        .filter(|s| s.kind != wordcraft_doc::StyleKind::Table && style_name(s) == s.name && s.name.chars().any(char::is_alphabetic))
        .map(|s| s.name.as_str())
        .collect();
    assert!(untranslated.is_empty(), "built-in styles without a Russian name: {untranslated:?}");
    set_current(Lang::EN);
    assert_eq!(name("Heading7"), "Heading 7");
    // The document itself never changes.
    assert_eq!(sheet.get("Normal").unwrap().name, "Normal");
}

#[test]
fn spanish_built_in_style_names_are_translated() {
    set_current(lang("es"));
    for (source, expected) in [
        ("Book Title", "Título de libro"),
        ("Caption", "Leyenda"),
        ("Emphasis", "Énfasis"),
        ("Endnote Reference", "Referencia de nota al final"),
        ("Endnote Text", "Texto de nota al final"),
        ("Footnote Reference", "Referencia de nota al pie"),
        ("Footnote Text", "Texto de nota al pie"),
        ("Hyperlink", "Hipervínculo"),
        ("Intense Emphasis", "Énfasis intenso"),
        ("Intense Quote", "Cita intensa"),
        ("Intense Reference", "Referencia intensa"),
        ("List Paragraph", "Párrafo de lista"),
        ("Quote", "Cita"),
        ("Strong", "Fuerte"),
        ("Subtitle", "Subtítulo"),
        ("Subtle Emphasis", "Énfasis sutil"),
        ("Subtle Reference", "Referencia sutil"),
        ("TOC Heading", "Título de tabla de contenido"),
    ] {
        assert_eq!(builtin_style_name(source), expected, "{source}");
    }
    for level in 4..=9 {
        assert_eq!(builtin_style_name(&format!("Heading {level}")), format!("Título {level}"));
    }
    for level in 1..=9 {
        assert_eq!(builtin_style_name(&format!("TOC {level}")), format!("Tabla de contenido {level}"));
    }
    set_current(Lang::EN);
}

#[test]
fn context_entries_override_only_their_context_and_unknown_contexts_are_rejected() {
    let (c, errors) = Catalog::parse("\tTitle\tЗаголовок\npreview\tTitle\tТитул\n\tFont\tШрифт\nnope\tFont\tX\n");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("unknown context"));
    assert_eq!(c.plain("Title"), Some("Заголовок"));
    assert_eq!(c.in_context("preview", "Title"), Some("Титул"));
    // Without a context entry, a context lookup uses the plain one.
    assert_eq!(c.in_context("preview", "Font"), Some("Шрифт"));
    assert_eq!(c.in_context("preview", "Missing"), None);
    let ru = lang("ru");
    assert_eq!(tr(ru, "Title"), "Заголовок");
    assert_eq!(trc(ru, "preview", "Title"), "Пример");
    assert_eq!(trc(Lang::EN, "preview", "Title"), "Title");
    assert_eq!(trc(lang("ja"), "preview", "Unknown"), "Unknown");
}

#[test]
fn estonian_locales_and_saved_preference_keep_document_content() {
    let et = lang("et");
    for tag in ["et", "et-EE", "ET_ee.UTF-8", "et-Latn-EE", "et_EE.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(et), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "et-EE", "en-US"]), Some(et));
    assert_eq!(normalize_pref("ET"), Some("et"));
    assert_eq!(et.name(), "Eesti");
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "ET"})).unwrap();
    assert_eq!(result["effective"], "et");
    assert_eq!(app.ui.language, "et");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), et);
}

#[test]
fn estonian_covers_the_interface_catalog_and_keeps_count_labels_neutral() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let et = lang("et");
    // Estonian carries the interface catalog; Spanish and Turkish add the equation gallery and
    // built-in style names on top.
    assert!(keys(et.0.source).is_subset(&keys(lang("es").0.source)));
    assert!(keys(et.0.source).is_subset(&keys(lang("tr").0.source)));
    assert!(keys(lang("zh-hans").0.source).is_subset(&keys(et.0.source)));
    assert_eq!(tr(et, "Home"), "Avaleht");
    assert_eq!(tr(et, "Save"), "Salvesta");
    assert_eq!(tr(et, "Spelling & Grammar"), "Õigekiri ja grammatika");
    assert_eq!(tr(et, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(et, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Eksporditud: draft-{words}.docx");
    for count in [0, 1, 2, 11, 21, 101] {
        let words = count.to_string();
        assert_eq!(fmt(tr(et, "{words} words"), &[("words", &words)]), format!("Sõnu: {count}"));
        assert_eq!(fmt(tr(et, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Sõnu: {count}; valitud: 1"));
    }
}

#[test]
fn bundled_interface_fonts_cover_estonian_letters() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in "ÕõÄäÖöÜüŠšŽž".chars() {
            assert_ne!(face.glyph_for(ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn turkish_locales_and_saved_preference_work_without_changing_the_document() {
    let tr = lang("tr");
    for tag in ["tr", "tr-TR", "TR_tr.UTF-8", "tr-CY", "tr-Latn-TR", "tr_TR.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(tr), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "tr-TR", "en-US"]), Some(tr));
    assert_eq!(normalize_pref("TR"), Some("tr"));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "TR"})).unwrap();
    assert_eq!(result["effective"], "tr");
    assert_eq!(app.ui.language, "tr");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "tr" && l["name"] == "Türkçe"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), tr);
    assert!(app.run("ui.language", serde_json::json!({"value": "tr-unknown"})).is_err());
    assert_eq!(app.ui.language, "tr", "invalid preferences must keep the saved language");
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
    assert_eq!(tr.name(), "Türkçe");
}

#[test]
fn turkish_covers_the_entire_existing_interface_catalog_and_preserves_templates() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Full parity with the Russian catalog: covers all UI strings, styles, math gallery, and presets.
    assert_eq!(keys(lang("tr").0.source), keys(lang("ru").0.source));
    assert!(keys(lang("es").0.source).is_subset(&keys(lang("tr").0.source)));
    assert!(keys(lang("zh-hans").0.source).is_subset(&keys(lang("tr").0.source)));
    let tr = lang("tr");
    for (source, expected) in [
        ("Home", "Giriş"),
        ("Font", "Yazı Tipi"),
        ("Review", "İncele"),
        ("Italic", "İtalik"),
        ("Table of Contents", "İçindekiler Tablosu"),
        ("Track Changes", "Değişiklikleri İzle"),
        ("Save", "Kaydet"),
        ("UPPERCASE", "BÜYÜK HARF"),
        ("lowercase", "küçük harf"),
        ("Square Root", "Karekök"),
        ("Euler's identity", "Euler Özdeşliği"),
        ("Default Paragraph Font", "Varsayılan Paragraf Yazı Tipi"),
        ("CONFIDENTIAL", "GİZLİ"),
        ("WordCraft", "WordCraft"),
        ("unknown future label", "unknown future label"),
    ] {
        assert_eq!(super::tr(tr, source), expected);
    }
    assert_eq!(fmt(super::tr(tr, "Exported {path}"), &[("path", "taslak-{words}.docx")]), "Dışa aktarıldı: taslak-{words}.docx");
    assert_eq!(fmt(super::tr(tr, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Sayfa 3 / 9");
    // The Font dialog uppercases its preview sample for All Caps, so the sample has no lowercase i.
    assert_eq!(super::tr(tr, "AaBbYyZz").to_uppercase(), "AABBYYZZ ĞĞİIŞŞ");
    // Turkish count nouns remain singular for both one and other, including zero and large counts.
    for count in [0, 1, 2, 5, 11, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(super::tr(tr, "{words} words"), &[("words", &words)]), format!("{count} sözcük"));
        assert_eq!(fmt(super::tr(tr, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("{count} sözcükten 1 seçili"));
    }
    set_current(tr);
    assert_eq!(location("Home › Font"), "Giriş › Yazı Tipi");
    assert_eq!(prefixed("Undo", "Italic"), "Geri Al: İtalik");
    set_current(Lang::EN);
    assert_eq!(super::tr(Lang::EN, "Italic"), "Italic");
}

#[test]
fn bundled_interface_fonts_cover_turkish_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("tr").0.source);
    assert!(errors.is_empty());
    let extra = "ÇçĞğİıÖöŞşÜü";
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| extra.contains(*c)).chain(extra.chars()).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
    assert!(!lang("tr").uses_cjk());
}

#[test]
fn hungarian_locales_and_saved_preference_work_without_changing_the_document() {
    let hu = lang("hu");
    for tag in ["hu", "hu-HU", "hu_HU.UTF-8", "HU_hu.UTF-8@euro", "hu-Latn-HU"] {
        assert_eq!(lang_from_tag(tag), Some(hu), "{tag}");
    }
    assert_eq!(first_supported(["fi-FI", "hu-HU", "en-US"]), Some(hu));
    assert_eq!(normalize_pref("HU"), Some("hu"));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "hu"})).unwrap();
    assert_eq!(result["effective"], "hu");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "hu" && l["name"] == "Magyar"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), hu);
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
}

#[test]
fn hungarian_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Every key of the most complete catalog: built-in style names, equation gallery labels, paper
    // sizes and watermarks included.
    assert!(keys(lang("hu").0.source).is_superset(&keys(lang("ru").0.source)));
    assert!(keys(lang("hu").0.source).is_superset(&keys(lang("es").0.source)));
    let hu = lang("hu");
    for (source, expected) in [
        ("Home", "Kezdőlap"),
        ("Font", "Betűtípus"),
        ("Review", "Véleményezés"),
        ("Header", "Élőfej"),
        ("Track Changes", "Változáskövetés"),
        ("Save", "Mentés"),
        ("Redo", "Újra"),
        ("UPPERCASE", "NAGYBETŰS"),
        ("WordCraft", "WordCraft"),
        ("unknown future label", "unknown future label"),
    ] {
        assert_eq!(super::tr(hu, source), expected);
    }
    assert_eq!(fmt(super::tr(hu, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Oldal: 3/9");
    // A noun after a number stays singular in Hungarian, whatever the count.
    for count in [0, 1, 2, 21, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(super::tr(hu, "{words} words"), &[("words", &words)]), format!("{count} szó"));
    }
    // The Font dialog uppercases its preview sample for All Caps.
    assert_eq!(super::tr(hu, "AaBbYyZz").to_uppercase(), "AABBYYZZ ŐŐŰŰ");
    set_current(hu);
    // Built-in styles show translated names; numbered families go through their template.
    for (style, expected) in [("Normal", "Normál"), ("No Spacing", "Nincs térköz"), ("Heading 2", "Címsor 2"), ("TOC 3", "Tartalomjegyzék 3")] {
        assert_eq!(builtin_style_name(style), expected);
    }
    assert_eq!(builtin_style_name("My Own Style"), "My Own Style");
    assert_eq!(location("Home › Font"), "Kezdőlap › Betűtípus");
    assert_eq!(prefixed("Undo", "Bold"), "Visszavonás: Félkövér");
    set_current(Lang::EN);
}

#[test]
fn bundled_interface_fonts_cover_hungarian_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("hu").0.source);
    assert!(errors.is_empty());
    let extra = "ÁáÉéÍíÓóÖöŐőÚúÜüŰű";
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| extra.contains(*c)).chain(extra.chars()).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
    assert!(!lang("hu").uses_cjk());
}

#[test]
fn french_locales_and_saved_preference_work_without_changing_the_document() {
    let fr = lang("fr");
    for tag in ["fr", "fr-FR", "FR_fr.UTF-8", "fr-CA", "fr_BE.UTF-8@euro", "fr-CH", "fr-Latn-FR"] {
        assert_eq!(lang_from_tag(tag), Some(fr), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "fr-CA", "en-US"]), Some(fr));
    assert_eq!(normalize_pref("FR"), Some("fr"));
    assert_eq!(fr.name(), "Français");
    assert_eq!(tr(fr, "Home"), "Accueil");
    assert_eq!(tr(fr, "Save"), "Enregistrer");
    assert_eq!(fmt(tr(fr, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Page 3 sur 9");
    assert_eq!(tr(fr, "unknown future label"), "unknown future label");
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "FR"})).unwrap();
    assert_eq!(result["effective"], "fr");
    assert_eq!(app.ui.language, "fr");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "fr" && l["name"] == "Français"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), fr);
    assert!(app.run("ui.language", serde_json::json!({"value": "fr-unknown"})).is_err());
    assert_eq!(app.ui.language, "fr", "invalid preferences must keep the saved language");
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
}

#[test]
fn bundled_interface_fonts_cover_french_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("fr").0.source);
    assert!(errors.is_empty());
    // French letters and punctuation: accents, ligatures, guillemets and the typographic apostrophe.
    let extra = "ÀàÂâÆæÇçÉéÈèÊêËëÎîÏïÔôŒœÙùÛûÜüŸÿ«»’";
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| ('\u{00C0}'..='\u{017F}').contains(c)).chain(extra.chars()).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch:?}");
        }
    }
    // The no-break spaces French puts before ; : ! ? and inside « » (the catalog uses plain
    // spaces today) are in the interface faces too.
    for style in ["Regular", "Medium", "SemiBold"] {
        let face = db.face("Inter", style);
        for ch in ['\u{00A0}', '\u{202F}'] {
            assert_ne!(face.glyph_for(ch), 0, "Inter {style} lacks {ch:?}");
        }
    }
    assert!(!lang("fr").uses_cjk());
}

#[test]
fn polish_locales_and_saved_preference_work_without_changing_the_document() {
    let pl = lang("pl");
    for tag in ["pl", "pl-PL", "pl_PL.UTF-8", "PL-pl", "pl_PL.UTF-8@euro", "pl-Latn-PL"] {
        assert_eq!(lang_from_tag(tag), Some(pl), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "pl-PL", "en-US"]), Some(pl));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "PL"})).unwrap();
    assert_eq!(result["effective"], "pl");
    assert_eq!(app.ui.language, "pl");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), pl);
    assert_eq!(pl.name(), "Polski");
}

#[test]
fn polish_covers_every_key_of_every_other_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let polish = keys(lang("pl").0.source);
    // Italian and Hungarian still carry "Style: APA", which the interface no longer shows;
    // Hungarian and Bulgarian also name the citation styles, and Bulgarian has a few one-line
    // strings of its own. Swedish, Croatian, Vietnamese and French carry newer strings (paper
    // sizes, bookmarks, printing) the Polish catalog hasn't caught up with yet.
    for other in Lang::all().filter(|l| *l != Lang::EN && !matches!(l.code(), "pl" | "it" | "hu" | "bg" | "vi" | "sv" | "hr" | "fr")) {
        let mut missing: Vec<String> = keys(other.0.source).difference(&polish).cloned().collect();
        missing.sort();
        assert!(missing.is_empty(), "pl lacks {}'s {missing:?}", other.code());
    }
    let pl = lang("pl");
    assert_eq!(tr(pl, "File"), "Plik");
    assert_eq!(tr(pl, "Home"), "Start");
    assert_eq!(tr(pl, "Insert"), "Wstaw");
    assert_eq!(tr(pl, "Review"), "Recenzja");
    assert_eq!(tr(pl, "Save"), "Zapisz");
    assert_eq!(tr(pl, "Spelling & Grammar"), "Pisownia i gramatyka");
    assert_eq!(tr(pl, "Polish"), "Polski");
    assert_eq!(tr(pl, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(pl, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Wyeksportowano: draft-{words}.docx");
    assert_eq!(fmt(tr(pl, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Strona 3 z 9");
    // Count-neutral wording reads grammatically for every Polish number form (1, 2–4, 5+, 12–14, 22).
    for count in [0, 1, 2, 4, 5, 12, 14, 22, 25, 101, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(pl, "{words} words"), &[("words", &words)]), format!("Słowa: {count}"));
        assert_eq!(fmt(tr(pl, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Słowa: 1 z {count}"));
    }
    set_current(pl);
    assert_eq!(location("Review › Proofing"), "Recenzja › Sprawdzanie");
    assert_eq!(prefixed("Undo", "Bold"), "Cofnij: Pogrubienie");
    set_current(Lang::EN);
}

#[test]
fn bundled_interface_fonts_cover_polish_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("pl").0.source);
    assert!(errors.is_empty(), "{errors:?}");
    // Polish letters beyond ASCII, both cases, and the Polish quotation marks.
    let polish = "ĄąĆćĘęŁłŃńÓóŚśŹźŻż„”";
    let chars: std::collections::HashSet<char> =
        entries.iter().flat_map(|e| e.translation.chars()).filter(|c| polish.contains(*c)).chain(polish.chars()).collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn a_polish_interface_writes_new_documents_in_polish() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let ctx = egui::Context::default();
    let frame = |app: &mut crate::WordApp| ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
    frame(&mut app);
    assert_eq!(app.session.editing_lang, None, "English interface: WordCraft's default, US English");
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("en-US"));
    app.run("ui.language", serde_json::json!({"value": "pl"})).unwrap();
    frame(&mut app);
    assert_eq!(app.session.editing_lang.as_deref(), Some("pl-PL"));
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"), "the untouched first document follows");
    assert!(!app.session.dirty);
    app.run("file.new", serde_json::json!({})).unwrap();
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"));
    // Typing Polish then shows no spelling marks; the status bar names the language.
    app.session.run("text.insert", &serde_json::json!({"text": "Zażółć gęślą jaźń"})).unwrap();
    assert_eq!(app.session.run("review.issues", &serde_json::json!({})).unwrap(), serde_json::json!([]));
    set_current(lang("pl"));
    assert_eq!(crate::ribbon::caret_language(&app), "Polski");
    set_current(Lang::EN);
    assert_eq!(crate::ribbon::caret_language(&app), "Polish");
    // Back to English: documents already written keep their language.
    app.run("ui.language", serde_json::json!({"value": "en"})).unwrap();
    frame(&mut app);
    assert_eq!(app.session.editing_lang, None);
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("pl-PL"));
}

#[test]
fn a_czech_interface_writes_new_documents_and_plain_text_in_czech() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let ctx = egui::Context::default();
    let frame = |app: &mut crate::WordApp| ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
    app.run("ui.language", serde_json::json!({"value": "cs"})).unwrap();
    frame(&mut app);
    assert_eq!(app.session.editing_lang.as_deref(), Some("cs-CZ"));
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("cs-CZ"));
    assert!(!app.session.dirty);
    app.run("file.new", serde_json::json!({})).unwrap();
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("cs-CZ"));
    set_current(lang("cs"));
    assert_eq!(crate::ribbon::caret_language(&app), "Čeština");
    set_current(Lang::EN);
    assert_eq!(crate::ribbon::caret_language(&app), "Czech");
    for path in ["poznamka.txt", "poznamka.md"] {
        // Base64 of our synthetic text, "Město"; file.open takes bytes without a dialog.
        app.session.run("file.open", &serde_json::json!({"path": path, "data": "TcSbc3Rv"})).unwrap();
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("cs-CZ"), "{path}");
    }
    app.run("ui.language", serde_json::json!({"value": "en"})).unwrap();
    frame(&mut app);
    assert_eq!(app.session.editing_lang, None);
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("cs-CZ"), "opened content keeps its language");
}

#[test]
fn an_italian_interface_writes_new_documents_in_italian() {
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let ctx = egui::Context::default();
    let frame = |app: &mut crate::WordApp| ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
    frame(&mut app);
    app.run("ui.language", serde_json::json!({"value": "it"})).unwrap();
    frame(&mut app);
    assert_eq!(app.session.editing_lang.as_deref(), Some("it-IT"));
    assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("it-IT"), "the untouched first document follows");
    // Typing Italian then shows no English misspellings; the status bar names the language.
    app.session.run("text.insert", &serde_json::json!({"text": "Città vecchie e caffè"})).unwrap();
    let issues = app.session.run("review.issues", &serde_json::json!({})).unwrap();
    assert!(issues.as_array().unwrap().iter().all(|i| i["lang"] == "it"), "{issues}");
    set_current(lang("it"));
    assert_eq!(crate::ribbon::caret_language(&app), "Italiano");
    set_current(Lang::EN);
    assert_eq!(crate::ribbon::caret_language(&app), "Italian");
}

#[test]
fn the_editing_languages_are_the_ones_wordcraft_can_proof() {
    let with: Vec<(&str, &str)> = Lang::all().filter_map(|l| l.editing_lang().map(|e| (l.code(), e))).collect();
    assert_eq!(with, [("pt-br", "pt-BR"), ("pl", "pl-PL"), ("cs", "cs-CZ"), ("tr", "tr-TR"), ("it", "it-IT")]);
}

#[test]
fn german_locales_and_saved_preference_keep_document_content() {
    let de = lang("de");
    for tag in ["de", "de-DE", "de-AT", "de-CH", "DE_de.UTF-8", "de_DE.UTF-8@euro", "de-Latn-DE"] {
        assert_eq!(lang_from_tag(tag), Some(de), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "de-AT", "en-US"]), Some(de));
    assert_eq!(normalize_pref("DE"), Some("de"));
    assert_eq!(de.name(), "Deutsch");
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "DE"})).unwrap();
    assert_eq!(result["effective"], "de");
    assert_eq!(app.ui.language, "de");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), de);
}

#[test]
fn german_covers_the_interface_catalog_and_keeps_count_labels_neutral() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let de = lang("de");
    assert_eq!(keys(de.0.source), keys(lang("zh-hans").0.source));
    assert_eq!(tr(de, "Home"), "Start");
    assert_eq!(tr(de, "Save"), "Speichern");
    assert_eq!(tr(de, "Spelling & Grammar"), "Rechtschreibung und Grammatik");
    assert_eq!(tr(de, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(de, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Exportiert: draft-{words}.docx");
    // Count-neutral wording is grammatical for every count: `Wörter: 21`, `Wörter: 21; ausgewählt: 1`.
    for count in [0, 1, 2, 11, 21, 101] {
        let words = count.to_string();
        assert_eq!(fmt(tr(de, "{words} words"), &[("words", &words)]), format!("Wörter: {count}"));
        assert_eq!(fmt(tr(de, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Wörter: {count}; ausgewählt: 1"));
    }
}

#[test]
fn bundled_interface_fonts_cover_german_letters() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in "ÄäÖöÜüß".chars() {
            assert_ne!(face.glyph_for(ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn italian_locales_and_saved_preference_keep_document_content() {
    let it = lang("it");
    for tag in ["it", "it-IT", "it_IT.UTF-8", "it-CH", "it_IT.UTF-8@euro"] {
        assert_eq!(lang_from_tag(tag), Some(it), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "it-IT", "en-US"]), Some(it));
    assert_eq!(normalize_pref("IT"), Some("it"));
    assert_eq!(it.name(), "Italiano");
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "IT"})).unwrap();
    assert_eq!(result["effective"], "it");
    assert_eq!(app.ui.language, "it");
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), it);
}

#[test]
fn italian_covers_the_entire_existing_interface_catalog_and_keeps_count_labels_neutral() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let it = lang("it");
    for other in ["es", "tr", "et", "zh-hans"] {
        assert!(keys(lang(other).0.source).is_subset(&keys(it.0.source)), "{other} has keys Italian lacks");
    }
    assert_eq!(tr(it, "Good morning"), "Buongiorno");
    assert_eq!(tr(it, "Save"), "Salva");
    assert_eq!(tr(it, "Insert"), "Inserisci");
    assert_eq!(tr(it, "Spelling & Grammar"), "Ortografia e grammatica");
    assert_eq!(tr(it, "unknown future label"), "unknown future label");
    // The Review tab's group and its toggle keep different names.
    assert_ne!(tr(it, "Changes"), tr(it, "Track Changes"));
    // Italian nouns agree with the number, so count labels put the noun first and read right at 1.
    for count in [0, 1, 2, 11, 21, 101] {
        let n = count.to_string();
        assert_eq!(fmt(tr(it, "{words} words"), &[("words", &n)]), format!("Parole: {count}"));
        assert_eq!(fmt(tr(it, "{selected} of {words} words"), &[("selected", "1"), ("words", &n)]), format!("Parole: {count}; selezionate: 1"));
        assert_eq!(fmt(tr(it, "{n} lines"), &[("n", &n)]), format!("Righe: {count}"));
        assert_eq!(fmt(tr(it, "{count} items highlighted"), &[("count", &n)]), format!("Elementi evidenziati: {count}"));
    }
}

#[test]
fn bundled_interface_fonts_cover_italian_letters() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in "ÀàÈèÉéÌìÒòÙù".chars() {
            assert_ne!(face.glyph_for(ch), 0, "{family} {style} lacks {ch}");
        }
    }
}

#[test]
fn bulgarian_locales_and_saved_preference_work_without_changing_the_document() {
    let bg = lang("bg");
    for tag in ["bg", "bg-BG", "bg_BG.UTF-8", "BG-bg", "bg-Cyrl-BG"] {
        assert_eq!(lang_from_tag(tag), Some(bg), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "bg-BG", "en-US"]), Some(bg));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "BG"})).unwrap();
    assert_eq!(result["effective"], "bg");
    assert_eq!(app.ui.language, "bg");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "bg" && l["name"] == "Български"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), bg);
    assert_eq!(bg.name(), "Български");
}

#[test]
fn bulgarian_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Every key the interface looks up (Spanish carries them all), and no catalog errors.
    assert!(keys(lang("bg").0.source).is_superset(&keys(lang("es").0.source)));
    assert!(parse_entries(lang("bg").0.source).1.is_empty());
    let bg = lang("bg");
    for (source, expected) in [
        ("Home", "Начало"),
        ("Insert", "Вмъкване"),
        ("Font", "Шрифт"),
        ("Paragraph", "Абзац"),
        ("Review", "Преглед"),
        ("Save", "Запазване"),
        ("Heading 1", "Заглавие 1"),
        ("Hyphenation", "Сричкопренасяне"),
        ("Track Changes", "Проследяване на промените"),
        ("WordCraft", "WordCraft"),
        ("unknown future label", "unknown future label"),
    ] {
        assert_eq!(tr(bg, source), expected, "{source}");
    }
    // The search hint's example must find the command it names.
    assert_eq!(tr(bg, "Insert Table"), "Вмъкване на таблица");
    assert!(tr(bg, "Type a command, e.g. \"insert table\"").contains("вмъкване на таблица"));
    assert_eq!(fmt(tr(bg, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Страница 3 от 9");
    // Count-neutral wording is grammatical for every count: `Думи: 21`, `Думи: 1 от 21`.
    for count in [0, 1, 2, 5, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(bg, "{words} words"), &[("words", &words)]), format!("Думи: {count}"));
        assert_eq!(fmt(tr(bg, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Думи: 1 от {count}"));
    }
}

#[test]
fn bundled_interface_fonts_cover_bulgarian_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("bg").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        // Cyrillic letters plus the Bulgarian quotes and dash; symbols come from the English sources.
        .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c) || matches!(c, '„' | '“' | '—' | '–'))
        .chain("АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЬЮЯабвгдежзийклмнопрстуфхцчшщъьюя".chars())
        .collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
    assert!(!lang("bg").uses_cjk());
}

#[test]
fn croatian_locales_and_saved_preference_work_without_changing_the_document() {
    let hr = lang("hr");
    for tag in ["hr", "hr-HR", "hr_HR.UTF-8", "HR-hr", "hr-Latn-HR", "hr-BA"] {
        assert_eq!(lang_from_tag(tag), Some(hr), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "hr-HR", "en-US"]), Some(hr));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "HR"})).unwrap();
    assert_eq!(result["effective"], "hr");
    assert_eq!(app.ui.language, "hr");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "hr" && l["name"] == "Hrvatski"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), hr);
    assert_eq!(hr.name(), "Hrvatski");
}

#[test]
fn croatian_covers_the_entire_existing_interface_catalog_and_keeps_count_labels_neutral() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    let hr = lang("hr");
    // Every key of every other catalog, and no catalog errors.
    for other in Lang::all().filter(|l| *l != Lang::EN && l.code() != "hr") {
        let mut missing: Vec<String> = keys(other.0.source).difference(&keys(hr.0.source)).cloned().collect();
        missing.sort();
        // Italian and Hungarian still carry "Style: APA", which the interface no longer shows.
        missing.retain(|k| k != "Style: APA" || !matches!(other.code(), "it" | "hu"));
        assert!(missing.is_empty(), "hr lacks {}'s {missing:?}", other.code());
    }
    assert!(parse_entries(hr.0.source).1.is_empty());
    for (source, expected) in [
        ("File", "Datoteka"),
        ("Home", "Početna"),
        ("Insert", "Umetni"),
        ("Review", "Pregled"),
        ("Save", "Spremi"),
        ("Heading 1", "Naslov 1"),
        ("Spelling & Grammar", "Pravopis i gramatika"),
        ("Hyphenation", "Rastavljanje riječi"),
        ("WordCraft", "WordCraft"),
        ("unknown future label", "unknown future label"),
    ] {
        assert_eq!(tr(hr, source), expected, "{source}");
    }
    assert_eq!(fmt(tr(hr, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Stranica 3 od 9");
    // The letter template is "Pismo"; the paper size of the same English name stays "Letter".
    assert_eq!(trc(hr, "template", "Letter"), "Pismo");
    assert_eq!(tr(hr, "Letter"), "Letter");
    // Croatian nouns take three forms after numerals (1, 2–4, 5+), so counts read `Riječi: 21`.
    for count in [0, 1, 2, 4, 5, 12, 14, 21, 22, 25, 101, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(hr, "{words} words"), &[("words", &words)]), format!("Riječi: {count}"));
        assert_eq!(fmt(tr(hr, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Riječi: 1 od {count}"));
    }
    set_current(hr);
    assert_eq!(location("Review › Proofing"), "Pregled › Jezična provjera");
    set_current(Lang::EN);
}

#[test]
fn bundled_interface_fonts_cover_croatian_without_system_fallbacks() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("hr").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        // Croatian letters plus its quotes and dashes; symbols come from the English sources.
        .filter(|c| matches!(c, '„' | '”' | '–'))
        .chain("ČčĆćĐđŠšŽž".chars())
        .collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
    assert!(!lang("hr").uses_cjk());
}

#[test]
fn swedish_locales_and_saved_preference_work_without_changing_the_document() {
    let sv = lang("sv");
    for tag in ["sv", "sv-SE", "sv_SE.UTF-8", "SV-se", "sv-FI"] {
        assert_eq!(lang_from_tag(tag), Some(sv), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "sv-SE", "en-US"]), Some(sv));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "SV"})).unwrap();
    assert_eq!(result["effective"], "sv");
    assert_eq!(app.ui.language, "sv");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "sv" && l["name"] == "Svenska"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(Lang::from_pref(&restored.language), sv);
    assert_eq!(sv.name(), "Svenska");
}

#[test]
fn swedish_covers_the_entire_existing_interface_catalog() {
    use std::collections::HashSet;
    let keys = |source| parse_entries(source).0.into_iter().map(|e| e.source).collect::<HashSet<_>>();
    // Every key the interface looks up (Spanish carries them all), and no catalog errors.
    assert!(keys(lang("sv").0.source).is_superset(&keys(lang("es").0.source)));
    assert!(parse_entries(lang("sv").0.source).1.is_empty());
    let sv = lang("sv");
    for (source, expected) in [
        ("File", "Arkiv"),
        ("Home", "Start"),
        ("Insert", "Infoga"),
        ("Font", "Typsnitt"),
        ("Paragraph", "Stycke"),
        ("Review", "Granska"),
        ("Save", "Spara"),
        ("Heading 1", "Rubrik 1"),
        ("Hyphenation", "Avstavning"),
        ("Track Changes", "Spåra ändringar"),
        ("WordCraft", "WordCraft"),
        ("unknown future label", "unknown future label"),
    ] {
        assert_eq!(tr(sv, source), expected, "{source}");
    }
    // The search hint's example must find the command it names.
    assert_eq!(tr(sv, "Insert Table"), "Infoga tabell");
    assert!(tr(sv, "Type a command, e.g. \"insert table\"").contains(&tr(sv, "Insert Table").to_lowercase()));
    assert_eq!(fmt(tr(sv, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]), "Sida 3 av 9");
    // `ord` is the same in the singular and the plural, so the counts read right for every number.
    for count in [0, 1, 2, 5, 21, 100, 1000] {
        let words = count.to_string();
        assert_eq!(fmt(tr(sv, "{words} words"), &[("words", &words)]), format!("{count} ord"));
        assert_eq!(fmt(tr(sv, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("1 av {count} ord"));
    }
}

#[test]
fn bundled_interface_fonts_cover_swedish_letters() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("sv").0.source);
    assert!(errors.is_empty());
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        // Latin letters with diacritics plus the Swedish quote and dash; symbols come from the English sources.
        .filter(|c| ('\u{00C0}'..='\u{00FF}').contains(c) || matches!(c, '”' | '–'))
        .chain("ÅÄÖåäöÉé".chars())
        .collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch}");
        }
    }
    assert!(!lang("sv").uses_cjk());
}

#[test]
fn vietnamese_locales_and_saved_preference_work_without_changing_the_document() {
    let vi = lang("vi");
    for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vi", "vi-Latn-VN", "vi-US"] {
        assert_eq!(lang_from_tag(tag), Some(vi), "{tag}");
    }
    assert_eq!(first_supported([UNSUPPORTED, "vi-VN", "en-US"]), Some(vi));
    let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document()), Default::default());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let result = app.run("ui.language", serde_json::json!({"value": "VI"})).unwrap();
    assert_eq!(result["effective"], "vi");
    assert_eq!(app.ui.language, "vi");
    assert!(result["available"].as_array().unwrap().iter().any(|l| l["code"] == "vi" && l["name"] == "Tiếng Việt"));
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.language, "vi");
    assert_eq!(Lang::from_pref(&restored.language), vi);
    assert_eq!(vi.name(), "Tiếng Việt");
}

#[test]
fn vietnamese_covers_existing_catalog_keys_with_an_explicit_preview_fallback() {
    use std::collections::HashSet;
    let vi = lang("vi");
    let (entries, errors) = parse_entries(vi.0.source);
    assert!(errors.is_empty(), "{errors:?}");
    let vi_keys: HashSet<_> = entries.into_iter().map(|e| (e.context, e.source)).collect();
    // The short plain title fits the preview too; this is the only deliberate context fallback.
    let preview_title = ("preview".to_string(), "Title".to_string());
    assert!(vi_keys.contains(&(String::new(), "Title".to_string())));
    assert_eq!(trc(vi, "preview", "Title"), "Tiêu đề");
    assert_eq!(trc(vi, "preview", "Title"), tr(vi, "Title"));
    // Compare complete (context, source) keys so a contextual entry cannot hide a missing plain
    // translation, or silently satisfy another context's key.
    for other in Lang::all().filter(|l| *l != Lang::EN && l.code() != "vi") {
        let (entries, errors) = parse_entries(other.0.source);
        assert!(errors.is_empty(), "{}: {errors:?}", other.code());
        let keys: HashSet<_> = entries.into_iter().map(|e| (e.context, e.source)).collect();
        let mut missing: Vec<_> = keys.difference(&vi_keys).filter(|key| **key != preview_title).cloned().collect();
        missing.sort();
        assert!(missing.is_empty(), "vi lacks {}'s {missing:?}", other.code());
    }
}

#[test]
fn vietnamese_translations_preserve_templates_and_count_labels() {
    let vi = lang("vi");
    assert_eq!(tr(vi, "Home"), "Trang chủ");
    assert_eq!(tr(vi, "Save"), "Lưu");
    assert_eq!(tr(vi, "Spelling & Grammar"), "Chính tả & Ngữ pháp");
    assert_eq!(tr(vi, "unknown future label"), "unknown future label");
    assert_eq!(fmt(tr(vi, "Exported {path}"), &[("path", "draft-{words}.docx")]), "Đã xuất draft-{words}.docx");
    // Count-neutral wording (`Từ: 21`): a Vietnamese noun stays unchanged after a number.
    for count in [0, 1, 2, 11, 21, 101] {
        let words = count.to_string();
        assert_eq!(fmt(tr(vi, "{words} words"), &[("words", &words)]), format!("Từ: {count}"));
        assert_eq!(fmt(tr(vi, "{selected} of {words} words"), &[("selected", "1"), ("words", &words)]), format!("Từ: 1 trên {count}"));
    }
}

#[test]
fn bundled_interface_fonts_cover_vietnamese_letters() {
    let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
    let (entries, errors) = parse_entries(lang("vi").0.source);
    assert!(errors.is_empty(), "{errors:?}");
    // Cover the catalog's Latin letters and combining marks, and all 134 non-ASCII Vietnamese
    // letters even when a letter is not used by the current translations. U+1EA0..U+1EF9 contains
    // the Vietnamese vowel/tone combinations; the other forms are in earlier Latin blocks.
    let extra = "ÀàÁáÃãÈèÉéÌìÍíÒòÓóÕõÙùÚúÝýÂâĂăĐđÊêÔôƠơƯưĨĩŨũ";
    let chars: std::collections::HashSet<char> = entries
        .iter()
        .flat_map(|e| e.translation.chars())
        .filter(|c| ('\u{00C0}'..='\u{024F}').contains(c) || ('\u{0300}'..='\u{036F}').contains(c) || ('\u{1E00}'..='\u{1EFF}').contains(c))
        .chain(extra.chars())
        .chain('\u{1EA0}'..='\u{1EF9}')
        .collect();
    for (family, style) in [("Inter", "Regular"), ("Inter", "Medium"), ("Inter", "SemiBold"), ("JetBrains Mono", "Regular")] {
        let face = db.face(family, style);
        assert_eq!(face.family, family, "must use the bundled interface face");
        for ch in &chars {
            assert_ne!(face.glyph_for(*ch), 0, "{family} {style} lacks {ch:?}");
        }
    }
    assert!(!lang("vi").uses_cjk());
}
