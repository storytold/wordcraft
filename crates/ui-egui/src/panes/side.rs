use std::borrow::Cow;

use craft::behavior::{IconId, Selection, SelectionChange, keyed};
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props, SelectMode, Show, ShowCollection, ShowContainer, clicked};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::menu::Menu;
use craft::ui::primitives::IconSpec;
use egui::Ui;
use serde_json::{Value, json};

use super::WordApp;

/// Items collected by Copy and Cut, newest first; clicking one pastes it at the caret.
pub(super) fn clipboard(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let items: Vec<String> = app.session.clip_history.items().iter().map(|i| i.preview()).collect();
    Stack::new().fill_width().show(ui, |ui, _| {
        Row::new().wrap(true).show(ui, |ui, _| {
            if Button::new(tl!("Paste All").to_owned()).enabled(!items.is_empty()).show(ui).clicked() {
                let _ = app.run("edit.pasteAllClipboard", json!({}));
                app.canvas.focus_page(&ctx);
            }
            if Button::new(tl!("Clear All").to_owned()).enabled(!items.is_empty()).show(ui).clicked() {
                let _ = app.run("edit.clearClipboard", json!({}));
                app.canvas.focus_page(&ctx);
            }
        });
        let hint = if items.is_empty() { "Nothing collected yet. Items you copy or cut appear here." } else { "Click an item to paste it." };
        Text::new(tl!(hint).to_owned()).show(ui);
        for (i, preview) in items.iter().enumerate() {
            Row::new().show(ui, |ui, _| {
                let text = if preview.is_empty() { tl!("(picture or object)").to_owned() } else { preview.clone() };
                // PLAN: assumed `Props::tooltip(impl Into<Label>)` on the common props.
                if Button::new(text).tooltip(tl!("Paste").to_owned()).show(ui).clicked() {
                    let _ = app.run("edit.pasteClipboardItem", json!({"index": i}));
                    app.canvas.focus_page(&ctx);
                }
                if IconButton::new(IconSpec::svg(IconId::new("wordcraft.close"))).label(tl!("Delete").to_owned()).show(ui).clicked() {
                    let _ = app.run("edit.deleteClipboardItem", json!({"index": i}));
                    app.canvas.focus_page(&ctx);
                }
            });
        }
    });
}

/// The row menu of a list: opened by the row's context request, kept open across passes in `id`'s temp slot until
/// the menu closes. Returns the row and the value of the entry that ran.
fn row_menu(
    ui: &mut Ui,
    label: &str,
    opener: &egui::Response,
    asked: Option<usize>,
    id: egui::Id,
    entries: impl Fn(usize) -> Vec<Entry<String>>,
) -> Option<(usize, String)> {
    let row = asked.or_else(|| ui.data(|d| d.get_temp::<usize>(id)))?;
    let menu = Menu::context(label.to_owned(), opener).pick(ui, &entries(row));
    if menu.closed.is_some() {
        ui.data_mut(|d| d.remove::<usize>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, row));
    }
    menu.picked.map(|picked| (row, picked))
}

pub(super) fn styles(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let st = wordcraft_engine::cmd::format::state(&app.session);
    let cur = st.get("style").and_then(Value::as_str).unwrap_or("Normal").to_string();
    let current_name = app
        .session
        .doc
        .styles
        .get(&cur)
        .map(crate::i18n::style_name)
        .unwrap_or_else(|| st.get("styleName").and_then(Value::as_str).unwrap_or("").to_string());
    Text::new(crate::i18n::fmt(tl!("Current style: {style}"), &[("style", &current_name)])).show(ui);
    Row::new().wrap(true).show(ui, |ui, _| {
        if Button::new(tl!("New Style…").to_owned()).show(ui).clicked() {
            // PLAN: assumed `dialogs::open(app, name, &Value) -> Option<DialogId>` (dg.A2, dialogs-w13)
            let _ = crate::dialogs::open(app, "newStyle", &Value::Null);
        }
        if Button::new(tl!("Update to Match").to_owned()).show(ui).clicked() {
            let _ = app.run("styles.updateToMatch", json!({}));
        }
        if Button::new(tl!("Clear").to_owned()).show(ui).clicked() {
            let _ = app.run("format.clear", json!({}));
        }
    });
    if Button::new(tl!("Manage Styles…").to_owned()).show(ui).clicked() {
        let _ = app.run("styles.manage", json!({}));
    }
    if clicked(&ToggleButton::new(tl!("Style Inspector").to_owned(), Bind::given(app.session.view.style_inspector)).show(ui).response) {
        let _ = app.run("styles.inspector", json!({}));
    }
    let mut list: Vec<(String, String, bool, bool)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| !s.hidden && s.kind != wordcraft_doc::StyleKind::Table)
        .map(|s| (s.id.clone(), crate::i18n::style_name(s), s.kind == wordcraft_doc::StyleKind::Character, s.builtin))
        .collect();
    list.sort_by_key(|s| s.1.to_lowercase());
    let names: Vec<(usize, String)> = list.iter().enumerate().map(|(at, s)| (at, s.1.clone())).collect();
    let source = keyed(&names, |name: &(usize, String)| name.0).with_text(|name: &(usize, String)| Cow::Borrowed(name.1.as_str()));
    let current = list.iter().position(|s| s.0 == cur);
    // The task pane's body scrolls, and a collection owns its own scroll: the list shows this many styles and scrolls
    // the rest itself.
    let out = List::<usize>::new(tl!("Styles").to_owned())
        .max_rows(18)
        .select(SelectMode::Single)
        .row_menu()
        .selection(Bind::given(current.map_or_else(Selection::default, Selection::single)))
        .show_items(ui, &source, &mut |key, _| {
            let (_, name, is_char, _) = &list[*key];
            // The marker tells a character style from a paragraph style.
            ListRow::new(name.clone()).trailing(if *is_char { "a" } else { "¶" })
        });
    let mut asked = None;
    for event in &out.events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some((id, ..)) = keys.first().and_then(|k| list.get(*k)) {
                    let _ = app.run("para.style", json!({"style": id}));
                    app.canvas.focus_page(&ctx);
                }
            }
            CollectionEvent::ContextMenu(request) => asked = Some(request.key),
            _ => {}
        }
    }
    let menu_id = app.window_id("styles_row_menu");
    let entries = |row: usize| {
        let builtin = list.get(row).is_some_and(|s| s.3);
        vec![
            Entry::new("modify".to_owned(), tl!("Modify…").to_owned()),
            Entry::new("update".to_owned(), tl!("Update to Match Selection").to_owned()),
            Entry::new("delete".to_owned(), tl!("Delete").to_owned()).enabled(!builtin),
        ]
    };
    if let Some((row, picked)) = row_menu(ui, tl!("Style"), &out.response, asked, menu_id, entries)
        && let Some((id, ..)) = list.get(row)
    {
        match picked.as_str() {
            "modify" => {
                let _ = crate::dialogs::style_forms::modify_style(app, id);
            }
            "update" => {
                let _ = app.run("styles.updateToMatch", json!({"style": id}));
                app.canvas.focus_page(&ctx);
            }
            "delete" => {
                let _ = app.run("styles.delete", json!({"style": id}));
                app.canvas.focus_page(&ctx);
            }
            _ => {}
        }
    }
}
