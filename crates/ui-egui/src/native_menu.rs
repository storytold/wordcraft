//! Toolkit-independent application menu. The desktop host exports this to Plasma; actions still
//! enter through `WordApp::run`, including the unsaved-document guard and native file dialogs.

use serde_json::{Value, json};

use crate::{WordApp, i18n, widgets};

pub use craft_desktop::{Action, Item, Menu};

fn blocked(app: &WordApp) -> bool {
    let blocked = app.dialog.is_some() || app.colors.is_some() || app.file_dialog.is_some() || app.quit_requested;
    #[cfg(not(target_arch = "wasm32"))]
    let blocked = blocked || app.file_browser.open.is_some();
    blocked
}

fn action(app: &WordApp, label: &str, command: &str, params: Value) -> Item {
    let a = Action { command: command.into(), params };
    let edits = app.session.registry.get(command).is_some_and(|spec| spec.mutates)
        || (command == "ui.dialog" && !matches!(a.params.get("name").and_then(Value::as_str), Some("about" | "zoom" | "find")));
    Item {
        key: format!("{}:{}", a.command, a.params),
        label: i18n::t(label).into(),
        enabled: !blocked(app) && widgets::enabled(app, command) && !(app.session.view.read_mode && edits),
        checked: None,
        shortcut: widgets::shortcut_text(app, command),
        action: Some(a),
        children: Vec::new(),
        separator: false,
    }
}

fn commands(app: &WordApp, ids: &str) -> Vec<Item> {
    ids.split_whitespace()
        .filter_map(|id| {
            let spec = app.session.registry.get(id)?;
            let mut item = action(app, spec.label, id, json!({}));
            match id {
                "edit.undo" => {
                    item.enabled &= app.session.can_undo();
                    item.label = i18n::prefixed("Undo", app.session.undo_label().unwrap_or(""));
                }
                "edit.redo" => item.enabled &= app.session.can_redo(),
                "view.ruler" => item.checked = Some(app.session.view.ruler),
                "view.gridlines" => item.checked = Some(app.session.view.gridlines),
                "view.navigationPane" => item.checked = Some(app.session.view.nav_pane),
                "view.stylesPane" => item.checked = Some(app.session.view.styles_pane),
                "view.commentsPane" => item.checked = Some(app.session.view.comments_pane),
                "view.hideWhiteSpace" => item.checked = Some(app.session.view.hide_white_space),
                "view.darkMode" => item.checked = Some(app.session.view.dark_mode),
                "view.marks" => item.checked = Some(app.session.view.marks),
                "review.trackChanges" => item.checked = Some(app.session.doc.settings.track_changes),
                _ => {}
            }
            Some(item)
        })
        .collect()
}

fn dialog(app: &WordApp, label: &str, name: &str) -> Item {
    action(app, label, "ui.dialog", json!({"name": name}))
}

fn section(label: &str, children: Vec<Item>) -> Item {
    Item {
        key: label.into(),
        label: i18n::t(label).into(),
        enabled: true,
        checked: None,
        shortcut: String::new(),
        action: None,
        children,
        separator: false,
    }
}

