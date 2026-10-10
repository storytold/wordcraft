//! Interface languages (#8). Strings in the code stay English and are the lookup keys; a catalog
//! per language (`*.tsv`, format in `zh-hans.tsv`) maps them to display text when the UI is drawn.
//! Command ids, document text, file names, the control channel, the CLI and MCP never see
//! translated text, so scripts and agents are unaffected. A string without a translation shows in
//! English, so coverage can grow one entry at a time.
//!
//! The `language` setting is `auto` by default: the UI follows the operating system's language
//! (Windows display language, macOS preferred languages, `LANG`/`LC_*` elsewhere) when there is a
//! catalog for it, and English otherwise. File ▸ Options ▸ Interface language picks one by hand.
//!
//! This is the system PdfCraft and PhotoCraft use (`pdfcraft/crates/ui-egui/src/i18n`), so the
//! catalog format and habits carry over between the Crafting Apps.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `zh-hans.tsv` (copy its header; translate from the *meaning* of the
//!    English text, clean-room: never copy Microsoft Word's or another product's localisation.
//!    Feature names may use the terms established in that language, as `de.tsv` does).
//! 2. Add one row to [`LANGUAGES`].
//!
//! The Options dropdown, the system-language match and the catalog tests pick it up from there.
//!
//! # Looking strings up
//! - [`tl!`](crate::tl) / [`t`]: a string in the current language. [`tr`]: in a given one.
//! - [`location`]: a ribbon location such as `Home › Font`, segment by segment.
//! - [`t_at`]: a label at a ribbon location, for the few English words a language translates
//!   two ways (German `Open`: `Öffnen` in File, `Offen` under Paragraph Spacing).
//! - [`fmt`]: fill `{name}` placeholders after a lookup; translators may reorder them.

mod catalog;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

use catalog::Catalog;

/// The `language` setting that follows the system.
pub const AUTO: &str = "auto";

/// One supported interface language.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hans`). Also the `language` setting's value.
    pub code: &'static str,
    /// The language's name in itself, shown in the Options dropdown.
    pub name: &'static str,
    /// Catalog file contents (empty for the built-in English).
    pub source: &'static str,
    /// Which CJK interface face comes first (Chinese text wants the Chinese face).
    pub prefer_hans: bool,
    /// Lengths in centimetres with a decimal comma, as Word shows them in this language (German);
    /// otherwise inches.
    pub centimetres: bool,
    catalog: OnceLock<Catalog>,
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 6] = [
    LangInfo { code: "en", name: "English", source: "", prefer_hans: false, centimetres: false, catalog: OnceLock::new() },
    // Simplified Chinese; `zh`, `zh-CN`, `zh-SG` and `zh-Hans-*` resolve here (see `candidates`).
    LangInfo {
        code: "zh-hans",
        name: "简体中文",
        source: include_str!("zh-hans.tsv"),
        prefer_hans: true,
        centimetres: false,
        catalog: OnceLock::new(),
    },
    // Traditional Chinese (Taiwan vocabulary); `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` resolve here.
    LangInfo {
        code: "zh-hant",
        name: "繁體中文",
        source: include_str!("zh-hant.tsv"),
        prefer_hans: true,
        centimetres: false,
        catalog: OnceLock::new(),
    },
    LangInfo { code: "ja", name: "日本語", source: include_str!("ja.tsv"), prefer_hans: false, centimetres: false, catalog: OnceLock::new() },
    // Brazilian Portuguese; `pt-BR` and `pt-BR-*` resolve here. Plain `pt` and `pt-PT` have no
    // catalog yet (the European vocabulary differs), so they stay in English.
    LangInfo {
        code: "pt-br",
        name: "Português (Brasil)",
        source: include_str!("pt-br.tsv"),
        prefer_hans: false,
        centimetres: false,
        catalog: OnceLock::new(),
    },
    // German; `de-DE`, `de-AT`, `de-CH` and the other regions resolve here.
    LangInfo { code: "de", name: "Deutsch", source: include_str!("de.tsv"), prefer_hans: false, centimetres: true, catalog: OnceLock::new() },
];

impl LangInfo {
    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| {
            let (catalog, errors) = Catalog::parse(self.source);
            for error in errors {
                log::warn!("{} interface catalog: {error}; that entry shows in English", self.code);
            }
            catalog
        })
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    pub fn prefers_hans(self) -> bool {
        self.0.prefer_hans
    }

    /// A language by its exact code (any case).
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// Resolve the `language` setting: a language code, or [`AUTO`] (and anything unknown, e.g. a
    /// code from a newer version) to follow the system.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(system_lang)
    }

    /// Every registered language.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

/// The canonical `language` setting for user input: [`AUTO`] or a registered code (any case).
/// `None` for anything else, so callers keep the current setting.
pub fn normalize_pref(pref: &str) -> Option<&'static str> {
    if pref.eq_ignore_ascii_case(AUTO) {
        return Some(AUTO);
    }
    Lang::from_code(pref).map(Lang::code)
}

/// Candidate codes for a locale tag, most specific first: `zh_TW.UTF-8` → `zh-tw`, `zh-hant`, `zh`.
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else { return Vec::new() };
    let mut out = Vec::new();
    for n in (1..=parts.len()).rev() {
        out.push(parts.get(..n).unwrap_or_default().join("-"));
    }
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        // Chinese by region when no script is given.
        let script = if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-hant" } else { "zh-hans" };
        out.insert(out.len().saturating_sub(1), script.to_string());
    }
    out
}

/// The registered language for a locale tag such as `ja_JP.UTF-8` or `zh-Hant-TW`; `None` when
/// there is no catalog for it. `C`/`POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let cands = candidates(tag);
    if matches!(cands.first().map(String::as_str), Some("c" | "posix")) {
        return Some(Lang::EN);
    }
    cands.iter().find_map(|c| Lang::from_code(c))
}

