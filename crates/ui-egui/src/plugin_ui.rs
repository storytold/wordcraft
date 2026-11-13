//! What add-ins (plugins) add to the app, and where they're managed.
//!
//! - Ribbon groups on a built-in tab or a tab of their own, and right-click menu items, declared
//!   in their manifest's `ui` (or `uiFor.wordcraft`). Each runs one of the plugin's commands
//!   through `plugins.run`.
//! - File › Options › Add-ins and Insert › Add-ins › Get Add-ins: the loaded add-ins as cards
//!   (Documentation, Activate/Deactivate, Unload), installing one from GitHub, loading one from a
//!   folder.
//! - Protocol 1.2: what an add-in command run from here asks for after its step ([`after_run`]: a
//!   file to open, a dialog — `plugin_dialog`), and what add-ins answer document events with
//!   ([`poll_events`]: the status bar).
//! - Manifest 1.3: a blocked add-in's card says why, with Install Missing; installing an add-in
//!   offers the add-ins it needs; updating one to a version an add-in that needs it doesn't accept,
//!   and turning off or unloading one others need, ask first.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use wordcraft_plugin::{ButtonSize, MenuKind};

use crate::WordApp;
use crate::widgets::{big, group_titled, small};

/// The icon a button gets when its manifest names none.
const DEFAULT_ICON: &str = "addins";

/// One button or menu item.
pub struct Item {
    pub plugin: String,
    pub command: String,
    pub label: String,
    pub tooltip: String,
    pub icon: String,
    pub small: bool,
    pub params: Value,
}

impl Item {
    /// The `plugins.run` params that run it.
    fn run_params(&self) -> Value {
        let params = if self.params.is_object() { self.params.clone() } else { json!({}) };
        json!({"plugin": self.plugin, "command": self.command, "params": params})
    }
}

/// The add-ins whose ribbon groups, tabs and menu items show: turned on and not blocked.
fn active_plugins() -> Vec<std::sync::Arc<wordcraft_plugin::Plugin>> {
    wordcraft_plugin::list().into_iter().filter(|p| p.active()).collect()
}

/// The tabs add-ins add (names not among the built-in tabs), in load order.
pub fn extra_tabs() -> Vec<String> {
    let mut tabs: Vec<String> = Vec::new();
    for p in active_plugins() {
        for g in &p.manifest.ui().ribbon {
            let builtin = wordcraft_plugin::RIBBON_TABS.contains(&g.tab.as_str()) || crate::ribbon::TABS.contains(&g.tab.as_str());
            if !builtin && !tabs.contains(&g.tab) {
                tabs.push(g.tab.clone());
            }
        }
    }
    tabs
}

/// The add-in groups on `tab`: (group name, buttons).
pub fn groups(tab: &str) -> Vec<(String, Vec<Item>)> {
    let mut out = Vec::new();
    for p in active_plugins() {
        let m = &p.manifest;
        for g in m.ui().ribbon.iter().filter(|g| g.tab == tab) {
            let buttons = g
                .buttons
                .iter()
                .map(|b| {
                    let decl = m.command(&b.command);
                    Item {
                        plugin: m.id.clone(),
                        command: b.command.clone(),
                        label: b.label.clone().or_else(|| decl.map(|c| c.label.clone())).unwrap_or_else(|| b.command.clone()),
                        tooltip: b.tooltip.clone().or_else(|| decl.map(|c| c.description.clone())).unwrap_or_default(),
                        icon: b.icon.clone().filter(|i| !i.trim().is_empty()).unwrap_or_else(|| DEFAULT_ICON.into()),
                        small: b.size == ButtonSize::Small,
                        params: b.params.clone(),
                    }
                })
                .collect();
            out.push((g.group.clone(), buttons));
        }
    }
    out
}