/// Menu commands are deliberately curated: the registry also contains agent-only queries and
/// parameterized primitives. Exposing those with empty parameters would silently do the wrong
/// thing. Dialogs and explicit choices below mirror the ribbon's interactive entry points.
pub fn snapshot(app: &WordApp) -> Menu {
    if !app.root_menu_active() {
        return Menu::default();
    }
    i18n::set_current(i18n::Lang::from_pref(&app.ui.language));
    let mut file = commands(app, "file.new file.open file.save file.saveAs file.print");
    file.push(action(app, "Export", "ui.backstage", json!({"value": true, "page": "export"})));
    file.push(action(app, "Options", "ui.backstage", json!({"value": true, "page": "options"})));
    file.extend(commands(app, "file.close"));

    let mut home = vec![section("Clipboard", commands(app, "edit.cut edit.copy edit.paste"))];
    home.insert(0, section("Undo", commands(app, "edit.undo edit.redo edit.repeat")));
    let mut font = commands(app, "format.bold format.italic format.underline format.strikethrough format.subscript format.superscript format.clear");
    let state = wordcraft_engine::cmd::format::state(&app.session);
    for item in &mut font {
        if let Some(a) = &item.action {
            let key = if a.command == "format.strikethrough" { Some("strike") } else { a.command.strip_prefix("format.") };
            item.checked = key.and_then(|key| state.get(key)).and_then(Value::as_bool);
        }
    }
    font.push(dialog(app, "Font", "font"));
    home.push(section("Font", font));
    let mut para =
        commands(app, "para.alignLeft para.alignCenter para.alignRight para.justify para.indent para.outdent para.bullets para.numbering view.marks");
    para.push(dialog(app, "Paragraph", "paragraph"));
    home.push(section("Paragraph", para));
    let mut editing = commands(app, "edit.find select.all");
    editing.push(dialog(app, "Replace", "replace"));
    home.push(section("Editing", editing));

    let mut insert =
        commands(app, "insert.blankPage insert.pageBreak insert.picture insert.smartArt insert.chart insert.symbol insert.field insert.bookmark");
    insert.push(dialog(app, "Table", "insertTable"));
    insert.push(dialog(app, "Link", "link"));
    let mut layout = vec![dialog(app, "Page Setup", "pageSetup")];
    layout.push(section(
        "Orientation",
        vec![
            action(app, "Portrait", "layout.orientation", json!({"value": "portrait"})),
            action(app, "Landscape", "layout.orientation", json!({"value": "landscape"})),
        ],
    ));

    let refs = commands(
        app,
        "references.footnote references.endnote references.noteOptions references.caption references.citation references.sources references.markEntry references.index",
    );
    let review = commands(
        app,
        "review.spelling review.thesaurus review.wordCount review.language review.trackChanges review.trackingOptions review.accept review.reject review.acceptAll review.rejectAll review.nextChange review.previousChange review.compare review.combine review.restrict",
    );
    let mut view = commands(
        app,
        "view.printLayout view.webLayout view.draft view.outline view.readMode view.ruler view.gridlines view.navigationPane view.stylesPane view.commentsPane view.hideWhiteSpace view.darkMode",
    );
    view.push(section("Zoom", {
        let mut items = commands(app, "view.zoomIn view.zoomOut view.zoom100 view.onePage view.pageWidth");
        items.push(dialog(app, "Zoom", "zoom"));
        items
    }));
    let mut items = vec![
        section("File", file),
        section("Home", home),
        section("Insert", insert),
        section("Layout", layout),
        section("References", refs),
        section("Review", review),
        section("View", view),
    ];
    if app.session.sel.focus.path.cell().is_some() {
        items.push(section("Table Layout", commands(app, "table.properties table.insertRowAbove table.insertRowBelow table.insertColumnLeft table.insertColumnRight table.deleteRow table.deleteColumn table.merge table.split table.viewGridlines")));
    }
    // The remaining rich ribbon controls (galleries, drawing and custom tabs) stay reachable
    // from the panel without pretending their widgets can be transported by DBusMenu.
    let tabs = app.session.ribbon.tabs();
    let mut tools: Vec<Item> = tabs
        .iter()
        .filter(|tab| !tab.hidden)
        .map(|tab| {
            let mut item = action(app, &tab.name, "ui.tab", json!({"tab": tab.name}));
            if tab.custom {
                item.label.clone_from(&tab.name);
            }
            item
        })
        .collect();
    tools.extend(commands(app, "tools.search tools.customizeRibbon tools.customizeKeyboard tools.autocorrect"));
    items.push(section("Tools", tools));
    items.push(section("Help", vec![dialog(app, "About WordCraft", "about")]));
    Menu { document: app.session.document_id(), items }
}

