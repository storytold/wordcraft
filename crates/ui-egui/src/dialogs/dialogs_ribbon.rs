//! Tools › Customize Ribbon (#379; File › Options, command search, right-click on the ribbon or
//! the Quick Access Toolbar): two pages. Ribbon: commands by category on the left, the ribbon's
//! tabs with their custom groups on the right; show or hide tabs, add custom tabs and groups,
//! put commands in custom groups, rename, reorder, remove, reset. Quick Access Toolbar: the same
//! command list next to the toolbar's commands. Every change runs `tools.customizeRibbon`, so
//! agents get the same result without the dialog; the layout persists with the preferences.
//!
//! The body only files requests (the `tools.customizeRibbon` objects); `settle` runs them, so the session
//! changes in one place and the next pass's `prepare` shows the result.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, ItemSource, Selection, SelectionChange, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, ToggleColumn, clicked,
    request_rename,
};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, PromptChoice, StackDialog, StandardPrompt, dialog_id};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ToggleCell, ToggleState};
use craft::ui::notices::Notice;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::Serialize;
use serde_json::{Value, json};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The dialog's two pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Page {
    #[default]
    Ribbon,
    QuickAccess,
}

/// An item in the ribbon tree: a tab, one of its custom groups, or a command in a group.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct TreeItem {
    pub tab: String,
    pub group: Option<String>,
    pub command: Option<String>,
}

/// The column of show boxes in the ribbon tree.
const SHOWN: ColumnKey = ColumnKey::new("shown");

/// A row in the ribbon tree: the item, its indent level, its text, and (tabs) whether it shows.
struct TreeRow {
    item: TreeItem,
    depth: usize,
    text: String,
    shown: Option<bool>,
}

/// The dialog: this pass's copy of the session's ribbon (from `prepare`), what is picked, and the requests the body
/// has filed for `settle`.
pub(crate) struct RibbonDialog {
    /// The name it opened under (`customizeRibbon` or `customizeQuickAccess`); it does not change while open, so the id is stable.
    key: &'static str,
    page: Page,
    /// The category picked on the left; empty for All Commands.
    category: String,
    /// The command picked in the picker.
    command: String,
    /// Filter-as-you-type query for the picker (#1845); UI-only dialog state, never persisted and never a command parameter.
    filter: String,
    /// The item picked in the ribbon tree.
    selected: TreeItem,
    /// The command picked in the Quick Access Toolbar list.
    qat_selected: String,
    /// Why the last change didn't happen.
    message: String,
    tree: Vec<TreeRow>,
    /// The toolbar's commands as (id, label).
    qat: Vec<(String, String)>,
    /// The categories as (id, label), All Commands last.
    categories: Vec<(String, String)>,
    /// The picker's commands as (id, label).
    commands: Vec<(String, String)>,
    /// The picked command's location and id, under the lists.
    detail: String,
    show_qat: bool,
    /// The names of the custom tabs: only those, and groups, can be renamed.
    custom_tabs: Vec<String>,
    /// Whether Reset… has anything to reset, per page (ribbon, toolbar).
    can_reset: [bool; 2],
    /// The Reset… question that is open, and the page it asks about.
    reset: Option<(DialogId, Page)>,
    requests: Vec<Value>,
}

/// The names the dialog opens under.
const KEYS: [&str; 2] = ["customizeRibbon", "customizeQuickAccess"];

impl RibbonDialog {
    fn new(key: &'static str, page: Page, category: String) -> Self {
        RibbonDialog {
            key,
            page,
            category,
            command: String::new(),
            filter: String::new(),
            selected: TreeItem::default(),
            qat_selected: String::new(),
            message: String::new(),
            tree: Vec::new(),
            qat: Vec::new(),
            categories: Vec::new(),
            commands: Vec::new(),
            detail: String::new(),
            show_qat: true,
            custom_tabs: Vec::new(),
            can_reset: [false; 2],
            reset: None,
            requests: Vec::new(),
        }
    }

