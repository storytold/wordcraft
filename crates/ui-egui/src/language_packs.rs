//! Language packs in the window (CP4.3): the list in File › Options › Language, the missing-pack
//! bar over the page and About › Language data. The engine does the work
//! ([`wordcraft_engine::packs`]): in the app installs and fetches of the list run on its thread,
//! whoever asks (these buttons, or `proof.installPack` from MCP and the control channel), so the
//! window keeps drawing. A click of the person's own on Install, Add a language… or
//! Refresh is their consent: it turns on the font store's downloading (there is no second
//! prompt, R6.3). A click the control channel made never does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{RichText, Ui, vec2};
use serde_json::json;
use wordcraft_engine::packs::{self, LanguageRow, Pack, Proofing};

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// How long the bar keeps what it worked out while the document is being typed in.
const MISSING_REFRESH_SECS: f64 = 1.0;
/// Most characters of a language's name the bar shows (an index may name it at any length).
const MAX_BAR_NAME: usize = 40;

/// Text from the pack index as shown: without control, bidi and other format characters (the
/// index isn't ours to trust), and at most `max` characters.
pub(crate) fn shown(text: &str, max: usize) -> String {
    shown_keeping(text, max, false)
}

/// [`shown`] keeping line breaks and tabs: a licence's text.
fn shown_text(text: &str) -> String {
    shown_keeping(text, usize::MAX, true)
}

/// Most characters of a failure the window shows: the font store's errors can quote the index
/// URL or the server's answer, which aren't ours to trust either.
const MAX_ERROR: usize = 500;

/// A failure from the engine or the font store as shown: [`shown`], at most [`MAX_ERROR`]
/// characters.
fn shown_error(e: &str) -> String {
    shown(e, MAX_ERROR)
}

fn shown_keeping(text: &str, max: usize, lines: bool) -> String {
    // Line and paragraph separators (U+2028/9) break lines even where `lines` is off, and tag
    // characters (U+E0000–U+E007F) are invisible text an index could hide words in.
    let format = |c: char| {
        (c.is_control() && !(lines && matches!(c, '\n' | '\t')))
            || matches!(
                c,
                '\u{061C}'
                    | '\u{00AD}'
                    | '\u{180E}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{2028}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
                    | '\u{FFF9}'..='\u{FFFB}'
                    | '\u{E0000}'..='\u{E007F}'
            )
    };
    let clean: String = text.chars().filter(|c| !format(*c)).collect();
    if clean.chars().count() <= max {
        return clean;
    }
    let mut cut: String = clean.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The window's pack state.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Failures the window had itself (turning downloading on, starting, removing), by locale
    /// ("" for the list), shown until the next try. The engine keeps the installs' own.
    errors: HashMap<String, String>,
    /// (document, pack locale) pairs whose bar was closed.
    dismissed: HashSet<(u64, String)>,
    /// (document, language tag) pairs whose bar was closed before the list was known: the pack
    /// the list then has for the tag stays closed.
    dismissed_tags: HashSet<(u64, String)>,
    /// What the bar offers for the shown document, what it was worked out from, and when.
    missing: Option<(MissingKey, f64, Offers)>,
    /// [`packs::finished`] when last looked at.
    finished: u64,
    /// The Your languages rows, and what they were worked out from.
    rows: Option<(RowsKey, Vec<LanguageRow>)>,
    /// Is the Add a language window open?
    picker: bool,
    /// Its search text.
    search: String,
    /// Has its search box been given focus since it opened?
    focused: bool,
    /// The person clicked the bar's Install before the list was known: the list's failure then
    /// shows in the bar (an automatic fetch's stays in Options).
    asked: bool,
}

