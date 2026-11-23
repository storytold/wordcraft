//! Interface language tests (#8).

use craft::ui::fonts::{Sample, assert_covers};

use super::*;

fn lang(code: &str) -> Lang {
    Lang::from_code(code).unwrap_or_else(|| panic!("{code} registered"))
}

/// Skips a test that needs the embedded catalogs when none were embedded (`CRAFT_I18N_DIR`
/// unset): the interface is then English only.
macro_rules! needs_catalogs {
    () => {
        if !catalogs_embedded() {
            return;
        }
    };
}

/// A language tag no catalog will ever have: `qaa`–`qtz` are reserved for local use (ISO 639-2),
/// so these tests keep working as real languages land.
const UNSUPPORTED: &str = "qaa-QM";

/// A new app over `doc`, and a frame that runs its logic the way the window does.
fn new_app(doc: wordcraft_doc::Document) -> crate::WordApp {
    crate::WordApp::new(wordcraft_engine::Session::new(doc), Default::default())
}

/// One frame as the shell gives it: the input the app hands the runtime first (it binds the
/// interface language, `WordApp::frame_input`), then the frame's logic.
fn frame(ctx: &egui::Context, app: &mut crate::WordApp) {
    let _ = app.frame_input(craft::ui::runtime::frame::FrameOutput::default());
    ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx())).drop_without_applying_deltas();
}

/// WordCraft's own rules over the system's locale tags: Portuguese other than Brazil's stays
/// English, and the first supported tag of the user's list wins. Matching a tag to a locale is
/// craft-i18n's.
#[test]
fn system_locales_map_to_languages() {
    needs_catalogs!();
    let cases = [
        ("pt-BR", Some(lang("pt-br"))),
        ("pt_BR.UTF-8", Some(lang("pt-br"))),
        ("pt", None),
        ("pt-PT", None),
        ("pt_AO.UTF-8", None),
        ("fr-FR", Some(lang("fr"))),
        ("C", Some(Lang::EN)),
        (UNSUPPORTED, None),
        ("", None),
    ];
    for (tag, expected) in cases {
        assert_eq!(lang_from_tag(tag), expected, "locale {tag}");
    }
    assert_eq!(first_supported(["pt-PT", "fr-FR"]), Some(lang("fr")), "European Portuguese is skipped, not matched to Brazil's");
    assert_eq!(first_supported([UNSUPPORTED, "de-DE"]), Some(lang("de")));
    assert_eq!(first_supported([]), None);
    // `auto` follows the system, which is English under test.
    assert_eq!(Lang::from_pref(AUTO), system_lang());
}