    /// Whether `item` can be renamed: a custom tab or a group.
    fn renamable(&self, item: &TreeItem) -> bool {
        item.command.is_none() && (item.group.is_some() || self.custom_tabs.contains(&item.tab))
    }

    fn file(&mut self, key: &str, value: Value) {
        self.requests.push(json!({ key: value }));
    }

    /// The tree's list id, so Rename… can ask its rename field to open.
    fn tree_id(&self, ui: &Ui) -> egui::Id {
        ui.scope_id().with("rb_tree")
    }

    fn body(&mut self, ui: &mut Ui) {
        self.page_tabs(ui);
        search(ui, &mut self.filter);
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(word("Choose a command")).show(ui);
                Row::new().show(ui, |ui, _| {
                    if let Some(category) = pick(ui, "rb_categories", "Categories:", &self.categories, &self.category) {
                        self.category = category;
                        self.filter.clear();
                        // The picker's rows for the new category arrive with the next pass's `prepare`; the pick
                        // falls to its first row then.
                        self.command.clear();
                    }
                    if self.commands.is_empty() {
                        Text::new(word("No matching commands")).show(ui);
                    }
                    if let Some(command) = pick(ui, "rb_commands", "Commands:", &self.commands, &self.command) {
                        self.command = command;
                    }
                });
            });
            match self.page {
                Page::Ribbon => self.ribbon_page(ui),
                Page::QuickAccess => self.qat_page(ui),
            }
        });
        if !self.detail.is_empty() {
            Text::new(Label::from(self.detail.clone())).show(ui);
        }
        if !self.message.is_empty() {
            Notice::new(Tone::Bad, Label::from(self.message.clone())).show(ui);
        }
        let at = usize::from(self.page == Page::QuickAccess);
        let what = if self.page == Page::Ribbon { "ribbon" } else { "qat" };
        if clicked(&Button::new(word("Reset…")).enabled(self.can_reset[at]).show(ui)) {
            self.file("reset", json!(what));
        }
    }

    /// The Ribbon and Quick Access Toolbar tabs; switching clears the last message.
    fn page_tabs(&mut self, ui: &mut Ui) {
        let tabs = [(Page::Ribbon, tl!("Ribbon").to_string()), (Page::QuickAccess, tl!("Quick Access Toolbar").to_string())];
        let source = keyed(&tabs[..], |tab: &(Page, String)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
        let out = TabStrip::new(word("Customize Ribbon")).kind(TabKind::Text).shown(self.page).natural_height(true).show_items(
            ui,
            &source,
            &mut |page, _| Tab::new(tabs.iter().find(|tab| tab.0 == *page).map_or_else(String::new, |tab| tab.1.clone())),
        );
        for event in out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(pages)) = event
                && let Some(page) = pages.last()
                && *page != self.page
            {
                self.page = *page;
                self.message.clear();
            }
        }
    }

    /// Add >> and << Remove, the ribbon tree with Up and Down, and New Tab, New Group and Rename….
    fn ribbon_page(&mut self, ui: &mut Ui) {
        let sel = self.selected.clone();
        let tab_custom = self.custom_tabs.contains(&sel.tab);
        Stack::new().show(ui, |ui, _| {
            let can_add = sel.group.is_some() && !self.command.is_empty();
            if clicked(&Button::new(word("Add >>")).enabled(can_add).show(ui)) {
                let mut m = address(&sel);
                m.insert("command".into(), json!(self.command));
                if let Some(after) = &sel.command {
                    m.insert("after".into(), json!(after));
                }
                self.file("add", Value::Object(m));
            }
            let can_remove = sel.group.is_some() || tab_custom;
            if clicked(&Button::new(word("<< Remove")).enabled(can_remove).show(ui)) {
                self.file("remove", Value::Object(address(&sel)));
            }
        });
        Stack::new().show(ui, |ui, _| {
            Text::new(word("Ribbon tabs")).show(ui);
            Row::new().show(ui, |ui, _| {
                self.tree(ui);
                Stack::new().show(ui, |ui, _| {
                    for (text, by) in [("Up", -1), ("Down", 1)] {
                        if clicked(&Button::new(word(text)).show(ui)) {
                            let mut m = address(&sel);
                            m.insert("by".into(), json!(by));
                            self.file("move", Value::Object(m));
                        }
                    }
                });
            });
            Row::new().show(ui, |ui, _| {
                if clicked(&Button::new(word("New Tab")).show(ui)) {
                    self.file("newTab", json!({"after": sel.tab}));
                }
                if clicked(&Button::new(word("New Group")).show(ui)) {
                    self.file("newGroup", json!({"tab": sel.tab}));
                }
                // The tree's own rename field takes the name in place.
                if clicked(&Button::new(word("Rename…")).enabled(self.renamable(&sel)).show(ui)) {
                    request_rename(ui.ctx(), self.tree_id(ui), &sel);
                }
            });
        });
    }

    /// The ribbon tree: tabs with a box to show or hide them, their custom groups and commands.
    fn tree(&mut self, ui: &mut Ui) {
        let source = TreeSource(&self.tree);
        let mut selection = Selection::single(self.selected.clone());
        let renamable: Vec<TreeItem> = self.tree.iter().filter(|row| self.renamable(&row.item)).map(|row| row.item.clone()).collect();
        let columns = vec![
            ColumnSpec::toggle(SHOWN, word("Show"), ToggleColumn::default()),
            ColumnSpec::new(ColumnKey::new("name"), word("Ribbon tabs"), ColumnWidth::fill(1.0)),
        ];
        let rows = &self.tree;
        let out = List::new(word("Ribbon tabs"))
            .id(self.tree_id(ui))
            .tree(true)
            .select(SelectMode::Single)
            .columns(columns)
            .rename(true)
            .can_rename(move |item: &TreeItem| renamable.contains(item))
            .rename_draft(|item: &TreeItem| Some(item.group.clone().unwrap_or_else(|| item.tab.clone())))
            // The tree scrolls its own rows, as main's fixed-height box did, so the dialog's body does not scroll it too.
            .max_rows(12)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &source, &mut |item, icx| {
                let row = rows.iter().find(|row| row.item == *item);
                let text = row.map(|row| row.text.clone()).unwrap_or_default();
                let list_row = ListRow::new(text).cells(icx.cells());
                match row.and_then(|row| row.shown) {
                    Some(shown) => list_row.toggle(ToggleCell::check(SHOWN, ToggleState::of(shown), tl!("Show {row}").to_string())),
                    None => list_row,
                }
            });
        for event in out.events {
            match event {
                CollectionEvent::Toggle(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                    self.file(if after.on { "show" } else { "hide" }, json!(after.key.tab));
                }
                CollectionEvent::Rename(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                    let mut m = address(&after.key);
                    m.insert("to".into(), json!(after.name));
                    self.file("rename", Value::Object(m));
                }
                _ => {}
            }
        }
        if let Some(item) = selection.items.first()
            && *item != self.selected
        {
            self.selected = item.clone();
        }
    }

    /// Add >> and << Remove, the toolbar's commands with Up and Down, and the Show Quick Access Toolbar box (#1945).
    fn qat_page(&mut self, ui: &mut Ui) {
        Stack::new().show(ui, |ui, _| {
            let can_add = !self.command.is_empty() && !self.qat.iter().any(|(id, _)| *id == self.command);
            if clicked(&Button::new(word("Add >>")).enabled(can_add).show(ui)) {
                self.file("qatAdd", json!(self.command));
            }
            if clicked(&Button::new(word("<< Remove")).enabled(!self.qat_selected.is_empty()).show(ui)) {
                self.file("qatRemove", json!(self.qat_selected));
            }
        });
        Stack::new().show(ui, |ui, _| {
            Text::new(word("Quick Access Toolbar")).show(ui);
            Row::new().show(ui, |ui, _| {
                if let Some(id) = pick(ui, "rb_qat", "Quick Access Toolbar", &self.qat, &self.qat_selected) {
                    self.qat_selected = id;
                }
                Stack::new().show(ui, |ui, _| {
                    for (text, by) in [("Up", -1), ("Down", 1)] {
                        if clicked(&Button::new(word(text)).enabled(!self.qat_selected.is_empty()).show(ui)) {
                            self.file("qatMove", json!({"command": self.qat_selected, "by": by}));
                        }
                    }
                });
            });
            // The same switch as the toolbar menu's Hide Quick Access Toolbar.
            let mut shown = self.show_qat;
            Checkbox::new(Bind::bound(&mut shown)).label(word("Show Quick Access Toolbar")).show(ui);
            if shown != self.show_qat {
                self.show_qat = shown;
                self.file("ui.quickAccessToolbar", json!(shown));
            }
        });
    }

    /// Copies the session's ribbon and the picker's rows in; called before every host run.
    fn refresh(&mut self, data: Snapshot) {
        self.tree = data.tree;
        self.qat = data.qat;
        self.categories = data.categories;
        self.commands = data.commands;
        self.detail = data.detail;
        self.show_qat = data.show_qat;
        self.custom_tabs = data.custom_tabs;
        self.can_reset = data.can_reset;
        self.command = select_in(&self.commands, &self.command);
        if !self.tree.iter().any(|row| row.item == self.selected) {
            self.selected = self.tree.first().map(|row| row.item.clone()).unwrap_or_default();
        }
        if !self.qat.iter().any(|(id, _)| *id == self.qat_selected) {
            self.qat_selected = self.qat.first().map(|(id, _)| id.clone()).unwrap_or_default();
        }
    }
}