/// The add-in items for the right-click menu: `text` ones always, `table` ones in a table,
/// `picture` ones with a picture or shape selected.
pub fn menu_items(in_table: bool, picture: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for p in active_plugins() {
        let m = &p.manifest;
        let shown = |k: MenuKind| match k {
            MenuKind::Text => true,
            MenuKind::Table => in_table,
            MenuKind::Picture => picture,
        };
        for item in m.ui().menus.iter().filter(|i| shown(i.menu)) {
            let decl = m.command(&item.command);
            out.push(Item {
                plugin: m.id.clone(),
                command: item.command.clone(),
                label: item.label.clone().or_else(|| decl.map(|c| c.label.clone())).unwrap_or_else(|| item.command.clone()),
                tooltip: decl.map(|c| c.description.clone()).unwrap_or_default(),
                icon: DEFAULT_ICON.into(),
                small: true,
                params: item.params.clone(),
            });
        }
    }
    out
}

/// Runs an item's command; a failure goes to the status bar.
pub fn run(app: &mut WordApp, item: &Item) {
    if let Err(e) = app.run("plugins.run", item.run_params()) {
        app.status(e);
    }
}

/// The right-click menu's add-in items, after a separator.
pub fn context_menu_items(app: &mut WordApp, ui: &mut Ui, in_table: bool, picture: bool) {
    let items = menu_items(in_table, picture);
    if items.is_empty() {
        return;
    }
    ui.separator();
    for item in items {
        let mut b = ui.button(&item.label);
        if !item.tooltip.is_empty() {
            b = b.on_hover_text(&item.tooltip);
        }
        if b.clicked() {
            run(app, &item);
            ui.close();
        }
    }
}

/// Draws the add-in groups on `tab`; small buttons stack three high.
pub fn ribbon_groups(app: &mut WordApp, ui: &mut Ui, tab: &str) {
    for (name, buttons) in groups(tab) {
        group_titled(ui, &format!("addin:{name}"), &name, None, app, |ui, app| {
            let mut k = 0;
            while let Some(b) = buttons.get(k) {
                if !b.small {
                    let resp = big(ui, app, &b.icon, &b.label, "plugins.run", b.run_params(), false);
                    described(resp, b);
                    k += 1;
                    continue;
                }
                let column: Vec<&Item> = buttons.iter().skip(k).take_while(|b| b.small).take(3).collect();
                k += column.len();
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
                    for b in column {
                        let resp = small(
                            ui,
                            app,
                            &b.icon,
                            Some(&b.label),
                            &b.label,
                            crate::widgets::ButtonAction::new("plugins.run", b.run_params(), false),
                        );
                        described(resp, b);
                    }
                });
            }
        });
    }
}

/// The button's name for screen readers (and tests), and the add-in's description on hover.
fn described(resp: egui::Response, b: &Item) {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &b.label));
    if !b.tooltip.is_empty() {
        resp.on_hover_text(&b.tooltip);
    }
}

// ---------------------------------------------------------------- managing add-ins

/// Installing an add-in from GitHub: the address typed, the confirmation step, and the
/// download, which runs on its own thread so the window keeps painting. Also the questions
/// requirements between add-ins raise (manifest 1.3): installing the add-ins an add-in needs,
/// updating one to a version an add-in that needs it doesn't accept, and turning off one others
/// need.
#[derive(Default)]
pub struct Install {
    pub url: String,
    /// The repository waiting for the user's go-ahead (`owner/name`).
    pub confirming: Option<String>,
    pending: Option<std::sync::mpsc::Receiver<Result<Done, String>>>,
    /// The last install's (or load's) outcome: `Ok(message)` or `Err(error)`.
    pub outcome: Option<Result<String, String>>,
    /// A download whose version an installed add-in that needs it doesn't accept: what to say.
    pub confirm_update: Option<(wordcraft_plugin::install::Fetched, String)>,
    /// The add-ins an add-in needs, offered for installing ("Install it too?").
    pub offer: Option<Offer>,
    /// Turning off or unloading an add-in others need: asking first.
    pub confirm_off: Option<ConfirmOff>,
}

/// The missing add-ins of `plugin` offered for installing, each with whether it's ticked
/// (required ones are, optional ones aren't).
pub struct Offer {
    pub plugin: String,
    pub name: String,
    pub items: Vec<(wordcraft_plugin::requires::Missing, bool)>,
}

/// Turning off (or unloading) `id`, which `dependents` need.
pub struct ConfirmOff {
    pub id: String,
    pub unload: bool,
    pub dependents: Vec<String>,
}