/// What the bar can offer.
#[derive(Clone, Default)]
struct Offers {
    /// Available packs the document asks for ([`packs::missing`]).
    packs: Vec<Pack>,
    /// Languages it asks for while there is no list yet ([`packs::unlisted`]).
    unlisted: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
struct MissingKey {
    document: u64,
    rev: u64,
    generation: u64,
    /// [`packs::index_generation`]: the list changed.
    packs: u64,
    /// The caret's typing language ([`packs::caret_language`]): choosing a language with nothing
    /// selected changes no text.
    caret: Option<String>,
}

/// Bytes as people read them: `1.9 MB`, `640 KB`.
pub(crate) fn size_text(bytes: u64) -> String {
    let d = crate::options::decimal().to_string();
    if bytes >= 1_000_000 {
        let tenths = bytes.saturating_add(50_000) / 100_000;
        crate::i18n::fmt(tl!("{n} MB"), &[("n", &format!("{}{d}{}", tenths / 10, tenths % 10))])
    } else {
        crate::i18n::fmt(tl!("{n} KB"), &[("n", &bytes.div_ceil(1000).max(1).to_string())])
    }
}

/// English names of the Language pack languages the Language dialog's list doesn't have, by the
/// tag a document stores for them ([`crate::ribbon::tag_for_pack`]): every pack in the published
/// index has a name in the interface language (CP4.3: Belarusian showed as Беларуская).
pub(crate) const MORE_LANGUAGES: [(&str, &str); 28] = [
    ("af-ZA", "Afrikaans"),
    ("be-BY", "Belarusian"),
    ("bg-BG", "Bulgarian"),
    ("bs-BA", "Bosnian"),
    ("ca-ES", "Catalan"),
    ("ca-ES-valencia", "Catalan (Valencian)"),
    ("cy-GB", "Welsh"),
    ("eo", "Esperanto"),
    ("fa-IR", "Persian"),
    ("ga-IE", "Irish"),
    ("gl-ES", "Galician"),
    ("hi-IN", "Hindi"),
    ("hu-HU", "Hungarian"),
    ("hy-AM", "Armenian"),
    ("is-IS", "Icelandic"),
    ("ka-GE", "Georgian"),
    ("lt-LT", "Lithuanian"),
    ("lv-LV", "Latvian"),
    ("mk-MK", "Macedonian"),
    ("nn-NO", "Norwegian (Nynorsk)"),
    ("ro-RO", "Romanian"),
    ("sk-SK", "Slovak"),
    ("sl-SI", "Slovenian"),
    ("sq-AL", "Albanian"),
    ("sw-KE", "Swahili"),
    ("ta-IN", "Tamil"),
    ("th-TH", "Thai"),
    ("vi-VN", "Vietnamese"),
];

/// The language a pack is for, in the interface language (`Dutch`, `Serbian (Latin)`): the name
/// of the tag text in it is marked with ([`crate::ribbon::tag_for_pack`]), as the status bar and
/// the Language dialog name it; else the pack's own name; else its locale.
pub(crate) fn language_of(pack_locale: &str, endonym: &str) -> String {
    let tag = crate::ribbon::tag_for_pack(pack_locale);
    if !crate::ribbon::language_name(Some(&tag)).eq_ignore_ascii_case(&tag) {
        return crate::ribbon::display_name(Some(&tag));
    }
    if endonym.is_empty() { pack_locale.to_string() } else { endonym.to_string() }
}

/// Once per frame: installs and fetches run on the engine's thread, which repaints the window as
/// they go; when one ends, the bar works out again what to offer. (Every open document is
/// proofed again by itself: its layout follows the dictionaries' generation.)
pub(crate) fn tick(app: &mut WordApp) {
    if !packs::runs_in_background()
        && packs::backend().is_some()
        && let Some(ctx) = app.ctx.clone()
    {
        packs::set_background(Some(Arc::new(move || ctx.request_repaint())));
    }
    // Downloading already on and no list (none cached, or one for an older pin): fetch it once.
    packs::refresh_if_stale();
    let finished = packs::finished();
    if finished != app.packs_rt.finished {
        app.packs_rt.finished = finished;
        app.packs_rt.missing = None;
    }
}

fn busy() -> bool {
    !packs::installing().is_empty() || packs::fetching()
}

/// The install's progress, (done, total) bytes, while `locale` is installing.
fn progress(locale: &str) -> Option<(u64, u64)> {
    packs::installing().into_iter().find(|p| p.locale == locale).map(|p| (p.done, p.total.max(1)))
}

/// The failure to show for `locale` ("" for the list): the window's own, else the engine's.
fn error_of(app: &WordApp, locale: &str) -> Option<String> {
    if let Some(e) = app.packs_rt.errors.get(locale) {
        return Some(e.clone());
    }
    let e = packs::error(locale)?;
    Some(if locale.is_empty() {
        crate::i18n::fmt(tl!("Couldn't get the list of languages: {error}"), &[("error", &shown_error(&e))])
    } else {
        crate::i18n::fmt(tl!("Couldn't install: {error}"), &[("error", &shown_error(&e))])
    })
}

/// Downloading on for a click on Install, Add a language… or Refresh: the person's
/// consent. A click the control channel made (an agent's `click`) leaves it off and says why.
fn consent(app: &mut WordApp, ctx: &egui::Context, key: &str) -> bool {
    let Some(backend) = packs::backend() else { return false };
    if backend.network_on() {
        return true;
    }
    if app.input_from_control(ctx) {
        let why = tl!("Only you can turn on downloading: click the button yourself. Agents can install once it is on.");
        app.packs_rt.errors.insert(key.to_string(), why.to_string());
        return false;
    }
    if let Err(e) = backend.enable_network() {
        app.packs_rt.errors.insert(key.to_string(), crate::i18n::fmt(tl!("Couldn't turn on downloading: {error}"), &[("error", &shown_error(&e))]));
        return false;
    }
    true
}

/// Install `pack` (on the engine's thread): the person clicked Install.
fn start_install(app: &mut WordApp, ctx: &egui::Context, pack: &Pack) {
    app.packs_rt.errors.remove(&pack.locale);
    if !consent(app, ctx, &pack.locale) {
        return;
    }
    if let Err(e) = packs::start_install(&pack.locale) {
        app.packs_rt.errors.insert(pack.locale.clone(), crate::i18n::fmt(tl!("Couldn't install: {error}"), &[("error", &shown_error(&e))]));
    }
    app.packs_rt.missing = None;
}

/// Fetch the list again (on the engine's thread): the person clicked Add a language…
/// or Refresh.
fn start_fetch(app: &mut WordApp, ctx: &egui::Context) {
    app.packs_rt.errors.remove("");
    if !consent(app, ctx, "") {
        return;
    }
    if let Err(e) = packs::start_refresh() {
        app.packs_rt
            .errors
            .insert(String::new(), crate::i18n::fmt(tl!("Couldn't get the list of languages: {error}"), &[("error", &shown_error(&e))]));
    }
}

fn remove(app: &mut WordApp, locale: &str) {
    match app.run("proof.removePack", json!({"locale": locale})) {
        Ok(_) => {
            app.packs_rt.errors.remove(locale);
            app.packs_rt.missing = None;
        }
        Err(e) => {
            app.packs_rt.errors.insert(locale.to_string(), crate::i18n::fmt(tl!("Couldn't remove: {error}"), &[("error", &shown_error(&e))]));
        }
    }
}

/// What the bar offers for the shown document. Worked out again when the document, the caret's
/// language, the packs or the list changed; while it is being typed in, at most once a second.
fn missing(app: &mut WordApp, now: f64) -> Offers {
    let key = MissingKey {
        document: app.session.document_id(),
        rev: app.session.rev(),
        generation: wordcraft_engine::proof::dictionary_generation(),
        packs: packs::index_generation(),
        caret: packs::caret_language(&app.session),
    };
    if let Some((k, at, m)) = &app.packs_rt.missing
        && (*k == key || (MissingKey { rev: key.rev, ..k.clone() } == key && now - at < MISSING_REFRESH_SECS))
    {
        return m.clone();
    }
    let packs = packs::missing(&app.session);
    let unlisted = if packs.is_empty() { packs::unlisted(&app.session) } else { Vec::new() };
    let m = Offers { packs, unlisted };
    app.packs_rt.missing = Some((key, now, m.clone()));
    m
}

/// Get `tag`'s pack before the list is known: the person clicked the bar's Install. Their click
/// turns on downloading ([`consent`]); the list is fetched, then the pack installed if it lists
/// one (on the engine's thread).
fn start_get(app: &mut WordApp, ctx: &egui::Context, tag: &str) {
    app.packs_rt.errors.remove("");
    if !consent(app, ctx, "") {
        return;
    }
    app.packs_rt.asked = true;
    if let Err(e) = packs::start_get(tag) {
        app.packs_rt
            .errors
            .insert(String::new(), crate::i18n::fmt(tl!("Couldn't get the list of languages: {error}"), &[("error", &shown_error(&e))]));
    }
    app.packs_rt.missing = None;
}

/// The missing-pack bar under the ribbon: "<Language> proofing isn't installed. Install (<size>)",
/// for the first language of the document with an available pack that isn't installed. Closing it
/// hides it for that language in this document. Never shown for a language with no pack.
pub(crate) fn bar(app: &mut WordApp, ui: &mut Ui) {
    if packs::backend().is_none() {
        return;
    }
    let document = app.session.document_id();
    let now = ui.input(|i| i.time);
    let offers = missing(app, now);
    // The whole list, for a tag closed before it was known (only then: it is a copy).
    let index = if app.packs_rt.dismissed_tags.is_empty() { Vec::new() } else { packs::available() };
    let Some(pack) = offers.packs.into_iter().find(|p| !dismissed(app, document, p, &index)) else {
        if let Some(tag) = offers.unlisted.into_iter().find(|t| !app.packs_rt.dismissed_tags.contains(&(document, t.clone()))) {
            unlisted_bar(app, ui, &tag);
        }
        return;
    };
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("language_pack_bar")
        .exact_size(36.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            let bottom = ui.max_rect().bottom();
            let full = ui.max_rect();
            ui.painter().hline(full.x_range(), bottom - 0.5, egui::Stroke::new(1.0, t.border));
            ui.horizontal_centered(|ui| {
                let language = shown(&language_of(&pack.locale, &pack.name), MAX_BAR_NAME);
                ui.label(RichText::new(crate::i18n::fmt(tl!("{language} proofing isn't installed."), &[("language", &language)])).color(t.text));
                match progress(&pack.locale) {
                    Some((done, total)) => {
                        let pct = (done.min(total) * 100 / total).to_string();
                        ui.label(RichText::new(crate::i18n::fmt(tl!("Installing… {percent}%"), &[("percent", &pct)])).color(t.text_dim));
                    }
                    None => {
                        let label = crate::i18n::fmt(tl!("Install ({size})"), &[("size", &size_text(pack.size))]);
                        if ui.add(crate::widgets::primary_button(ui, label)).on_hover_text(licence_hover(&pack)).clicked() {
                            start_install(app, &ui.ctx().clone(), &pack);
                        }
                    }
                }
                if let Some(e) = error_of(app, &pack.locale) {
                    ui.add(egui::Label::new(RichText::new(e).color(t.red)).truncate());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, "close", tl!("Not now")).clicked() {
                        app.packs_rt.dismissed.insert((document, pack.locale.clone()));
                    }
                });
            });
        });
}

/// Whether the bar for `pack` was closed in `document`: for the pack itself, or for a language
/// tag it serves, closed before the list was known (`kl-GL` for `kl`).
fn dismissed(app: &WordApp, document: u64, pack: &Pack, index: &[Pack]) -> bool {
    // A closed pack is that pack alone (closing `de-ch` leaves `de`); a closed tag is the pack it
    // finds in the whole list (`de-CH` is `de-ch`'s, not `de`'s, once the list has both).
    app.packs_rt.dismissed.contains(&(document, pack.locale.clone()))
        || app
            .packs_rt
            .dismissed_tags
            .iter()
            .any(|(d, tag)| *d == document && packs::pack_for_tag(tag, index).is_some_and(|p| p.locale == pack.locale))
}

/// The bar for a language the document asks for while there is no list of packs yet (a fresh
/// install, or a list cached for an older pin): "<Language> proofing isn't installed. Install".
/// Install fetches the list, then installs the language's pack if it has one; if not, the bar
/// goes. Hidden while an automatic fetch of the list runs.
fn unlisted_bar(app: &mut WordApp, ui: &mut Ui, tag: &str) {
    let fetching = packs::fetching();
    if fetching && !app.packs_rt.asked {
        return;
    }
    let document = app.session.document_id();
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("language_pack_bar")
        .exact_size(36.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(ui, |ui| {
            let bottom = ui.max_rect().bottom();
            let full = ui.max_rect();
            ui.painter().hline(full.x_range(), bottom - 0.5, egui::Stroke::new(1.0, t.border));
            ui.horizontal_centered(|ui| {
                let language = shown(&language_of(&tag.to_ascii_lowercase(), ""), MAX_BAR_NAME);
                ui.label(RichText::new(crate::i18n::fmt(tl!("{language} proofing isn't installed."), &[("language", &language)])).color(t.text));
                if fetching {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Getting the list of languages…")).color(t.text_dim));
                } else {
                    let mut hover = vec![tl!("Gets the list of Language packs, then this language's pack if there is one.").to_string()];
                    if packs::backend().is_some_and(|b| !b.network_on()) {
                        hover.push(tl!("Installing turns on downloading of fonts and Language packs from GitHub.").to_string());
                    }
                    if ui.add(crate::widgets::primary_button(ui, tl!("Install").to_string())).on_hover_text(hover.join("\n")).clicked() {
                        start_get(app, &ui.ctx().clone(), tag);
                    }
                }
                // The window's own failure (consent), else the list's when the person asked.
                let e = app.packs_rt.errors.get("").cloned().or_else(|| if app.packs_rt.asked { error_of(app, "") } else { None });
                if let Some(e) = e {
                    ui.add(egui::Label::new(RichText::new(e).color(t.red)).truncate());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, "close", tl!("Not now")).clicked() {
                        app.packs_rt.dismissed_tags.insert((document, tag.to_string()));
                    }
                });
            });
        });
}

