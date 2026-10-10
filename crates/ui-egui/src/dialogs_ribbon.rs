//! Tools › Customize Ribbon (#379; File › Options, command search, right-click on the ribbon or
//! the Quick Access Toolbar): two pages. Ribbon: commands by category on the left, the ribbon's
//! tabs with their custom groups on the right; show or hide tabs, add custom tabs and groups,
//! put commands in custom groups, rename, reorder, remove, reset. Quick Access Toolbar: the same
//! command list next to the toolbar's commands. Every change runs `tools.customizeRibbon`, so
//! agents get the same result without the dialog; the layout persists with the preferences.

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::dialogs::Dialog;
use crate::dialogs_keyboard::{categories, commands, list};
use crate::theme::semibold;

/// The dialog's two pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Page {
    #[default]
    Ribbon,
    QuickAccess,
}

/// An item in the ribbon tree: a tab, one of its custom groups, or a command in a group.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TreeItem {
    pub tab: String,
    pub group: Option<String>,
    pub command: Option<String>,
}

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RibbonForm {
    pub page: Page,
    /// The left list: a category (empty for All Commands) and the command picked in it.
    pub category: String,
    pub command: String,
    /// The item picked in the ribbon tree.
    pub selected: TreeItem,
    /// The command picked in the Quick Access Toolbar list.
    pub qat_selected: String,
    /// Rename… asked: the name being typed.
    pub renaming: Option<String>,
    /// Reset asked; waiting for Yes / No.
    pub confirm_reset: bool,
    /// Why the last change didn't happen.
    pub message: String,
}

impl RibbonForm {
    pub fn read(app: &WordApp, page: Page) -> RibbonForm {
        let category = categories(app).into_iter().next().unwrap_or_default();
        let command = commands(app, &category).first().map(|(id, _)| (*id).to_string()).unwrap_or_default();
        let selected = TreeItem { tab: app.session.ribbon.tabs().first().map(|t| t.name.clone()).unwrap_or_default(), ..Default::default() };
        RibbonForm { page, category, command, selected, ..Default::default() }
    }
}

/// Open the dialog on a page.
pub fn open(app: &mut WordApp, page: Page) {
    app.dialog = Some(Dialog::CustomizeRibbon { form: Box::new(RibbonForm::read(app, page)) });
}

/// Run `tools.customizeRibbon`; the result, or why it failed (shown in the dialog).
fn change(app: &mut WordApp, f: &mut RibbonForm, params: Value) -> Option<Value> {
    match app.session.run("tools.customizeRibbon", &params) {
        Ok(v) => {
            f.message.clear();
            Some(v)
        }
        Err(e) => {
            f.message = e.to_string();
            None
        }
    }
}

fn label_of(app: &WordApp, id: &str) -> String {
    app.session.registry.get(id).map_or_else(|| id.to_string(), |s| tl!(s.label).to_string())
}