/// What a download thread hands back.
enum Done {
    /// Downloaded; the add-ins that need it don't accept its version (the messages).
    Conflicts(Box<wordcraft_plugin::install::Fetched>, Vec<String>),
    /// Unpacked into this folder.
    Unpacked(std::path::PathBuf),
    /// Required add-ins: per add-in, its folder and release, or why it failed.
    Requirements(Vec<(String, Result<(std::path::PathBuf, Option<String>), String>)>),
}

impl Install {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

/// Runs `job` on a thread; its outcome is picked up by [`poll_install`].
fn spawn(app: &mut WordApp, job: impl FnOnce() -> Result<Done, String> + Send + 'static) {
    let st = &mut app.plugin_install;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("plugin-install".into()).spawn(move || {
            let _ = tx.send(job());
        });
        match spawned {
            Ok(_) => {
                st.pending = Some(rx);
                st.outcome = None;
            }
            Err(e) => st.outcome = Some(Err(e.to_string())),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        drop(job);
        st.outcome = Some(Err(tl!("Add-ins need the desktop app.").to_string()));
    }
}

/// The plugins folder, or why there's none (the outcome says so).
fn install_root(app: &mut WordApp) -> Option<std::path::PathBuf> {
    let root = wordcraft_plugin::install::install_dir();
    if root.is_none() {
        app.plugin_install.outcome = Some(Err(tl!("There's no add-ins folder to install into.").to_string()));
    }
    root
}

/// Starts downloading the add-in at `url` (after the user confirmed).
pub fn start_install(app: &mut WordApp, url: &str) {
    let url = url.trim().to_string();
    app.plugin_install.confirming = None;
    let Some(root) = install_root(app) else { return };
    spawn(app, move || {
        let fetched = wordcraft_plugin::install::fetch(&url, None)?;
        let conflicts = wordcraft_plugin::conflicts(&fetched.manifest);
        if !conflicts.is_empty() {
            return Ok(Done::Conflicts(Box::new(fetched), conflicts.into_iter().map(|c| c.message).collect()));
        }
        fetched.unpack(&root).map(Done::Unpacked)
    });
}

/// Installs the ticked add-ins of the offer: for each, the newest release in its range.
pub fn install_offer(app: &mut WordApp) {
    let Some(offer) = app.plugin_install.offer.take() else { return };
    let wanted: Vec<wordcraft_plugin::requires::Missing> = offer.items.into_iter().filter(|(_, on)| *on).map(|(m, _)| m).collect();
    if wanted.is_empty() {
        return;
    }
    let Some(root) = install_root(app) else { return };
    spawn(app, move || {
        let done = wanted
            .into_iter()
            .map(|m| {
                let r = (|| -> Result<(std::path::PathBuf, Option<String>), String> {
                    let github = m.github.as_deref().ok_or_else(|| format!("{} has no GitHub repository to install from", m.name))?;
                    let range = wordcraft_plugin::version::Range::parse(&m.version)?;
                    let fetched = wordcraft_plugin::install::fetch(github, Some(&range))?;
                    if fetched.manifest.id != m.id {
                        return Err(format!("github.com/{github} holds {}, not {}", fetched.manifest.id, m.id));
                    }
                    let fits =
                        wordcraft_plugin::version::Version::parse(&fetched.manifest.version).map(|v| range.matches(&v)).unwrap_or(range.is_any());
                    if !fits {
                        return Err(format!("{} {} from github.com/{github} isn't in {range}", m.name, fetched.manifest.version));
                    }
                    if let Some(c) = wordcraft_plugin::conflicts(&fetched.manifest).first() {
                        return Err(c.message.clone());
                    }
                    Ok((fetched.unpack(&root)?, fetched.release.clone()))
                })();
                (m.id, r)
            })
            .collect();
        Ok(Done::Requirements(done))
    });
}

/// Offers the missing or out-of-range add-ins `id` needs that name a GitHub repository; does
/// nothing when there are none.
pub fn offer_missing(app: &mut WordApp, id: &str) {
    let items: Vec<(wordcraft_plugin::requires::Missing, bool)> =
        wordcraft_plugin::missing(id).into_iter().filter(|m| m.github.is_some()).map(|m| (m.clone(), !m.optional)).collect();
    if items.is_empty() {
        return;
    }
    let name = wordcraft_plugin::get(id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.to_string());
    app.plugin_install.offer = Some(Offer { plugin: id.to_string(), name, items });
}

/// Whether `id` is blocked by required add-ins that Install Missing can install.
pub fn can_install_missing(id: &str) -> bool {
    wordcraft_plugin::missing(id).iter().any(|m| !m.optional && m.github.is_some())
}

/// Loads an add-in unpacked by a download thread (its version was checked, or the user agreed).
fn load_unpacked(app: &mut WordApp, dir: &std::path::Path) -> Result<Value, String> {
    app.run("plugins.load", json!({"path": dir, "force": true}))
}

/// After an add-in command the user ran (a ribbon button, a menu item, a dialog's OK): opens the
/// file it asked for through File › Open's path (so unsaved changes are asked about first), then
/// shows the dialog it asked for. A script running `plugins.run` gets both in the result instead.
pub fn after_run(app: &mut WordApp, plugin: &str, result: &Value) {
    // Protocol 2: the add-in (a transport) asks to start or join a live session.
    if let Some(j) = result.get("collab").and_then(|c| c.get("join")).filter(|j| j.is_object()) {
        let mut params = j.clone();
        params["plugin"] = json!(plugin);
        if let Err(e) = app.run("collab.join", params) {
            app.status(e);
        }
    }
    if let Some(o) = result.get("open").filter(|o| !o.is_null()) {
        match wordcraft_plugin::dialog::parse_open(o) {
            // `readOnly`: WordCraft has no read-only opening; the document opens as usual.
            Ok(o) => {
                let mut params = json!({"path": o.path});
                if let Some(label) = o.label {
                    params["label"] = json!(label);
                }
                if let Err(e) = app.run("file.open", params) {
                    app.status(e);
                }
            }
            Err(e) => app.status(e),
        }
    }
    if let Some(d) = result.get("dialog").filter(|d| !d.is_null()) {
        match wordcraft_plugin::dialog::parse_dialog(d) {
            // Not over a question the open asked (Save Changes): the open goes first.
            Ok(d) if app.dialog.is_none() => {
                app.dialog = Some(crate::dialogs::Dialog::Plugin(Box::new(crate::plugin_dialog::PluginDialog::new(plugin, d))))
            }
            Ok(_) => log::info!("add-in {plugin}: its dialog waits for nothing; another dialog is open"),
            Err(e) => app.status(e),
        }
    }
}

/// Shows what add-ins answered document events with: a message on the status bar, an error as
/// one too, naming the add-in (every frame; nothing to do most of them).
pub fn poll_events(app: &mut WordApp, ctx: &egui::Context) {
    for o in wordcraft_plugin::take_event_outcomes() {
        if let Some(m) = o.message {
            app.status(m);
        }
        if let Some(e) = o.error {
            app.status(crate::i18n::fmt(tl!("The add-in {name} reported an error: {error}"), &[("name", &o.name), ("error", &e)]));
        }
    }
    if wordcraft_plugin::events_in_flight() > 0 {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// Picks up a finished download and loads the add-in (every frame; nothing to do most of them).
pub fn poll_install(app: &mut WordApp, ctx: &egui::Context) {
    let Some(rx) = &app.plugin_install.pending else { return };
    let result = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the download stopped unexpectedly".into()),
    };
    app.plugin_install.pending = None;
    match result {
        Err(e) => app.plugin_install.outcome = Some(Err(e)),
        Ok(Done::Conflicts(fetched, why)) => {
            let name = format!("{} {}", fetched.manifest.name, fetched.manifest.version);
            let text = crate::i18n::fmt(tl!("{why}. Install {name} anyway?"), &[("why", &why.join("; ")), ("name", name.trim())]);
            app.plugin_install.confirm_update = Some((*fetched, text));
        }
        Ok(Done::Unpacked(dir)) => {
            let outcome =
                load_unpacked(app, &dir).map(|info| (info.get("id").and_then(Value::as_str).unwrap_or("").to_string(), loaded_message(&info)));
            if let Ok((id, _)) = &outcome {
                app.plugin_install.url.clear();
                let id = id.clone();
                offer_missing(app, &id);
            }
            app.plugin_install.outcome = Some(outcome.map(|(_, m)| m));
        }
        Ok(Done::Requirements(list)) => {
            let (mut done, mut failed, mut ids) = (Vec::new(), Vec::new(), Vec::new());
            for (id, r) in list {
                match r.and_then(|(dir, release)| load_unpacked(app, &dir).map(|info| (info, release))) {
                    Ok((info, release)) => {
                        let name = info.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
                        let version = info.get("version").and_then(Value::as_str).unwrap_or("").to_string();
                        let name = format!("{name} {version}").trim_end().to_string();
                        done.push(match release {
                            Some(tag) => crate::i18n::fmt(tl!("{name} (release {tag})"), &[("name", &name), ("tag", &tag)]),
                            None => name,
                        });
                        ids.push(id);
                    }
                    Err(e) => failed.push(e),
                }
            }
            // What the new ones need in turn.
            for id in ids {
                if app.plugin_install.offer.is_none() {
                    offer_missing(app, &id);
                }
            }
            let installed = crate::i18n::fmt(tl!("Installed {name}"), &[("name", &done.join(", "))]);
            app.plugin_install.outcome = Some(if failed.is_empty() {
                Ok(installed)
            } else if done.is_empty() {
                Err(failed.join("; "))
            } else {
                Err(format!("{installed}; {}", failed.join("; ")))
            });
        }
    }
}

pub(crate) fn loaded_message(info: &Value) -> String {
    let name = info.get("name").and_then(Value::as_str).unwrap_or("");
    let version = info.get("version").and_then(Value::as_str).unwrap_or("");
    crate::i18n::fmt(tl!("Installed {name}"), &[("name", format!("{name} {version}").trim())])
}

/// "Install from GitHub": an address field, a confirmation naming the repository, progress and
/// the outcome; then the questions an install can raise (an update the add-ins that need it
/// don't accept; the add-ins it needs).
pub fn install_row(app: &mut WordApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(tl!("Install from GitHub:"));
        let busy = app.plugin_install.busy();
        let field = egui::TextEdit::singleline(&mut app.plugin_install.url).hint_text(tl!("owner/repository")).desired_width(320.0);
        let resp = ui.add_enabled(!busy, field);
        let typed = !app.plugin_install.url.trim().is_empty();
        let go = ui.add_enabled(!busy && typed, egui::Button::new(tl!("Install"))).clicked()
            || (typed && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if go {
            match wordcraft_plugin::install::parse_github_url(&app.plugin_install.url) {
                Ok(r) => app.plugin_install.confirming = Some(format!("github.com/{}/{}", r.owner, r.name)),
                Err(e) => app.plugin_install.outcome = Some(Err(e)),
            }
        }
        if busy {
            ui.spinner();
            ui.label(tl!("Installing…"));
        }
    });
    if let Some(repo) = app.plugin_install.confirming.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(tl!("An add-in is a program that runs with your permissions. Install it only if you trust its author."));
            ui.label(RichText::new(crate::i18n::fmt(tl!("Install and run {repo}?"), &[("repo", &repo)])).strong());
            ui.horizontal(|ui| {
                if ui.add(crate::widgets::primary_button(ui, tl!("Install"))).clicked() {
                    let url = app.plugin_install.url.clone();
                    start_install(app, &url);
                }
                if ui.button(tl!("Cancel")).clicked() {
                    app.plugin_install.confirming = None;
                }
            });
        });
    }
    if let Some((_, text)) = &app.plugin_install.confirm_update {
        let text = text.clone();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(text).wrap());
            ui.label(RichText::new(tl!("The add-ins that need the installed version are blocked until it suits them again.")).weak());
            ui.horizontal(|ui| {
                if ui.button(tl!("Install Anyway")).clicked()
                    && let Some((fetched, _)) = app.plugin_install.confirm_update.take()
                    && let Some(root) = install_root(app)
                {
                    spawn(app, move || fetched.unpack(&root).map(Done::Unpacked));
                }
                if ui.button(tl!("Cancel")).clicked() {
                    app.plugin_install.confirm_update = None;
                }
            });
        });
    }
    offer_frame(app, ui);
    match &app.plugin_install.outcome {
        Some(Ok(m)) => {
            ui.label(RichText::new(m).color(crate::theme::Tokens::get(ui.ctx()).accent_text));
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
        }
        None => {}
    }
}

