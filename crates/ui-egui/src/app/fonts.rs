#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use craft::ui::fonts::SystemCandidates;
use craft::ui::fonts::{EmbeddedFace, I18nScripts, UiFonts};
#[cfg(test)]
use craft::ui::fonts::{FontRef, Stack};
#[cfg(all(test, not(target_arch = "wasm32")))]
use craft::ui::fonts::{OnDemandFonts, han_for_locale};

use super::WordApp;
use crate::i18n::Lang;

/// The craft-fonts faces embedded for the interface.
// PLAN(chrome-w12): build.rs embeds the craft faces into `craft_fonts.rs`
pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The interface fonts for `lang`: the embedded craft faces, ordered by the language's scripts,
/// and installed system fonts loaded on demand where the platform has them.
pub fn ui_fonts(lang: Lang) -> UiFonts<'static> {
    let codes: Vec<&str> = Lang::all().map(Lang::code).collect();
    let os_locale = wordcraft_engine::system_locale();
    let fonts = UiFonts::new(&I18nScripts).craft_faces(CRAFT_FONTS).languages(&codes).ui_language(lang.code());
    let fonts = match os_locale {
        Some(locale) => fonts.os_locale(locale),
        None => fonts,
    };
    #[cfg(not(target_arch = "wasm32"))]
    let fonts = fonts.on_demand(Arc::new(SystemCandidates));
    fonts
}

#[cfg(test)]
/// One character of each script the app ships, to find the document fonts that cover it.
const FALLBACK_SAMPLES: [char; 6] = ['ع', 'א', 'ก', 'ა', 'ա', '汉'];

#[cfg(test)]
/// The largest document face added to the interface stack.
const MAX_FALLBACK_BYTES: usize = 16 << 20;

/// Add the document database's fallback faces to the interface families, so a script only a
/// user-added font covers still draws in file names, comments and search results.
#[cfg(test)]
pub fn add_document_fallbacks(stack: &mut Stack) {
    let db = wordcraft_fonts::FontDb::global();
    let families = [FontRef::proportional(), FontRef::named("medium"), FontRef::named("semibold")];
    for sample in FALLBACK_SAMPLES {
        let Some(f) = db.fallback_for(sample, u32::MAX) else { continue };
        if f.data().len() > MAX_FALLBACK_BYTES {
            continue;
        }
        let name = format!("fallback {} {} {}", f.family, f.style, f.id());
        for family in &families {
            let mut data = egui::FontData::from_owned(f.data().to_vec());
            data.index = f.index();
            stack.insert(family, usize::MAX, &name, data);
        }
    }
}

#[cfg(test)]
/// The app's one font install: the interface stack with the document fallbacks, then the loader
/// of installed system faces. Run it at start-up and again on a language change.
pub fn install_ui_fonts(ctx: &egui::Context, lang: Lang) -> Stack {
    let fonts = ui_fonts(lang);
    let mut stack = fonts.stack();
    add_document_fallbacks(&mut stack);
    stack.install(ctx);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let os_locale = wordcraft_engine::system_locale();
        OnDemandFonts::new(Arc::new(SystemCandidates), han_for_locale(lang.code(), os_locale)).install(ctx);
    }
    stack
}