/// The bar's Install tooltip: the licence and source, that it turns on downloading when it is
/// off, and the test source when packs come from one.
fn licence_hover(pack: &Pack) -> String {
    let mut lines = vec![crate::i18n::fmt(
        tl!("Licence: {licence}. Source: {source}"),
        &[("licence", &shown(&pack.licence, 300)), ("source", &shown(pack.source_link(), 300))],
    )];
    if packs::backend().is_some_and(|b| !b.network_on()) {
        lines.push(tl!("Installing turns on downloading of fonts and Language packs from GitHub.").to_string());
    }
    if let Some(source) = packs::index_override() {
        lines.push(crate::i18n::fmt(tl!("Language packs come from a test source: {source}"), &[("source", &shown(&source, 300))]));
    }
    lines.join("\n")
}

/// What the Your languages list needs of the engine's rows, worked out again when the document,
/// the packs, the dictionaries, the default language or the folder change.
type RowsKey = (MissingKey, Option<String>, Option<String>, Option<String>);

fn your_rows(app: &mut WordApp) -> Vec<LanguageRow> {
    let key: RowsKey = (
        MissingKey {
            document: app.session.document_id(),
            rev: app.session.rev(),
            generation: wordcraft_engine::proof::dictionary_generation(),
            packs: packs::index_generation(),
            // The rows don't follow the caret (it is only made bold when drawn).
            caret: None,
        },
        app.session.prefs.default_language.clone(),
        app.session.prefs.dictionaries_folder.clone(),
        // The default when none is saved: changing the interface language changes it, with no edit.
        app.session.editing_lang.clone(),
    );
    if let Some((k, rows)) = &app.packs_rt.rows
        && *k == key
    {
        return rows.clone();
    }
    let rows = packs::your_languages(&app.session);
    app.packs_rt.rows = Some((key, rows.clone()));
    rows
}

/// The tag Set as default sets for a row: the one the document or default uses, else the one a
/// text in the pack's language is marked with.
fn row_tag(row: &LanguageRow) -> String {
    row.tags.first().cloned().or_else(|| row.pack.as_deref().map(crate::ribbon::tag_for_pack)).unwrap_or_default()
}

/// A row's interface name and, beneath it, its own name and code (`Nederlands · nl`).
fn row_names(row: &LanguageRow, index: &[Pack], installed: &[packs::Installed]) -> (String, String) {
    let tag = row_tag(row);
    let endonym = row
        .pack
        .as_deref()
        .and_then(|l| {
            index.iter().find(|p| p.locale == l).map(|p| p.name.clone()).or_else(|| installed.iter().find(|i| i.locale == l).map(|i| i.name.clone()))
        })
        .unwrap_or_default();
    let named = crate::ribbon::language_name(Some(&tag));
    let name = if named.eq_ignore_ascii_case(&tag) && !endonym.trim().is_empty() {
        shown(&endonym, MAX_BAR_NAME)
    } else {
        crate::ribbon::display_name(Some(&tag))
    };
    let endonym = shown(&endonym, MAX_BAR_NAME);
    // The tag comes from the document (or the index): shown like the pack's own name.
    let tag = shown(&tag, MAX_BAR_NAME);
    let detail = if endonym.is_empty() || endonym == name { tag } else { format!("{endonym} · {tag}") };
    (name, detail)
}

/// Does `query` find a pack: part of its interface name or own name, or the start of its code
/// (dutch, nederlands, nl).
fn matches(query: &str, name: &str, endonym: &str, locale: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || name.to_lowercase().contains(&q) || endonym.to_lowercase().contains(&q) || locale.to_lowercase().starts_with(&q)
}

/// The packs the picker offers for `query`: not installed, by interface name.
fn picker_packs(index: &[Pack], installed: &[packs::Installed], query: &str) -> Vec<(String, Pack)> {
    let mut out: Vec<(String, Pack)> = index
        .iter()
        .filter(|p| !installed.iter().any(|i| i.locale == p.locale))
        .map(|p| (shown(&language_of(&p.locale, &p.name), MAX_BAR_NAME), p.clone()))
        .filter(|(name, p)| matches(query, name, &shown(&p.name, MAX_BAR_NAME), &p.locale))
        .collect();
    out.sort_by_cached_key(|(name, p)| (crate::ribbon::sort_key(name), p.locale.clone()));
    out
}

/// Your languages as listed: interface name, own name and code, and the row, by name.
fn your_view(app: &mut WordApp, index: &[Pack], installed: &[packs::Installed]) -> Vec<(String, String, LanguageRow)> {
    let mut view: Vec<(String, String, LanguageRow)> = your_rows(app)
        .into_iter()
        .map(|r| {
            let (name, detail) = row_names(&r, index, installed);
            (name, detail, r)
        })
        .collect();
    view.sort_by_cached_key(|(name, _, r)| (crate::ribbon::sort_key(name), r.pack.clone()));
    view
}

/// File › Options › Language › Your languages: the languages in use and installed, each with
/// where its proofing comes from and what can be done, and Add a language….
pub(crate) fn your_languages_section(app: &mut WordApp, ui: &mut Ui) {
    // Language names include Arabic and other scripts; ask for a system Arabic face when none is embedded.
    app.want_system_arabic = true;
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Your languages")).font(semibold(15.0)));
    ui.add_space(2.0);
    ui.label(
        RichText::new(tl!(
            "English, the languages of this document and every installed Language pack. Language packs add spelling dictionaries and hyphenation."
        ))
        .small()
        .weak(),
    );
    ui.add_space(4.0);
    let backend = packs::backend();
    if backend.is_none() {
        let why = if cfg!(target_arch = "wasm32") {
            tl!("Language packs aren't available in the browser.")
        } else {
            tl!("Language packs need the font store folder, and there is none on this computer.")
        };
        ui.label(RichText::new(why).small().weak());
    }
    if let Some(source) = packs::index_override() {
        let line = crate::i18n::fmt(tl!("Language packs come from a test source: {source}"), &[("source", &shown(&source, 300))]);
        ui.add(egui::Label::new(RichText::new(line).small().color(t.orange)).wrap());
    }
    let index = packs::available();
    let installed = packs::installed();
    let caret = crate::ribbon::caret_lang_tag(app);
    let view = your_view(app, &index, &installed);
    let mut install = None;
    let mut removal = None;
    let mut default = None;
    // As wide as the window leaves room for (the rows scroll sideways in a narrow one).
    let room = (ui.ctx().content_rect().right() - ui.cursor().min.x - 16.0).clamp(200.0, 760.0);
    let narrow = room < 660.0;
    egui::ScrollArea::both().id_salt("options_your_languages").max_width(room).max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
        crate::rtl::grid(
            ui,
            egui::Grid::new("options_your_languages_grid").num_columns(if narrow { 2 } else { 3 }).spacing(vec2(16.0, 8.0)).striped(true),
            |ui| {
                for (name, detail, row) in &view {
                    let mut actions = |ui: &mut Ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                            if !row.default
                                && ui
                                    .small_button(tl!("Set as default"))
                                    .on_hover_text(tl!("New blank documents are written in this language"))
                                    .clicked()
                            {
                                default = Some(row_tag(row));
                            }
                            if row.installed
                                && let Some(locale) = &row.pack
                                && ui.add_enabled(!busy(), egui::Button::new(tl!("Remove")).small()).clicked()
                            {
                                removal = Some(locale.clone());
                            }
                        });
                    };
                    let here = caret.as_deref().is_some_and(|c| row.is_tag(c));
                    ui.vertical(|ui| {
                        ui.set_width(if narrow { 170.0 } else { 250.0 });
                        let shown_name = shown(name, MAX_BAR_NAME);
                        let line = if row.default { crate::i18n::fmt(tl!("{language} (default)"), &[("language", &shown_name)]) } else { shown_name };
                        ui.label(if here { RichText::new(line).font(semibold(14.0)) } else { RichText::new(line) });
                        ui.label(RichText::new(detail.as_str()).small().weak());
                    });
                    let pack = row.pack.as_deref().and_then(|l| index.iter().find(|p| p.locale == l));
                    let size = size_text(row.size);
                    ui.vertical(|ui| {
                        ui.set_width(190.0);
                        match row.proofing {
                            Proofing::BuiltIn => {
                                ui.label(tl!("Built in"));
                            }
                            Proofing::Installed => {
                                ui.label(RichText::new(tl!("Installed")).color(t.green));
                            }
                            Proofing::OwnDictionary => {
                                ui.label(tl!("Your own dictionary"));
                            }
                            Proofing::SystemDictionary => {
                                ui.label(tl!("System dictionary"));
                            }
                            Proofing::NotAvailable => {
                                ui.label(RichText::new(tl!("Not available yet")).weak());
                            }
                            Proofing::GrammarBuiltIn => {
                                ui.label(tl!("Grammar built in"));
                            }
                            Proofing::Install => {}
                        }
                        if let Some(pack) = pack
                            && !row.installed
                            && matches!(row.proofing, Proofing::Install | Proofing::GrammarBuiltIn)
                        {
                            match progress(&pack.locale) {
                                Some((done, total)) => {
                                    ui.horizontal(|ui| {
                                        ui.add(egui::ProgressBar::new(done.min(total) as f32 / total as f32).desired_width(90.0).show_percentage());
                                        if ui.small_button(tl!("Cancel")).clicked() {
                                            packs::cancel(&pack.locale);
                                        }
                                    });
                                }
                                None => {
                                    let label = if row.proofing == Proofing::Install {
                                        crate::i18n::fmt(tl!("Install ({size})"), &[("size", &size)])
                                    } else {
                                        crate::i18n::fmt(tl!("Install spelling ({size})"), &[("size", &size)])
                                    };
                                    if ui.button(label).on_hover_text(licence_hover(pack)).clicked() {
                                        install = Some(pack.clone());
                                    }
                                }
                            }
                        }
                        if narrow {
                            actions(ui);
                        }
                    });
                    if !narrow {
                        actions(ui);
                    }
                    ui.end_row();
                    let error = row.pack.as_deref().and_then(|l| error_of(app, l));
                    let unreadable = (matches!(row.proofing, Proofing::OwnDictionary | Proofing::SystemDictionary))
                        .then(|| wordcraft_engine::proof::dictionary_load_error(wordcraft_engine::proof::ProofLang::from_tag(Some(&row_tag(row)))))
                        .flatten();
                    if error.is_some() || unreadable.is_some() {
                        ui.label("");
                        ui.vertical(|ui| {
                            if let Some(e) = error {
                                ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
                            }
                            if let Some(reason) = unreadable {
                                let line = crate::i18n::fmt(tl!("Its dictionary couldn't be read: {reason}"), &[("reason", &shown_error(&reason))]);
                                ui.add(egui::Label::new(RichText::new(line).small().color(t.red)).wrap());
                            }
                        });
                        if !narrow {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                }
            },
        );
    });
    ui.add_space(4.0);
    if let Some(e) = error_of(app, "") {
        ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(backend.is_some(), egui::Button::new(tl!("Add a language…"))).clicked() {
            open_picker(app, &ui.ctx().clone());
        }
        if packs::fetching() {
            ui.spinner();
            ui.label(RichText::new(tl!("Getting the list of languages…")).small());
        }
    });
    if backend.as_ref().is_some_and(|b| !b.network_on()) {
        ui.label(
            RichText::new(tl!(
                "Downloading is off. Clicking Add a language… or Install turns it on: WordCraft then downloads language packs and fonts from GitHub."
            ))
            .small()
            .weak(),
        );
    }
    if let Some(p) = install {
        start_install(app, &ui.ctx().clone(), &p);
    }
    if let Some(l) = removal {
        remove(app, &l);
    }
    if let Some(tag) = default
        && let Err(e) = app.run("review.language.preferred", json!({"lang": tag}))
    {
        app.status(e);
    }
}