#[test]
fn settings_resolve_with_fallback() {
    needs_catalogs!();
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

/// The parallel test threads must not change each other's language: the language view is per thread.
#[test]
fn current_language_is_per_thread() {
    let ctx = egui::Context::default();
    fixture_host(
        &ctx,
        "x-fixture",
        "\tInsert\tInsert-FX\n\tHome\tHome-FX\n\tFont\tFont-FX\n\tUndo\tUndo-FX\n\tUndo {action}\tUndo-FX: {action}\n\tBold\tBold-FX\n",
    );
    assert_eq!(t("Insert"), "Insert-FX");
    assert_eq!(location("Home › Font"), "Home-FX › Font-FX");
    assert_eq!(prefixed("Undo", "Bold"), "Undo-FX: Bold-FX");
    assert_eq!(prefixed("Undo", ""), "Undo-FX");
    // An action without a translation keeps its English inside the translated template.
    assert_eq!(prefixed("Undo", "Typing"), "Undo-FX: Typing");
    std::thread::spawn(|| assert_eq!(current(), Lang::EN)).join().unwrap();
    apply_language(Lang::EN);
    assert_eq!(t("Insert"), "Insert");
    assert_eq!(prefixed("Undo", "Typing"), "Undo Typing");
}

#[test]
fn the_language_setting_is_a_ui_command_and_persists() {
    needs_catalogs!();
    let ctx = egui::Context::default();
    embedded_host(&ctx);
    let mut app = new_app(wordcraft_doc::Document::new());
    assert_eq!(app.ui.language, AUTO);
    let r = app.run("ui.language", serde_json::json!({"value": "ZH-HANT"})).unwrap();
    assert_eq!(app.ui.language, "zh-hant");
    assert_eq!(r["effective"], "zh-hant");
    assert!(r["available"].as_array().is_some_and(|a| a.len() == LANGUAGES.len()));
    assert!(app.run("ui.language", serde_json::json!({"value": "xx"})).is_err());
    assert_eq!(app.ui.language, "zh-hant", "an unknown code keeps the setting");
    // The next frame binds the setting's language.
    frame(&ctx, &mut app);
    assert_eq!(current(), lang("zh-hant"));
    let saved = serde_json::to_string(&app.ui).unwrap();
    let restored: crate::prefs::Prefs = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.language, "zh-hant");
    // Settings saved before this option existed follow the system.
    let legacy: crate::prefs::Prefs = serde_json::from_str(r#"{"tab":"Home"}"#).unwrap();
    assert_eq!(legacy.language, AUTO);
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
    // Reading without a value reports the current setting.
    assert_eq!(app.run("ui.language", serde_json::json!({})).unwrap()["language"], AUTO);
    apply_language(Lang::EN);
}

/// One row per interface language: the tags the system may name it by and, when its name is
/// known, its name in itself.
const LOCALES: &[(&str, &[&str], Option<&str>)] = &[
    ("uk", &["uk", "uk-UA", "uk_UA.UTF-8"], None),
    ("pt-br", &["pt-BR", "pt_BR.UTF-8"], None),
    ("sr", &["sr", "sr-RS", "sr-Cyrl-RS"], None),
    ("sr-latn", &["sr-Latn-RS", "sr-Latn"], None),
    ("cs", &["cs", "cs-CZ", "cs_CZ.UTF-8"], None),
    ("ru", &["ru", "ru-RU", "ru_RU.UTF-8", "RU-ru", "ru-Cyrl-RU"], Some("Русский")),
    ("et", &["et", "et-EE", "ET_ee.UTF-8", "et_EE.UTF-8@euro"], Some("Eesti")),
    ("tr", &["tr", "tr-TR", "TR_tr.UTF-8", "tr_TR.UTF-8@euro"], Some("Türkçe")),
    ("hu", &["hu", "hu-HU", "HU_hu.UTF-8@euro"], Some("Magyar")),
    ("fr", &["fr", "fr-FR", "fr-CA", "fr_BE.UTF-8@euro"], Some("Français")),
    ("pl", &["pl", "pl-PL", "pl_PL.UTF-8", "pl_PL.UTF-8@euro"], Some("Polski")),
    ("de", &["de", "de-AT", "de-CH", "DE_de.UTF-8@euro"], Some("Deutsch")),
    ("it", &["it", "it-IT", "it-CH", "it_IT.UTF-8@euro"], Some("Italiano")),
    ("bg", &["bg", "bg-BG", "bg-Cyrl-BG"], Some("Български")),
    ("hr", &["hr", "hr-HR", "hr-BA"], Some("Hrvatski")),
    ("sv", &["sv", "sv-SE", "sv-FI"], Some("Svenska")),
    ("vi", &["vi", "vi-VN", "vi-US"], Some("Tiếng Việt")),
    ("ro", &["ro", "ro-RO", "ro-MD"], Some("Română")),
    ("sl", &["sl", "sl-SI", "SL-si"], Some("Slovenščina")),
    ("bs", &["bs", "bs-BA", "bs-Latn-BA"], Some("Bosanski")),
    ("ar", &["ar", "ar-MA", "ar_EG.UTF-8", "AR-sa", "ar-Arab-AE"], None),
];

/// A system locale and a saved preference pick each language, and choosing one leaves the open
/// document as it was.
#[test]
fn a_system_locale_and_a_saved_preference_pick_the_language_and_leave_the_document() {
    needs_catalogs!();
    let mut app = new_app(wordcraft_engine::sample::sample_document());
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let editing = app.session.editing_lang.clone();
    for (code, tags, name) in LOCALES {
        let Some(l) = Lang::from_code(code) else {
            eprintln!("skipped {code}: the embedded catalogs have no such language");
            continue;
        };
        for tag in *tags {
            assert_eq!(lang_from_tag(tag), Some(l), "{code}: {tag}");
            assert_eq!(first_supported([UNSUPPORTED, tag, "en-US"]), Some(l), "{code}: {tag} after an unsupported one");
        }
        let upper = code.to_uppercase();
        assert_eq!(normalize_pref(&upper), Some(*code));
        let result = app.run("ui.language", serde_json::json!({"value": upper})).unwrap();
        assert_eq!(result["effective"], *code);
        assert_eq!(app.ui.language, *code);
        if let Some(name) = name {
            assert_eq!(l.name(), *name);
            assert!(
                result["available"].as_array().unwrap().iter().any(|a| a["code"] == *code && a["name"] == *name),
                "{code} is offered by its name"
            );
        }
        assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before, "{code}: the document is untouched");
        assert_eq!(app.session.editing_lang, editing, "{code}: choosing it changes no document");
        let saved = serde_json::to_string(&app.ui).unwrap();
        let restored: crate::prefs::Prefs = serde_json::from_str(&saved).unwrap();
        assert_eq!(Lang::from_pref(&restored.language), l, "{code}: the saved preference restores it");
        assert!(app.run("ui.language", serde_json::json!({"value": format!("{code}-unknown")})).is_err());
        assert_eq!(app.ui.language, *code, "{code}: an invalid preference keeps the saved language");
    }
    app.run("ui.language", serde_json::json!({"value": "auto"})).unwrap();
    assert_eq!(app.ui.language, AUTO);
    assert_eq!(lang_from_tag("bs-Cyrl-BA"), None, "Cyrillic Bosnian has no catalog: a script is never crossed");
    assert_eq!(lang_from_tag("sr-Latn-RS"), Some(lang("sr-latn")));
    if let Some(ar) = Lang::from_code("ar") {
        assert!(ar.rtl());
        assert!(Lang::all().filter(|l| l.rtl()).eq([ar]), "Arabic is the only right-to-left language");
    }
}