/// "Reports needs craft-vba ^0.1.4 (Runs the report macros). Install it too?", one tick box per
/// add-in (optional ones unticked), and Install Selected / Not Now.
fn offer_frame(app: &mut WordApp, ui: &mut Ui) {
    let busy = app.plugin_install.busy();
    let Some(offer) = app.plugin_install.offer.as_mut() else { return };
    let mut install = false;
    let mut dismiss = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (m, on) in offer.items.iter_mut() {
            let why = if m.why.trim().is_empty() { String::new() } else { format!(" ({})", m.why.trim()) };
            let mut what = format!("{} {}{why}", m.name, m.version);
            if let Some(v) = &m.installed {
                what = crate::i18n::fmt(tl!("{addin}; version {version} is installed"), &[("addin", &what), ("version", v)]);
            }
            let text = if m.optional {
                crate::i18n::fmt(tl!("{name} can use {addin}. Install it too?"), &[("name", &offer.name), ("addin", &what)])
            } else {
                crate::i18n::fmt(tl!("{name} needs {addin}. Install it too?"), &[("name", &offer.name), ("addin", &what)])
            };
            ui.checkbox(on, text);
            if let Some(g) = &m.github {
                ui.indent(("offer", &m.id), |ui| {
                    ui.label(RichText::new(format!("github.com/{g} · {}", tl!("the newest release in range"))).weak());
                });
            }
        }
        ui.label(RichText::new(tl!("Add-ins are programs that run with your permissions. Install them only if you trust their authors.")).weak());
        ui.horizontal(|ui| {
            install = ui.add_enabled(!busy && offer.items.iter().any(|(_, on)| *on), egui::Button::new(tl!("Install Selected"))).clicked();
            dismiss = ui.button(tl!("Not Now")).clicked();
        });
    });
    if install {
        install_offer(app);
    } else if dismiss {
        app.plugin_install.offer = None;
    }
}