impl WordApp {
    /// Fonts were added or removed (a command, the web shell loading stored fonts): lay out
    /// again and refresh the font lists and their previews.
    pub(crate) fn watch_fonts(&mut self, ctx: &egui::Context) {
        let db = wordcraft_fonts::FontDb::global();
        let generation = db.generation();
        if generation == self.font_generation {
            return;
        }
        self.font_generation = generation;
        self.session.relayout();
        self.previews.fonts_changed();
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use craft::ui::fonts::{Sample, missing_glyphs};

    use super::*;

    /// A name in every script the app ships must draw in the window caption, comments and the
    /// recent list, which use the interface faces.
    #[test]
    fn every_shipped_name_in_a_non_latin_script_draws_in_the_caption() {
        let mut stack = ui_fonts(Lang::EN).stack();
        add_document_fallbacks(&mut stack);
        let names = [("Arab", "مرحبا"), ("Hebr", "שלום"), ("Thai", "สวัสดี"), ("Geor", "გამარჯობა"), ("Armn", "բարեւ"), ("Hans", "汉语")];
        for (script, name) in names {
            if !CRAFT_FONTS.iter().any(|f| f.has_script(script)) {
                eprintln!("skipped {script}: no embedded face (set CRAFT_FONTS_DIR)");
                continue;
            }
            let missing = missing_glyphs(&stack, &FontRef::proportional(), [Sample::new("en", script, name)], false);
            assert!(missing.is_empty(), "{script}: {missing:?}");
        }
    }

    /// Symbols written as text in the interface code must exist in the interface fonts; a missing
    /// glyph draws as an empty box, as the panes' "✕" close button once did. Letters are the
    /// catalogs' business (see the i18n font tests).
    #[test]
    fn interface_symbols_have_glyphs() {
        let stack = ui_fonts(Lang::EN).stack();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut symbols = BTreeSet::new();
        // The i18n module's literals are catalog and pseudo-locale test text, not interface text.
        for path in crate::rust_sources(&dir).into_iter().filter(|p| !p.starts_with(dir.join("i18n"))) {
            let code = std::fs::read_to_string(&path).unwrap();
            let file = path.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
            // Test files and the test modules at the end of a file hold test data (emoji, full-width digits), not
            // interface text.
            if file.ends_with("tests.rs") || file.contains("/tests/") {
                continue;
            }
            let production = code.split("#[cfg(test)]").next().unwrap_or_default();
            // Characters inside string literals, line by line (good enough for this code base).
            for line in production.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for (i, part) in line.split('"').enumerate() {
                    if i % 2 == 1 {
                        symbols.extend(part.chars().filter(|c| !c.is_ascii() && !c.is_alphabetic() && !c.is_whitespace()).map(|c| (c, file.clone())));
                    }
                }
            }
        }
        // Equation gallery templates are UnicodeMath input that the math renderer draws with
        // document fonts, not interface text. The ➢ bullet is a value for `para.bullets`; its
        // menu tile draws the shape instead.
        let exempt = |c: char, file: &str| match file {
            "equation_tab.rs" => matches!(c, '≜' | '┬'),
            _ if file == "ribbon.rs" || file.starts_with("ribbon/") => c == '➢',
            _ => false,
        };
        symbols.retain(|(c, file)| !exempt(*c, file));
        let files: Vec<(char, String)> = symbols.into_iter().collect();
        let texts: Vec<(String, String)> = files.iter().map(|(c, f)| (c.to_string(), f.clone())).collect();
        let samples = texts.iter().map(|(t, f)| Sample::new("en", f, t));
        let missing = missing_glyphs(&stack, &FontRef::proportional(), samples, false);
        assert!(missing.is_empty(), "symbols without a glyph: {missing:#?}");
    }

    /// An installed Arabic font is asked for only when Arabic text needs one (and no embedded
    /// Arabic face covers it): neither installing the UI font stack for English/CJK/Arabic nor
    /// triggering CJK on-demand fallback ever asks for an Arabic system font, and Arabic on-demand
    /// fallback never asks for a CJK system font.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_installed_arabic_font_is_asked_for_by_arabic_alone() {
        use std::path::PathBuf;
        use std::sync::Mutex;

        use craft::host::Wake;
        use craft::i18n::Han;
        use craft::ui::fonts::{FontCandidates, on_demand_registered};

        struct RecordingCandidates {
            scripts: Mutex<Vec<String>>,
        }

        impl RecordingCandidates {
            fn new() -> Arc<Self> {
                Arc::new(Self { scripts: Mutex::new(Vec::new()) })
            }

            fn scripts(&self) -> Vec<String> {
                self.scripts.lock().map(|guard| guard.clone()).unwrap_or_default()
            }
        }

        impl FontCandidates for RecordingCandidates {
            fn files(&self, script: &str, han: Option<Han>) -> Vec<PathBuf> {
                if let Ok(mut scripts) = self.scripts.lock() {
                    scripts.push(script.to_owned());
                }
                SystemCandidates.files(script, han)
            }

            fn covers(&self, data: &[u8], c: char) -> Option<u32> {
                SystemCandidates.covers(data, c)
            }

            fn pending(&self) -> bool {
                SystemCandidates.pending()
            }

            fn wake_when_ready(&self, wake: Wake) {
                SystemCandidates.wake_when_ready(wake);
            }
        }