/// Everything `prepare` reads from the session for one pass.
struct Snapshot {
    tree: Vec<TreeRow>,
    qat: Vec<(String, String)>,
    categories: Vec<(String, String)>,
    commands: Vec<(String, String)>,
    detail: String,
    show_qat: bool,
    custom_tabs: Vec<String>,
    can_reset: [bool; 2],
}

/// A translated word as a framework label.
fn word(text: &str) -> Label {
    Label::from(tl!(text).to_string())
}

/// The search box over the picker: filters as you type.
fn search(ui: &mut Ui, filter: &mut String) {
    craft::ui::form::Field::new(word("Search")).id_salt("rb_search").show(ui, |ui, _| {
        TextField::new(word("Search"), Bind::bound(filter)).show(ui);
    });
}

/// The rows of a (id, label) list as a source: the position is the key.
struct Rows<'a>(&'a [(String, String)]);

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|row| Cow::Borrowed(row.1.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

/// A single-selection list of (id, label) rows with `current` picked; returns the id of a row the user picked that
/// is not `current`.
fn pick(ui: &mut Ui, salt: &'static str, label: &str, rows: &[(String, String)], current: &str) -> Option<String> {
    let at = rows.iter().position(|(id, _)| id == current);
    let mut selection = at.map_or_else(Selection::default, Selection::single);
    List::new(word(label))
        .id(ui.scope_id().with(salt))
        .select(SelectMode::Single)
        // The list scrolls its own rows, as main's fixed-height boxes did, so the dialog's body does not scroll it too.
        .max_rows(12)
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &Rows(rows), &mut |row, _| ListRow::new(rows.get(*row).map(|r| r.1.clone()).unwrap_or_default()));
    let picked = selection.items.first().copied().filter(|picked| Some(*picked) != at)?;
    rows.get(picked).map(|(id, _)| id.clone())
}