/// The dialog body; true closes it.
pub fn customize_ribbon(app: &mut WordApp, ui: &mut Ui, f: &mut RibbonForm) -> bool {
    ui.horizontal(|ui| {
        for (page, name) in [(Page::Ribbon, "Ribbon"), (Page::QuickAccess, "Quick Access Toolbar")] {
            if ui.selectable_label(f.page == page, tl!(name)).clicked() && f.page != page {
                f.page = page;
                f.renaming = None;
                f.confirm_reset = false;
                f.message.clear();
            }
        }
    });
    ui.separator();
    let (left, right, h) = (150.0, 230.0, 300.0);
    let mut cats: Vec<(String, String)> = categories(app).into_iter().map(|c| (c.clone(), tl!(&c).to_string())).collect();
    cats.push((String::new(), tl!("All Commands").to_string()));
    let cmds: Vec<(String, String)> = commands(app, &f.category).into_iter().map(|(id, l)| (id.to_string(), l)).collect();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Choose a command")).font(semibold(12.5)));
            ui.horizontal_top(|ui| {
                if let Some(c) = list(ui, "rb_categories", vec2(left, h), &cats, &f.category) {
                    f.category = c;
                    f.command = commands(app, &f.category).first().map(|(id, _)| (*id).to_string()).unwrap_or_default();
                }
                if let Some(c) = list(ui, "rb_commands", vec2(right, h), &cmds, &f.command) {
                    f.command = c;
                }
            });
        });
        ui.add_space(4.0);
        match f.page {
            Page::Ribbon => ribbon_page(app, ui, f, h),
            Page::QuickAccess => qat_page(app, ui, f, h),
        }
    });
    if let Some(spec) = app.session.registry.get(&f.command) {
        ui.label(egui::RichText::new(format!("{} · {}", crate::i18n::location(spec.location), spec.id)).small().weak());
    }
    if !f.message.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, f.message.as_str());
    }
    ui.add_space(6.0);
    if f.confirm_reset {
        let ask = match f.page {
            Page::Ribbon => tl!("Remove every custom tab and group and show all tabs again?"),
            Page::QuickAccess => tl!("Put the default commands back on the Quick Access Toolbar?"),
        };
        ui.label(ask);
        ui.horizontal(|ui| {
            if ui.button(tl!("Yes")).clicked() {
                let what = if f.page == Page::Ribbon { "ribbon" } else { "qat" };
                change(app, f, json!({"reset": what}));
                f.selected = TreeItem { tab: app.session.ribbon.tabs().first().map(|t| t.name.clone()).unwrap_or_default(), ..Default::default() };
                f.confirm_reset = false;
            }
            if ui.button(tl!("No")).clicked() {
                f.confirm_reset = false;
            }
        });
        return false;
    }
    let mut close = false;
    ui.horizontal(|ui| {
        let custom = match f.page {
            Page::Ribbon => !app.session.ribbon.ribbon_is_default(),
            Page::QuickAccess => app.session.ribbon.qat.is_some(),
        };
        if ui.add_enabled(custom, egui::Button::new(tl!("Reset…"))).clicked() {
            f.confirm_reset = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Close")).clicked() {
                close = true;
            }
        });
    });
    close
}

/// A row in the ribbon tree: the item, its indent level, its text, and (tabs) whether it shows.
struct Row {
    item: TreeItem,
    depth: u8,
    text: String,
    shown: Option<bool>,
}

fn tree_rows(app: &WordApp) -> Vec<Row> {
    let custom = tl!("(Custom)");
    let mut rows = Vec::new();
    for t in app.session.ribbon.tabs().iter() {
        let text = if t.custom { format!("{} {custom}", t.name) } else { tl!(&t.name).to_string() };
        rows.push(Row { item: TreeItem { tab: t.name.clone(), group: None, command: None }, depth: 0, text, shown: Some(!t.hidden) });
        for g in &t.groups {
            let item = TreeItem { tab: t.name.clone(), group: Some(g.name.clone()), command: None };
            rows.push(Row { item, depth: 1, text: format!("{} {custom}", g.name), shown: None });
            for id in &g.commands {
                let item = TreeItem { tab: t.name.clone(), group: Some(g.name.clone()), command: Some(id.clone()) };
                rows.push(Row { item, depth: 2, text: label_of(app, id), shown: None });
            }
        }
    }
    rows
}

/// The params naming a tree item (`{tab, group?, command?}`).
fn address(item: &TreeItem) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("tab".into(), json!(item.tab));
    if let Some(g) = &item.group {
        m.insert("group".into(), json!(g));
    }
    if let Some(c) = &item.command {
        m.insert("command".into(), json!(c));
    }
    m
}

