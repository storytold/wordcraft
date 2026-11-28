//! Language packs in the window (CP4.3): the list in File › Options › Language, the missing-pack
//! bar over the page and About › Language data. The engine does the work
//! ([`wordcraft_engine::packs`]): in the app installs and fetches of the list run on its thread,
//! whoever asks (these buttons, or `proof.installPack` from MCP and the control channel), so the
//! window keeps drawing. A click of the person's own on Install, Add a language… or
//! Refresh is their consent: it turns on the font store's downloading (there is no second
//! prompt, R6.3). A click the control channel made never does.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft::behavior::{ColumnKey, ColumnWidth, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, BodyCx, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Progress, Shade, Spinner, Text};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::notices::{Notice, NoticeAction};
use craft::ui::patterns::form_dialogs::AboutAppTab;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::windows::WindowHost;
use craft::ui::sections::{DialogRole, FooterButton, GroupBox};
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;
use wordcraft_engine::packs::{self, LanguageRow, Pack, Proofing};

use crate::WordApp;
use crate::dialogs::host::{DialogHooks, Settle, open_with};

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

/// Bytes as people read them: `1.9 MB`, `640 KB`; the decimal mark is the region's `format`.
pub(crate) fn size_text(bytes: u64, format: &craft::i18n::Format) -> String {
    let d = format.separators().0;
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
    error_in(&app.packs_rt.errors, locale)
}