/// The tree's rows as a source: keys are the items, so a reorder cannot move the selection to another row.
struct TreeSource<'a>(&'a [TreeRow]);

impl DisplayOrder<TreeItem> for TreeSource<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> TreeItem {
        self.0.get(at).map(|row| row.item.clone()).unwrap_or_default()
    }

    fn depth(&self, at: usize) -> usize {
        self.0.get(at).map_or(0, |row| row.depth)
    }

    fn has_children(&self, at: usize) -> bool {
        match (self.0.get(at), self.0.get(at + 1)) {
            (Some(row), Some(next)) => next.depth > row.depth,
            _ => false,
        }
    }

    fn open(&self, at: usize) -> bool {
        self.has_children(at)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|row| Cow::Borrowed(row.text.as_str()))
    }
}

impl ItemSource<TreeItem> for TreeSource<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == SHOWN && self.0.get(at).and_then(|row| row.shown) == Some(true)
    }

    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == SHOWN && self.0.get(at).is_some_and(|row| row.shown.is_some())
    }
}

/// Categories: the ribbon tabs commands live on, in ribbon order, then the other places
/// (Navigation, Editing…) alphabetically.
pub(crate) fn categories(app: &WordApp) -> Vec<String> {
    let mut other: Vec<String> = Vec::new();
    let mut tabs: Vec<&str> = Vec::new();
    for spec in app.session.registry.all() {
        let tab = spec.location.split(" › ").next().unwrap_or("");
        if crate::ribbon::TABS.contains(&tab) {
            if !tabs.contains(&tab) {
                tabs.push(tab);
            }
        } else if !tab.is_empty() && !other.iter().any(|t| t == tab) {
            other.push(tab.to_string());
        }
    }
    other.sort();
    crate::ribbon::TABS.iter().filter(|t| tabs.contains(t)).map(|t| (*t).to_string()).chain(other).collect()
}

