//! Interface languages (#8). Strings in the code stay English and are the lookup keys; the
//! craft-i18n catalogs map them to display text when the UI is drawn. Command ids, document text,
//! file names, the control channel, the CLI and MCP never see translated text, so scripts and
//! agents are unaffected. A string without a translation shows in English, so coverage can grow
//! one entry at a time, and a missing translation never fails a build, a test or a release.
//!
//! The catalogs live in [storytold/craft-i18n](https://github.com/storytold/craft-i18n)
//! (`locales/<locale>/common.tsv` and `wordcraft.tsv`), not in this repository. `build.rs` embeds
//! them from `CRAFT_I18N_DIR` (CI and releases check out the SHA in `craft-i18n.rev`); without it
//! the build is English only. Lookup, fallback (`pt-br` → `pt` → English, app rows over shared
//! ones), plurals, language names and locale matching are the `craft-i18n` crate's (craft-libs).
//!
//! The `language` setting is `auto` by default: the UI follows the operating system's language
//! (Windows display language, macOS preferred languages, `LANG`/`LC_*` elsewhere) when there is a
//! catalog for it, and English otherwise. File ▸ Options ▸ Interface language picks one by hand.
//!
//! # Adding or changing a translation
//! In craft-i18n: add rows to `locales/<locale>/wordcraft.tsv` (its `docs/integration.md`); they
//! show here once `craft-i18n.rev` moves to a commit that has them. A feature PR here adds the
//! English only.
//!
//! # Adding a language
//! 1. craft-i18n gets `locales/<code>/wordcraft.tsv` and the language's `@id language.name` row.
//! 2. Add one row to [`LANGUAGES`]. Set its `editing_lang` only when `wordcraft-proof` can proof
//!    the language (Polish: `pl-PL`): new documents are then written in it.
//!
//! The Options dropdown and the system-language match pick it up from there.
//!
//! # Looking strings up
//! - [`tl!`](crate::tl) / [`t`]: a string in the current language. [`tr`]: in a given one.
//! - [`tc`] / [`trc`]: worded for a context (`preview`), for text that must fit a tight spot.
//! - [`get`]: one context's translation only, never the plain row (`spacing`'s "Open").
//! - [`location`]: a ribbon location such as `Home › Font`, segment by segment.
//! - [`fmt`]: fill `{name}` placeholders after a lookup; translators may reorder them.
//! - [`fmt_plain`]: [`fmt`] without bidi isolate marks, for text that goes into the document.

use std::cell::Cell;
use std::sync::{LazyLock, OnceLock};

use craft_i18n::I18n;

pub use craft_i18n::{fmt, fmt_plain};

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The embedded catalogs, offering exactly the languages in [`LANGUAGES`]. Built on first use;
/// a locale is parsed on its first lookup.
static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let offered: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
    let (i18n, problems) = I18n::new(CRAFT_I18N, &offered);
    problems.iter().for_each(|p| log::warn!("interface catalogs: {p}"));
    i18n
});

/// The `language` setting that follows the system.
pub const AUTO: &str = "auto";

/// One interface language WordCraft offers: what the app needs beyond the catalogs. Its name and
/// translations come from craft-i18n.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hans`). Also the `language` setting's value and the
    /// craft-i18n locale.
    pub code: &'static str,
    /// Which CJK interface face comes first (Chinese text wants the Chinese face).
    pub prefer_hans: bool,
    /// The language (BCP 47) new blank documents are written in with this interface, for
    /// languages WordCraft can proof; `None` keeps WordCraft's default, US English.
    pub editing_lang: Option<&'static str>,
}