/// The interface language names the language of new documents where WordCraft can proof it, and
/// what was written already keeps its own.
#[test]
fn the_interface_language_sets_the_language_of_new_documents() {
    needs_catalogs!();
    let rows =
        [("pl", Some("pl-PL"), Some("Polish")), ("cs", Some("cs-CZ"), Some("Czech")), ("it", Some("it-IT"), Some("Italian")), ("de", None, None)];
    let ctx = egui::Context::default();
    embedded_host(&ctx);
    for (pref, editing, name) in rows {
        let mut app = new_app(wordcraft_doc::Document::new());
        frame(&ctx, &mut app);
        assert_eq!(app.session.editing_lang, None, "English interface: WordCraft's default, US English");
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some("en-US"));
        app.run("ui.language", serde_json::json!({"value": pref})).unwrap();
        frame(&ctx, &mut app);
        assert_eq!(app.session.editing_lang.as_deref(), editing, "{pref}");
        let written = editing.unwrap_or("en-US");
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some(written), "{pref}: the untouched first document follows");
        assert!(!app.session.dirty, "{pref}");
        app.run("file.new", serde_json::json!({})).unwrap();
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some(written), "{pref}: a new document is written in it");
        // Writing in it makes the document written content; an untouched one would follow the interface back.
        app.session.run("text.insert", &serde_json::json!({"text": "Written words"})).unwrap();
        if let Some(name) = name {
            // The frames bind the interface language; name the language in English first.
            apply_language(Lang::EN);
            assert_eq!(crate::ribbon::caret_language(&app), name);
            apply_language(Lang::PSEUDO);
            assert_eq!(crate::ribbon::caret_language(&app), format!("⟦{name}⟧"), "the language name goes through the catalog");
            apply_language(Lang::EN);
        }
        // Back to English: documents already written keep their language.
        app.run("ui.language", serde_json::json!({"value": "en"})).unwrap();
        frame(&ctx, &mut app);
        assert_eq!(app.session.editing_lang, None, "{pref}");
        assert_eq!(app.session.doc.styles.default_chr.lang.as_deref(), Some(written), "{pref}: opened content keeps its language");
    }
}