/// Turns `id` off (or unloads it), and first the add-ins that need it when the user agreed.
fn turn_off(app: &mut WordApp, id: &str, unload: bool, dependents: &[String]) {
    // The ones furthest up the chain first.
    for d in dependents.iter().rev() {
        run_or_status(app, "plugins.setEnabled", json!({"id": d, "enabled": false}));
    }
    if unload {
        run_or_status(app, "plugins.unload", json!({"id": id}));
    } else {
        run_or_status(app, "plugins.setEnabled", json!({"id": id, "enabled": false}));
    }
}

/// Deactivate or Unload: asks first when other add-ins need it.
fn ask_turn_off(app: &mut WordApp, id: &str, unload: bool) {
    let dependents = wordcraft_plugin::dependents(id);
    if dependents.is_empty() {
        turn_off(app, id, unload, &[]);
    } else {
        app.plugin_install.confirm_off = Some(ConfirmOff { id: id.to_string(), unload, dependents });
    }
}

fn names(ids: &[String]) -> Vec<String> {
    ids.iter().map(|d| wordcraft_plugin::get(d).map(|p| p.manifest.name.clone()).unwrap_or_else(|| d.clone())).collect()
}

/// Loads an add-in from its `plugin.json`, picked in a file dialog (desktop only).
fn load_from_file(app: &mut WordApp) {
    if cfg!(target_arch = "wasm32") {
        app.plugin_install.outcome = Some(Err(tl!("Add-ins need the desktop app.").to_string()));
    } else {
        app.pick_addin();
    }
}