fn ribbon_page(app: &mut WordApp, ui: &mut Ui, f: &mut RibbonForm, h: f32) {
    let rows = tree_rows(app);
    if !rows.iter().any(|r| r.item == f.selected) {
        f.selected = rows.first().map(|r| r.item.clone()).unwrap_or_default();
    }
    let sel = f.selected.clone();
    let tab_custom = app.session.ribbon.tabs().iter().any(|t| t.name == sel.tab && t.custom);
    // Add >> / << Remove.
    ui.vertical(|ui| {
        ui.add_space(h / 2.0 - 10.0);
        let can_add = sel.group.is_some() && !f.command.is_empty();
        let add = ui.add_enabled(can_add, egui::Button::new(tl!("Add >>")).min_size(vec2(96.0, 0.0)));
        if add.on_disabled_hover_text(tl!("Pick a custom group (New Group makes one)")).clicked() {
            let mut m = address(&sel);
            m.remove("command");
            m.insert("command".into(), json!(f.command));
            if let Some(after) = &sel.command {
                m.insert("after".into(), json!(after));
            }
            if change(app, f, json!({"add": m})).is_some() {
                f.selected = TreeItem { command: Some(f.command.clone()), ..sel.clone() };
            }
        }
        let can_remove = sel.group.is_some() || tab_custom;
        if ui.add_enabled(can_remove, egui::Button::new(tl!("<< Remove")).min_size(vec2(96.0, 0.0))).clicked()
            && change(app, f, json!({"remove": address(&sel)})).is_some()
        {
            f.selected = match (&sel.group, &sel.command) {
                (Some(_), Some(_)) => TreeItem { command: None, ..sel.clone() },
                _ => TreeItem { tab: sel.tab.clone(), ..Default::default() },
            };
        }
    });
    ui.add_space(4.0);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(tl!("Ribbon tabs")).font(semibold(12.5)));
        ui.horizontal_top(|ui| {
            tree(app, ui, f, &rows, h);
            ui.vertical(|ui| {
                ui.add_space(h / 2.0 - 10.0);
                for (text, by) in [("Up", -1), ("Down", 1)] {
                    if ui.add(egui::Button::new(tl!(text)).min_size(vec2(56.0, 0.0))).clicked() {
                        let mut m = address(&sel);
                        m.insert("by".into(), json!(by));
                        change(app, f, json!({"move": m}));
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            if ui.button(tl!("New Tab")).clicked()
                && let Some(v) = change(app, f, json!({"newTab": {"after": sel.tab}}))
                && let Some(tab) = v.get("tab").and_then(Value::as_str)
            {
                f.selected = TreeItem { tab: tab.to_string(), group: Some("New Group".into()), command: None };
            }
            if ui.button(tl!("New Group")).clicked()
                && let Some(v) = change(app, f, json!({"newGroup": {"tab": sel.tab}}))
                && let Some(g) = v.get("group").and_then(Value::as_str)
            {
                f.selected = TreeItem { tab: sel.tab.clone(), group: Some(g.to_string()), command: None };
            }
            let can_rename = sel.command.is_none() && (sel.group.is_some() || tab_custom);
            if ui.add_enabled(can_rename, egui::Button::new(tl!("Rename…"))).clicked() {
                f.renaming = Some(sel.group.clone().unwrap_or_else(|| sel.tab.clone()));
            }
        });
        let mut done = None;
        if let Some(name) = &mut f.renaming {
            ui.horizontal(|ui| {
                ui.label(tl!("Name:"));
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(140.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(tl!("OK")).clicked() || enter {
                    done = Some(Some(name.clone()));
                }
                if ui.button(tl!("Cancel")).clicked() {
                    done = Some(None);
                }
            });
        }
        if let Some(answer) = done {
            f.renaming = None;
            if let Some(to) = answer {
                let mut m = address(&sel);
                m.insert("to".into(), json!(to));
                if let Some(v) = change(app, f, json!({"rename": m}))
                    && let Some(name) = v.get("name").and_then(Value::as_str)
                {
                    f.selected = match &sel.group {
                        Some(_) => TreeItem { group: Some(name.to_string()), ..sel.clone() },
                        None => TreeItem { tab: name.to_string(), ..Default::default() },
                    };
                }
            }
        }
    });
}

/// The ribbon tree: tabs with a box to show or hide them, their custom groups and commands.
fn tree(app: &mut WordApp, ui: &mut Ui, f: &mut RibbonForm, rows: &[Row], h: f32) {
    egui::Frame::new().stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(2.0).show(ui, |ui| {
        ui.set_width(250.0);
        ui.set_height(h);
        egui::ScrollArea::vertical().id_salt("rb_tree").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| tree_rows_ui(app, ui, f, rows));
        });
    });
}