#[test]
fn the_editing_languages_are_the_ones_wordcraft_can_proof() {
    let with: Vec<(&str, &str)> = LANGUAGES.iter().filter_map(|l| l.editing_lang.map(|e| (l.code, e))).collect();
    assert_eq!(with, [("pt-br", "pt-BR"), ("pl", "pl-PL"), ("cs", "cs-CZ"), ("tr", "tr-TR"), ("it", "it-IT")]);
}

/// The framework's coverage check is each app's to run over its own catalogs: every shipped
/// translation is drawn by the interface fonts, with no installed font to fall back on.
#[test]
fn every_shipped_translation_is_drawn_by_the_interface_fonts() {
    needs_catalogs!();
    // The interface fonts are the craft-fonts faces; with none embedded (no `CRAFT_FONTS_DIR`) the stack is
    // egui's bundled fallback, which lacks letters the catalogs use (the Spanish `ⁿ`), so there is nothing to check.
    if crate::app::fonts::CRAFT_FONTS.is_empty() {
        eprintln!("skipped: no craft-fonts face is embedded (set CRAFT_FONTS_DIR)");
        return;
    }
    for l in Lang::all().filter(|l| *l != Lang::EN) {
        let code = l.code();
        // A Han language is drawn by its craft-fonts face alone; a build without that face (no
        // `CRAFT_FONTS_DIR`, or a directory lacking it) has nothing to check, as in the caption test.
        let face = match code {
            "zh-hans" => Some("Hans"),
            "zh-hant" => Some("Hant"),
            "ja" => Some("Jpan"),
            "ko" => Some("Kore"),
            _ => None,
        };
        if let Some(script) = face.filter(|s| !crate::app::fonts::CRAFT_FONTS.iter().any(|f| f.has_script(s))) {
            eprintln!("skipped {code}: no embedded {script} face (set CRAFT_FONTS_DIR)");
            continue;
        }
        // A symbol a translation keeps from its English source (the Spanish `ⁿ` of "(x+a)ⁿ") is the source's to
        // draw, as in English; the check is of the letters the translation adds.
        let rows: Vec<(String, String)> = CRAFT_I18N
            .iter()
            .filter(|(c, ..)| *c == code)
            .flat_map(|(_, shared, app)| [*shared, *app])
            .flat_map(str::lines)
            .filter(|row| !row.starts_with('#'))
            .filter_map(|row| {
                let mut columns = row.split('\t');
                let key = columns.nth(1)?;
                let text = columns.next()?;
                Some((key.to_string(), text.chars().filter(|c| c.is_ascii() || !key.contains(*c)).collect()))
            })
            .collect();
        let samples: Vec<Sample<'_>> = rows.iter().map(|(key, text)| Sample::new(code, key, text)).collect();
        // PLAN(shell-w11): sh.A10 `ui_fonts(lang) -> UiFonts<'static>`; `stack()` is its edit-first view.
        assert_covers(&crate::app::fonts::ui_fonts(l).stack(), samples);
    }
}

#[test]
fn built_in_style_names_are_translated_for_display_and_user_styles_are_not() {
    let ctx = egui::Context::default();
    let sheet = wordcraft_doc::StyleSheet::builtin();
    let name = |id: &str| style_name(sheet.get(id).unwrap());
    fixture_host(
        &ctx,
        "x-fixture",
        "\tNormal\tNormal-FX\n\tHeading 1\tHeading-one-FX\n\tHeading {n}\tHeading-FX {n}\n\tTOC {n}\tTOC-FX {n}\n\tIndex {n}\tIndex-FX {n}\n",
    );
    assert_eq!(name("Normal"), "Normal-FX");
    // An explicit entry wins over its family's template.
    assert_eq!(name("Heading1"), "Heading-one-FX");
    // Numbered families beyond the catalog's explicit entries go through one template.
    assert_eq!(name("Heading7"), "Heading-FX 7");
    assert_eq!(name("TOC9"), "TOC-FX 9");
    assert_eq!(builtin_style_name("Index 2"), "Index-FX 2");
    // A style the author made keeps its name, even when it matches a built-in one.
    let mine = wordcraft_doc::Style { name: "Normal".into(), builtin: false, ..Default::default() };
    assert_eq!(style_name(&mine), "Normal");
    // A built-in name without a translation stays English.
    assert_eq!(builtin_style_name("Some Future Style"), "Some Future Style");
    assert_eq!(builtin_style_name("Heading x"), "Heading x");
    apply_language(Lang::EN);
    assert_eq!(name("Heading7"), "Heading 7");
    // The document itself never changes.
    assert_eq!(sheet.get("Normal").unwrap().name, "Normal");
}

