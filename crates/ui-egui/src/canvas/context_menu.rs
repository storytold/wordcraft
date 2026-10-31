use super::{Ui, WordApp, json};

/// The spelling suggestions and synonyms the context menu offers for the selection or the word
/// at the caret.
pub(super) fn context_suggestions(app: &mut WordApp) {
    app.canvas.context_issue = app.session.run("review.suggestions", &json!({})).ok().filter(|v| !v.is_null());
    app.canvas.context_synonyms = app.session.run("review.thesaurus", &json!({})).ok().and_then(|v| v.get("synonyms").cloned());
}

/// An item of the menu: a menu item to screen readers (egui draws a button), named by its label
/// and key, and kept in the order the keyboard goes through (#4977). One that is off (not `on`)
/// still takes the keyboard, as in Word's menus, and screen readers say it is unavailable.
fn menu_item(app: &mut WordApp, resp: &egui::Response, name: &str, on: bool) {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, on, name));
    app.canvas.context_menu_items.push((resp.layer_id, resp.id));
}

/// An item of a choice in the menu (Wrap Text's styles): a radio menu item to screen readers,
/// checked when it is the current one.
fn menu_choice(app: &mut WordApp, resp: &egui::Response, name: &str, checked: bool) {
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::MenuItemRadio, resp.enabled(), checked, name));
    app.canvas.context_menu_items.push((resp.layer_id, resp.id));
}

/// The item that opens a submenu: a menu item that has one, collapsed or expanded, which screen
/// readers say ("Insert, collapsed") as in Word's menus.
fn submenu_item(app: &mut WordApp, ui: &Ui, trigger: &egui::InnerResponse<Option<()>>, name: &str) {
    menu_item(app, &trigger.response, name, true);
    let open = trigger.inner.is_some();
    ui.ctx().accesskit_node_builder(trigger.response.id, |n| {
        n.set_has_popup(egui::accesskit::HasPopup::Menu);
        n.set_expanded(open);
    });
}

/// Each item's place in the menu (or submenu) `ui` draws, and the number of its items, which
/// screen readers say ("2 of 9") as in Word's menus; items that are off count too. The number
/// goes on the menu: UI Automation takes it from the item's container.
fn number_items(ui: &Ui, items: &[(egui::LayerId, egui::Id)]) {
    let here: Vec<egui::Id> = items.iter().filter(|(layer, _)| *layer == ui.layer_id()).map(|(_, id)| *id).collect();
    for (at, id) in here.iter().enumerate() {
        ui.ctx().accesskit_node_builder(*id, |n| n.set_position_in_set(at));
    }
    if let Some(menu) = ui.stack().iter().find(|s| s.kind() == Some(egui::UiKind::Menu)).map(|s| s.unique_id) {
        ui.ctx().accesskit_node_builder(menu, |n| n.set_size_of_set(here.len()));
    }
}

/// The keyboard in a menu, as in Word's: opened from the keyboard, the first item takes it (one
/// that is off too, as in Word); Up and Down go through the items of the menu (or submenu) the focus is in, round from the last to
/// the first, Home and End to the first and the last, and Left and Right don't leave it. egui
/// would move the focus to the nearest control in that direction, out of the menu (#4977).
fn keyboard(app: &mut WordApp, ui: &Ui) {
    let items = std::mem::take(&mut app.canvas.context_menu_items);
    number_items(ui, &items);
    if app.canvas.context_menu_focus_first {
        // Not in the popup's sizing pass, where nothing takes the keyboard yet.
        if let Some((_, first)) = items.iter().find(|(layer, _)| *layer == ui.layer_id()).filter(|_| !ui.is_sizing_pass()) {
            ui.memory_mut(|m| m.request_focus(*first));
            app.canvas.context_menu_focus_first = false;
        }
        return;
    }
    let Some((layer, focused)) = items.iter().copied().find(|(_, id)| ui.memory(|m| m.has_focus(*id))) else {
        // Escape takes the focus away at the start of the pass: what the keyboard was in stays
        // known for it (`context_menu_submenu_item`).
        return;
    };
    // In a submenu, the item that opens it: egui gives a submenu its item's id "with submenu".
    app.canvas.context_menu_submenu_item = if layer == ui.layer_id() {
        None
    } else {
        items.iter().map(|(_, id)| *id).find(|id| egui::containers::menu::SubMenu::id_from_widget_id(*id) == layer.id)
    };
    let level: Vec<egui::Id> = items.iter().filter(|(l, _)| *l == layer).map(|(_, id)| *id).collect();
    let Some(at) = level.iter().position(|id| *id == focused) else {
        return;
    };
    let last = level.len().saturating_sub(1);
    let (up, down, home, end, side) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Home),
            i.key_pressed(egui::Key::End),
            i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowRight),
        )
    });
    let to = if down {
        Some(if at >= last { 0 } else { at + 1 })
    } else if up {
        Some(if at == 0 { last } else { at - 1 })
    } else if home {
        Some(0)
    } else if end {
        Some(last)
    } else {
        None
    };
    if to.is_some() || side {
        ui.memory_mut(|m| {
            // Instead of egui's move to the nearest control in the arrow's direction.
            m.move_focus(egui::FocusDirection::None);
            if let Some(id) = to.and_then(|i| level.get(i)) {
                m.request_focus(*id);
            }
        });
    }
}

