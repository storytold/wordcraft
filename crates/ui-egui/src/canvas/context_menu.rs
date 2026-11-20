use std::cell::RefCell;

use craft::keymap::CommandRef;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Divider;
use craft::ui::menu::{Menu, MenuItem};
use egui::{Pos2, Response, Ui, vec2};
use serde_json::{Value, json};

use crate::WordApp;

/// Where the canvas context menu opens. The framework's `ContextRequest` cannot be stored by the
/// app, so the pass's one ask is this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ContextAsk {
    /// At a point on screen: a right click, or the equation strip's lower left corner.
    AtPointer(Pos2),
    /// Below the caret: the Menu key, Shift+F10 or `ui.contextMenu`.
    AtCaret,
}

/// The spelling suggestions and synonyms the context menu offers for the selection or the word
/// at the caret.
pub(super) fn context_suggestions(app: &mut WordApp) {
    app.canvas.context_issue = app.session.run("review.suggestions", &json!({})).ok().filter(|v| !v.is_null());
    app.canvas.context_synonyms = app.session.run("review.thesaurus", &json!({})).ok().and_then(|v| v.get("synonyms").cloned());
}

/// The heading of a proofing issue's context menu, in the interface language like the rest of
/// the menu: a spelling issue's own message is in the text's language (craft-proof's, "Faute
/// d'orthographe possible" for French text), so it says Possible spelling mistake instead.
/// English grammar rules' messages are catalog keys; other languages' rules explain in their own.
pub(crate) fn issue_heading(issue: &serde_json::Value) -> String {
    if issue.get("kind").and_then(|k| k.as_str()) == Some("spelling") {
        return tl!("Possible spelling mistake").to_string();
    }
    tl!(issue.get("message").and_then(|m| m.as_str()).unwrap_or("")).to_string()
}

/// Shows the canvas context menu, opened from `opener` (the page view's canvas) by `ask`; `None`
/// opens nothing new and keeps an open menu up. The open state is `CanvasState::context_menu_open`
/// and its point `CanvasState::context_menu_at`: the framework keeps an anchor only for a context
/// opening, and this menu opens by the app's one ask.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, opener: &Response, ask: Option<ContextAsk>) {
    let mut open = app.canvas.context_menu_open;
    match ask {
        Some(ContextAsk::AtCaret) => {
            context_suggestions(app);
            if let Some((p, h)) = super::cursor::caret_screen(app) {
                app.canvas.context_menu_at = Some(p + vec2(0.0, h));
            }
            open = true;
        }
        Some(ContextAsk::AtPointer(p)) => {
            app.canvas.context_menu_at = Some(p);
            // The equation strip's menu is the equation's own: it has no proofing rows.
            if app.session.math.is_none() {
                if wordcraft_engine::cmd::objects::selected(&app.session).is_some() {
                    app.canvas.context_issue = None;
                    app.canvas.context_synonyms = None;
                } else {
                    context_suggestions(app);
                }
            }
            open = true;
        }
        None => {}
    }
    if !open {
        app.canvas.context_menu_open = false;
        app.canvas.context_menu_at = None;
        return;
    }
    let at = app.canvas.context_menu_at.unwrap_or(opener.rect.left_top());
    let menu = Menu::new(tl!("Context Menu").to_string()).opened_by(opener, Trigger::Manual).anchor(Anchor::Point(at)).open(Bind::bound(&mut open));

    if app.session.math.is_some() {
        let rows = super::equation_frame::equation_menu_rows(app);
        let pick = menu.pick(ui, &rows);
        if let Some(act) = pick.picked {
            super::equation_frame::run_equation_act(app, &act);
            app.canvas.focus_page(ui.ctx());
        }
    } else {
        // The companion bar and the rows both need the app; they run one after the other, never nested.
        let bar_visible = crate::mini_toolbar::context_menu_visible(app);
        let cell = RefCell::new(&mut *app);
        let mut bar = |ui: &mut Ui| {
            if let Ok(mut app) = cell.try_borrow_mut() {
                crate::mini_toolbar::members(&mut app, ui, false);
            }
        };
        let menu = if bar_visible { menu.companion(&mut bar) } else { menu };
        let _ = menu.show(ui, |ui, _cx| {
            if let Ok(mut app) = cell.try_borrow_mut() {
                rows(&mut app, ui);
            }
        });
    }
    app.canvas.context_menu_open = open;
    if !open {
        app.canvas.context_menu_at = None;
    }
}