/// The category listing every command.
const ALL: &str = "";

/// A category's commands as (id, label in the interface language), sorted by label.
pub(crate) fn commands(app: &WordApp, category: &str) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = app
        .session
        .registry
        .all()
        .iter()
        .filter(|s| category == ALL || s.location.split(" › ").next() == Some(category))
        .map(|s| (s.id, tl!(s.label).to_string()))
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(b.0)));
    out
}

/// The commands to list for `category`, narrowed by `query` (#1845): an empty query keeps the
/// category scope; any other text searches every registered command by its id and its English or
/// translated label, case-insensitive (the id lets "file.save" or "save" find Save). Sorted as
/// [`commands`].
pub(crate) fn matching_commands(app: &WordApp, category: &str, query: &str) -> Vec<(String, String)> {
    let fold = craft::i18n::Folder::new(crate::i18n::current().code());
    let q = query.trim();
    if q.is_empty() {
        return commands(app, category).into_iter().map(|(id, l)| (id.to_string(), l)).collect();
    }
    let mut out: Vec<(String, String)> = app
        .session
        .registry
        .all()
        .iter()
        .filter(|s| fold.contains(s.id, q) || fold.contains(s.label, q) || fold.contains(tl!(s.label), q))
        .map(|s| (s.id.to_string(), tl!(s.label).to_string()))
        .collect();
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
    out
}

/// Keep `wanted` when it is still listed, else fall back to the first row (#1845).
pub(crate) fn select_in(rows: &[(String, String)], wanted: &str) -> String {
    if rows.iter().any(|(id, _)| id == wanted) { wanted.to_string() } else { rows.first().map(|(id, _)| id.clone()).unwrap_or_default() }
}

/// Run `tools.customizeRibbon`; the result, or why it failed (kept in `message`, shown in the dialog).
fn change(app: &mut WordApp, message: &mut String, params: Value) -> Option<Value> {
    match app.session.run("tools.customizeRibbon", &params) {
        Ok(v) => {
            message.clear();
            Some(v)
        }
        Err(e) => {
            *message = e.to_string();
            None
        }
    }
}

fn label_of(app: &WordApp, id: &str) -> String {
    app.session.registry.get(id).map_or_else(|| id.to_string(), |s| tl!(s.label).to_string())
}

