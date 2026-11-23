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
}