        let paint = |ctx: &egui::Context, text: &str| {
            for _ in 0..6 {
                ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.label(text);
                })
                .drop_without_applying_deltas();
            }
        };
        let is_cjk_script = |script: &str| matches!(script, "Hani" | "Hira" | "Kana" | "Hang" | "Bopo");

        let en = Lang::EN;
        let ja = crate::i18n::fixture_lang("ja", "\tHome\tホーム\n");
        let ar = crate::i18n::fixture_lang("ar", "\tHome\tالرئيسية\n");

        // Installing the production `ui_fonts` stack and painting Latin text never requests or
        // loads an installed Arabic or CJK system font up front.
        for lang in [en, ja, ar] {
            let ctx = egui::Context::default();
            let spy = RecordingCandidates::new();
            ui_fonts(lang).on_demand(Arc::clone(&spy) as Arc<dyn FontCandidates>).install(&ctx);
            assert!(on_demand_registered(&ctx).is_empty(), "{}: installing ui_fonts reads no system font eagerly", lang.code());
            assert!(spy.scripts().is_empty(), "{}: installing ui_fonts requests no system font candidates", lang.code());
            paint(&ctx, "Hello");
            assert!(on_demand_registered(&ctx).is_empty(), "{}: Latin UI text reads no system font", lang.code());
            assert!(spy.scripts().is_empty(), "{}: Latin UI text requests no system font candidates", lang.code());
        }

        let embedded_arabic: Vec<String> = CRAFT_FONTS.iter().filter(|f| f.has_script("Arab")).map(EmbeddedFace::name).collect();
        let ar_ctx = egui::Context::default();
        let ar_stack = ui_fonts(ar).install(&ar_ctx);
        let en_stack = ui_fonts(en).stack();
        let ja_stack = ui_fonts(ja).stack();
        if !embedded_arabic.is_empty() {
            assert!(
                ar_stack.chain(&FontRef::proportional()).first().is_some_and(|name| embedded_arabic.contains(name)),
                "Arabic UI leads with its embedded Arabic face"
            );
            assert!(
                !en_stack.chain(&FontRef::proportional()).first().is_some_and(|name| embedded_arabic.contains(name)),
                "English UI does not lead with an Arabic face"
            );
            assert!(
                !ja_stack.chain(&FontRef::proportional()).first().is_some_and(|name| embedded_arabic.contains(name)),
                "CJK UI does not lead with an Arabic face"
            );
            paint(&ar_ctx, "مرحبا");
            assert!(ar_ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), 'ع')), "embedded Arabic face draws Arabic text");
            assert!(on_demand_registered(&ar_ctx).is_empty(), "no installed Arabic system font is read when an embedded Arabic face covers Arabic");
        } else {
            paint(&ar_ctx, "مرحبا");
            let registered = on_demand_registered(&ar_ctx);
            let arabic_candidates = SystemCandidates.files("Arab", None);
            assert!(!registered.is_empty() || cfg!(not(any(target_os = "macos", windows))), "macOS and Windows ship an installed Arabic font");
            if !registered.is_empty() {
                assert_eq!(registered.len(), 1, "Arabic text registers one installed Arabic face");
                assert!(arabic_candidates.contains(&registered[0].1), "registered face {:?} came from Arabic system candidates", registered[0].1);
                assert!(ar_ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), 'ع')), "installed Arabic system font draws Arabic text");
            }
        }

        // CJK on-demand lookup through `ui_fonts` never asks for an Arabic system font (U+3400 is
        // CJK Extension A, outside embedded catalog subsets, so it exercises CJK on-demand lookup
        // even in builds with `CRAFT_FONTS_DIR`).
        let cjk_spy = RecordingCandidates::new();
        let cjk_ctx = egui::Context::default();
        ui_fonts(ja).on_demand(Arc::clone(&cjk_spy) as Arc<dyn FontCandidates>).install(&cjk_ctx);
        paint(&cjk_ctx, "日本語 㐀");
        let cjk_scripts = cjk_spy.scripts();
        assert!(!cjk_scripts.iter().any(|s| s == "Arab"), "CJK fallback never asks for an Arabic font: {cjk_scripts:?}");
        if !cjk_ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), '\u{3400}')) || !on_demand_registered(&cjk_ctx).is_empty() {
            assert!(cjk_scripts.iter().any(|s| is_cjk_script(s)), "uncovered CJK text asks for CJK candidates: {cjk_scripts:?}");
        }

        // Arabic on-demand lookup when no embedded Arabic face is present asks for Arabic alone
        // and never triggers a CJK system-font lookup.
        let ar_spy = RecordingCandidates::new();
        let ar_no_embed_ctx = egui::Context::default();
        if embedded_arabic.is_empty() {
            ui_fonts(ar).on_demand(Arc::clone(&ar_spy) as Arc<dyn FontCandidates>).install(&ar_no_embed_ctx);
        } else {
            UiFonts::new(&I18nScripts)
                .languages(&["en", "ja", "ar"])
                .ui_language(ar.code())
                .on_demand(Arc::clone(&ar_spy) as Arc<dyn FontCandidates>)
                .install(&ar_no_embed_ctx);
        }
        paint(&ar_no_embed_ctx, "مرحبا");
        let ar_scripts = ar_spy.scripts();
        assert!(ar_scripts.iter().any(|s| s == "Arab"), "Arabic text without an embedded Arabic face asks for Arab: {ar_scripts:?}");
        assert!(!ar_scripts.iter().any(|s| is_cjk_script(s)), "Arabic fallback never asks for a CJK font: {ar_scripts:?}");
    }
}