fn tree_rows(app: &WordApp) -> Vec<TreeRow> {
    let custom = tl!("(Custom)");
    let mut rows = Vec::new();
    for t in app.session.ribbon.tabs().iter() {
        let text = if t.custom { format!("{} {custom}", t.name) } else { tl!(&t.name).to_string() };
        rows.push(TreeRow { item: TreeItem { tab: t.name.clone(), group: None, command: None }, depth: 0, text, shown: Some(!t.hidden) });
        for g in &t.groups {
            let item = TreeItem { tab: t.name.clone(), group: Some(g.name.clone()), command: None };
            rows.push(TreeRow { item, depth: 1, text: format!("{} {custom}", g.name), shown: None });
            for id in &g.commands {
                let item = TreeItem { tab: t.name.clone(), group: Some(g.name.clone()), command: Some(id.clone()) };
                rows.push(TreeRow { item, depth: 2, text: label_of(app, id), shown: None });
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

/// The item a request's params name.
fn item_of(params: &Value) -> TreeItem {
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_string);
    TreeItem { tab: text("tab").unwrap_or_default(), group: text("group"), command: text("command") }
}

/// The first tab's item, where the selection goes after a reset.
fn first_tab(app: &WordApp) -> TreeItem {
    TreeItem { tab: app.session.ribbon.tabs().first().map(|t| t.name.clone()).unwrap_or_default(), ..Default::default() }
}

/// What a request moves in the dialog: the messages and picks that depend on the change's result.
struct Picks {
    message: String,
    selected: TreeItem,
    qat_selected: String,
}

/// Runs one request through `tools.customizeRibbon` and moves the picks as the change asks.
fn run_request(app: &mut WordApp, picks: &mut Picks, request: &Value) {
    let Some((key, params)) = request.as_object().and_then(|m| m.iter().next()) else { return };
    if key == "ui.quickAccessToolbar" {
        if let Err(error) = app.run("ui.quickAccessToolbar", json!({"value": params})) {
            app.status(error);
        }
        return;
    }
    let Some(reply) = change(app, &mut picks.message, json!({ key: params })) else { return };
    let named = |field: &str| reply.get(field).and_then(Value::as_str).map(str::to_string);
    match key.as_str() {
        "add" => picks.selected = item_of(params),
        "remove" => {
            let item = item_of(params);
            picks.selected = match (&item.group, &item.command) {
                (Some(_), Some(_)) => TreeItem { command: None, ..item },
                _ => TreeItem { tab: item.tab, ..Default::default() },
            };
        }
        "newTab" => {
            if let Some(tab) = named("tab") {
                picks.selected = TreeItem { tab, group: Some("New Group".into()), command: None };
            }
        }
        "newGroup" => {
            if let Some(group) = named("group") {
                picks.selected = TreeItem { tab: item_of(params).tab, group: Some(group), command: None };
            }
        }
        "rename" => {
            if let Some(name) = named("name") {
                let item = item_of(params);
                picks.selected = match &item.group {
                    Some(_) => TreeItem { group: Some(name), ..item },
                    None => TreeItem { tab: name, ..Default::default() },
                };
            }
        }
        "qatAdd" => picks.qat_selected = params.as_str().unwrap_or_default().to_string(),
        "reset" => picks.selected = first_tab(app),
        _ => {}
    }
}

impl StackDialog for RibbonDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(self.key)
    }

    fn key(&self) -> Option<&str> {
        Some(self.key)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        // PLAN: the sheet says `FormDialog::own`, which always has OK and Cancel; the Close-only footer is `Dialog` with one button.
        let shown = Dialog::new(word("Customize Ribbon")).buttons(vec![FooterButton::new(word("Close"), DialogRole::Cancel)]).show(ui, |ui, _| {
            self.body(ui);
        });
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({
            "page": self.page,
            "category": self.category,
            "command": self.command,
            "filter": self.filter,
            "selected": self.selected,
            "qatSelected": self.qat_selected,
            "message": self.message,
        }))
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let text = || value.as_str().map(str::to_owned).ok_or_else(|| FieldRefusal::Rejects(&value).message(field));
        match field {
            "page" => {
                self.page = match value.as_str() {
                    Some("ribbon") => Page::Ribbon,
                    Some("quickAccess") => Page::QuickAccess,
                    _ => return Err(FieldRefusal::Rejects(&value).message(field)),
                };
                self.message.clear();
            }
            "category" => self.category = text()?,
            "command" => self.command = text()?,
            "filter" => self.filter = text()?,
            "qatSelected" => self.qat_selected = text()?,
            "selected" if value.is_object() => self.selected = item_of(&value),
            "selected" => return Err(FieldRefusal::Rejects(&value).message(field)),
            "message" => return Err(FieldRefusal::ReadOnly.message(field)),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// Before the host runs: the session's ribbon and the picker's rows go into the dialog.
fn prepare(app: &mut WordApp, id: DialogId) {
    let Some((category, filter)) = app.dialog_host.get_by_id::<RibbonDialog>(id).map(|d| (d.category.clone(), d.filter.clone())) else { return };
    let mut categories: Vec<(String, String)> = self::categories(app).into_iter().map(|c| (c.clone(), tl!(&c).to_string())).collect();
    categories.push((ALL.to_string(), tl!("All Commands").to_string()));
    let commands = matching_commands(app, &category, &filter);
    let ids: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    let qat: Vec<(String, String)> = ids
        .into_iter()
        .map(|id| {
            let label = label_of(app, &id);
            (id, label)
        })
        .collect();
    let custom_tabs: Vec<String> = app.session.ribbon.tabs().iter().filter(|t| t.custom).map(|t| t.name.clone()).collect();
    let snapshot = Snapshot {
        tree: tree_rows(app),
        qat,
        categories,
        commands,
        detail: String::new(),
        show_qat: app.ui.quick_access_toolbar,
        custom_tabs,
        can_reset: [!app.session.ribbon.ribbon_is_default(), app.session.ribbon.qat.is_some()],
    };
    let wanted = app.dialog_host.get_by_id::<RibbonDialog>(id).map(|d| select_in(&snapshot.commands, &d.command)).unwrap_or_default();
    let detail = app.session.registry.get(&wanted).map(|spec| format!("{} · {}", crate::i18n::location(spec.location), spec.id)).unwrap_or_default();
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<RibbonDialog>(id) {
        dialog.refresh(Snapshot { detail, ..snapshot });
    }
}

/// The Reset… question: the production wording, Yes and No.
fn reset_prompt(page: Page) -> StandardPrompt {
    let ask = match page {
        Page::Ribbon => tl!("Remove every custom tab and group and show all tabs again?"),
        Page::QuickAccess => tl!("Put the default commands back on the Quick Access Toolbar?"),
    };
    StandardPrompt::custom(Label::from(ask.to_string())).button(PromptChoice::Proceed, word("Yes")).button(PromptChoice::Cancel, word("No"))
}

/// A question the dialog opened needs no hook of its own: the dialog takes its answer.
fn prompt_settle(_: &mut WordApp, _: &Context, _: DialogId) -> Settle {
    Settle::Done
}

/// After the host runs: the Reset… answer, then the requests the body filed. True once the dialog is gone.
fn settle(app: &mut WordApp, _ctx: &Context, id: DialogId) -> Settle {
    let Some(state) = app.dialog_host.get_mut_by_id::<RibbonDialog>(id).map(|d| {
        (std::mem::take(&mut d.requests), std::mem::take(&mut d.message), d.selected.clone(), std::mem::take(&mut d.qat_selected), d.reset, d.page)
    }) else {
        return if app.dialog_host.take_by_id::<RibbonDialog>(id).is_some() { Settle::Done } else { Settle::Keep };
    };
    let (requests, message, selected, qat_selected, mut reset, page) = state;
    let mut picks = Picks { message, selected, qat_selected };
    if let Some((prompt, asked)) = reset
        && let Some(answer) = app.dialog_host.take_by_id::<StandardPrompt>(prompt)
    {
        reset = None;
        if answer.ok().is_some_and(|answer| answer.choice == PromptChoice::Proceed) {
            let what = if asked == Page::Ribbon { "ribbon" } else { "qat" };
            run_request(app, &mut picks, &json!({"reset": what}));
        }
    }
    for request in &requests {
        if request.get("reset").is_some() {
            // The question is a child of this dialog; its answer comes back to `reset` above.
            let prompt = open_with(app, Some(id), reset_prompt(page), DialogHooks { prepare: None, settle: prompt_settle });
            reset = Some((prompt, page));
        } else {
            run_request(app, &mut picks, request);
        }
    }
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<RibbonDialog>(id) {
        dialog.message = picks.message;
        dialog.selected = picks.selected;
        dialog.qat_selected = picks.qat_selected;
        dialog.reset = reset;
    }
    Settle::Keep
}

/// Opens Customize Ribbon or Customize Quick Access Toolbar (`customizeRibbon`, `customizeQuickAccess`; dg.A2).
/// There is one dialog: when it is open under either name, that dialog switches to the page and is returned.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let page = if name == KEYS[1] { Page::QuickAccess } else { Page::Ribbon };
    for key in KEYS {
        let id = dialog_id(key);
        if let Some(dialog) = app.dialog_host.get_mut_by_id::<RibbonDialog>(id) {
            dialog.page = page;
            dialog.message.clear();
            return Some(id);
        }
    }
    let category = categories(app).into_iter().next().unwrap_or_default();
    let key = if page == Page::QuickAccess { KEYS[1] } else { KEYS[0] };
    let id = open_with(app, parent, RibbonDialog::new(key, page, category), DialogHooks { prepare: Some(prepare), settle });
    // The first pass shows the session's ribbon, not an empty tree.
    prepare(app, id);
    Some(id)
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_engine::Session;

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
        assert!(a.dialog_host.open_dialogs().iter().any(|d| d.key.as_deref() == Some("customizeRibbon")), "the dialog is open");
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
        assert_eq!(b.session.ribbon.qat().last(), Some(&"format.bold"));

        let mut c = app();
        c.apply_prefs(serde_json::from_str(r#"{"ribbon": {"tabs": 3, "qat": ["gone.command", "edit.undo"]}}"#).unwrap());
        assert!(c.session.ribbon.ribbon_is_default());
        assert_eq!(c.session.ribbon.qat(), ["edit.undo"]);
    }

    /// The app in a harness: its logic and its window each pass, as the shell runs them.
    fn harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        )
    }

    /// #379: a custom tab shows its group's commands as working buttons, and a hidden tab isn't
    /// shown (the ribbon falls back to a visible one).
    #[test]
    fn custom_tabs_draw_their_commands_and_hidden_tabs_dont_show() {
        let mut a = app();
        a.run("tools.customizeRibbon", json!({"newTab": {"name": "Mine"}, "hide": "Home"})).unwrap();
        a.run("tools.customizeRibbon", json!({"add": {"tab": "Mine", "group": "New Group", "command": "view.ruler"}})).unwrap();
        let ruler = a.session.view.ruler;
        a.ui.tab = "Home".into();
        let mut h = harness(a);
        // The custom button is named by its command and runs through the app's command input.
        crate::test_harness::frames(&mut h, 4);
        let shown = h.state().ui.tab.clone();
        assert_ne!(shown, "Home", "a hidden tab isn't shown");
        assert!(h.state().session.ribbon.tabs().iter().any(|t| t.name == shown && !t.hidden), "the ribbon falls back to a visible tab");

        h.get_by_label("Mine").click();
        crate::test_harness::frames(&mut h, 3);
        assert_eq!(h.state().ui.tab, "Mine");
        h.get_by_label("Ruler").click();
        crate::test_harness::frames(&mut h, 3);
        assert_ne!(h.state().session.view.ruler, ruler, "the custom group's button runs its command");
    }
}