/// The Add a language… click: the person's own click is their consent to download (an agent's
/// never is: it leaves downloading off and says why); then the picker opens, and the list of
/// languages is fetched when downloading was just turned on or there is none yet.
fn open_picker(app: &mut WordApp, ctx: &egui::Context) {
    let Some(backend) = packs::backend() else { return };
    app.packs_rt.errors.remove("");
    let was_on = backend.network_on();
    if !consent(app, ctx, "") {
        return;
    }
    if !was_on || packs::available().is_empty() {
        start_fetch(app, ctx);
    }
    app.packs_rt.search.clear();
    app.packs_rt.picker = true;
}

/// The Add a language window: search, then the packs not installed, by name, each with its size,
/// licence, source and Install. Stays open while installs run.
pub(crate) fn add_language_window(app: &mut WordApp, ctx: &egui::Context) {
    if !app.packs_rt.picker {
        return;
    }
    // Pack names include Arabic and other scripts.
    app.want_system_arabic = true;
    let t = Tokens::get(ctx);
    let mut open = true;
    egui::Window::new(tl!("Add a language"))
        .open(&mut open)
        .collapsible(false)
        .auto_sized()
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -80.0))
        .show(ctx, |ui| {
            // Right to left the search row and the list mirror, as the other windows.
            crate::rtl::root(ui, |ui| {
                let index = packs::available();
                let installed = packs::installed();
                ui.horizontal(|ui| {
                    let search_label = ui.label(tl!("Search:"));
                    let mut search = std::mem::take(&mut app.packs_rt.search);
                    let r = ui
                        .add(egui::TextEdit::singleline(&mut search).desired_width(280.0).hint_text(tl!("Dutch, Nederlands or nl")))
                        .labelled_by(search_label.id);
                    if !r.has_focus() && app.packs_rt.search.is_empty() && search.is_empty() && !app.packs_rt.focused {
                        r.request_focus();
                    }
                    app.packs_rt.focused = true;
                    app.packs_rt.search = search;
                    if packs::fetching() {
                        ui.spinner();
                        ui.label(RichText::new(tl!("Getting the list of languages…")).small());
                    } else if ui.button(tl!("Refresh")).on_hover_text(tl!("Get the list of languages again")).clicked() {
                        start_fetch(app, &ui.ctx().clone());
                    }
                });
                if let Some(e) = error_of(app, "") {
                    ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
                }
                ui.add_space(4.0);
                let list = picker_packs(&index, &installed, &app.packs_rt.search);
                if list.is_empty() {
                    let text = if index.is_empty() { tl!("No languages to show yet.") } else { tl!("No language matches your search.") };
                    ui.label(RichText::new(text).weak());
                }
                let mut install = None;
                egui::ScrollArea::both().id_salt("add_language_list").max_height(340.0).auto_shrink([true, true]).show(ui, |ui| {
                    crate::rtl::grid(ui, egui::Grid::new("add_language_grid").num_columns(4).spacing(vec2(14.0, 8.0)).striped(true), |ui| {
                        for (name, pack) in &list {
                            ui.vertical(|ui| {
                                ui.set_width(180.0);
                                ui.label(name.as_str());
                                let endonym = shown(&pack.name, MAX_BAR_NAME);
                                let detail = if endonym == *name { pack.locale.clone() } else { format!("{endonym} · {}", pack.locale) };
                                ui.label(RichText::new(detail.as_str()).small().weak());
                            });
                            ui.label(if pack.size > 0 { size_text(pack.size) } else { String::new() });
                            ui.vertical(|ui| {
                                ui.set_width(150.0);
                                let licence = shown(&pack.licence, 300);
                                ui.add(egui::Label::new(RichText::new(licence.as_str()).small()).truncate()).on_hover_text(licence_hover(pack));
                                let source = pack.source_link();
                                if !source.is_empty() {
                                    ui.hyperlink_to(tl!("Source"), source).on_hover_text(shown(source, 300));
                                }
                            });
                            ui.horizontal(|ui| match progress(&pack.locale) {
                                Some((done, total)) => {
                                    ui.add(egui::ProgressBar::new(done.min(total) as f32 / total as f32).desired_width(80.0).show_percentage());
                                    if ui.small_button(tl!("Cancel")).clicked() {
                                        packs::cancel(&pack.locale);
                                    }
                                }
                                None => {
                                    if ui.button(tl!("Install")).clicked() {
                                        install = Some(pack.clone());
                                    }
                                }
                            });
                            ui.end_row();
                            if let Some(e) = error_of(app, &pack.locale) {
                                ui.label("");
                                ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                            }
                        }
                    });
                });
                if let Some(p) = install {
                    start_install(app, &ui.ctx().clone(), &p);
                }
            })
        });
    if !open {
        app.packs_rt.picker = false;
        app.packs_rt.focused = false;
    }
}

/// Height of About › Language data's list.
const ABOUT_HEIGHT: f32 = 420.0;

/// The licence texts About shows, read again only when the dictionaries change (they can run to
/// megabytes).
static LICENCES: std::sync::Mutex<Option<(u64, Arc<Vec<(String, String)>>)>> = std::sync::Mutex::new(None);

fn licence_texts() -> Arc<Vec<(String, String)>> {
    let generation = wordcraft_engine::proof::dictionary_generation();
    let mut cached = LICENCES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((g, texts)) = cached.as_ref()
        && *g == generation
    {
        return texts.clone();
    }
    let texts: Arc<Vec<(String, String)>> =
        Arc::new(packs::licence_texts().into_iter().map(|(title, text)| (shown(&title, 200), shown_text(&text))).collect());
    *cached = Some((generation, texts.clone()));
    texts
}

