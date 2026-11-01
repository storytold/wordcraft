//! Language packs in the window (CP4.3): the list in File › Options › Language, the missing-pack
//! bar over the page and About › Language data. The engine does the work
//! ([`wordcraft_engine::packs`]): in the app installs and fetches of the list run on its thread,
//! whoever asks (these buttons, or `proof.installPack` from MCP and the control channel), so the
//! window keeps drawing. A click of the person's own on Install, Show available languages or
//! Refresh is their consent: it turns on the font store's downloading (there is no second
//! prompt, R6.3). A click the control channel made never does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{RichText, Ui, vec2};
use serde_json::json;
use wordcraft_engine::packs::{self, Pack};

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

fn shown_keeping(text: &str, max: usize, lines: bool) -> String {
    let format = |c: char| {
        (c.is_control() && !(lines && matches!(c, '\n' | '\t')))
            || matches!(c, '\u{061C}' | '\u{00AD}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
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
    /// (document, locale) pairs whose bar was closed.
    dismissed: HashSet<(u64, String)>,
    /// The missing packs of the shown document, what they were worked out from, and when.
    missing: Option<(MissingKey, f64, Vec<Pack>)>,
    /// [`packs::finished`] when last looked at.
    finished: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MissingKey {
    document: u64,
    rev: u64,
    generation: u64,
    /// [`packs::index_generation`]: the list changed.
    packs: u64,
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

/// English names of pack languages the Language dialog's list doesn't have.
const MORE_LANGUAGES: [(&str, &str); 14] = [
    ("bg", "Bulgarian"),
    ("bs", "Bosnian"),
    ("ca", "Catalan"),
    ("ga", "Irish"),
    ("hi", "Hindi"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("ro", "Romanian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sq", "Albanian"),
    ("sw", "Swahili"),
    ("ta", "Tamil"),
    ("vi", "Vietnamese"),
];

/// The language a pack is for, in the interface language (`Dutch`, `Serbian (Latin)`), else its
/// own name.
pub(crate) fn language_of(pack_locale: &str, endonym: &str) -> String {
    let langs = crate::dialogs_layout::LANGUAGES;
    let exact = langs.iter().find(|(t, _)| t.eq_ignore_ascii_case(pack_locale)).map(|(_, n)| *n);
    // `sr-latn` is `sr-Latn-RS` (Serbian (Latin)); a bare `nl` is the language without the region.
    let prefixed = || {
        langs
            .iter()
            .find(|(t, _)| {
                t.len() > pack_locale.len()
                    && t.get(..pack_locale.len()).is_some_and(|p| p.eq_ignore_ascii_case(pack_locale))
                    && t.as_bytes().get(pack_locale.len()) == Some(&b'-')
            })
            .map(|(_, n)| if pack_locale.contains('-') { n } else { n.split(" (").next().unwrap_or(n) })
    };
    let base = pack_locale.split('-').next().unwrap_or(pack_locale);
    let more = || MORE_LANGUAGES.iter().find(|(t, _)| *t == base).map(|(_, n)| *n);
    match exact.or_else(prefixed).or_else(more) {
        Some(n) => tl!(n).to_string(),
        None if !endonym.is_empty() => endonym.to_string(),
        None => pack_locale.to_string(),
    }
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
        crate::i18n::fmt(tl!("Couldn't get the list of languages: {error}"), &[("error", &e)])
    } else {
        crate::i18n::fmt(tl!("Couldn't install: {error}"), &[("error", &e)])
    })
}

/// Downloading on for a click on Install, Show available languages or Refresh: the person's
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
        app.packs_rt.errors.insert(key.to_string(), crate::i18n::fmt(tl!("Couldn't turn on downloading: {error}"), &[("error", &e)]));
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
        app.packs_rt.errors.insert(pack.locale.clone(), crate::i18n::fmt(tl!("Couldn't install: {error}"), &[("error", &e)]));
    }
    app.packs_rt.missing = None;
}

/// Fetch the list again (on the engine's thread): the person clicked Show available languages
/// or Refresh.
fn start_fetch(app: &mut WordApp, ctx: &egui::Context) {
    app.packs_rt.errors.remove("");
    if !consent(app, ctx, "") {
        return;
    }
    if let Err(e) = packs::start_refresh() {
        app.packs_rt.errors.insert(String::new(), crate::i18n::fmt(tl!("Couldn't get the list of languages: {error}"), &[("error", &e)]));
    }
}

fn remove(app: &mut WordApp, locale: &str) {
    match app.run("proof.removePack", json!({"locale": locale})) {
        Ok(_) => {
            app.packs_rt.errors.remove(locale);
            app.packs_rt.missing = None;
        }
        Err(e) => {
            app.packs_rt.errors.insert(locale.to_string(), crate::i18n::fmt(tl!("Couldn't remove: {error}"), &[("error", &e)]));
        }
    }
}

/// The packs the shown document asks for. Worked out again when the document, the packs or the
/// list changed; while it is being typed in, at most once a second.
fn missing(app: &mut WordApp, now: f64) -> Vec<Pack> {
    let key = MissingKey {
        document: app.session.document_id(),
        rev: app.session.rev(),
        generation: wordcraft_engine::proof::dictionary_generation(),
        packs: packs::index_generation(),
    };
    if let Some((k, at, m)) = &app.packs_rt.missing
        && (*k == key || (MissingKey { rev: key.rev, ..*k } == key && now - at < MISSING_REFRESH_SECS))
    {
        return m.clone();
    }
    let m = packs::missing(&app.session);
    app.packs_rt.missing = Some((key, now, m.clone()));
    m
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
    let Some(pack) = missing(app, now).into_iter().find(|p| !app.packs_rt.dismissed.contains(&(document, p.locale.clone()))) else {
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

/// File › Options › Language › Language packs: every available and installed pack with its
/// name, size, licence and source, and Install or Remove.
pub(crate) fn options_section(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Language packs")).font(semibold(15.0)));
    ui.add_space(2.0);
    ui.label(RichText::new(tl!("Spelling dictionaries and hyphenation for more languages. English is built in.")).small().weak());
    let Some(backend) = packs::backend() else {
        let why = if cfg!(target_arch = "wasm32") {
            tl!("Language packs aren't available in the browser.")
        } else {
            tl!("Language packs need the font store folder, and there is none on this computer.")
        };
        ui.label(RichText::new(why).small().weak());
        return;
    };
    if let Some(source) = packs::index_override() {
        let line = crate::i18n::fmt(tl!("Language packs come from a test source: {source}"), &[("source", &shown(&source, 300))]);
        ui.add(egui::Label::new(RichText::new(line).small().color(t.orange)).wrap());
    }
    let index = packs::available();
    let installed = packs::installed();
    let network = backend.network_on();
    if let Some(e) = error_of(app, "") {
        ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
    }
    ui.horizontal(|ui| {
        if packs::fetching() {
            ui.spinner();
            ui.label(RichText::new(tl!("Getting the list of languages…")).small());
        } else if index.is_empty() && !network {
            if ui.button(tl!("Show available languages")).clicked() {
                start_fetch(app, &ui.ctx().clone());
            }
        } else if ui.button(tl!("Refresh")).on_hover_text(tl!("Get the list of languages again")).clicked() {
            start_fetch(app, &ui.ctx().clone());
        }
    });
    if !network {
        ui.label(
            RichText::new(tl!(
                "Downloading is off. Clicking Show available languages, Refresh or Install turns it on: WordCraft then downloads language packs and fonts from GitHub."
            ))
            .small()
            .weak(),
        );
    }
    // Installed packs the list no longer has stay removable.
    let mut rows: Vec<(Pack, bool)> = index.iter().map(|p| (p.clone(), installed.iter().any(|i| i.locale == p.locale))).collect();
    for i in &installed {
        if !rows.iter().any(|(p, _)| p.locale == i.locale) {
            let pack = Pack {
                locale: i.locale.clone(),
                name: i.name.clone(),
                version: String::new(),
                url: String::new(),
                source: None,
                size: 0,
                licence: i.licence.clone(),
                class: String::new(),
                spelling: false,
                hyphenation: false,
            };
            rows.push((pack, true));
        }
    }
    if rows.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let mut install = None;
    let mut removal = None;
    egui::ScrollArea::vertical().id_salt("options_language_packs").max_height(260.0).show(ui, |ui| {
        egui::Grid::new("options_language_packs_grid").num_columns(5).spacing(vec2(14.0, 6.0)).striped(true).show(ui, |ui| {
            for h in ["Language", "Size", "Licence", "Source", ""] {
                ui.label(RichText::new(if h.is_empty() { "" } else { tl!(h) }).small().color(t.text_dim));
            }
            ui.end_row();
            for (pack, is_installed) in &rows {
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    let language = shown(&language_of(&pack.locale, &pack.name), MAX_BAR_NAME);
                    let name = shown(&pack.name, MAX_BAR_NAME);
                    // The pack's own name when it differs (Nederlands for Dutch), and its locale.
                    let detail = if language == name { pack.locale.clone() } else { format!("{name} · {}", pack.locale) };
                    ui.label(language);
                    ui.label(RichText::new(detail).small().weak());
                });
                ui.label(if pack.size > 0 { size_text(pack.size) } else { String::new() });
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    ui.add(egui::Label::new(RichText::new(shown(&pack.licence, 300)).small()).wrap());
                });
                let source = pack.source_link();
                if source.is_empty() {
                    ui.label("");
                } else {
                    ui.hyperlink_to(tl!("Source"), source).on_hover_text(shown(source, 300));
                }
                ui.horizontal(|ui| {
                    if let Some((done, total)) = progress(&pack.locale) {
                        ui.add(egui::ProgressBar::new(done.min(total) as f32 / total as f32).desired_width(90.0).show_percentage());
                        if ui.small_button(tl!("Cancel")).clicked() {
                            packs::cancel(&pack.locale);
                        }
                    } else if *is_installed {
                        ui.label(RichText::new(tl!("Installed")).small().color(t.green));
                        if ui.add_enabled(!busy(), egui::Button::new(tl!("Remove"))).clicked() {
                            removal = Some(pack.locale.clone());
                        }
                    } else if ui.button(tl!("Install")).clicked() {
                        install = Some(pack.clone());
                    }
                });
                ui.end_row();
                if let Some(e) = error_of(app, &pack.locale) {
                    ui.label("");
                    ui.add(egui::Label::new(RichText::new(e).small().color(t.red)).wrap());
                    ui.end_row();
                }
            }
        });
    });
    if let Some(p) = install {
        start_install(app, &ui.ctx().clone(), &p);
    }
    if let Some(l) = removal {
        remove(app, &l);
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

    /// CP4.3 review: a click the control channel made (MCP's `click`, `ui.click`) on the bar's
    /// Install never turns on downloading, which is the font store's one setting for fonts too;
    /// it says why instead. The person's own click on the same button does, and installs.
    #[test]
    fn a_faked_click_never_turns_on_downloading() {
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
}
