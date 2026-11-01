//! Language packs in the window (CP4.3): the list in File › Options › Language, the missing-pack
//! bar over the page and About › Language data. The engine does the work
//! ([`wordcraft_engine::packs`]); removing goes through `proof.removePack`. Installing and fetching
//! the list run on a thread so the window keeps drawing, through the same engine calls the
//! `proof.installPack` and `proof.packs` commands make. A click on Install or Show available
//! languages is the person's consent: it turns on the font store's downloading (there is no
//! second prompt, R6.3).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use egui::{RichText, Ui, vec2};
use serde_json::json;
use wordcraft_engine::packs::{self, Cancel, Pack};

use crate::WordApp;
use crate::theme::{Tokens, semibold};

/// What a running install or fetch reports to the window.
#[derive(Default)]
struct Job {
    done: u64,
    total: u64,
    /// `Some` when it finished.
    result: Option<Result<(), String>>,
}

type Shared = Arc<Mutex<Job>>;

/// The window's pack state.
#[derive(Default)]
pub(crate) struct Runtime {
    /// Installs running, by locale.
    installs: HashMap<String, (Shared, Cancel)>,
    /// The list being fetched.
    fetch: Option<Shared>,
    /// The last failure of each locale's install or removal ("" for the list), shown until the
    /// next try.
    errors: HashMap<String, String>,
    /// (document, locale) pairs whose bar was closed.
    dismissed: HashSet<(u64, String)>,
    /// The missing packs of the shown document, with what they were worked out from.
    missing: Option<(MissingKey, Vec<Pack>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MissingKey {
    document: u64,
    rev: u64,
    generation: u64,
    packs: usize,
}

fn lock(job: &Shared) -> std::sync::MutexGuard<'_, Job> {
    job.lock().unwrap_or_else(PoisonError::into_inner)
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

/// The language a pack is for, in the interface language (`Dutch`), else its own name.
pub(crate) fn language_of(pack_locale: &str, endonym: &str) -> String {
    let langs = crate::dialogs_layout::LANGUAGES;
    let exact = langs.iter().find(|(t, _)| t.eq_ignore_ascii_case(pack_locale)).map(|(_, n)| *n);
    let base = || {
        langs
            .iter()
            .find(|(t, _)| t.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(pack_locale)))
            .map(|(_, n)| n.split(" (").next().unwrap_or(n))
    };
    match exact.or_else(base) {
        Some(n) => tl!(n).to_string(),
        None if !endonym.is_empty() => endonym.to_string(),
        None => pack_locale.to_string(),
    }
}

/// Finished installs and fetches: proofing reads the new pack, the bar and the list update.
pub(crate) fn tick(app: &mut WordApp) {
    let rt = &mut app.packs_rt;
    let mut finished = Vec::new();
    for (locale, (job, _)) in &rt.installs {
        if let Some(r) = lock(job).result.take() {
            finished.push((locale.clone(), r));
        }
    }
    let mut changed = !finished.is_empty();
    for (locale, r) in finished {
        rt.installs.remove(&locale);
        match r {
            Ok(()) => {
                rt.errors.remove(&locale);
            }
            Err(e) => {
                rt.errors.insert(locale, e);
            }
        }
    }
    if let Some(job) = &rt.fetch {
        let r = lock(job).result.take();
        if let Some(r) = r {
            rt.fetch = None;
            changed = true;
            match r {
                Ok(()) => rt.errors.remove(""),
                Err(e) => rt.errors.insert(String::new(), e),
            };
        }
    }
    if changed {
        rt.missing = None;
        // Spelling and hyphenation in that language change.
        app.session.relayout();
    }
}

fn busy(app: &WordApp) -> bool {
    !app.packs_rt.installs.is_empty() || app.packs_rt.fetch.is_some()
}

/// Install `pack` on a thread: the person clicked Install, which turns downloading on.
fn start_install(app: &mut WordApp, ctx: &egui::Context, pack: &Pack) {
    if app.packs_rt.installs.contains_key(&pack.locale) {
        return;
    }
    let Some(backend) = packs::backend() else { return };
    if let Err(e) = backend.enable_network() {
        app.packs_rt.errors.insert(pack.locale.clone(), e);
        return;
    }
    app.packs_rt.errors.remove(&pack.locale);
    let job: Shared = Arc::new(Mutex::new(Job { done: 0, total: pack.size, result: None }));
    let cancel = Cancel::new();
    app.packs_rt.installs.insert(pack.locale.clone(), (job.clone(), cancel.clone()));
    let locale = pack.locale.clone();
    let ctx = ctx.clone();
    spawn(move || {
        let progress_job = job.clone();
        let progress_ctx = ctx.clone();
        let r = packs::install(&locale, &cancel, &mut |done, total| {
            let mut j = lock(&progress_job);
            j.done = done;
            j.total = total;
            progress_ctx.request_repaint();
        });
        lock(&job).result = Some(r.map(|_| ()));
        ctx.request_repaint();
    });
}

/// Fetch the list on a thread: the person clicked Show available languages (or Refresh).
fn start_fetch(app: &mut WordApp, ctx: &egui::Context) {
    if app.packs_rt.fetch.is_some() {
        return;
    }
    let Some(backend) = packs::backend() else { return };
    if let Err(e) = backend.enable_network() {
        app.packs_rt.errors.insert(String::new(), e);
        return;
    }
    let job: Shared = Arc::default();
    app.packs_rt.fetch = Some(job.clone());
    let ctx = ctx.clone();
    spawn(move || {
        let r = packs::refresh(&Cancel::new()).map(|_| ());
        lock(&job).result = Some(r);
        ctx.request_repaint();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("language-pack".into()).spawn(f) {
        log::warn!("language pack thread: {e}");
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn(_: impl FnOnce() + Send + 'static) {}

fn remove(app: &mut WordApp, locale: &str) {
    match app.run("proof.removePack", json!({"locale": locale})) {
        Ok(_) => {
            app.packs_rt.errors.remove(locale);
            app.packs_rt.missing = None;
        }
        Err(e) => {
            app.packs_rt.errors.insert(locale.to_string(), e);
        }
    }
}

/// The packs the shown document asks for, worked out again only when it or the packs changed.
fn missing(app: &mut WordApp) -> Vec<Pack> {
    let key = MissingKey {
        document: app.session.document_id(),
        rev: app.session.rev(),
        generation: wordcraft_engine::proof::dictionary_generation(),
        packs: packs::available().len(),
    };
    if let Some((k, m)) = &app.packs_rt.missing
        && *k == key
    {
        return m.clone();
    }
    let m = packs::missing(&app.session);
    app.packs_rt.missing = Some((key, m.clone()));
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
    let Some(pack) = missing(app).into_iter().find(|p| !app.packs_rt.dismissed.contains(&(document, p.locale.clone()))) else {
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
                let language = language_of(&pack.locale, &pack.name);
                ui.label(RichText::new(crate::i18n::fmt(tl!("{language} proofing isn't installed."), &[("language", &language)])).color(t.text));
                let installing = app.packs_rt.installs.get(&pack.locale).map(|(j, _)| {
                    let j = lock(j);
                    (j.done, j.total.max(1))
                });
                match installing {
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
                if let Some(e) = app.packs_rt.errors.get(&pack.locale) {
                    ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.red)).truncate());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, "close", tl!("Not now")).clicked() {
                        app.packs_rt.dismissed.insert((document, pack.locale.clone()));
                    }
                });
            });
        });
}