/// A row labelled `label` that runs command `id` with `params`; its chord, enabled state and
/// action come from the command, its wording from the menu (one command id serves several rows).
fn item(ui: &mut Ui, label: &str, id: &str, params: Value) {
    let _ = MenuItem::new(tl!(label).to_string()).command(CommandRef::with_params(id, params)).show(ui);
}

fn rule(ui: &mut Ui) {
    let _ = Divider::horizontal().show(ui);
}

/// The text menu's rows.
fn rows(app: &mut WordApp, ui: &mut Ui) {
    if let Some(issue) = app.canvas.context_issue.clone() {
        let sugg: Vec<String> = issue
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let _ = MenuItem::heading(issue_heading(&issue)).show(ui);
        if sugg.is_empty() {
            let _ = MenuItem::heading(tl!("(no suggestions)").to_string()).show(ui);
        }
        for sgt in sugg {
            if MenuItem::new(sgt.clone()).show(ui).ran {
                let _ = app.run("review.applySuggestion", json!({"text": sgt}));
            }
        }
        if issue.get("kind").and_then(|k| k.as_str()) == Some("spelling") {
            item(ui, "Ignore All", "review.ignoreAll", json!({}));
            item(ui, "Add to Dictionary", "review.addToDictionary", json!({}));
        }
        rule(ui);
    }
    item(ui, "Cut", "edit.cut", json!({}));
    item(ui, "Copy", "edit.copy", json!({}));
    // How to paste, chosen before pasting (#2583): the ribbon's Paste menu, item for item. Paste
    // keeps the clipboard's formatting (Word's Keep Source Formatting).
    item(ui, "Paste", "edit.paste", json!({}));
    item(ui, "Keep Text Only", "edit.pasteText", json!({}));
    item(ui, "Merge Formatting", "edit.pasteMerge", json!({}));
    item(ui, "Paste Special…", "edit.pasteSpecial", json!({}));
    rule(ui);
    if let Some(syn) = app.canvas.context_synonyms.clone().and_then(|v| v.as_array().cloned()).filter(|a| !a.is_empty()) {
        let _ = Menu::submenu_row(ui, tl!("Synonyms").to_string(), |ui, _cx| {
            for w in syn.iter().filter_map(|x| x.as_str()) {
                if MenuItem::new(w.to_string()).show(ui).ran {
                    let _ = app.run("select.word", json!({}));
                    let (a, b) = app.session.sel.ordered();
                    let trimmed = app.session.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off)).map(|t| t.trim_end().len()).unwrap_or(0);
                    let end = wordcraft_doc::Pos { off: a.off + trimmed, ..b };
                    app.session.set_selection(wordcraft_engine::Selection { anchor: a, focus: end });
                    let _ = app.run("text.insert", json!({"text": w, "raw": true}));
                }
            }
        });
    }
    item(ui, "Font…", "ui.dialog", json!({"name": "font"}));
    item(ui, "Paragraph…", "ui.dialog", json!({"name": "paragraph"}));
    item(ui, "Link…", "ui.dialog", json!({"name": "link"}));
    item(ui, "New Comment", "review.newComment", json!({}));
    if !wordcraft_engine::cmd::fields::selected(&app.session).is_empty() {
        rule(ui);
        item(ui, "Update Field", "references.updateFields", json!({}));
        item(ui, "Edit Field…", "fields.setCode", json!({}));
        item(ui, "Toggle Field Codes", "fields.toggleCode", json!({}));
    } else if wordcraft_engine::cmd::references::caret_in_toc(&app.session) {
        // The TOC field sits in the table's heading; its entries are plain paragraphs.
        rule(ui);
        item(ui, "Update Field", "references.updateToc", json!({}));
        item(ui, "Remove Table of Contents", "references.removeToc", json!({}));
    }
    let selected_object = wordcraft_engine::cmd::objects::selected(&app.session).is_some();
    if selected_object || app.session.sel.focus.path.cell().is_some() {
        item(ui, "View Alt Text…", "view.altTextPane", json!({"value": true}));
    }
    // A selected picture, shape, text box or chart: its own items, then Wrap Text (#513).
    let picture = crate::ribbon::has_picture_selected(&app.session);
    let object = wordcraft_engine::cmd::objects::object_selection(&app.session).is_some();
    if picture || object {
        rule(ui);
    }
    if picture {
        item(ui, "Change Picture…", "ui.changePicture", json!({}));
        item(ui, "Reset Picture", "picture.reset", json!({}));
        item(ui, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
        item(ui, "Crop", "ui.crop", json!({}));
    }
    if picture || object {
        // A numbered caption under (or over) the picture, shape, text box or chart (#1230).
        item(ui, "Insert Caption…", "references.caption", json!({}));
    }
    if object {
        let _ = Menu::submenu_row(ui, tl!("Wrap Text").to_string(), |ui, _cx| crate::ribbon::wrap_text_items(ui, app));
    }
    if craft::ui::runtime::commands::resolve(ui.ctx(), CommandRef::new("shape.fill")).enabled {
        item(ui, "Format Shape…", "view.formatShapePane", json!({"value": true}));
    }
    if app.session.sel.focus.path.cell().is_some() {
        // The table's items follow what is selected, as in Word (2019, Windows).
        let pick = table_pick(&app.session);
        rule(ui);
        let _ = Menu::submenu_row(ui, tl!("Insert").to_string(), |ui, _cx| {
            item(ui, "Insert Columns to the Left", "table.insertColumnLeft", json!({}));
            item(ui, "Insert Columns to the Right", "table.insertColumnRight", json!({}));
            item(ui, "Insert Rows Above", "table.insertRowAbove", json!({}));
            item(ui, "Insert Rows Below", "table.insertRowBelow", json!({}));
        });
        // Every Delete is there whatever is selected, as on Word's Delete button: with the caret
        // in a cell, Delete Table is how the table goes (#1198).
        let _ = Menu::submenu_row(ui, tl!("Delete").to_string(), |ui, _cx| {
            item(ui, "Delete Cells", "table.deleteCells", json!({}));
            item(ui, "Delete Columns", "table.deleteColumn", json!({}));
            item(ui, "Delete Rows", "table.deleteRow", json!({}));
            item(ui, "Delete Table", "table.deleteTable", json!({}));
        });
        if pick == TablePick::Cell {
            item(ui, "Split Cells", "table.split", json!({}));
        }
        if matches!(pick, TablePick::Cells | TablePick::Rows | TablePick::Columns) {
            item(ui, "Merge Cells", "table.merge", json!({}));
        }
        if matches!(pick, TablePick::Rows | TablePick::Columns | TablePick::Table) {
            item(ui, "Distribute Rows", "table.distributeRows", json!({}));
            item(ui, "Distribute Columns", "table.distributeColumns", json!({}));
        }
        if pick == TablePick::Table {
            let _ = Menu::submenu_row(ui, tl!("AutoFit").to_string(), |ui, _cx| {
                item(ui, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
                item(ui, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
                item(ui, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
            });
        }
        item(ui, "Text Direction", "table.textDirection", json!({}));
        if pick == TablePick::Table {
            item(ui, "Insert Caption…", "references.caption", json!({}));
        }
        item(ui, "Table Properties…", "table.properties", json!({}));
    }
}

/// What of a table the selection is: Word's context menu offers different table items for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TablePick {
    /// The caret or text in one cell, or that one cell.
    Cell,
    /// Cells of more than one row or column, but not whole rows or columns.
    Cells,
    Rows,
    Columns,
    Table,
}

fn table_pick(s: &wordcraft_engine::Session) -> TablePick {
    if let Some(r) = s.cell_range()
        && let Some(t) = s.doc.table(r.story, &r.table)
    {
        let all_rows = r.rows == (0, t.rows.len().saturating_sub(1));
        let all_cols = r.cols == (0, t.cols().saturating_sub(1));
        return match (all_rows, all_cols) {
            (true, true) => TablePick::Table,
            (true, false) => TablePick::Columns,
            (false, true) => TablePick::Rows,
            _ if r.cells(t).len() > 1 => TablePick::Cells,
            _ => TablePick::Cell,
        };
    }
    // Text running from one cell into another of the same table.
    match (s.sel.anchor.path.cell(), s.sel.focus.path.cell()) {
        (Some((ta, ra, ca)), Some((tb, rb, cb))) if ta == tb && (ra, ca) != (rb, cb) => TablePick::Cells,
        _ => TablePick::Cell,
    }
}