/// About › Language data: the full licence text of the embedded English data and of each
/// installed pack.
pub(crate) fn about_tab(ui: &mut Ui) {
    ui.label(RichText::new(tl!("The spelling and hyphenation data WordCraft uses, and their licences.")).small().weak());
    ui.add_space(4.0);
    let texts = licence_texts();
    // A fixed height: entries open and long texts scroll inside it, and the dialog keeps its size.
    egui::ScrollArea::vertical()
        .id_salt("about_language_data")
        .min_scrolled_height(ABOUT_HEIGHT)
        .max_height(ABOUT_HEIGHT)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (title, text) in texts.iter() {
                egui::CollapsingHeader::new(RichText::new(title.as_str()).font(semibold(13.0))).id_salt(("language_data", title.as_str())).show(
                    ui,
                    |ui| {
                        ui.add(egui::Label::new(RichText::new(text).small().monospace()).wrap());
                    },
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::{Value, json};
    use wordcraft_engine::packs::{self, Backend, FixedBackend};

    use crate::WordApp;

    /// One 1440 by 900 frame as the desktop runs it, with the user's own `events`; control input
    /// goes in through the raw-input hook. Returns the accessibility tree.
    fn frame(a: &mut WordApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
        let time = ctx.input(|i| i.time) + 0.05;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        let mut raw = egui::RawInput { focused: true, time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
        a.raw_input_hook(ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        });
        out.textures_delta.clear();
        out.platform_output.accesskit_update.take().unwrap_or_else(|| egui::accesskit::TreeUpdate {
            nodes: Vec::new(),
            tree: None,
            tree_id: egui::accesskit::TreeId::ROOT,
            focus: egui::accesskit::NodeId(0),
        })
    }

    /// A request as a control client (or MCP's `click`) sends it, with frames until its input
    /// has gone in.
    fn as_agent(a: &mut WordApp, ctx: &egui::Context, method: &str, params: Value) {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        let crate::control::Outcome::Done(_) = crate::control::handle(a, ctx, &req) else { panic!("{method} replies at once") };
        for _ in 0..200 {
            frame(a, ctx, Vec::new());
            if a.synthetic.is_empty() && !a.synthetic_pending {
                break;
            }
        }
        for _ in 0..3 {
            frame(a, ctx, Vec::new());
        }
    }

    fn user_click(a: &mut WordApp, ctx: &egui::Context, (x, y): (f64, f64)) {
        let pos = egui::pos2(x as f32, y as f32);
        frame(a, ctx, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(a, ctx, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
        }
    }

    /// The centre of the first widget whose name starts with `label`.
    fn spot(tree: &egui::accesskit::TreeUpdate, label: &str) -> Option<(f64, f64)> {
        let (_, node) = tree.nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.starts_with(label)))?;
        let b = node.bounds()?;
        Some(((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0))
    }

    /// craft-proof calls Swiss German and Portugal's Portuguese pack languages (`de-ch`,
    /// `pt-pt`); the status bar and the pack list still name them as German and Portuguese.
    #[test]
    fn swiss_and_portugal_text_keep_their_names() {
        assert_eq!(crate::ribbon::language_name(Some("de-CH")), "German (Switzerland)");
        assert_eq!(crate::ribbon::language_name(Some("pt-PT")), "Portuguese (Portugal)");
        assert_eq!(super::language_of("de-ch", ""), "German (Switzerland)");
        assert_eq!(super::language_of("pt-pt", ""), "Portuguese (Portugal)");
        assert_eq!(super::language_of("ja", "日本語"), "Japanese");
    }

    /// Tests that set the process-wide pack backend take turns.
    static BACKEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Every locale of the published index (`language-packs-2/index.json`), with its own name.
    const INDEX_LOCALES: [(&str, &str); 75] = [
        ("af", "Afrikaans"),
        ("ar", "العربية"),
        ("be", "Беларуская"),
        ("bg", "Български"),
        ("bs", "Bosanski"),
        ("ca", "Català"),
        ("ca-valencia", "Català (valencià)"),
        ("cs", "Čeština"),
        ("cy", "Cymraeg"),
        ("da", "Dansk"),
        ("de", "Deutsch (Deutschland)"),
        ("de-at", "Deutsch (Österreich)"),
        ("de-ch", "Deutsch (Schweiz)"),
        ("el", "Ελληνικά"),
        ("eo", "Esperanto"),
        ("es", "Español (España)"),
        ("es-ar", "Español (Argentina)"),
        ("es-bo", "Español (Bolivia)"),
        ("es-cl", "Español (Chile)"),
        ("es-co", "Español (Colombia)"),
        ("es-cr", "Español (Costa Rica)"),
        ("es-cu", "Español (Cuba)"),
        ("es-do", "Español (República Dominicana)"),
        ("es-ec", "Español (Ecuador)"),
        ("es-gq", "Español (Guinea Ecuatorial)"),
        ("es-gt", "Español (Guatemala)"),
        ("es-hn", "Español (Honduras)"),
        ("es-mx", "Español (México)"),
        ("es-ni", "Español (Nicaragua)"),
        ("es-pa", "Español (Panamá)"),
        ("es-pe", "Español (Perú)"),
        ("es-ph", "Español (Filipinas)"),
        ("es-pr", "Español (Puerto Rico)"),
        ("es-py", "Español (Paraguay)"),
        ("es-sv", "Español (El Salvador)"),
        ("es-us", "Español (Estados Unidos)"),
        ("es-uy", "Español (Uruguay)"),
        ("es-ve", "Español (Venezuela)"),
        ("et", "Eesti"),
        ("fa", "فارسی"),
        ("fr", "Français"),
        ("ga", "Gaeilge"),
        ("gl", "Galego"),
        ("he", "עברית"),
        ("hi", "\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}"),
        ("hr", "Hrvatski"),
        ("hu", "Magyar"),
        ("hy", "Հայերեն"),
        ("id", "Bahasa Indonesia"),
        ("is", "Íslenska"),
        ("it", "Italiano"),
        ("ka", "ქართული"),
        ("ko", "한국어"),
        ("lt", "Lietuvių"),
        ("lv", "Latviešu"),
        ("mk", "Македонски"),
        ("nb", "Norsk bokmål"),
        ("nl", "Nederlands"),
        ("nn", "Norsk nynorsk"),
        ("pl", "Polski"),
        ("pt-br", "Português (Brasil)"),
        ("pt-pt", "Português (Portugal)"),
        ("ro", "Română"),
        ("ru", "Русский"),
        ("sk", "Slovenčina"),
        ("sl", "Slovenščina"),
        ("sq", "Shqip"),
        ("sr", "Српски"),
        ("sr-latn", "Srpski (latinica)"),
        ("sv", "Svenska"),
        ("sv-fi", "Svenska (Finland)"),
        ("sw", "Kiswahili"),
        ("ta", "\u{ba4}\u{bae}\u{bbf}\u{bb4}\u{bcd}"),
        ("tr", "Türkçe"),
        ("vi", "Tiếng Việt"),
    ];

    fn index_packs() -> Vec<packs::Pack> {
        let base = FixedBackend::sample().packs.remove(0);
        INDEX_LOCALES.iter().map(|(l, n)| packs::Pack { locale: (*l).into(), name: (*n).into(), ..base.clone() }).collect()
    }

    /// CP4.3: every pack of the published index has an English name (Belarusian showed as
    /// Беларуская, Arabic as its own name), no two packs share one, and a pack's language is
    /// named as the status bar names the tag text in it is marked with.
    #[test]
    fn every_published_pack_has_an_english_name() {
        let mut seen = std::collections::HashSet::new();
        for (locale, endonym) in INDEX_LOCALES {
            let name = super::language_of(locale, endonym);
            // In Latin letters (Norwegian (Bokmål)), never the pack's own name in its script.
            assert!(name.chars().all(|c| (c as u32) < 0x250) && name != *locale, "{locale}: {name}");
            assert!(seen.insert(name.clone()), "{locale}: {name} twice");
            assert_eq!(name, crate::ribbon::language_name(Some(&crate::ribbon::tag_for_pack(locale))), "{locale}");
        }
        assert_eq!(super::language_of("be", "Беларуская"), "Belarusian");
        assert_eq!(super::language_of("ar", "العربية"), "Arabic");
        assert_eq!(super::language_of("nn", "Norsk nynorsk"), "Norwegian (Nynorsk)");
        assert_eq!(super::language_of("es-cu", ""), "Spanish (Cuba)");
        // A locale WordCraft has no name for keeps the pack's own name.
        assert_eq!(super::language_of("xq", "Xq name"), "Xq name");
    }

    /// CP4.3: the tag text is marked with when a pack's language is chosen finds that pack again,
    /// for every published pack: Bulgarian chosen from its pack is proofed by it, Serbian (Latin)
    /// by `sr-latn` rather than the Cyrillic `sr`.
    #[test]
    fn a_pack_language_tag_finds_its_pack() {
        let index = index_packs();
        for p in &index {
            let tag = crate::ribbon::tag_for_pack(&p.locale);
            assert_eq!(packs::pack_for_tag(&tag, &index).map(|f| f.locale.as_str()), Some(p.locale.as_str()), "{} → {tag}", p.locale);
            assert!(packs::proofable(&tag), "{tag}");
        }
        // Valencian is stored as Word writes it, and Word's tag finds the Valencian pack, not the
        // Catalan one; Catalan of Spain stays the Catalan pack's.
        assert_eq!(crate::ribbon::language_name(Some("ca-ES-valencia")), "Catalan (Valencian)");
        assert_eq!(super::language_of("ca-valencia", ""), "Catalan (Valencian)");
        assert_eq!(packs::pack_for_tag("ca-ES-valencia", &index).map(|f| f.locale.as_str()), Some("ca-valencia"));
        assert_eq!(packs::pack_for_tag("ca-ES", &index).map(|f| f.locale.as_str()), Some("ca"));
        for (locale, tag) in [
            ("ca-valencia", "ca-ES-valencia"),
            ("ca", "ca-ES"),
            ("bg", "bg-BG"),
            ("sr-latn", "sr-Latn-RS"),
            ("sr", "sr-Cyrl-RS"),
            ("nl", "nl-NL"),
            ("de-at", "de-AT"),
            ("pt-br", "pt-BR"),
            ("eo", "eo"),
            ("es", "es-ES"),
        ] {
            assert_eq!(crate::ribbon::tag_for_pack(locale), tag);
        }
    }

    /// CP4.3: the Set Proofing Language dialog lists every installed and available pack's language
    /// beside the built-in ones, each once and sorted by name; Review › Language and the status
    /// bar's menu add the installed ones to the short list.
    #[test]
    fn pack_languages_are_in_the_language_lists() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                packs::set_background(None);
                packs::set_backend(None);
            }
        }
        let _reset = Reset;
        let backend = std::sync::Arc::new(FixedBackend { packs: index_packs(), ..Default::default() });
        backend.installed.lock().unwrap().push(packs::Installed {
            locale: "bg".into(),
            name: "Български".into(),
            licence: String::new(),
            path: std::path::PathBuf::new(),
        });
        packs::set_backend(Some(backend.clone() as std::sync::Arc<dyn Backend>));
        let all = crate::ribbon::all_languages();
        for (tag, name) in [
            ("bg-BG", "Bulgarian"),
            ("be-BY", "Belarusian"),
            ("sr-Latn-RS", "Serbian (Latin)"),
            ("es-CU", "Spanish (Cuba)"),
            ("ar-SA", "Arabic"),
            ("hr-HR", "Croatian"),
            ("ja-JP", "Japanese"),
        ] {
            assert!(all.iter().any(|(t, n)| t == tag && n == name), "{tag} {name}: {all:?}");
        }
        let mut tags: Vec<String> = all.iter().map(|(t, _)| t.to_ascii_lowercase()).collect();
        let n = tags.len();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), n, "each language once");
        assert!(all.windows(2).all(|w| crate::ribbon::sort_key(&w[0].1) <= crate::ribbon::sort_key(&w[1].1)), "sorted by name");
        assert!(!all.iter().any(|(_, n)| n == "Беларуская" || n == "Български"), "every pack named in the interface language");
        // One Catalan row and one Valencian, no stray Catalan (Spain).
        assert_eq!(all.iter().filter(|(_, n)| n.starts_with("Catalan")).map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["ca-ES", "ca-ES-valencia"]);
        // A bare tag is its regioned row, not a second row of the same name.
        for (current, row) in [("nl", "nl-NL"), ("sv", "sv-SE"), ("ca-valencia", "ca-ES-valencia")] {
            let list = crate::ribbon::language_list(current);
            assert_eq!(list, all, "{current}");
            assert_eq!(list.iter().filter(|(t, _)| crate::ribbon::same_language(current, t)).map(|(t, _)| t.as_str()).collect::<Vec<_>>(), [row]);
        }
        // The dialog shows the same list, and a tag none of them has besides.
        assert_eq!(crate::ribbon::language_list("bg-BG"), all);
        assert!(crate::ribbon::language_list("xx-YY").iter().any(|(t, _)| t == "xx-YY"));
        // The short list: the built-in sixteen, then the installed Bulgarian.
        let short = crate::ribbon::short_languages();
        assert_eq!(short.len(), crate::ribbon::PROOFING_LANGUAGES.len() + 1);
        assert_eq!(short.last().map(|(t, n)| (t.as_str(), n.as_str())), Some(("bg-BG", "Bulgarian")));
        // Choosing it marks the text with the tag its pack proofs.
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("document.setText", json!({"text": "Добър ден"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("review.language", json!({"lang": "bg-BG"})).unwrap();
        assert_eq!(crate::ribbon::caret_language(&a), "Bulgarian");
        assert_eq!(packs::pack_for_tag("bg-BG", &packs::available()).map(|p| p.locale.as_str()), Some("bg"));
    }

    /// Characters a pack index or a server's answer could use to hide, reorder or break text.
    fn hostile(c: char) -> bool {
        matches!(c, '\u{2028}' | '\u{2029}' | '\u{202E}' | '\u{E0000}'..='\u{E007F}') || (c.is_control() && c != '\n' && c != '\t')
    }

    /// FU.2: tag characters (U+E0000–U+E007F) and line/paragraph separators (U+2028/9) are
    /// stripped like the other format characters, from names and from licence texts alike.
    #[test]
    fn shown_strips_tag_characters_and_separators() {
        let name = "Ned\u{E0001}\u{E0041}\u{E007F}erlands\u{2028}\u{2029}";
        assert_eq!(super::shown(name, 100), "Nederlands");
        assert_eq!(super::shown_text("one\u{2028}two\nthree\u{2029}\tfour\u{E0020}"), "onetwo\nthree\tfour");
        // A cut never splits what's left, and counts characters, not bytes.
        assert_eq!(super::shown("\u{E0041}ab\u{2028}cd", 3), "ab…");
    }

    /// A store that fails with a hostile message: the index URL with a right-to-left override,
    /// separators, tag characters, and far too long.
    struct Hostile {
        inner: FixedBackend,
        message: String,
    }

    impl Backend for Hostile {
        fn index(&self) -> Result<Vec<packs::Pack>, String> {
            self.inner.index()
        }
        fn fetch_index(&self, _: &packs::Cancel) -> Result<Vec<packs::Pack>, String> {
            Err(self.message.clone())
        }
        fn network_on(&self) -> bool {
            self.inner.network_on()
        }
        fn enable_network(&self) -> Result<(), String> {
            Err(self.message.clone())
        }
        fn install(&self, _: &packs::Pack, _: &packs::Cancel, _: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            Err(self.message.clone())
        }
        fn remove(&self, _: &str) -> Result<(), String> {
            Err(self.message.clone())
        }
        fn packs_dir(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    /// FU.2: the font store's failures (which can quote the index URL and the server's answer)
    /// are shown through the same sanitiser as the index's text, whether the window had them
    /// (turning downloading on) or the engine did (an install, the list).
    #[test]
    fn font_store_errors_are_sanitised() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                packs::set_background(None);
                packs::set_backend(None);
            }
        }
        let _reset = Reset;
        let message = format!("https://evil.example/\u{202E}gnp.exe\u{2028}second line\u{E0041}\u{E0042}{}", "x".repeat(5000));
        let backend = std::sync::Arc::new(Hostile { inner: FixedBackend::sample(), message });
        packs::set_backend(Some(backend.clone() as std::sync::Arc<dyn Backend>));
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let clean = |shown: &str| {
            assert!(!shown.chars().any(hostile), "{shown:?}");
            assert!(shown.contains("https://evil.example/gnp.exe"), "{shown:?}");
            assert!(shown.chars().count() < super::MAX_ERROR + 100, "capped: {} characters", shown.chars().count());
        };
        // Turning downloading on fails: the window's own failure.
        assert!(!super::consent(&mut a, &ctx, "nl"));
        clean(&super::error_of(&a, "nl").expect("a failure is shown"));
        // An install and the list fail in the engine. Another test's window, drawn meanwhile,
        // can move them onto the engine's thread (`tick`): the call then starts the job, and its
        // failure is recorded when the job ends.
        let failed = |r: Result<bool, String>| {
            if r == Ok(true) {
                let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while super::busy() && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            r != Ok(false)
        };
        a.packs_rt.errors.clear();
        backend.inner.network.store(true, Ordering::Relaxed);
        assert!(failed(packs::start_install("nl")));
        clean(&super::error_of(&a, "nl").expect("the install's failure is shown"));
        assert!(failed(packs::start_refresh()));
        clean(&super::error_of(&a, "").expect("the list's failure is shown"));
    }

    /// CP4.3 review: a click the control channel made (MCP's `click`, `ui.click`) on the bar's
    /// Install never turns on downloading, which is the font store's one setting for fonts too;
    /// it says why instead. The person's own click on the same button does, and installs.
    #[test]
    fn a_faked_click_never_turns_on_downloading() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx.enable_accesskit();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                packs::set_background(None);
                packs::set_backend(None);
            }
        }
        let _reset = Reset;
        let backend = std::sync::Arc::new(FixedBackend::sample());
        packs::set_backend(Some(backend.clone() as std::sync::Arc<dyn Backend>));
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("document.setText", json!({"text": "Dzień dobry, jak się masz?"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("review.language", json!({"lang": "pl-PL"})).unwrap();
        let mut tree = frame(&mut a, &ctx, Vec::new());
        for _ in 0..4 {
            tree = frame(&mut a, &ctx, Vec::new());
        }
        let install = spot(&tree, "Install (").expect("the bar offers Polish");
        as_agent(&mut a, &ctx, "ui.click", json!({"x": install.0, "y": install.1}));
        assert!(!backend.network.load(Ordering::Relaxed), "a faked click leaves downloading off");
        assert!(packs::installing().is_empty() && !packs::is_installed("pl"));
        assert!(a.packs_rt.errors.get("pl").is_some_and(|e| e.contains("Only you")), "{:?}", a.packs_rt.errors);
        user_click(&mut a, &ctx, install);
        assert!(backend.network.load(Ordering::Relaxed), "the person's click turns it on");
        for _ in 0..500 {
            if packs::is_installed("pl") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(packs::is_installed("pl"));
    }

    // ---- Options › Language: Your languages, Add a language (CP4.3) ----

    struct ResetBackend;
    impl Drop for ResetBackend {
        fn drop(&mut self) {
            packs::set_background(None);
            packs::set_backend(None);
        }
    }

    /// An app on File › Options › `pane`, with the sample packs (downloading as `network`), and
    /// the context to draw it in.
    fn options_app(pane: &str, network: bool) -> (WordApp, egui::Context, std::sync::Arc<FixedBackend>) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx.enable_accesskit();
        let backend = std::sync::Arc::new(FixedBackend::sample());
        backend.network.store(network, Ordering::Relaxed);
        packs::set_backend(Some(backend.clone() as std::sync::Arc<dyn Backend>));
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.ui.backstage = true;
        a.ui.backstage_page = "options".into();
        a.ui.options_pane = pane.into();
        (a, ctx, backend)
    }

    /// The widgets after a few frames: accessibility updates carry only what changed, so each
    /// frame's nodes are merged (the latest of a widget wins).
    fn settled(a: &mut WordApp, ctx: &egui::Context) -> egui::accesskit::TreeUpdate {
        // A command (a document edit, an install) can close the Backstage: Options stays open.
        a.ui.backstage = true;
        a.ui.backstage_page = "options".into();
        let mut merged: std::collections::BTreeMap<u64, (egui::accesskit::NodeId, egui::accesskit::Node)> = Default::default();
        let mut last = frame(a, ctx, Vec::new());
        for i in 0..5 {
            for (id, node) in last.nodes.drain(..) {
                merged.insert(id.0, (id, node));
            }
            if i < 4 {
                last = frame(a, ctx, Vec::new());
            }
        }
        last.nodes = merged.into_values().collect();
        last
    }

    /// Installs Dutch: on the engine's thread when another test's window moved installs there
    /// (`tick`), so waits for it.
    fn install_nl(a: &mut WordApp) {
        a.run("proof.installPack", json!({"locale": "nl"})).unwrap();
        for _ in 0..500 {
            if packs::is_installed("nl") {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn labels(tree: &egui::accesskit::TreeUpdate) -> Vec<String> {
        tree.nodes.iter().filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_string)).collect()
    }

    fn has(tree: &egui::accesskit::TreeUpdate, label: &str) -> bool {
        labels(tree).iter().any(|l| l == label)
    }

    /// A fresh install lists English, built in and the default; with Dutch installed, Dutch is a
    /// row with Installed and Remove; a document in Thai (no pack) says Not available yet, one in
    /// Polish (a pack, grammar built in) offers to install the spelling.
    #[test]
    fn your_languages_list_the_status_of_each() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, _backend) = options_app("language", true);
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "English (United States) (default)"), "{:?}", labels(&tree));
        assert!(has(&tree, "Built in"));
        assert!(!labels(&tree).iter().any(|l| l == "Remove" || l == "Installed"));
        install_nl(&mut a);
        a.run("document.setText", json!({"text": "สวัสดี\nCześć"})).unwrap();
        for (para, lang) in [(0, "th-TH"), (1, "pl-PL")] {
            a.run("caret.set", json!({"pos": {"block": para, "off": 0}})).unwrap();
            a.run("select.paragraph", json!({})).unwrap();
            a.run("review.language", json!({"lang": lang})).unwrap();
        }
        let tree = settled(&mut a, &ctx);
        let all = labels(&tree);
        assert!(all.iter().any(|l| l == "Dutch"), "{all:?}");
        assert!(all.iter().any(|l| l == "Installed") && all.iter().any(|l| l == "Remove"));
        assert!(all.iter().any(|l| l == "Thai") && all.iter().any(|l| l == "Not available yet"), "{all:?}");
        assert!(all.iter().any(|l| l == "Polish") && all.iter().any(|l| l == "Grammar built in"), "{all:?}");
        assert!(all.iter().any(|l| l.starts_with("Install spelling (")), "{all:?}");
        // Listed by name: Dutch, English, Polish, Thai.
        let view: Vec<String> = super::your_view(&mut a, &packs::available(), &packs::installed()).into_iter().map(|(n, ..)| n).collect();
        assert_eq!(view, ["Dutch", "English (United States)", "Polish", "Thai"], "by the name shown");
        // The Proofing tab no longer has the Dictionaries section; the folder and the dictionary
        // each language found (#1063) are on this tab.
        assert!(all.iter().any(|l| l == "Dictionaries folder:") && all.iter().any(|l| l == "Add a language…"));
        assert!(all.iter().any(|l| l == "Dictionary files"), "{all:?}");
        a.ui.options_pane = "proofing".into();
        let tree = settled(&mut a, &ctx);
        let all = labels(&tree);
        assert!(!all.iter().any(|l| l.starts_with("Dictionaries") || l.starts_with("Dictionary files")), "{all:?}");
        assert!(all.iter().any(|l| l == "Custom dictionary"));
    }

    /// Installing and removing a pack show in Your languages at once (the rows follow the
    /// dictionaries' generation, which both bump).
    #[test]
    fn removing_a_pack_takes_its_remove_button_away() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, _backend) = options_app("language", true);
        let tree = settled(&mut a, &ctx);
        assert!(!has(&tree, "Remove"));
        install_nl(&mut a);
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "Remove") && has(&tree, "Installed"), "{:?}", labels(&tree));
        a.run("proof.removePack", json!({"locale": "nl"})).unwrap();
        let tree = settled(&mut a, &ctx);
        assert!(!has(&tree, "Remove") && !has(&tree, "Installed"), "{:?}", labels(&tree));
    }

    /// A row's detail line shows the document's tag cut and without control or bidi characters.
    #[test]
    fn a_hostile_tag_is_shown_cleaned_and_cut() {
        let row = |tag: &str| packs::LanguageRow {
            tags: vec![tag.to_string()],
            pack: None,
            installed: false,
            size: 0,
            proofing: packs::Proofing::NotAvailable,
            default: false,
        };
        let long = format!("x-{}", "a".repeat(5000));
        let (_, detail) = super::row_names(&row(&long), &[], &[]);
        assert!(detail.chars().count() <= super::MAX_BAR_NAME, "{}", detail.chars().count());
        let (_, detail) = super::row_names(&row("xx-\u{202e}AA\u{0}\u{200f}-\n"), &[], &[]);
        assert_eq!(detail, "xx-AA-");
        // A language with a region the lists don't name is translated in parts (English as built).
        assert_eq!(super::row_names(&row("sv-FI"), &[], &[]), ("Swedish (Finland)".to_string(), "sv-FI".to_string()));
        // Next to a pack's own name, too.
        let index = [packs::Pack { locale: "nl".into(), name: "Neder\u{202e}lands".into(), ..FixedBackend::sample().packs[0].clone() }];
        let mut r = row("nl-NL");
        r.pack = Some("nl".into());
        assert_eq!(super::row_names(&r, &index, &[]).1, "Nederlands · nl-NL");
    }

    /// The picker finds Dutch by its English name, its own name and its code, leaves out what is
    /// installed and sorts by the name shown.
    #[test]
    fn the_picker_searches_by_name_own_name_and_code() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let index = index_packs();
        let none: Vec<packs::Installed> = Vec::new();
        for q in ["dutch", "Dutch", "nederlands", "NL", "nl", " nl "] {
            let found: Vec<String> = super::picker_packs(&index, &none, q).into_iter().map(|(_, p)| p.locale).collect();
            assert!(found.iter().any(|l| l == "nl"), "{q}: {found:?}");
        }
        assert_eq!(super::picker_packs(&index, &none, "zzzz").len(), 0);
        let all = super::picker_packs(&index, &none, "");
        assert_eq!(all.len(), index.len(), "regional variants are rows of their own");
        assert!(all.windows(2).all(|w| crate::ribbon::sort_key(&w[0].0) <= crate::ribbon::sort_key(&w[1].0)), "sorted by shown name");
        let installed = vec![packs::Installed { locale: "nl".into(), name: "Nederlands".into(), licence: String::new(), path: Default::default() }];
        assert!(!super::picker_packs(&index, &installed, "dutch").iter().any(|(_, p)| p.locale == "nl"));
    }

    /// Add a language…: a click the control channel made never turns downloading on (it says
    /// why and opens nothing); the person's own click does, and opens the picker.
    #[test]
    fn a_faked_click_on_add_a_language_never_turns_on_downloading() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, backend) = options_app("language", false);
        let tree = settled(&mut a, &ctx);
        let add = spot(&tree, "Add a language").expect("the button");
        as_agent(&mut a, &ctx, "ui.click", json!({"x": add.0, "y": add.1}));
        assert!(!backend.network.load(Ordering::Relaxed), "a faked click leaves downloading off");
        assert!(!a.packs_rt.picker, "and opens nothing");
        assert!(a.packs_rt.errors.get("").is_some_and(|e| e.contains("Only you")), "{:?}", a.packs_rt.errors);
        // The reason is shown above the button, which moves down.
        let add = spot(&settled(&mut a, &ctx), "Add a language").expect("the button");
        user_click(&mut a, &ctx, add);
        assert!(backend.network.load(Ordering::Relaxed), "the person's click is the consent");
        assert!(a.packs_rt.picker);
        for _ in 0..500 {
            if !packs::fetching() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "Search:") && has(&tree, "Refresh"), "{:?}", labels(&tree));
        assert!(labels(&tree).iter().any(|l| l == "Catalan"), "the packs are listed by name: {:?}", labels(&tree));
    }

    /// Set as default sets only the saved default proofing language (`review.language.preferred`); the
    /// document, its undo history, the redo stack and the modified flag are untouched.
    #[test]
    fn set_as_default_changes_new_texts_language() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, _backend) = options_app("language", true);
        install_nl(&mut a);
        let tree = settled(&mut a, &ctx);
        // Dutch is listed first and isn't the default: its Set as default is the first one.
        let set = spot(&tree, "Set as default").expect("Dutch can be made the default");
        assert_eq!(a.session.prefs.default_language, None);
        let before = a.session.doc.clone();
        // An edit and an undo leave a redo step that Set as default must not clear.
        a.run("text.insert", json!({"text": "x"})).unwrap();
        a.run("edit.undo", json!({})).unwrap();
        let (depth, redo, dirty) = (a.session.undo_depth(), a.session.can_redo(), a.session.dirty);
        assert!(redo);
        user_click(&mut a, &ctx, set);
        assert_eq!(a.session.prefs.default_language.as_deref(), Some("nl-NL"));
        assert_eq!(a.session.doc, before, "the open document is untouched");
        assert_eq!((a.session.undo_depth(), a.session.can_redo(), a.session.dirty), (depth, redo, dirty), "no undo step, redo kept, not modified");
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "Dutch (default)"), "{:?}", labels(&tree));
        // English can be set back.
        assert!(spot(&tree, "Set as default").is_some());
    }

    /// Changing the interface language changes the default proofing language when none is saved
    /// (the session's editing language), without an edit: the list follows.
    #[test]
    fn the_list_follows_the_interface_languages_default() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, _backend) = options_app("language", true);
        // A document in use: only the editing language changes, with no revision of its own.
        a.run("document.setText", json!({"text": "Hello"})).unwrap();
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "English (United States) (default)"), "{:?}", labels(&tree));
        let rev = a.session.rev();
        a.session.set_editing_language(Some("pl-PL"));
        let tree = settled(&mut a, &ctx);
        assert_eq!(a.session.rev(), rev, "no edit");
        assert!(has(&tree, "Polish (default)"), "{:?}", labels(&tree));
    }

    /// A font store that never fetched the list (a fresh install): `index` has nothing.
    struct Uncached(FixedBackend);

    impl Backend for Uncached {
        fn index(&self) -> Result<Vec<packs::Pack>, String> {
            Ok(Vec::new())
        }
        fn fetch_index(&self, c: &packs::Cancel) -> Result<Vec<packs::Pack>, String> {
            self.0.fetch_index(c)
        }
        fn network_on(&self) -> bool {
            self.0.network_on()
        }
        fn enable_network(&self) -> Result<(), String> {
            self.0.enable_network()
        }
        fn install(&self, pack: &packs::Pack, cancel: &packs::Cancel, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
            self.0.install(pack, cancel, progress)
        }
        fn remove(&self, locale: &str) -> Result<(), String> {
            self.0.remove(locale)
        }
        fn packs_dir(&self) -> Option<std::path::PathBuf> {
            None
        }
        fn installed(&self) -> Vec<packs::Installed> {
            self.0.installed()
        }
    }

    /// Martin's test of a fresh install: no list fetched, Greenlandic text. The bar still offers
    /// it; a faked click leaves downloading off, and the person's click fetches the list and
    /// installs the pack.
    #[test]
    fn with_no_list_yet_the_bar_offers_the_language_and_gets_it() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx.enable_accesskit();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                packs::set_background(None);
                packs::set_backend(None);
            }
        }
        let _reset = Reset;
        let mut inner = FixedBackend::sample();
        let kl = packs::Pack { locale: "kl".into(), name: "Kalaallisut".into(), ..inner.packs[0].clone() };
        inner.packs.push(kl);
        let backend = std::sync::Arc::new(Uncached(inner));
        packs::set_backend(Some(backend.clone() as std::sync::Arc<dyn Backend>));
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("document.setText", json!({"text": "Aluu, qanoq ippit?"})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("review.language", json!({"lang": "kl-GL"})).unwrap();
        let mut tree = frame(&mut a, &ctx, Vec::new());
        for _ in 0..4 {
            tree = frame(&mut a, &ctx, Vec::new());
        }
        assert!(packs::available().is_empty());
        let install = spot(&tree, "Install").expect("the bar offers Greenlandic with no list");
        as_agent(&mut a, &ctx, "ui.click", json!({"x": install.0, "y": install.1}));
        assert!(!backend.0.network.load(Ordering::Relaxed), "a faked click leaves downloading off");
        assert!(!packs::fetched() && !packs::is_installed("kl"));
        assert!(a.packs_rt.errors.get("").is_some_and(|e| e.contains("Only you")), "{:?}", a.packs_rt.errors);
        user_click(&mut a, &ctx, install);
        assert!(backend.0.network.load(Ordering::Relaxed), "the person's click turns it on");
        for _ in 0..500 {
            if packs::is_installed("kl") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(packs::fetched(), "the list was fetched");
        assert!(packs::is_installed("kl"), "then its pack installed");
    }

    /// Review: a language whose bar was closed before the list was known stays closed once the
    /// list lists its pack (the dismissal is kept by document tag, the pack bar by pack locale).
    #[test]
    fn a_language_closed_before_the_list_stays_closed_after_it() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let doc = a.session.document_id();
        let kl = packs::Pack { locale: "kl".into(), ..FixedBackend::sample().packs[0].clone() };
        let pt_pt = packs::Pack { locale: "pt-pt".into(), ..kl.clone() };
        let index = vec![kl.clone(), pt_pt.clone()];
        assert!(!super::dismissed(&a, doc, &kl, &index));
        a.packs_rt.dismissed_tags.insert((doc, "kl-GL".into()));
        assert!(super::dismissed(&a, doc, &kl, &index), "closed as kl-GL: kl stays closed");
        assert!(!super::dismissed(&a, doc, &pt_pt, &index), "other languages aren't");
        assert!(!super::dismissed(&a, doc.wrapping_add(1), &kl, &index), "nor other documents");
        // Closing a regional pack's bar leaves the base language's (Swiss German, then German).
        let de = packs::Pack { locale: "de".into(), ..kl.clone() };
        let de_ch = packs::Pack { locale: "de-ch".into(), ..kl.clone() };
        let index = vec![de.clone(), de_ch.clone()];
        a.packs_rt.dismissed.insert((doc, "de-ch".into()));
        assert!(super::dismissed(&a, doc, &de_ch, &index));
        assert!(!super::dismissed(&a, doc, &de, &index), "German's bar still shows");
        // Closed as de-CH before the list was known: Swiss German's pack, not German's.
        a.packs_rt.dismissed.clear();
        a.packs_rt.dismissed_tags.insert((doc, "de-CH".into()));
        assert!(super::dismissed(&a, doc, &de_ch, &index));
        assert!(!super::dismissed(&a, doc, &de, &index), "a de-CH tag doesn't close German");
    }

    /// Martin's test: a spelling issue's context menu heading is in the interface language, not
    /// the text's (craft-proof's message for French text is "Faute d'orthographe possible").
    #[test]
    fn the_spelling_heading_is_in_the_interface_language() {
        let french = json!({"kind": "spelling", "message": "Faute d\u{2019}orthographe possible", "lang": "fr"});
        assert_eq!(crate::canvas::issue_heading(&french), tl!("Possible spelling mistake"));
        let grammar = json!({"kind": "grammar", "message": "Repeated word"});
        assert_eq!(crate::canvas::issue_heading(&grammar), tl!("Repeated word"));
    }
}
