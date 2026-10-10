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
//!    English text, clean-room: never from Microsoft Word's or another product's localisation).
//! 2. Add one row to [`LANGUAGES`].
//!
//! The Options dropdown, the system-language match and the catalog tests pick it up from there.
//!
//! # Looking strings up
//! - [`tl!`](crate::tl) / [`t`]: a string in the current language. [`tr`]: in a given one.
//! - [`tc`] / [`trc`]: worded for a context (`preview`), for text that must fit a tight spot.
//! - [`location`]: a ribbon location such as `Home › Font`, segment by segment.
//! - [`fmt`]: fill `{name}` placeholders after a lookup; translators may reorder them.

mod catalog;

use std::cell::Cell;
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
    catalog: OnceLock<Catalog>,
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 10] = [
    LangInfo { code: "en", name: "English", source: "", prefer_hans: false, catalog: OnceLock::new() },
    // Simplified Chinese; `zh`, `zh-CN`, `zh-SG` and `zh-Hans-*` resolve here (see `candidates`).
    LangInfo { code: "zh-hans", name: "简体中文", source: include_str!("zh-hans.tsv"), prefer_hans: true, catalog: OnceLock::new() },
    // Traditional Chinese (Taiwan vocabulary); `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` resolve here.
    LangInfo { code: "zh-hant", name: "繁體中文", source: include_str!("zh-hant.tsv"), prefer_hans: true, catalog: OnceLock::new() },
    LangInfo { code: "ja", name: "日本語", source: include_str!("ja.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    LangInfo { code: "uk", name: "Українська", source: include_str!("uk.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    // Spanish, neutral across Spain and Latin America; `es-ES`, `es-MX`, `es-419` … resolve here.
    LangInfo { code: "es", name: "Español", source: include_str!("es.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    // Brazilian Portuguese; `pt-BR` and `pt-BR-*` resolve here. Plain `pt` and `pt-PT` have no
    // catalog yet (the European vocabulary differs), so they stay in English.
    LangInfo { code: "pt-br", name: "Português (Brasil)", source: include_str!("pt-br.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    // Serbian, Cyrillic script (the default per BCP 47 when no script is given); `sr`, `sr-RS`,
    // `sr-Cyrl-*` resolve here.
    LangInfo { code: "sr", name: "Српски", source: include_str!("sr.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    // Serbian, Latin script; `sr-Latn-*` resolves here (see `candidates`'s generic prefix
    // matching — no special-casing needed, unlike Chinese's script-by-region fallback).
    LangInfo { code: "sr-latn", name: "Srpski (latinica)", source: include_str!("sr-latn.tsv"), prefer_hans: false, catalog: OnceLock::new() },
    // Russian; `ru`, `ru-RU`, `ru_RU.UTF-8` and other regions resolve here.
    LangInfo { code: "ru", name: "Русский", source: include_str!("ru.tsv"), prefer_hans: false, catalog: OnceLock::new() },
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

    /// Is the interface written in CJK script (its own name is), so it needs a CJK face?
    pub fn uses_cjk(self) -> bool {
        self.0.name.chars().any(|c| ('\u{2E80}'..='\u{9FFF}').contains(&c) || ('\u{AC00}'..='\u{D7AF}').contains(&c))
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

/// Does `lang` have a catalog entry for this string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// `s` in the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// `s` in the current language as worded for `context` (one of [`catalog::CONTEXTS`]), falling
/// back to the plain translation and then to English.
pub fn tc<'a>(context: &str, s: &'a str) -> &'a str {
    trc(current(), context, s)
}

/// [`tc`] in a given language.
pub fn trc<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    #[cfg(test)]
    if pseudo::on() {
        return pseudo::mark(s);
    }
    lang.catalog().in_context(context, s).unwrap_or(s)
}

/// `s` in `lang`; strings without a translation come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    #[cfg(test)]
    if pseudo::on() {
        return pseudo::mark(s);
    }
    lang.catalog().plain(s).unwrap_or(s)
}

/// A style's name as the interface shows it: built-in styles (`Normal`, `Heading 2`, `TOC 3`) in
/// the current language, styles the author made exactly as named. Display only: the document,
/// commands and saved files keep the English name.
pub fn style_name(style: &wordcraft_doc::Style) -> String {
    if style.builtin { builtin_style_name(&style.name) } else { style.name.clone() }
}

/// A built-in style's English name in the current language; numbered families go through one
/// template each (`Heading {n}`), and names without a translation stay English.
pub fn builtin_style_name(name: &str) -> String {
    let direct = t(name);
    if direct != name {
        return direct.to_string();
    }
    for (prefix, template) in [("Heading ", "Heading {n}"), ("TOC ", "TOC {n}"), ("Index ", "Index {n}")] {
        if let Some(n) = name.strip_prefix(prefix)
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            return fmt(t(template), &[("n", n)]);
        }
    }
    name.to_string()
}

/// A ribbon location (`Home › Font`) in the current language, segment by segment.
pub fn location(loc: &str) -> String {
    loc.split(" › ").map(t).collect::<Vec<_>>().join(" › ")
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
mod audit;
#[cfg(test)]
pub(crate) mod pseudo;
#[cfg(test)]
mod tests;