/// The languages WordCraft offers, when their catalogs are embedded. English first: it is the
/// fallback and the source language.
pub static LANGUAGES: [LangInfo; 27] = [
    LangInfo { code: "en", prefer_hans: false, editing_lang: None },
    // Simplified Chinese; `zh`, `zh-CN`, `zh-SG` and `zh-Hans-*` resolve here (the crate's matching).
    LangInfo { code: "zh-hans", prefer_hans: true, editing_lang: None },
    // Traditional Chinese (Taiwan vocabulary); `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*` resolve here.
    LangInfo { code: "zh-hant", prefer_hans: true, editing_lang: None },
    LangInfo { code: "ja", prefer_hans: false, editing_lang: None },
    LangInfo { code: "ko", prefer_hans: false, editing_lang: None },
    LangInfo { code: "uk", prefer_hans: false, editing_lang: None },
    // Spanish, neutral across Spain and Latin America; `es-ES`, `es-MX`, `es-419` … resolve here.
    // No fixed editing language: new documents are written in the region's Spanish
    // (`Session::region_spanish`), like English.
    LangInfo { code: "es", prefer_hans: false, editing_lang: None },
    // Brazilian Portuguese; `pt-BR` and `pt-BR-*` resolve here. Plain `pt` and `pt-PT` have no
    // catalog yet (the European vocabulary differs), so they stay in English ([`lang_from_tag`]).
    LangInfo { code: "pt-br", prefer_hans: false, editing_lang: Some("pt-BR") },
    // Serbian, Cyrillic script (the default per BCP 47 when no script is given); `sr`, `sr-RS`,
    // `sr-Cyrl-*` resolve here.
    LangInfo { code: "sr", prefer_hans: false, editing_lang: None },
    // Serbian, Latin script; `sr-Latn-*` resolves here; a script is never
    // crossed, so Latin Serbian never shows Cyrillic.
    LangInfo { code: "sr-latn", prefer_hans: false, editing_lang: None },
    // Polish; `pl`, `pl-PL` and `pl_PL.UTF-8` resolve here. WordCraft proofs Polish (spelling,
    // grammar, hyphenation), so new documents are written in Polish with this interface.
    LangInfo { code: "pl", prefer_hans: false, editing_lang: Some("pl-PL") },
    LangInfo { code: "cs", prefer_hans: false, editing_lang: Some("cs-CZ") },
    LangInfo { code: "et", prefer_hans: false, editing_lang: None },
    // German, neutral across Germany, Austria and Switzerland; `de`, `de-DE`, `de-AT`, `de-CH` …
    // resolve here.
    LangInfo { code: "de", prefer_hans: false, editing_lang: None },
    // Russian; `ru`, `ru-RU`, `ru_RU.UTF-8` and other regions resolve here.
    LangInfo { code: "ru", prefer_hans: false, editing_lang: None },
    LangInfo { code: "tr", prefer_hans: false, editing_lang: Some("tr-TR") },
    // Italian; `it-IT`, `it-CH` and `it-*` resolve here. New documents are written in
    // Italian: `wordcraft-proof` checks it (`ProofLang::It`).
    LangInfo { code: "it", prefer_hans: false, editing_lang: Some("it-IT") },
    // Hungarian; `hu`, `hu-HU` and `hu_HU.UTF-8` resolve here.
    LangInfo { code: "hu", prefer_hans: false, editing_lang: None },
    // French; `fr`, `fr-FR`, `fr_CA` and other regions resolve here.
    LangInfo { code: "fr", prefer_hans: false, editing_lang: None },
    // Bulgarian; `bg`, `bg-BG` and `bg_BG.UTF-8` resolve here.
    LangInfo { code: "bg", prefer_hans: false, editing_lang: None },
    // Vietnamese; `vi`, `vi-VN` and `vi_VN.UTF-8` resolve here.
    LangInfo { code: "vi", prefer_hans: false, editing_lang: None },
    // Swedish; sv, sv-SE, sv-FI and sv_SE.UTF-8 resolve here.
    LangInfo { code: "sv", prefer_hans: false, editing_lang: None },
    // Croatian; `hr`, `hr-HR` and `hr_HR.UTF-8` resolve here.
    LangInfo { code: "hr", prefer_hans: false, editing_lang: None },
    // Dutch; `nl`, `nl-NL`, `nl-BE` and `nl_NL.UTF-8` resolve here.
    LangInfo { code: "nl", prefer_hans: false, editing_lang: None },
    // Danish; `da`, `da-DK` and `da_DK.UTF-8` resolve here.
    LangInfo { code: "da", prefer_hans: false, editing_lang: None },
    // Romanian; `ro`, `ro-RO`, `ro_RO.UTF-8` and `ro-MD` resolve here.
    LangInfo { code: "ro", prefer_hans: false, editing_lang: None },
    // Slovenian; `sl`, `sl-SI` and `sl_SI.UTF-8` resolve here.
    LangInfo { code: "sl", prefer_hans: false, editing_lang: None },
];