/// Right-click inside an equation: its structures' actions, display and conversion.
fn equation_menu(app: &mut WordApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    app.canvas.context_menu_items.clear();
    let acts = app.session.run("equation.structureActions", &json!({})).ok().and_then(|v| v.get("actions").cloned()).unwrap_or_default();
    let mut last_level = None;
    for a in acts.as_array().into_iter().flatten() {
        let (Some(action), Some(label), Some(level)) =
            (a.get("action").and_then(|v| v.as_str()), a.get("label").and_then(|v| v.as_str()), a.get("level").and_then(|v| v.as_u64()))
        else {
            continue;
        };
        if last_level.is_some_and(|l| l != level) {
            ui.separator();
        }
        last_level = Some(level);
        let resp = ui.button(tl!(label));
        menu_item(app, &resp, tl!(label), true);
        if resp.clicked() {
            let _ = app.run("equation.structure", json!({"action": action, "level": level}));
            app.canvas.want_focus = true;
            ui.close();
        }
    }
    if last_level.is_some() {
        ui.separator();
    }
    let display = app.session.run("equation.get", &json!({})).ok().and_then(|v| v.get("display").and_then(|d| d.as_bool())).unwrap_or(false);
    let items: [(&str, &str, serde_json::Value); 5] = [
        (if display { "Change to Inline" } else { "Change to Display" }, "equation.display", json!({"value": !display})),
        ("Professional", "equation.convert", json!({"to": "professional"})),
        ("Linear", "equation.convert", json!({"to": "linear"})),
        ("Equation Number", "equation.number", json!({})),
        ("Close Equation", "equation.exit", json!({})),
    ];
    for (label, id, params) in items {
        let resp = ui.button(tl!(label));
        menu_item(app, &resp, tl!(label), true);
        if resp.clicked() {
            let _ = app.run(id, params);
            app.canvas.want_focus = true;
            ui.close();
        }
    }
    keyboard(app, ui);
}