/// Revalidate a queued click on the UI thread. Modal dialogs and document changes can happen
/// after Plasma read the menu; a stale event must not dismiss a save prompt or edit another file.
pub fn activate(app: &mut WordApp, document: u64, action: &Action) -> Result<(), String> {
    if !app.root_menu_active() {
        return Err("the root window's menu is inactive while another window is active".into());
    }
    if document != app.session.document_id() {
        return Err("the menu belongs to a document that has been closed".into());
    }
    fn allowed(items: &[Item], action: &Action) -> bool {
        items.iter().any(|item| (item.enabled && item.action.as_ref() == Some(action)) || allowed(&item.children, action))
    }
    if !allowed(&snapshot(app).items, action) {
        return Err("this menu action is not available right now".into());
    }
    app.run(&action.command, action.params.clone())?;
    if !action.command.starts_with("ui.") {
        app.canvas.want_focus = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    fn app() -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
    }
    fn find<'a>(items: &'a [Item], command: &str) -> Option<&'a Item> {
        items.iter().find_map(|i| if i.action.as_ref().is_some_and(|a| a.command == command) { Some(i) } else { find(&i.children, command) })
    }

    #[test]
    fn native_menu_undo_and_checked_format_follow_edits() {
        let mut a = app();
        assert!(!find(&snapshot(&a).items, "edit.undo").unwrap().enabled);
        let bold = find(&snapshot(&a).items, "format.bold").unwrap().action.clone().unwrap();
        let doc = a.session.document_id();
        activate(&mut a, doc, &bold).unwrap();
        assert_eq!(find(&snapshot(&a).items, "format.bold").unwrap().checked, Some(true));
        a.run("text.insert", json!({"text": "Menu test"})).unwrap();
        assert!(find(&snapshot(&a).items, "edit.undo").unwrap().enabled);
    }

    #[test]
    fn native_menu_new_preserves_dirty_document_and_retires_old_root_actions() {
        let mut a = app();
        a.run("text.insert", json!({"text": "Do not lose this"})).unwrap();
        let doc = a.session.document_id();
        let new = find(&snapshot(&a).items, "file.new").unwrap().action.clone().unwrap();
        activate(&mut a, doc, &new).unwrap();
        // Current upstream opens New in another window rather than replacing the dirty one.
        assert!(a.session.documents().iter().any(|d| d.id == doc && d.dirty));
        assert_ne!(a.session.document_id(), doc);
        assert!(activate(&mut a, doc, &new).is_err());
    }

    #[test]
    fn native_menu_localizes_without_changing_action_identity() {
        let mut a = app();
        a.ui.language = "en".into();
        let en = snapshot(&a);
        a.ui.language = "ru".into();
        let ru = snapshot(&a);
        let en = find(&en.items, "file.save").unwrap();
        let ru = find(&ru.items, "file.save").unwrap();
        assert_ne!(en.label, ru.label);
        assert_eq!(en.key, ru.key);
        assert_eq!(en.action, ru.action);
        assert_eq!(ru.shortcut, if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" });
    }

    #[test]
    fn native_menu_dialogs_and_parameter_choices_use_interactive_paths() {
        let mut a = app();
        let doc = a.session.document_id();
        let orientation = Action { command: "layout.orientation".into(), params: json!({"value": "landscape"}) };
        activate(&mut a, doc, &orientation).unwrap();
        assert!(a.session.dirty);
        let para = Action { command: "ui.dialog".into(), params: json!({"name": "paragraph"}) };
        activate(&mut a, doc, &para).unwrap();
        assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("paragraph"));
        a.dialog = None;
        a.run("view.readMode", json!({})).unwrap();
        assert!(activate(&mut a, doc, &para).is_err());
        assert!(activate(&mut a, doc, &orientation).is_err());
    }

    #[test]
    fn native_menu_contextual_table_actions_follow_selection() {
        let mut a = app();
        assert!(find(&snapshot(&a).items, "table.insertColumnLeft").is_none());
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        assert!(find(&snapshot(&a).items, "table.insertColumnLeft").unwrap().enabled);
        assert!(find(&snapshot(&a).items, "table.split").is_some());
    }
}
