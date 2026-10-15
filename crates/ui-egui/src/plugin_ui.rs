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

fn enabled_plugins() -> Vec<std::sync::Arc<wordcraft_plugin::Plugin>> {
    wordcraft_plugin::list().into_iter().filter(|p| p.enabled()).collect()
}

/// The tabs add-ins add (names not among the built-in tabs), in load order.
pub fn extra_tabs() -> Vec<String> {
    let mut tabs: Vec<String> = Vec::new();
    for p in enabled_plugins() {
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
    for p in enabled_plugins() {
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
    for p in enabled_plugins() {
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
                        let resp = small(ui, app, &b.icon, Some(&b.label), &b.label, crate::widgets::ButtonAction::new("plugins.run", b.run_params(), false));
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
/// download, which runs on its own thread so the window keeps painting.
#[derive(Default)]
pub struct Install {
    pub url: String,
    /// The repository waiting for the user's go-ahead (`owner/name`).
    pub confirming: Option<String>,
    pending: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// The last install's (or load's) outcome: `Ok(message)` or `Err(error)`.
    pub outcome: Option<Result<String, String>>,
}

impl Install {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

/// Starts downloading the add-in at `url` (after the user confirmed).
pub fn start_install(app: &mut WordApp, url: &str) {
    let url = url.trim().to_string();
    let st = &mut app.plugin_install;
    st.confirming = None;
    let Some(root) = wordcraft_plugin::install::install_dir() else {
        st.outcome = Some(Err(tl!("There's no add-ins folder to install into.").to_string()));
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("plugin-install".into()).spawn(move || {
            let _ = tx.send(wordcraft_plugin::install::from_github(&url, &root));
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
        let _ = (url, root);
        st.outcome = Some(Err(tl!("Add-ins need the desktop app.").to_string()));
    }
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
    let outcome = result.and_then(|dir| app.run("plugins.load", json!({"path": dir})).map_err(|e| e.to_string())).map(|info| loaded_message(&info));
    if outcome.is_ok() {
        app.plugin_install.url.clear();
    }
    app.plugin_install.outcome = Some(outcome);
}

pub(crate) fn loaded_message(info: &Value) -> String {
    let name = info.get("name").and_then(Value::as_str).unwrap_or("");
    let version = info.get("version").and_then(Value::as_str).unwrap_or("");
    crate::i18n::fmt(tl!("Installed {name}"), &[("name", format!("{name} {version}").trim())])
}

/// "Install from GitHub": an address field, a confirmation naming the repository, progress and
/// the outcome.
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
/// the right. `commands` adds a button per command (the Add-ins dialog).
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
                    run_or_status(app, "plugins.unload", json!({"id": m.id}));
                }
                if ui.button(if on { tl!("Deactivate") } else { tl!("Activate") }).clicked() {
                    run_or_status(app, "plugins.setEnabled", json!({"id": m.id, "enabled": !on}));
                }
                if !m.homepage.trim().is_empty() && ui.link(tl!("Documentation")).on_hover_text(m.homepage.trim()).clicked() {
                    run_or_status(app, "plugins.openHomepage", json!({"id": m.id}));
                }
            });
        });
        if !m.description.is_empty() {
            ui.add(egui::Label::new(&m.description).wrap());
        }
        if commands && on && !m.commands.is_empty() {
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