/// A language the UI can be shown in: English, or an offered language whose catalogs are embedded.
#[derive(Clone, Copy)]
pub struct Lang {
    lang: craft_i18n::Lang,
    /// The catalogs it is looked up in: the embedded ones ([`I18N`]), or a test fixture's.
    catalogs: Option<&'static I18n>,
}

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.code())
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.code() == other.code()
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang { lang: craft_i18n::Lang::EN, catalogs: None };

    fn i18n(self) -> &'static I18n {
        self.catalogs.unwrap_or_else(|| LazyLock::force(&I18N))
    }

    fn info(self) -> Option<&'static LangInfo> {
        let code = self.code();
        LANGUAGES.iter().find(|l| l.code == code)
    }

    pub fn code(self) -> &'static str {
        self.i18n().code(self.lang)
    }

    /// The language's name in itself (its `@id language.name` row), shown in the Options dropdown.
    pub fn name(self) -> &'static str {
        self.i18n().name(self.lang)
    }

    pub fn prefers_hans(self) -> bool {
        self.info().is_some_and(|l| l.prefer_hans)
    }

    pub fn prefers_korean(self) -> bool {
        self.code() == "ko"
    }

    /// The language new blank documents are written in with this interface ([`LangInfo::editing_lang`]).
    pub fn editing_lang(self) -> Option<&'static str> {
        self.info().and_then(|l| l.editing_lang)
    }

    /// Is the interface written in CJK script (its own name is), so it needs a CJK face?
    pub fn uses_cjk(self) -> bool {
        self.name().chars().any(|c| ('\u{2E80}'..='\u{9FFF}').contains(&c) || ('\u{AC00}'..='\u{D7AF}').contains(&c))
    }

    /// The embedded catalogs (shared, then WordCraft's) this language reads; none for English.
    fn catalog_texts(self) -> impl Iterator<Item = &'static str> {
        let code = self.code();
        CRAFT_I18N.iter().filter(move |(c, ..)| c.eq_ignore_ascii_case(code)).flat_map(|(_, shared, app)| [*shared, *app])
    }

    /// The CJK interface text the fonts must cover with the Chinese (`hans`) or other CJK fallback
    /// order: the catalogs of the CJK languages using that order, and every language's own name (the
    /// Options list). The interface fonts check the embedded faces against it (#486).
    pub fn cjk_font_text(hans: bool) -> (Vec<&'static str>, String) {
        let catalogs = Lang::all().filter(|l| l.uses_cjk() && l.prefers_hans() == hans).flat_map(Lang::catalog_texts).collect();
        (catalogs, Lang::all().map(Lang::name).collect::<Vec<_>>().join(" "))
    }

    /// The CJK interface catalogs using the Korean or Chinese writing-system preference.
    pub fn cjk_font_text_for(hans: bool, korean: bool) -> (Vec<&'static str>, String) {
        let catalogs =
            Lang::all().filter(|l| l.uses_cjk() && l.prefers_hans() == hans && l.prefers_korean() == korean).flat_map(Lang::catalog_texts).collect();
        (catalogs, Lang::all().map(Lang::name).collect::<Vec<_>>().join(" "))
    }

    /// A language by its exact code (any case): one of [`LANGUAGES`] whose catalogs are embedded.
    pub fn from_code(code: &str) -> Option<Lang> {
        let info = LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))?;
        I18N.from_code(info.code).map(|lang| Lang { lang, catalogs: None })
    }

    /// Resolve the `language` setting: a language code, or [`AUTO`] (and anything unknown, e.g. a
    /// code from a newer version) to follow the system.
    pub fn from_pref(pref: &str) -> Lang {
        Lang::from_code(pref).unwrap_or_else(system_lang)
    }

    /// Every language the interface can be shown in: [`LANGUAGES`] in order, those with embedded
    /// catalogs (English only in a build without craft-i18n).
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().filter_map(|l| Lang::from_code(l.code))
    }

    /// Every registered language in the order a picker lists them: English first (the source
    /// language and the fallback), then the rest by their own name with accents folded, so
    /// Čeština sits under C rather than after Z. Names in other scripts follow the Latin ones in
    /// code point order: Greek, Cyrillic, Arabic, then the CJK languages.
    pub fn for_picker() -> Vec<Lang> {
        let mut rest: Vec<Lang> = Lang::all().filter(|l| l.code() != Lang::EN.code()).collect();
        rest.sort_by_cached_key(|l| name_sort_key(l.name()));
        std::iter::once(Lang::EN).chain(rest).collect()
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

/// Portuguese other than Brazil's (`pt`, `pt-PT`, `pt_AO.UTF-8`): WordCraft has no catalog for
/// it (the European vocabulary differs), so it stays English rather than falling to `pt-br` as the
/// crate's matching would.
fn non_brazilian_portuguese(tag: &str) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    let mut parts = tag.split(['.', '@']).next().unwrap_or("").split(['-', '_']);
    parts.next() == Some("pt") && !parts.any(|p| p == "br")
}

/// The registered language for a locale tag such as `ja_JP.UTF-8` or `zh-Hant-TW`; `None` when
/// there is no catalog for it. `C`/`POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    if non_brazilian_portuguese(tag) {
        return None;
    }
    I18N.match_tag(tag).and_then(|lang| Lang::from_code(I18N.code(lang)))
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