/// #3317: the spooler's English receipt is not spliced into a translated sentence.
#[test]
fn spooled_print_receipt_is_translated() {
    use crate::printing::{Feedback, feedback};
    let fx = fixture_lang(
        "x-fixture",
        "\tPrint job submitted: spooled {count} sheet(s)\tSpooled-FX: {count}\n\tPrint job submitted: {receipt}\tSubmitted-FX: {receipt}\n",
    );
    let receipt = || Feedback::Submitted("spooled 2 sheet(s)".into());
    assert_eq!(feedback(&receipt(), |s| tr(fx, s).to_owned()), "Spooled-FX: 2");
    assert_eq!(feedback(&receipt(), str::to_owned), "Print job submitted: spooled 2 sheet(s)");
}

/// WordCraft offers exactly the languages in `LANGUAGES` when craft-i18n is embedded, and English
/// alone when it isn't: a missing catalog never fails, it shows English.
#[test]
fn the_languages_offered_are_the_embedded_ones_of_the_list() {
    let all: Vec<&str> = Lang::all().map(Lang::code).collect();
    if CRAFT_I18N.is_empty() {
        assert_eq!(all, ["en"]);
        assert_eq!(Lang::from_code("fr"), None);
        assert_eq!(Lang::from_pref("fr"), Lang::EN, "a saved language without a catalog shows English");
        assert_eq!(normalize_pref("fr"), None);
        assert_eq!(tr(Lang::EN, "File"), "File");
        return;
    }
    let listed: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
    assert_eq!(all, listed, "every language WordCraft lists has its catalog in craft-i18n");
    assert!(Lang::from_code("pseudo").is_none(), "the pseudo locale is never a setting");
}

/// The flatpak manifest builds with the craft-i18n commit `craft-i18n.rev` pins (CI and releases
/// read the file; the manifest has to name it itself).
#[test]
fn the_flatpak_manifest_pins_the_craft_i18n_rev() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rev = std::fs::read_to_string(root.join("craft-i18n.rev")).unwrap();
    let rev = rev.trim();
    assert!(rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()), "craft-i18n.rev holds one full SHA: {rev:?}");
    let manifest = std::fs::read_to_string(root.join("packaging/linux/flatpak/ai.storyteller.wordcraft.yml")).unwrap();
    let source = manifest.split("url: https://github.com/storytold/craft-i18n").nth(1).expect("a craft-i18n git source");
    let source = source.split("- type:").next().unwrap_or(source);
    let commit = source.lines().find_map(|l| l.trim().strip_prefix("commit:")).expect("the craft-i18n source pins a commit");
    assert_eq!(commit.trim().trim_matches(['\'', '"']), rev);
    assert!(manifest.contains("CRAFT_I18N_DIR: /run/build/wordcraft/craft-i18n"), "the build embeds the catalogs");
}

/// A right-to-left test language whose catalog is `source`, bound as the current language.
fn arabic_fixture(ctx: &egui::Context, source: &'static str) -> Lang {
    fixture_host(ctx, "ar", source)
}