fn licence_hover(pack: &Pack) -> String {
    crate::i18n::fmt(tl!("Licence: {licence}. Downloaded from {source}"), &[("licence", &pack.licence), ("source", pack.source_link())])
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
        let line = crate::i18n::fmt(tl!("Language packs come from a test source: {source}"), &[("source", &source)]);
        ui.add(egui::Label::new(RichText::new(line).small().color(t.orange)).wrap());
    }
    let index = packs::available();
    let installed = packs::installed();
    let network = backend.network_on();
    if let Some(e) = app.packs_rt.errors.get("") {
        ui.label(RichText::new(e.as_str()).small().color(t.red));
    }
    ui.horizontal(|ui| {
        if app.packs_rt.fetch.is_some() {
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
                "Downloading is off. Show available languages or Install turns it on: WordCraft then downloads language packs and fonts from GitHub."
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
                    let language = language_of(&pack.locale, &pack.name);
                    // The pack's own name when it differs (Nederlands for Dutch), and its locale.
                    let detail = if language == pack.name { pack.locale.clone() } else { format!("{} · {}", pack.name, pack.locale) };
                    ui.label(language);
                    ui.label(RichText::new(detail).small().weak());
                });
                ui.label(if pack.size > 0 { size_text(pack.size) } else { String::new() });
                ui.vertical(|ui| {
                    ui.set_width(220.0);
                    ui.add(egui::Label::new(RichText::new(pack.licence.as_str()).small()).wrap());
                });
                let source = pack.source_link();
                if source.is_empty() {
                    ui.label("");
                } else {
                    ui.hyperlink_to(tl!("Source"), source).on_hover_text(source);
                }
                ui.horizontal(|ui| {
                    let job = app.packs_rt.installs.get(&pack.locale).map(|(j, c)| {
                        let j = lock(j);
                        (j.done, j.total.max(1), c.clone())
                    });
                    if let Some((done, total, cancel)) = job {
                        ui.add(egui::ProgressBar::new(done.min(total) as f32 / total as f32).desired_width(90.0).show_percentage());
                        if ui.small_button(tl!("Cancel")).clicked() {
                            cancel.cancel();
                        }
                    } else if *is_installed {
                        ui.label(RichText::new(tl!("Installed")).small().color(t.green));
                        if ui.add_enabled(!busy(app), egui::Button::new(tl!("Remove"))).clicked() {
                            removal = Some(pack.locale.clone());
                        }
                    } else if ui.button(tl!("Install")).clicked() {
                        install = Some(pack.clone());
                    }
                });
                ui.end_row();
                if let Some(e) = app.packs_rt.errors.get(&pack.locale) {
                    ui.label("");
                    ui.add(egui::Label::new(RichText::new(e.as_str()).small().color(t.red)).wrap());
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

/// About › Language data: the full licence text of the embedded English data and of each
/// installed pack.
pub(crate) fn about_tab(ui: &mut Ui) {
    ui.label(RichText::new(tl!("The spelling and hyphenation data WordCraft uses, and their licences.")).small().weak());
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("about_language_data").max_height(420.0).show(ui, |ui| {
        for (title, text) in packs::licence_texts() {
            egui::CollapsingHeader::new(RichText::new(title.as_str()).font(semibold(13.0))).id_salt(("language_data", title.as_str())).show(
                ui,
                |ui| {
                    ui.add(egui::Label::new(RichText::new(text).small().monospace()).wrap());
                },
            );
        }
    });
}