/// The system region (#838): it picks the paper File › New starts on, the default measurement
/// unit and the decimal separator, independently of the interface language (an English
/// interface in Germany still gets A4 and centimetres). `en-US` in tests.
pub fn system_region() -> wordcraft_engine::Region {
    if cfg!(test) {
        return wordcraft_engine::Region::default();
    }
    // The environment's locale variables (Unix), else the platform's first preferred locale (on
    // macOS it carries the region, as in `en-PL`; on the web it is the browser's language).
    wordcraft_engine::Region::from_locale_vars(|name| std::env::var(name).ok(), sys_locale::get_locale)
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

/// Does `lang` have a translation for this string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.i18n().get(lang.lang, "", s).is_some()
}

/// `s` in the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// `s` in the current language as worded for `context` (`preview`, `template`), falling back to
/// the plain translation and then to English.
pub fn tc<'a>(context: &str, s: &'a str) -> &'a str {
    trc(current(), context, s)
}

/// [`tc`] in a given language.
pub fn trc<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    #[cfg(test)]
    if pseudo::on() {
        return I18N.tc(craft_i18n::Lang::PSEUDO, context, s);
    }
    lang.i18n().tc(lang.lang, context, s)
}

/// `s`'s translation for `context` in the current language, with no fallback to the plain row
/// or English: for a word whose plain translation means something else here (the "Open"
/// paragraph-spacing preset is not the file verb).
pub fn get<'a>(context: &str, s: &'a str) -> Option<&'a str> {
    #[cfg(test)]
    if pseudo::on() {
        return I18N.get(craft_i18n::Lang::PSEUDO, context, s);
    }
    let lang = current();
    lang.i18n().get(lang.lang, context, s)
}

/// Lowercase translated interface labels and UI search queries in the current language.
/// Turkish has two distinct I pairs: `İ`/`i` and `I`/`ı`. Unicode's default casing turns `İ`
/// into `i` plus a combining dot, which would prevent `italik` from finding `İtalik`.
/// Command IDs and document text keep their own existing casing semantics.
pub fn lowercase(s: &str) -> String {
    if current().code() != "tr" {
        return s.to_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'İ' => out.push('i'),
            'I' => out.push('ı'),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// `s` in `lang`; strings without a translation come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    #[cfg(test)]
    if pseudo::on() {
        return I18N.t(craft_i18n::Lang::PSEUDO, s);
    }
    lang.i18n().t(lang.lang, s)
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

/// A test language whose WordCraft catalog is `source` (catalog format), so tests of lookup code
/// don't depend on the embedded catalogs' wording. `code` selects code-specific rules such as
/// Turkish casing.
#[cfg(test)]
pub(crate) fn fixture_lang(code: &'static str, source: &'static str) -> Lang {
    let embedded: &'static craft_i18n::Embedded = Box::leak(Box::new([(code, "", source)]));
    let (i18n, _) = I18n::new(embedded, &[code]);
    let i18n: &'static I18n = Box::leak(Box::new(i18n));
    let lang = i18n.from_code(code).unwrap_or(craft_i18n::Lang::EN);
    Lang { lang, catalogs: Some(i18n) }
}

/// For a test that needs the embedded catalogs: `false`, with a note, when none were embedded
/// (`CRAFT_I18N_DIR` unset), so the test can skip; the interface is then English only.
#[cfg(test)]
pub(crate) fn catalogs_embedded() -> bool {
    if CRAFT_I18N.is_empty() {
        eprintln!("skipped: built without craft-i18n (CRAFT_I18N_DIR); the interface is English only");
    }
    !CRAFT_I18N.is_empty()
}

#[cfg(test)]
mod audit;
#[cfg(test)]
pub(crate) mod pseudo;
#[cfg(test)]
mod tests;

/// `name` lowercased with the Latin accents that occur in language names folded away, so a
/// byte-wise sort reads alphabetically. Other scripts are left as they are.
fn name_sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è'..='ë' | 'ē' | 'ě' | 'ę' | 'ế' | 'ệ' => 'e',
            'ì'..='ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò'..='ö' | 'ø' | 'ő' => 'o',
            'ś' | 'š' => 's',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod picker_order_tests {
    use super::*;

    #[test]
    fn picker_lists_english_first_then_names_alphabetically() {
        let order = Lang::for_picker();
        assert_eq!(order.first().map(|l| l.code()), Some(Lang::EN.code()));
        assert_eq!(order.len(), Lang::all().count());
        let keys: Vec<String> = order[1..].iter().map(|l| name_sort_key(l.name())).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
        assert_eq!(name_sort_key("Čeština"), "cestina");
        assert_eq!(name_sort_key("Türkçe"), "turkce");
    }
}