/// One add-in as a full-width card: name and version, then status, author, id and where it came
/// from, the description across the width, and Documentation / Activate or Deactivate / Unload on
/// the right. `commands` adds a button per command (the Add-ins dialog). A blocked add-in says
/// why, with Install Missing when that can help; turning off one that others need asks first.
pub fn plugin_card(app: &mut WordApp, ui: &mut Ui, p: &wordcraft_plugin::Plugin, commands: bool) {
    let m = &p.manifest;
    let on = p.enabled();
    egui::Frame::group(ui.style()).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{} {}", m.name, m.version).trim_end()).strong());
                let status = p.status();
                let mut about = vec![match status.as_str() {
                    "off" => tl!("Inactive").to_string(),
                    s if s.starts_with("blocked") => tl!("Blocked").to_string(),
                    s if s.starts_with("failed") => s.to_string(),
                    _ => tl!("Active").to_string(),
                }];
                if !m.author.is_empty() {
                    about.push(m.author.clone());
                }
                about.push(m.id.clone());
                if let Some(repo) = p.source.as_ref().and_then(|s| s.get("repository")).and_then(Value::as_str) {
                    about.push(repo.trim_start_matches("https://").to_string());
                }
                ui.label(RichText::new(about.join(" · ")).weak());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Unload")).on_hover_text(tl!("Until the next start")).clicked() {
                    ask_turn_off(app, &m.id, true);
                }
                if ui.button(if on { tl!("Deactivate") } else { tl!("Activate") }).clicked() {
                    if on {
                        ask_turn_off(app, &m.id, false);
                    } else {
                        run_or_status(app, "plugins.setEnabled", json!({"id": m.id, "enabled": true}));
                    }
                }
                if on
                    && p.is_blocked()
                    && can_install_missing(&m.id)
                    && ui.add_enabled(!app.plugin_install.busy(), egui::Button::new(tl!("Install Missing"))).clicked()
                {
                    offer_missing(app, &m.id);
                }
                if !m.homepage.trim().is_empty() && ui.link(tl!("Documentation")).on_hover_text(m.homepage.trim()).clicked() {
                    run_or_status(app, "plugins.openHomepage", json!({"id": m.id}));
                }
            });
        });
        if on {
            for b in p.blocked_by() {
                ui.add(egui::Label::new(RichText::new(&b.message).color(ui.visuals().warn_fg_color)).wrap());
            }
        }
        if !m.description.is_empty() {
            ui.add(egui::Label::new(&m.description).wrap());
        }
        let asking = app.plugin_install.confirm_off.as_ref().filter(|c| c.id == m.id).map(|c| (c.unload, c.dependents.clone()));
        if let Some((unloading, dependents)) = asking {
            let who = names(&dependents).join(", ");
            let one = dependents.len() == 1;
            let question = if one { tl!("{who} needs {name}. Turn both off?") } else { tl!("{who} need {name}. Turn them all off?") };
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.add(egui::Label::new(crate::i18n::fmt(question, &[("who", &who), ("name", &m.name)])).wrap());
                ui.horizontal(|ui| {
                    if ui.button(if one { tl!("Turn Both Off") } else { tl!("Turn All Off") }).clicked() {
                        app.plugin_install.confirm_off = None;
                        turn_off(app, &m.id, unloading, &dependents);
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        app.plugin_install.confirm_off = None;
                    }
                });
            });
        }
        if commands && p.active() && !m.commands.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tl!("Commands:")).weak());
                for c in &m.commands {
                    let mut b = ui.button(&c.label);
                    if !c.description.is_empty() {
                        b = b.on_hover_text(&c.description);
                    }
                    if b.clicked() {
                        run_or_status(app, "plugins.run", json!({"plugin": m.id, "command": c.id}));
                    }
                }
            });
        }
    });
}

fn run_or_status(app: &mut WordApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.status(e);
    }
}

/// The add-ins page: File › Options › Add-ins, and the Add-ins dialog (`commands`: with each
/// add-in's command buttons).
pub fn page(app: &mut WordApp, ui: &mut Ui, commands: bool) {
    let plugins = wordcraft_plugin::list();
    if plugins.is_empty() {
        ui.label(RichText::new(tl!("No add-ins are loaded.")).weak());
    }
    for p in &plugins {
        plugin_card(app, ui, p, commands);
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
    install_row(app, ui);
    ui.add_space(4.0);
    if ui.button(tl!("Load Add-in…")).on_hover_text(tl!("Pick the add-in's plugin.json")).clicked() {
        load_from_file(app);
    }
}