/// Arabic comes back from the lookups exactly as the catalog has it, in typing order: egui's text
/// layout orders the words of each line itself, so nothing here reverses them. Placeholders are
/// filled in place and text for the document stays as typed.
#[test]
fn arabic_lookups_keep_typing_order() {
    let ctx = egui::Context::default();
    let ar = arabic_fixture(
        &ctx,
        concat!(
            "\tTable of Contents\tجدول المحتويات\n",
            "\tPaste Special…\tلصق خاص…\n",
            "\tAbout WordCraft\tحول WordCraft\n",
            "\tPassword:\tكلمة المرور:\n",
            "\tTable of\\nContents\tجدول\\nالمحتويات\n",
            "\tPage {page} of {pages}\tالصفحة {page} من {pages}\n",
            "\tExported {path}\tتم التصدير إلى {path}\n",
            "preview\tTitle\tالعنوان\n",
            "\tHome\tالرئيسية\n",
            "\tFont\tالخط\n",
        ),
    );
    assert!(ar.rtl());
    assert_eq!(tr(ar, "Table of Contents"), "جدول المحتويات");
    assert_eq!(tr(ar, "Paste Special…"), "لصق خاص…");
    assert_eq!(tr(ar, "About WordCraft"), "حول WordCraft");
    assert_eq!(tr(ar, "Password:"), "كلمة المرور:");
    // A two-line ribbon label keeps its two lines.
    assert_eq!(tr(ar, "Table of\nContents"), "جدول\nالمحتويات");
    // Placeholders are filled in place; inserted text is never touched, and the text for the
    // document is the same string.
    let page = fmt_plain(tr(ar, "Page {page} of {pages}"), &[("page", "3"), ("pages", "9")]);
    assert_eq!(page, "الصفحة 3 من 9");
    assert_eq!(fmt_plain(tr(ar, "Exported {path}"), &[("path", "draft-{words}.docx")]), "تم التصدير إلى draft-{words}.docx");
    // The preview context is looked up the same way.
    assert_eq!(trc(ar, "preview", "Title"), "العنوان");
    // A string without a translation stays English, untouched.
    assert_eq!(tr(ar, "unknown future label"), "unknown future label");
    // A ribbon location reads from the right: the tab first, the mark pointing left.
    assert_eq!(location("Home › Font"), "الرئيسية ‹ الخط");
    // A search query typed in reading order is a plain substring of the label.
    assert!(craft::i18n::Folder::new(current().code()).contains(t("Table of Contents"), "جدول المحتويات"));
    apply_language(Lang::EN);
    assert_eq!(location("Home › Font"), "Home › Font");
}

/// A path inside a sentence (the open failure on the status bar) is isolated in a right-to-left
/// interface, so Arabic folders do not scramble; in English it carries no isolate.
#[test]
fn a_path_in_a_message_is_isolated_in_a_right_to_left_interface() {
    use craft::i18n::Direction;
    let path = r"C:\ملفات\تقرير.docx";
    let rtl = fmt("تعذر فتح {path}: {error}", &[("path", &craft::i18n::path(path, Direction::Rtl)), ("error", "bad")]);
    assert_eq!(rtl, "تعذر فتح \u{2066}C:\\\u{200E}ملفات\\\u{200E}تقرير\u{200E}.docx\u{2069}: \u{2068}bad\u{2069}");
    assert_eq!(craft::i18n::path(path, Direction::Ltr), path, "a left-to-right interface adds no marks");
    let ltr = fmt("Couldn't open {path}: {error}", &[("path", &craft::i18n::path(path, Direction::Ltr)), ("error", "bad")]);
    assert!(!ltr.contains('\u{2066}') && !ltr.contains('\u{200E}'), "{ltr:?}");
}

#[test]
fn diagram_tab_display_keeps_the_legacy_identity() {
    assert!(wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(&"SmartArt Design"));
    assert_eq!(tr(Lang::EN, "SmartArt Design"), "Diagram Design");
    assert_eq!(trc(Lang::EN, "preview", "SmartArt Design"), "Diagram Design");
    assert_eq!(tr(Lang::EN, "Home"), "Home");
    let fx = fixture_lang("x-fixture", "\tDiagram Design\tDiagram-FX\npreview\tDiagram Design\tDiagram-Preview-FX\n\tDiagram Styles\tStyles-FX\n");
    assert!(has(fx, "SmartArt Design"));
    assert_eq!(tr(fx, "SmartArt Design"), "Diagram-FX");
    assert_eq!(trc(fx, "preview", "SmartArt Design"), "Diagram-Preview-FX");
    let ctx = egui::Context::default();
    let fx =
        fixture_host(&ctx, "x-fixture", "\tDiagram Design\tDiagram-FX\npreview\tDiagram Design\tDiagram-Preview-FX\n\tDiagram Styles\tStyles-FX\n");
    assert_eq!(current(), fx);
    assert_eq!(get("preview", "SmartArt Design"), Some("Diagram-Preview-FX"));
    assert_eq!(location("SmartArt Design › Diagram Styles"), "Diagram-FX › Styles-FX");
    apply_language(Lang::EN);
}