pub(super) fn context_menu(app: &mut WordApp, ui: &mut Ui) {
    if app.session.math.is_some() {
        return equation_menu(app, ui);
    }
    ui.set_min_width(220.0);
    app.canvas.context_menu_items.clear();
    let item = |ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: serde_json::Value| {
        let sc = crate::widgets::shortcut_text(app, id);
        let on = crate::widgets::enabled(app, id);
        // The name egui gives a button with a shortcut: its label, a space, its key.
        let name = format!("{} {sc}", tl!(label));
        // An item that is off takes the keyboard too (`menu_item`): egui gives none to a disabled
        // button, so it is drawn faded instead, and does nothing.
        let resp = ui
            .scope(|ui| {
                if !on {
                    ui.multiply_opacity(ui.visuals().disabled_alpha());
                }
                ui.add(egui::Button::new(tl!(label)).shortcut_text(sc))
            })
            .inner;
        menu_item(app, &resp, &name, on);
        if on && resp.clicked() {
            let _ = app.run(id, params);
            ui.close();
        }
    };
    if let Some(issue) = app.canvas.context_issue.clone() {
        let sugg: Vec<String> = issue
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        // English rules' messages are catalog keys; Polish rules already explain in Polish.
        ui.label(egui::RichText::new(tl!(issue.get("message").and_then(|m| m.as_str()).unwrap_or(""))).small().weak());
        if sugg.is_empty() {
            ui.label(egui::RichText::new(tl!("(no suggestions)")).italics());
        }
        for sgt in sugg {
            let resp = ui.button(egui::RichText::new(&sgt).strong());
            menu_item(app, &resp, &sgt, true);
            if resp.clicked() {
                let _ = app.run("review.applySuggestion", json!({"text": sgt}));
                ui.close();
            }
        }
        if issue.get("kind").and_then(|k| k.as_str()) == Some("spelling") {
            item(ui, app, "Ignore All", "review.ignoreAll", json!({}));
            item(ui, app, "Add to Dictionary", "review.addToDictionary", json!({}));
        }
        ui.separator();
    }
    item(ui, app, "Cut", "edit.cut", json!({}));
    item(ui, app, "Copy", "edit.copy", json!({}));
    // How to paste, chosen before pasting (#2583): the ribbon's Paste menu, item for item. Paste
    // keeps the clipboard's formatting (Word's Keep Source Formatting).
    item(ui, app, "Paste", "edit.paste", json!({}));
    item(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
    item(ui, app, "Merge Formatting", "edit.pasteMerge", json!({}));
    item(ui, app, "Paste Special…", "edit.pasteSpecial", json!({}));
    ui.separator();
    if let Some(syn) = app.canvas.context_synonyms.clone().and_then(|v| v.as_array().cloned()).filter(|a| !a.is_empty()) {
        let trigger = ui.menu_button(tl!("Synonyms"), |ui| {
            for w in syn.iter().filter_map(|x| x.as_str()) {
                let resp = ui.button(w);
                menu_item(app, &resp, w, true);
                if resp.clicked() {
                    let _ = app.run("select.word", json!({}));
                    let (a, b) = app.session.sel.ordered();
                    let trimmed = app.session.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off)).map(|t| t.trim_end().len()).unwrap_or(0);
                    let end = wordcraft_doc::Pos { off: a.off + trimmed, ..b };
                    app.session.sel = wordcraft_engine::Selection { anchor: a, focus: end };
                    let _ = app.run("text.insert", json!({"text": w, "raw": true}));
                    ui.close();
                }
            }
            number_items(ui, &app.canvas.context_menu_items);
        });
        submenu_item(app, ui, &trigger, tl!("Synonyms"));
    }
    item(ui, app, "Font…", "ui.dialog", json!({"name": "font"}));
    item(ui, app, "Paragraph…", "ui.dialog", json!({"name": "paragraph"}));
    item(ui, app, "Link…", "ui.dialog", json!({"name": "link"}));
    item(ui, app, "New Comment", "review.newComment", json!({}));
    if !wordcraft_engine::cmd::fields::selected(&app.session).is_empty() {
        ui.separator();
        item(ui, app, tl!("Update Field"), "references.updateFields", json!({}));
        item(ui, app, tl!("Edit Field…"), "fields.setCode", json!({}));
        item(ui, app, tl!("Toggle Field Codes"), "fields.toggleCode", json!({}));
    } else if wordcraft_engine::cmd::references::caret_in_toc(&app.session) {
        // The TOC field sits in the table's heading; its entries are plain paragraphs.
        ui.separator();
        item(ui, app, tl!("Update Field"), "references.updateToc", json!({}));
        item(ui, app, tl!("Remove Table of Contents"), "references.removeToc", json!({}));
    }
    let selected_object = wordcraft_engine::cmd::objects::selected(&app.session).is_some();
    if selected_object || app.session.sel.focus.path.cell().is_some() {
        item(ui, app, "View Alt Text…", "view.altTextPane", json!({"value": true}));
    }
    // A selected picture, shape, text box or chart: its own items, then Wrap Text (#513).
    let picture = crate::ribbon::has_picture_selected(&app.session);
    let object = wordcraft_engine::cmd::objects::object_selection(&app.session).is_some();
    if picture || object {
        ui.separator();
    }
    if picture {
        item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
        item(ui, app, "Reset Picture", "picture.reset", json!({}));
        item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
        item(ui, app, "Crop", "ui.crop", json!({}));
    }
    if picture || object {
        // A numbered caption under (or over) the picture, shape, text box or chart (#1230).
        item(ui, app, "Insert Caption…", "references.caption", json!({}));
    }
    if object {
        let trigger = ui.menu_button(tl!("Wrap Text"), |ui| {
            let choices = crate::ribbon::wrap_text_items(ui, app);
            let ids: Vec<egui::Id> = choices.iter().map(|(resp, _, _)| resp.id).collect();
            for (resp, name, checked) in choices {
                menu_choice(app, &resp, &name, checked);
            }
            // Two choices, as the separator shows: the styles, and Move with Text or Fix Position
            // on Page. Each item belongs to its own.
            let (styles, anchoring) = ids.split_at_checked(crate::ribbon::WRAP_STYLES.len()).unwrap_or((&ids, &[]));
            for group in [styles, anchoring] {
                let nodes: Vec<egui::accesskit::NodeId> = group.iter().map(|id| id.accesskit_id()).collect();
                for id in group {
                    ui.ctx().accesskit_node_builder(*id, |n| n.set_radio_group(nodes.clone()));
                }
            }
            number_items(ui, &app.canvas.context_menu_items);
        });
        submenu_item(app, ui, &trigger, tl!("Wrap Text"));
    }
    if crate::widgets::enabled(app, "shape.fill") {
        item(ui, app, "Format Shape…", "view.formatShapePane", json!({"value": true}));
    }
    if app.session.sel.focus.path.cell().is_some() {
        // The table's items follow what is selected, as in Word (2019, Windows).
        let pick = table_pick(&app.session);
        ui.separator();
        let trigger = ui.menu_button(tl!("Insert"), |ui| {
            item(ui, app, "Insert Columns to the Left", "table.insertColumnLeft", json!({}));
            item(ui, app, "Insert Columns to the Right", "table.insertColumnRight", json!({}));
            item(ui, app, "Insert Rows Above", "table.insertRowAbove", json!({}));
            item(ui, app, "Insert Rows Below", "table.insertRowBelow", json!({}));
            number_items(ui, &app.canvas.context_menu_items);
        });
        submenu_item(app, ui, &trigger, tl!("Insert"));
        // Every Delete is there whatever is selected, as on Word's Delete button: with the caret
        // in a cell, Delete Table is how the table goes (#1198).
        let trigger = ui.menu_button(tl!("Delete"), |ui| {
            item(ui, app, "Delete Cells", "table.deleteCells", json!({}));
            item(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
            item(ui, app, "Delete Rows", "table.deleteRow", json!({}));
            item(ui, app, "Delete Table", "table.deleteTable", json!({}));
            number_items(ui, &app.canvas.context_menu_items);
        });
        submenu_item(app, ui, &trigger, tl!("Delete"));
        if pick == TablePick::Cell {
            item(ui, app, "Split Cells", "table.split", json!({}));
        }
        if matches!(pick, TablePick::Cells | TablePick::Rows | TablePick::Columns) {
            item(ui, app, "Merge Cells", "table.merge", json!({}));
        }
        if matches!(pick, TablePick::Rows | TablePick::Columns | TablePick::Table) {
            item(ui, app, "Distribute Rows", "table.distributeRows", json!({}));
            item(ui, app, "Distribute Columns", "table.distributeColumns", json!({}));
        }
        if pick == TablePick::Table {
            let trigger = ui.menu_button(tl!("AutoFit"), |ui| {
                item(ui, app, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
                item(ui, app, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
                item(ui, app, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
                number_items(ui, &app.canvas.context_menu_items);
            });
            submenu_item(app, ui, &trigger, tl!("AutoFit"));
        }
        item(ui, app, "Text Direction", "table.textDirection", json!({}));
        if pick == TablePick::Table {
            item(ui, app, "Insert Caption…", "references.caption", json!({}));
        }
        item(ui, app, "Table Properties…", "table.properties", json!({}));
    }
    keyboard(app, ui);
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