/// [`error_of`] over a copy of the window's own failures (the Add a language dialog holds one).
fn error_in(own: &HashMap<String, String>, locale: &str) -> Option<String> {
    if let Some(e) = own.get(locale) {
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
    consent_with(app, injected(ctx), key)
}

/// Whether the running pass carries injected input (an automation client's), by the framework's own signal: the
/// same on every platform and on the web. A second pass of a frame has none of its own input, so a click read from
/// it is never taken for the person's; a click's pass is where this is asked.
fn injected(ctx: &egui::Context) -> bool {
    WindowHost::of(ctx).injected_pass()
}

/// [`consent`] for a click whose pass was `injected` (or not): the Add a language dialog records that at the click
/// and the app applies it a pass later.
fn consent_with(app: &mut WordApp, injected: bool, key: &str) -> bool {
    let Some(backend) = packs::backend() else { return false };
    if backend.network_on() {
        return true;
    }
    if injected {
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
    start_install_with(app, injected(ctx), pack);
}

/// [`start_install`] for a click whose pass was `injected` (or not).
fn start_install_with(app: &mut WordApp, injected: bool, pack: &Pack) {
    app.packs_rt.errors.remove(&pack.locale);
    if !consent_with(app, injected, &pack.locale) {
        return;
    }
    if let Err(e) = packs::start_install(&pack.locale) {
        app.packs_rt.errors.insert(pack.locale.clone(), crate::i18n::fmt(tl!("Couldn't install: {error}"), &[("error", &shown_error(&e))]));
    }
    app.packs_rt.missing = None;
}

/// Fetch the list again (on the engine's thread): the person clicked Add a language…
/// or Refresh.
fn start_fetch(app: &mut WordApp, injected: bool) {
    app.packs_rt.errors.remove("");
    if !consent_with(app, injected, "") {
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

/// What the bar offers now.
enum Bar {
    /// A pack the list has for a language of the document.
    Pack(Pack),
    /// A language of the document asked for while there is no list yet (its tag).
    Unlisted(String),
}

/// What the bar shows for the shown document: the first language with an available pack that isn't
/// installed and wasn't closed, else the first one asked for before the list was known (hidden while
/// an automatic fetch of the list runs). The one place that decides, for [`bar_offer`] and
/// [`pack_bar`] alike.
fn shown_bar(app: &mut WordApp) -> Option<Bar> {
    packs::backend()?;
    let now = app.ctx.as_ref().map_or(0.0, |ctx| ctx.input(|i| i.time));
    let document = app.session.document_id();
    let offers = missing(app, now);
    // The whole list, for a tag closed before it was known (only then: it is a copy).
    let index = if app.packs_rt.dismissed_tags.is_empty() { Vec::new() } else { packs::available() };
    if let Some(pack) = offers.packs.into_iter().find(|p| !dismissed(app, document, p, &index)) {
        return Some(Bar::Pack(pack));
    }
    let tag = offers.unlisted.into_iter().find(|t| !app.packs_rt.dismissed_tags.contains(&(document, t.clone())))?;
    if packs::fetching() && !app.packs_rt.asked {
        return None;
    }
    Some(Bar::Unlisted(tag))
}

/// Whether the bar has something to show for the shown document: an offer, an install in progress or
/// its error. The `languagePack` slot is visible only then, so it takes no room otherwise.
// PLAN: the decision names `&WordApp`; the offers are cached in `app.packs_rt`, so this takes `&mut WordApp` (app-hub's slot call passes `self`).
pub(crate) fn bar_offer(app: &mut WordApp) -> bool {
    shown_bar(app).is_some()
}

/// The missing-pack bar under the ribbon: "<Language> proofing isn't installed. Install (<size>)",
/// for the first language of the document with an available pack that isn't installed. Closing it
/// hides it for that language in this document. Never shown for a language with no pack.
pub(crate) fn pack_bar(ui: &mut Ui, _cx: &mut BodyCx<'_>, app: &mut WordApp) {
    let Some(bar) = shown_bar(app) else { return };
    let document = app.session.document_id();
    let ctx = ui.ctx().clone();
    let env = Env::of(ui);
    let format = env.format();
    match bar {
        Bar::Pack(pack) => {
            let language = shown(&language_of(&pack.locale, &pack.name), MAX_BAR_NAME);
            let message = crate::i18n::fmt(tl!("{language} proofing isn't installed."), &[("language", &language)]);
            let running = progress(&pack.locale);
            let error = error_of(app, &pack.locale);
            let install =
                (running.is_none()).then(|| crate::i18n::fmt(tl!("Install ({size})"), &[("size", &size_text(pack.size, format))]).to_string());
            // The body can run twice in a pass (measuring, then drawing): the notice is built in it each time.
            let shown_bar = Row::new().show(ui, |ui, _| {
                let mut notice = Notice::new(if error.is_some() { Tone::Bad } else { Tone::Info }, message.to_string()).dismiss().id_salt("pack");
                if let Some(label) = &install {
                    notice = notice.action(NoticeAction::new(label.clone()));
                }
                if let Some(e) = &error {
                    notice = notice.lines([e.clone()]);
                }
                let out = notice.show(ui);
                if let Some((done, total)) = running {
                    Progress::new(done.min(total) as f32 / total as f32).show(ui);
                    let pct = (done.min(total) * 100 / total).to_string();
                    Text::new(crate::i18n::fmt(tl!("Installing… {percent}%"), &[("percent", &pct)]).to_string()).show(ui);
                }
                (out.action, out.dismissed)
            });
            let (action, closed) = shown_bar.inner.unwrap_or((None, false));
            if action == Some(0) {
                start_install(app, &ctx, &pack);
            }
            if closed {
                app.packs_rt.dismissed.insert((document, pack.locale.clone()));
            }
        }
        Bar::Unlisted(tag) => {
            let fetching = packs::fetching();
            let language = shown(&language_of(&tag.to_ascii_lowercase(), ""), MAX_BAR_NAME);
            let message = crate::i18n::fmt(tl!("{language} proofing isn't installed."), &[("language", &language)]);
            // The window's own failure (consent), else the list's when the person asked.
            let error = app.packs_rt.errors.get("").cloned().or_else(|| if app.packs_rt.asked { error_of(app, "") } else { None });
            let shown_bar = Row::new().show(ui, |ui, _| {
                let mut notice = Notice::new(if error.is_some() { Tone::Bad } else { Tone::Info }, message.to_string()).dismiss().id_salt("unlisted");
                if !fetching {
                    notice = notice.action(NoticeAction::new(tl!("Install").to_string()));
                }
                if let Some(e) = &error {
                    notice = notice.lines([e.clone()]);
                }
                let out = notice.show(ui);
                if fetching {
                    Spinner::new().show(ui);
                    Text::new(tl!("Getting the list of languages…").to_string()).show(ui);
                }
                (out.action, out.dismissed)
            });
            let (action, closed) = shown_bar.inner.unwrap_or((None, false));
            if action == Some(0) {
                start_get(app, &ctx, &tag);
            }
            if closed {
                app.packs_rt.dismissed_tags.insert((document, tag));
            }
        }
    }
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

/// What a pass of the Your languages table asked for; applied after the table is drawn.
#[derive(Default)]
struct Chosen {
    /// The pack whose Install was clicked.
    install: Option<Pack>,
    /// The pack whose installation Cancel was clicked.
    cancel: Option<String>,
    /// The locale of the pack whose Remove was clicked.
    removal: Option<String>,
    /// The tag Set as default was clicked for.
    default: Option<String>,
}

/// The Your languages table's columns: Language and its own name and code, the proofing status, then the
/// action columns (Install or Cancel, Set as default, Remove).
fn language_columns() -> Vec<ColumnSpec> {
    vec![
        ColumnSpec::new(ColumnKey::new("language"), tl!("Language").to_string(), ColumnWidth::fill(1.0)),
        ColumnSpec::new(ColumnKey::new("names"), tl!("Own name and code").to_string(), ColumnWidth::Content),
        ColumnSpec::new(ColumnKey::new("status"), tl!("Status").to_string(), ColumnWidth::Content),
        ColumnSpec::new(ColumnKey::new("install"), tl!("Actions").to_string(), ColumnWidth::Content),
        ColumnSpec::new(ColumnKey::new("default"), Label::Empty, ColumnWidth::Content),
        ColumnSpec::new(ColumnKey::new("remove"), Label::Empty, ColumnWidth::Content),
    ]
}

/// The status text of a row, when it has one in words (an install in progress shows its percentage).
fn status_text(row: &LanguageRow, running: Option<(u64, u64)>) -> String {
    if let Some((done, total)) = running {
        let pct = (done.min(total) * 100 / total).to_string();
        return crate::i18n::fmt(tl!("Installing… {percent}%"), &[("percent", &pct)]).to_string();
    }
    match row.proofing {
        Proofing::BuiltIn => tl!("Built in"),
        Proofing::Installed => tl!("Installed"),
        Proofing::OwnDictionary => tl!("Your own dictionary"),
        Proofing::SystemDictionary => tl!("System dictionary"),
        Proofing::NotAvailable => tl!("Not available yet"),
        Proofing::GrammarBuiltIn => tl!("Grammar built in"),
        Proofing::Install => "",
    }
    .to_string()
}

/// File › Options › Language › Your languages: the languages in use and installed, each with
/// where its proofing comes from and what can be done, and Add a language….
pub(crate) fn your_languages(app: &mut WordApp, ui: &mut Ui) {
    let env = Env::of(ui);
    let format = env.format();
    let ctx = ui.ctx().clone();
    Text::new(tl!(
        "English, the languages of this document and every installed Language pack. Language packs add spelling dictionaries and hyphenation."
    ))
    .shade(Shade::Faint)
    .show(ui);
    let backend = packs::backend();
    if backend.is_none() {
        let why = if cfg!(target_arch = "wasm32") {
            tl!("Language packs aren't available in the browser.")
        } else {
            tl!("Language packs need the font store folder, and there is none on this computer.")
        };
        Text::new(why).shade(Shade::Faint).show(ui);
    }
    if let Some(source) = packs::index_override() {
        let line = crate::i18n::fmt(tl!("Language packs come from a test source: {source}"), &[("source", &shown(&source, 300))]);
        Notice::new(Tone::Warn, line.to_string()).id_salt("test_source").show(ui);
    }
    let index = packs::available();
    let installed = packs::installed();
    let view = your_view(app, &index, &installed);
    let errors: Vec<Option<String>> = view
        .iter()
        .map(|(_, _, row)| {
            let error = row.pack.as_deref().and_then(|l| error_of(app, l));
            let unreadable = (matches!(row.proofing, Proofing::OwnDictionary | Proofing::SystemDictionary))
                .then(|| wordcraft_engine::proof::dictionary_load_error(wordcraft_engine::proof::ProofLang::from_tag(Some(&row_tag(row)))))
                .flatten()
                .map(|reason| crate::i18n::fmt(tl!("Its dictionary couldn't be read: {reason}"), &[("reason", &shown_error(&reason))]).to_string());
            match (error, unreadable) {
                (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
                (a, b) => a.or(b),
            }
        })
        .collect();
    let chosen = RefCell::new(Chosen::default());
    let busy = busy();
    let order: Vec<usize> = (0..view.len()).collect();
    let source = keyed(&order[..], |at: &usize| *at).with_text(|at| Cow::Owned(view[*at].0.clone()));
    let _ = Table::<usize>::new(tl!("Your languages").to_string(), language_columns()).select(SelectMode::None).natural_height(true).show_items(
        ui,
        &source,
        &mut |key, icx| {
            let at = *key;
            let (name, detail, row) = &view[at];
            let shown_name = shown(name, MAX_BAR_NAME);
            let line = if row.default { crate::i18n::fmt(tl!("{language} (default)"), &[("language", &shown_name)]).to_string() } else { shown_name };
            let pack = row.pack.as_deref().and_then(|l| index.iter().find(|p| p.locale == l));
            let running = pack.and_then(|p| progress(&p.locale));
            let mut list_row = ListRow::new(line)
                .cells(icx.cells())
                .striped(icx.row.striped)
                .cell(ColumnKey::new("names"), detail.clone())
                .cell(ColumnKey::new("status"), status_text(row, running));
            if let Some(pack) = pack
                && !row.installed
                && matches!(row.proofing, Proofing::Install | Proofing::GrammarBuiltIn)
            {
                let chosen = &chosen;
                if running.is_some() {
                    let locale = pack.locale.clone();
                    list_row = list_row.cell_field(ColumnKey::new("install"), Button::new(tl!("Cancel")).id_salt(("cancel", at)), move |out| {
                        if clicked(&out) {
                            chosen.borrow_mut().cancel = Some(locale);
                        }
                    });
                } else {
                    let size = size_text(row.size, format);
                    let label = if row.proofing == Proofing::Install {
                        crate::i18n::fmt(tl!("Install ({size})"), &[("size", &size)])
                    } else {
                        crate::i18n::fmt(tl!("Install spelling ({size})"), &[("size", &size)])
                    };
                    let pack = pack.clone();
                    list_row = list_row.cell_field(ColumnKey::new("install"), Button::new(label.to_string()).id_salt(("install", at)), move |out| {
                        if clicked(&out) {
                            chosen.borrow_mut().install = Some(pack);
                        }
                    });
                }
            }
            if !row.default {
                let chosen = &chosen;
                let tag = row_tag(row);
                list_row = list_row.cell_field(ColumnKey::new("default"), Button::new(tl!("Set as default")).id_salt(("default", at)), move |out| {
                    if clicked(&out) {
                        chosen.borrow_mut().default = Some(tag);
                    }
                });
            }
            if row.installed
                && let Some(locale) = &row.pack
            {
                let chosen = &chosen;
                let locale = locale.clone();
                list_row =
                    list_row.cell_field(ColumnKey::new("remove"), Button::new(tl!("Remove")).enabled(!busy).id_salt(("remove", at)), move |out| {
                        if clicked(&out) {
                            chosen.borrow_mut().removal = Some(locale);
                        }
                    });
            }
            if let Some(Some(e)) = errors.get(at) {
                list_row = list_row.below(Notice::new(Tone::Bad, e.clone()).id_salt(("row_error", at)), |_| {});
            }
            list_row
        },
    );
    if let Some(e) = error_of(app, "") {
        Notice::new(Tone::Bad, e).id_salt("list_error").show(ui);
    }
    let add = Row::new().show(ui, |ui, _| {
        let add = clicked(&Button::new(tl!("Add a language…")).enabled(backend.is_some()).id_salt("add_language").show(ui));
        if packs::fetching() {
            Spinner::new().show(ui);
            Text::new(tl!("Getting the list of languages…")).show(ui);
        }
        add
    });
    if add.inner == Some(true) {
        open_picker(app, &ctx);
    }
    if backend.as_ref().is_some_and(|b| !b.network_on()) {
        Text::new(tl!(
            "Downloading is off. Clicking Add a language… or Install turns it on: WordCraft then downloads language packs and fonts from GitHub."
        ))
        .shade(Shade::Faint)
        .show(ui);
    }
    let Chosen { install, cancel, removal, default } = chosen.into_inner();
    if let Some(p) = install {
        start_install(app, &ctx, &p);
    }
    if let Some(locale) = cancel {
        packs::cancel(&locale);
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
/// languages is fetched when downloading was just turned on or there is none yet. Opening is the
/// click's act only, never a side effect of a pass.
fn open_picker(app: &mut WordApp, ctx: &egui::Context) {
    let Some(backend) = packs::backend() else { return };
    app.packs_rt.errors.remove("");
    let was_on = backend.network_on();
    let injected = injected(ctx);
    if !consent_with(app, injected, "") {
        return;
    }
    if !was_on || packs::available().is_empty() {
        start_fetch(app, injected);
    }
    if !app.dialog_host.is_open::<AddLanguage>() {
        let _ = open_with(app, None, AddLanguage::new(), DialogHooks { prepare: Some(prepare_picker), settle: settle_picker });
    }
}

/// The key of the Add a language dialog on the host.
const ADD_KEY: &str = "addLanguage";

/// What the Add a language dialog asks of the app. The dialog cannot reach the app, so a click is recorded
/// with whether its pass carried injected input, and [`settle_picker`] applies it with that answer: only a
/// person's click turns downloading on.
enum Want {
    /// Install the pack.
    Install(Pack, bool),
    /// Cancel the installation of the pack with this locale.
    Cancel(String),
    /// Fetch the list again.
    Refresh(bool),
}

/// The Add a language dialog: search, then the packs not installed, by name, each with its size,
/// licence, source and Install. It stays open while installs run, and holds its search text only while it is open.
pub(crate) struct AddLanguage {
    /// The search text; empty each time the dialog opens.
    search: String,
    /// What the clicks of this pass ask of the app; [`settle_picker`] takes them in the same pass.
    asks: Vec<Want>,
    /// The window's own failures, written by [`prepare_picker`] before each pass for the rows to show.
    errors: HashMap<String, String>,
}

impl AddLanguage {
    /// A dialog with an empty search.
    pub(crate) fn new() -> Self {
        AddLanguage { search: String::new(), asks: Vec::new(), errors: HashMap::new() }
    }
}

impl StackDialog for AddLanguage {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(ADD_KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(ADD_KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let injected = injected(ui.ctx());
        let env = Env::of(ui);
        let format = env.format();
        let index = packs::available();
        let installed = packs::installed();
        let list = picker_packs(&index, &installed, &self.search);
        let fetching = packs::fetching();
        let wants = RefCell::new(Vec::new());
        let AddLanguage { search, errors, .. } = self;
        let list_error = error_in(errors, "");
        let shown_dialog = Dialog::new(tl!("Add a language")).buttons([FooterButton::new(tl!("Close"), DialogRole::Cancel)]).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Row::new().align(RowAlign::Center).show(ui, |ui, _| {
                    TextField::new(tl!("Search"), Bind::bound(&mut *search)).id_salt("search").show(ui);
                    if fetching {
                        Spinner::new().show(ui);
                        Text::new(tl!("Getting the list of languages…")).show(ui);
                    } else if clicked(&Button::new(tl!("Refresh")).id_salt("refresh").show(ui)) {
                        wants.borrow_mut().push(Want::Refresh(injected));
                    }
                });
                if let Some(e) = &list_error {
                    Notice::new(Tone::Bad, e.clone()).id_salt("list_error").show(ui);
                }
                if list.is_empty() {
                    let text = if index.is_empty() { tl!("No languages to show yet.") } else { tl!("No language matches your search.") };
                    Text::new(text).shade(Shade::Faint).show(ui);
                }
                let order: Vec<usize> = (0..list.len()).collect();
                let source = keyed(&order[..], |at: &usize| *at).with_text(|at| Cow::Owned(list[*at].0.clone()));
                let _ = List::new(tl!("Languages")).id_salt("packs").select(SelectMode::None).max_rows(PICKER_ROWS).show_items(
                    ui,
                    &source,
                    &mut |key, _| {
                        let at = *key;
                        let (name, pack) = &list[at];
                        let endonym = shown(&pack.name, MAX_BAR_NAME);
                        let detail = if endonym == *name { pack.locale.clone() } else { format!("{endonym} · {}", pack.locale) };
                        let wants = &wants;
                        // The licence is the row's wrapped secondary text: `below` components share one line, and the error
                        // notice (or the progress bar while installing) needs the row's width there.
                        let mut row = ListRow::new(format!("{name} · {detail}")).lines(licence_hover(pack));
                        match progress(&pack.locale) {
                            Some((done, total)) => {
                                let locale = pack.locale.clone();
                                row = row
                                    .below(Progress::new(done.min(total) as f32 / total as f32).id_salt(("progress", at)), |_| {})
                                    .trailing_field(Button::new(tl!("Cancel")).id_salt(("cancel", at)), move |out| {
                                        if clicked(&out) {
                                            wants.borrow_mut().push(Want::Cancel(locale));
                                        }
                                    });
                            }
                            None => {
                                let size = if pack.size > 0 { format!(" ({})", size_text(pack.size, format)) } else { String::new() };
                                let pack = pack.clone();
                                row = row.trailing_field(Button::new(format!("{}{size}", tl!("Install"))).id_salt(("install", at)), move |out| {
                                    if clicked(&out) {
                                        wants.borrow_mut().push(Want::Install(pack, injected));
                                    }
                                });
                            }
                        }
                        if let Some(e) = error_in(errors, &pack.locale) {
                            row = row.below(Notice::new(Tone::Bad, e).id_salt(("row_error", at)), |_| {});
                        }
                        row
                    },
                );
            });
        });
        self.asks.extend(wants.into_inner());
        shown_dialog.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// Rows of the picker's list shown before it scrolls.
const PICKER_ROWS: u16 = 9;

/// Hands the Add a language dialog the window's own failures to show, before each of its passes.
fn prepare_picker(app: &mut WordApp, id: DialogId) {
    let errors = app.packs_rt.errors.clone();
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<AddLanguage>(id) {
        dialog.errors = errors;
    }
}

/// Applies what the Add a language dialog recorded in the pass that just ran. Never opens the dialog: a click on
/// Add a language… does ([`open_picker`]).
fn settle_picker(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    // The dialog answers `()`: taking it keeps the host's results from piling up.
    let _ = app.dialog_host.take_by_id::<AddLanguage>(id);
    let Some(dialog) = app.dialog_host.get_mut_by_id::<AddLanguage>(id) else { return Settle::Done };
    let asks = std::mem::take(&mut dialog.asks);
    for ask in asks {
        match ask {
            Want::Install(pack, injected) => start_install_with(app, injected, &pack),
            Want::Cancel(locale) => packs::cancel(&locale),
            Want::Refresh(injected) => start_fetch(app, injected),
        }
    }
    Settle::Keep
}

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
/// installed pack. The body reads only the static licence texts, so the tab needs no app; the About dialog's tab
/// body scrolls, so a long text does not change the dialog's size.
pub(crate) fn language_data_tab() -> AboutAppTab {
    AboutAppTab::new("languageData", tl!("Language data"), |ui: &mut Ui| {
        Text::new(tl!("The spelling and hyphenation data WordCraft uses, and their licences.")).shade(Shade::Faint).show(ui);
        for (at, (title, text)) in licence_texts().iter().enumerate() {
            GroupBox::new(title.clone(), None).id_salt(("language_data", at)).show(ui, |ui, _| {
                Text::new(text.clone()).shade(Shade::Faint).show(ui);
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::{Value, json};
    use wordcraft_engine::packs::{self, Backend, FixedBackend};

    use crate::WordApp;

    /// One 1440 by 900 frame as the desktop runs it, with the user's own `events`; control input
    /// goes in through the shell's before-input step. Returns the accessibility tree.
    fn frame(a: &mut WordApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::accesskit::TreeUpdate {
        let time = ctx.input(|i| i.time) + 0.05;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        let raw = egui::RawInput { focused: true, time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
        // PLAN: assumed `WordApp::frame_input(FrameOutput) -> FrameInput` (app-hub)
        craft::ui::shell::embed::before_input(ctx, |out| a.frame_input(out));
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
        let (req, _reply) = craft::automation::control::ControlRequest::new(method, params);
        let _ = crate::control::handle(a, ctx, &req);
        let host = craft::ui::runtime::windows::WindowHost::of(ctx);
        for _ in 0..200 {
            frame(a, ctx, Vec::new());
            if host.injected_idle() {
                break;
            }
        }
        // The control server ends the client's injection session when its connection closes
        // (`WindowHost::end_injection`); until then the framework keeps the pointer where the agent left it, which
        // would stand in for the person's own pointer in `user_click`.
        host.end_injection();
        for _ in 0..3 {
            frame(a, ctx, Vec::new());
        }
    }

    fn user_click(a: &mut WordApp, ctx: &egui::Context, (x, y): (f64, f64)) {
        let pos = egui::pos2(x as f32, y as f32);
        frame(a, ctx, vec![egui::Event::PointerMoved(pos)]);
        // The pointer rests over the button for a frame before it presses, as a hand's does.
        frame(a, ctx, Vec::new());
        for pressed in [true, false] {
            frame(a, ctx, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
        }
        // The framework reports a control's action a pass after the release that takes it (the runtime hands the
        // app's input over at the head of the next pass), so the click's effect is read after a few idle frames, as a
        // person's next moment would be.
        for _ in 0..3 {
            frame(a, ctx, Vec::new());
        }
    }

    /// The centre of the first widget whose name starts with `label`.
    fn spot(tree: &egui::accesskit::TreeUpdate, label: &str) -> Option<(f64, f64)> {
        let (_, node) = tree.nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.starts_with(label)))?;
        let b = node.bounds()?;
        Some(((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0))
    }

    /// [`spot`] once the widget has stopped moving: a notice that gains a line eases to its new height, and a
    /// press aimed at a button on its way misses it.
    fn settled_spot(a: &mut WordApp, ctx: &egui::Context, label: &str) -> Option<(f64, f64)> {
        let mut at: Option<(f64, f64)> = None;
        let mut still = 0;
        for _ in 0..60 {
            let tree = frame(a, ctx, Vec::new());
            if let Some(now) = spot(&tree, label) {
                if at == Some(now) {
                    still += 1;
                } else {
                    at = Some(now);
                    still = 0;
                }
            }
            if still >= 5 {
                break;
            }
        }
        at
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
        craft::ui::shell::embed::install(&ctx, crate::test_harness::install_spec(), &crate::ui_fonts(crate::i18n::Lang::EN)).unwrap();
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
        // The reason now sits in the notice, which moves the button: find it again.
        let install = settled_spot(&mut a, &ctx, "Install (").expect("the bar still offers Polish");
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
        craft::ui::shell::embed::install(&ctx, crate::test_harness::install_spec(), &crate::ui_fonts(crate::i18n::Lang::EN)).unwrap();
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
    /// why and opens nothing); the person's own click does, and opens the picker. In the picker a row's
    /// Install follows the same rule: an agent's click leaves downloading off and says why, the person's
    /// click turns it on.
    #[test]
    fn a_faked_click_on_add_a_language_never_turns_on_downloading() {
        let _g = BACKEND_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = ResetBackend;
        let (mut a, ctx, backend) = options_app("language", false);
        let tree = settled(&mut a, &ctx);
        let add = spot(&tree, "Add a language").expect("the button");
        as_agent(&mut a, &ctx, "ui.click", json!({"x": add.0, "y": add.1}));
        assert!(!backend.network.load(Ordering::Relaxed), "a faked click leaves downloading off");
        assert!(!a.dialog_host.is_open::<super::AddLanguage>(), "and opens nothing");
        assert!(a.packs_rt.errors.get("").is_some_and(|e| e.contains("Only you")), "{:?}", a.packs_rt.errors);
        // The reason is shown above the button, which moves down.
        let add = spot(&settled(&mut a, &ctx), "Add a language").expect("the button");
        user_click(&mut a, &ctx, add);
        assert!(backend.network.load(Ordering::Relaxed), "the person's click is the consent");
        assert!(a.dialog_host.is_open::<super::AddLanguage>(), "the dialog opens on the host");
        for _ in 0..500 {
            if !packs::fetching() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Inside the dialog: with downloading off again, an agent's click on a row's Install is refused. Off, each row
        // gains a licence line and its Install moves down, so the click is aimed at the rows as they are then.
        backend.network.store(false, Ordering::Relaxed);
        let tree = settled(&mut a, &ctx);
        assert!(has(&tree, "Search") && has(&tree, "Refresh"), "{:?}", labels(&tree));
        assert!(labels(&tree).iter().any(|l| l.starts_with("Catalan")), "the packs are listed by name: {:?}", labels(&tree));
        let install = spot(&tree, "Install").expect("a row offers Install");
        as_agent(&mut a, &ctx, "ui.click", json!({"x": install.0, "y": install.1}));
        assert!(!backend.network.load(Ordering::Relaxed), "an agent's click in the picker leaves downloading off");
        assert!(packs::installing().is_empty());
        assert!(a.packs_rt.errors.values().any(|e| e.contains("Only you")), "{:?}", a.packs_rt.errors);
        let tree = settled(&mut a, &ctx);
        assert!(labels(&tree).iter().any(|l| l.contains("Only you")), "the row says why: {:?}", labels(&tree));
        // The reason has room to be read: it is at least as wide as Install and within the list's width (the row may sit
        // partly below the list's visible edge).
        let rect = |pick: &dyn Fn(&egui::accesskit::Node) -> bool| {
            tree.nodes.iter().filter(|(_, n)| pick(n)).filter_map(|(_, n)| n.bounds()).collect::<Vec<_>>()
        };
        let list = rect(&|n| n.role() == egui::accesskit::Role::List && n.label() == Some("Languages"));
        let button = rect(&|n| n.label().is_some_and(|l| l.starts_with("Install (")));
        let why = rect(&|n| n.label().is_some_and(|l| l.contains("Only you")));
        let (list, button) = (list.first().expect("the list"), button.first().expect("an Install button"));
        assert!(
            why.iter().any(|w| w.width() >= button.width() && w.x0 >= list.x0 && w.x1 <= list.x1 && w.y0 < list.y1),
            "the reason is as wide as Install and within the list's width: {why:?}, Install {button:?}, list {list:?}"
        );
        // The person's own click on the same row turns it on.
        let install = spot(&tree, "Install").expect("the row still offers Install");
        user_click(&mut a, &ctx, install);
        assert!(backend.network.load(Ordering::Relaxed), "the person's click in the picker is the consent");
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
        craft::ui::shell::embed::install(&ctx, crate::test_harness::install_spec(), &crate::ui_fonts(crate::i18n::Lang::EN)).unwrap();
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
        // The reason now sits in the notice, which moves the button: find it again.
        let install = settled_spot(&mut a, &ctx, "Install").expect("the bar still offers Greenlandic");
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