/// The first of the system's preferred languages WordCraft has, in the user's order: someone
/// who prefers French, then Japanese, gets Japanese rather than English.
pub fn first_supported<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Lang> {
    tags.into_iter().find_map(lang_from_tag)
}

/// The system language (looked up once). English when it can't be determined or isn't supported.
pub fn system_lang() -> Lang {
    // Tests drive the UI by its English labels whatever the machine's language is.
    if cfg!(test) {
        return Lang::EN;
    }
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(detect_system_lang)
}

fn detect_system_lang() -> Lang {
    // An explicit override, then the platform's preferred UI languages (Windows display language,
    // macOS preferred languages, `LC_ALL`/`LC_MESSAGES`/`LANG` on Unix, the browser on the web).
    if let Some(l) = std::env::var("WORDCRAFT_LANGUAGE").ok().and_then(|v| lang_from_tag(&v)) {
        return l;
    }
    let tags: Vec<String> = sys_locale::get_locales().take(16).collect();
    first_supported(tags.iter().map(String::as_str)).unwrap_or(Lang::EN)
}

thread_local! {
    // Per thread: parallel test harnesses must not change each other's language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// Set the language the UI is drawn in (the app does this every frame from its setting).
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    CURRENT.get()
}

/// The date part of an ISO timestamp (`2026-10-10T09:30:00Z`) as the current language writes a
/// short date: `10.10.2026` in German; other languages keep `2026-10-10`.
pub fn short_date(iso: &str) -> String {
    let date = iso.get(..10).unwrap_or(iso);
    let mut parts = date.split('-');
    match (current().code(), parts.next(), parts.next(), parts.next()) {
        ("de", Some(y), Some(m), Some(d)) if y.len() == 4 && m.len() == 2 && d.len() == 2 => format!("{d}.{m}.{y}"),
        _ => date.to_string(),
    }
}

/// Does `lang` have a catalog entry for this string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// `s` in the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// `s` in `lang`; strings without a translation come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// The unit lengths are shown in (rulers, indent and margin fields, paper sizes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthUnit {
    /// Points per unit.
    pub points: f32,
    pub suffix: &'static str,
    /// Drag speed of a length field, in units.
    pub speed: f64,
    /// Ruler ticks per unit (eighths of an inch, quarters of a centimetre).
    pub ruler_steps: i32,
    pub decimal_comma: bool,
}

impl LengthUnit {
    pub const INCHES: LengthUnit = LengthUnit { points: 72.0, suffix: "\"", speed: 0.05, ruler_steps: 8, decimal_comma: false };
    pub const CENTIMETRES: LengthUnit = LengthUnit { points: 72.0 / 2.54, suffix: " cm", speed: 0.1, ruler_steps: 4, decimal_comma: true };

    /// A number in this unit's notation, with at most `decimals` decimals (`2,5` in German).
    pub fn number(self, value: f64, decimals: usize) -> String {
        let s = format!("{value:.decimals$}");
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        if self.decimal_comma { s.replace('.', ",") } else { s }
    }

    /// Read a number typed in this unit's notation (a decimal comma is fine in either).
    pub fn parse(self, text: &str) -> Option<f64> {
        text.trim().trim_end_matches(self.suffix.trim()).trim().replace(',', ".").parse().ok()
    }
}

/// The length unit of the current language.
pub fn length_unit() -> LengthUnit {
    if current().0.centimetres { LengthUnit::CENTIMETRES } else { LengthUnit::INCHES }
}

/// A ribbon location (`Home › Font`) in the current language, segment by segment.
pub fn location(loc: &str) -> String {
    loc.split(" › ").map(t).collect::<Vec<_>>().join(" › ")
}

/// `s` as labelled at a ribbon location (`Home › Font › Text Effects`): a catalog entry scoped
/// to the location's last segment (`Text Effects › Outline`) wins over the plain one. Only the
/// few English words a language needs to translate two ways have scoped entries.
pub fn t_at(location: &str, s: &str) -> String {
    let group = location.rsplit(" › ").next().unwrap_or(location);
    let lang = current();
    match lang.catalog().plain(&format!("{group} › {s}")) {
        Some(scoped) => scoped.to_string(),
        None => tr(lang, s).to_string(),
    }
}

/// A style's name for display. Built-in styles (`Normal`, `Heading 1`, `Title` …) show their
/// name in the interface language, as Word does; documents keep the English names, and styles
/// people name themselves are never translated. Word files store some built-in names in lower
/// case (`heading 1`, `toc 1`), so the match ignores case.
pub fn style_name(name: &str) -> &str {
    static BUILTIN: OnceLock<HashMap<String, String>> = OnceLock::new();
    let builtin = BUILTIN.get_or_init(|| wordcraft_doc::StyleSheet::builtin().styles.into_iter().map(|s| (s.name.to_lowercase(), s.name)).collect());
    match builtin.get(&name.to_lowercase()) {
        Some(canonical) => t(canonical),
        None => name,
    }
}

/// A label with an action after a fixed English prefix, e.g. `Undo Typing`: both parts translated
/// where the catalog has them.
pub fn prefixed(prefix: &str, rest: &str) -> String {
    if rest.is_empty() {
        return t(prefix).to_string();
    }
    fmt(t(&format!("{prefix} {{action}}")), &[("action", t(rest))])
}

/// Fill `{name}` placeholders in one pass. Unknown placeholders stay as written, and inserted
/// values are never read as templates again, so a file name containing `{n}` stays intact.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, after_close)) = after_open.split_once('}') else {
            out.push('{');
            out.push_str(after_open);
            return out;
        };
        match args.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after_close;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