fn tree_rows_ui(app: &mut WordApp, ui: &mut Ui, f: &mut RibbonForm, rows: &[Row]) {
    for row in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Groups and commands line up under the tab names (past the tabs' boxes).
            ui.add_space(if row.depth == 0 { 0.0 } else { 24.0 + f32::from(row.depth.saturating_sub(1)) * 16.0 });
            if let Some(shown) = row.shown {
                let mut on = shown;
                if ui.checkbox(&mut on, "").changed() {
                    let key = if on { "show" } else { "hide" };
                    change(app, f, json!({ key: row.item.tab }));
                }
            }
            let resp = ui.add(egui::Button::selectable(f.selected == row.item, row.text.as_str()));
            if resp.clicked() {
                f.selected = row.item.clone();
                f.renaming = None;
            }
        });
    }
}

fn qat_page(app: &mut WordApp, ui: &mut Ui, f: &mut RibbonForm, h: f32) {
    let ids: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    if !ids.contains(&f.qat_selected) {
        f.qat_selected = ids.first().cloned().unwrap_or_default();
    }
    ui.vertical(|ui| {
        ui.add_space(h / 2.0 - 10.0);
        let can_add = !f.command.is_empty() && !ids.contains(&f.command);
        if ui.add_enabled(can_add, egui::Button::new(tl!("Add >>")).min_size(vec2(96.0, 0.0))).clicked()
            && change(app, f, json!({"qatAdd": f.command})).is_some()
        {
            f.qat_selected = f.command.clone();
        }
        if ui.add_enabled(!f.qat_selected.is_empty(), egui::Button::new(tl!("<< Remove")).min_size(vec2(96.0, 0.0))).clicked() {
            change(app, f, json!({"qatRemove": f.qat_selected}));
        }
    });
    ui.add_space(4.0);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(tl!("Quick Access Toolbar")).font(semibold(12.5)));
        ui.horizontal_top(|ui| {
            let rows: Vec<(String, String)> = ids.iter().map(|id| (id.clone(), label_of(app, id))).collect();
            if let Some(id) = list(ui, "rb_qat", vec2(250.0, h), &rows, &f.qat_selected) {
                f.qat_selected = id;
            }
            ui.vertical(|ui| {
                ui.add_space(h / 2.0 - 10.0);
                for (text, by) in [("Up", -1), ("Down", 1)] {
                    if ui.add_enabled(!f.qat_selected.is_empty(), egui::Button::new(tl!(text)).min_size(vec2(56.0, 0.0))).clicked() {
                        change(app, f, json!({"qatMove": {"command": f.qat_selected, "by": by}}));
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use crate::dialogs::Dialog;
    use crate::{Services, WordApp};

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }

    /// #379: Tools › Customize Ribbon opens the dialog; a custom tab with a group, a hidden tab and
    /// the Quick Access Toolbar survive a restart, and a damaged saved layout doesn't lose the
    /// other preferences.
    #[test]
    fn customize_ribbon_opens_and_the_layout_persists() {
        let mut a = app();
        a.run("tools.customizeRibbon", json!({})).unwrap();
        assert!(matches!(a.dialog, Some(Dialog::CustomizeRibbon { .. })));
        a.run("tools.customizeRibbon", json!({"newTab": {"name": "Mine"}, "hide": "Draw", "qatAdd": "format.bold"})).unwrap();
        a.run("tools.customizeRibbon", json!({"add": {"tab": "Mine", "group": "New Group", "command": "format.italic"}})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app();
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.session.ribbon, a.session.ribbon);
        let tabs = b.session.ribbon.tabs();
        let mine = tabs.iter().find(|t| t.name == "Mine").unwrap();
        assert_eq!(mine.groups[0].commands, ["format.italic"]);
        assert!(tabs.iter().any(|t| t.name == "Draw" && t.hidden));
        assert_eq!(b.session.ribbon.qat(), ["file.save", "edit.undo", "edit.redo", "format.bold"]);

        let mut c = app();
        c.apply_prefs(serde_json::from_str(r#"{"tab": "Insert", "ribbon": {"tabs": 3, "qat": ["gone.command", "edit.undo"]}}"#).unwrap());
        assert_eq!(c.ui.tab, "Insert");
        assert!(c.session.ribbon.ribbon_is_default());
        assert_eq!(c.session.ribbon.qat(), ["edit.undo"]);
    }

    /// A context with the interface fonts and theme (they take effect from the next frame).
    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, crate::theme::Appearance::Light);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    /// One frame at 60 fps with these input events.
    fn frame(ctx: &egui::Context, n: &mut u32, events: Vec<egui::Event>, draw: impl FnMut(&mut egui::Ui)) {
        *n += 1;
        let input = egui::RawInput { time: Some(f64::from(*n) / 60.0), predicted_dt: 1.0 / 60.0, events, ..Default::default() };
        ctx.run_ui(input, draw).drop_without_applying_deltas();
    }

    /// A click at `pos` over two frames.
    fn click(ctx: &egui::Context, n: &mut u32, pos: egui::Pos2, mut draw: impl FnMut(&mut egui::Ui)) {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, n, vec![egui::Event::PointerMoved(pos), button(true)], &mut draw);
        frame(ctx, n, vec![button(false)], &mut draw);
    }

    /// #379: a custom tab shows its group's commands as working buttons, and a hidden tab isn't
    /// shown (the ribbon falls back to a visible one).
    #[test]
    fn custom_tabs_draw_their_commands_and_hidden_tabs_dont_show() {
        let ctx = ctx();
        let mut n = 0;
        let mut a = app();
        a.run("tools.customizeRibbon", json!({"newTab": {"name": "Mine"}, "hide": "Home"})).unwrap();
        a.run("tools.customizeRibbon", json!({"add": {"tab": "Mine", "group": "New Group", "command": "view.ruler"}})).unwrap();
        a.ui.tab = "Mine".into();
        let ruler = a.session.view.ruler;
        frame(&ctx, &mut n, vec![], |ui| crate::ribbon::show(&mut a, ui));
        // The group's one command is a large button at the ribbon's left edge, under the tab strip.
        click(&ctx, &mut n, egui::pos2(24.0, 60.0), |ui| crate::ribbon::show(&mut a, ui));
        assert_eq!(a.ui.tab, "Mine");
        assert_ne!(a.session.view.ruler, ruler, "the custom group's button runs its command");

        a.ui.tab = "Home".into();
        frame(&ctx, &mut n, vec![], |ui| crate::ribbon::show(&mut a, ui));
        assert_eq!(a.ui.tab, "Insert", "a hidden tab isn't shown");
    }

    /// #376: the Quick Access Toolbar's … button opens its menu, and the toolbar shows the user's
    /// commands.
    #[test]
    fn quick_access_menu_opens_and_commands_come_and_go() {
        let ctx = ctx();
        let mut n = 0;
        let mut a = app();
        frame(&ctx, &mut n, vec![], |ui| crate::chrome::title_bar(&mut a, ui));
        let more = ctx.read_response(crate::chrome::qat_menu_button_id()).unwrap();
        let popup = egui::Popup::default_response_id(&more);
        assert!(!egui::Popup::is_id_open(&ctx, popup));
        click(&ctx, &mut n, more.rect.center(), |ui| crate::chrome::title_bar(&mut a, ui));
        assert!(egui::Popup::is_id_open(&ctx, popup), "the … button opens the Customize Quick Access Toolbar menu");

        a.run("tools.customizeRibbon", json!({"qatAdd": "file.new"})).unwrap();
        assert!(a.session.ribbon.qat().contains(&"file.new"));
        a.run("tools.customizeRibbon", json!({"qatRemove": "file.new"})).unwrap();
        assert_eq!(a.session.ribbon.qat(), ["file.save", "edit.undo", "edit.redo"]);
    }
}
