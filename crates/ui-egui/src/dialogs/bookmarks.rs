use super::chrome::buttons;
use super::*;

/// The innermost bookmark around the caret, which the Bookmark dialog opens on (as Word does);
/// empty when the caret is in none.
pub(super) fn bookmark_at_caret(s: &wordcraft_engine::Session) -> String {
    let at = &s.sel.focus;
    s.doc
        .bookmarks()
        .into_iter()
        .rev()
        .find(|(name, start)| !name.starts_with('_') && start <= at && *at <= wordcraft_engine::cmd::insert::bookmark_span(&s.doc, name, start).1)
        .map(|(name, _)| name)
        .unwrap_or_default()
}

/// The bookmarks the Bookmark dialog lists: by name or in document order, the hidden ones
/// (`_Toc…`, `_Ref…`) only when asked for.
pub(super) fn bookmark_names(doc: &wordcraft_doc::Document, by_location: bool, hidden: bool) -> Vec<String> {
    let mut names: Vec<String> = doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| hidden || !n.starts_with('_')).collect();
    if !by_location {
        names.sort_by_key(|n| n.to_lowercase());
    }
    names
}

/// Insert › Bookmark, laid out as Word's (#1236): the name box over the list, Add / Delete / Go To
/// beside them, Sort by and Hidden bookmarks below. Add (Enter) closes the dialog; Delete and Go
/// To leave it open, and once a bookmark is deleted Cancel reads Close.
pub(super) fn bookmark(app: &mut WordApp, ui: &mut Ui, name: &mut String, by_location: &mut bool, hidden: &mut bool, deleted: &mut bool) -> bool {
    let marks = bookmark_names(&app.session.doc, *by_location, *hidden);
    let t = Tokens::get(ui.ctx());
    let (mut add, mut delete, mut go_to) = (false, false, false);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            let name_label = ui.label(tl!("Bookmark name:"));
            ui.add(egui::TextEdit::singleline(name).desired_width(240.0)).labelled_by(name_label.id);
            egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
                ui.set_width(232.0);
                ui.set_height(150.0);
                egui::ScrollArea::vertical().id_salt("bookmark_list").max_height(150.0).auto_shrink([false, false]).show(ui, |ui| {
                    if marks.is_empty() {
                        // #1904: a blank box says nothing; reuse the Field dialog's wording.
                        ui.label(egui::RichText::new(tl!("The document has no bookmarks yet.")).small().color(t.text_dim));
                    }
                    for n in &marks {
                        if ui.selectable_label(*n == *name, n.as_str()).clicked() {
                            *name = n.clone();
                        }
                    }
                });
            });
        });
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let listed = marks.iter().any(|n| n == name.trim());
            let w = vec2(90.0, 0.0);
            add = ui.add_enabled(wordcraft_engine::cmd::insert::bookmark_name_ok(name.trim()), egui::Button::new(tl!("Add")).min_size(w)).clicked();
            delete = ui.add_enabled(listed, egui::Button::new(tl!("Delete")).min_size(w)).clicked();
            go_to = ui.add_enabled(listed, egui::Button::new(tl!("Go To")).min_size(w)).clicked();
        });
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.label(tl!("Sort by:"));
        ui.vertical(|ui| {
            ui.radio_value(by_location, false, tl!("Name"));
            ui.radio_value(by_location, true, tl!("Location"));
        });
    });
    ui.checkbox(hidden, tl!("Hidden bookmarks"));
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if add || (enter && wordcraft_engine::cmd::insert::bookmark_name_ok(name.trim())) {
        let _ = app.run("insert.bookmark", json!({"name": name.trim()}));
        return true;
    }
    if delete && app.run("insert.deleteBookmark", json!({"name": name.trim()})).is_ok() {
        name.clear();
        *deleted = true;
    }
    if go_to {
        let _ = app.run("edit.goto", json!({"bookmark": name.trim()}));
    }
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= ui.button(if *deleted { tl!("Close") } else { tl!("Cancel") }).clicked();
        });
    });
    close
}

/// path data; Insert (or a double click) runs `insert.icon`. Returns true to close.
pub(super) fn icons_body(app: &mut WordApp, ui: &mut Ui, query: &mut String, category: &mut String, selected: &mut String) -> bool {
    use wordcraft_render::icon_lib::{self, Category};
    let t = Tokens::get(ui.ctx());
    ui.set_width(560.0);
    // Typing goes to the search box (never to the document behind the dialog).
    let r = ui.add(egui::TextEdit::singleline(query).hint_text(tl!("Search icons")).desired_width(560.0));
    if !r.has_focus() {
        r.request_focus();
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        if ui.selectable_label(category.is_empty(), tl!("All")).clicked() {
            category.clear();
        }
        for c in Category::ALL {
            if ui.selectable_label(*category == c.name(), tl!(c.name())).clicked() {
                *category = c.name().to_string();
            }
        }
    });
    ui.add_space(4.0);
    let hits = icon_lib::search(query, Category::parse(category));
    let mut insert = false;
    egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([true, false]).show(ui, |ui| {
        if hits.is_empty() {
            ui.label(egui::RichText::new(tl!("No icons match your search.")).color(t.text_dim));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for icon in &hits {
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                let on = *selected == icon.id;
                let fill = if on {
                    t.checked
                } else if resp.hovered() {
                    t.hover
                } else {
                    egui::Color32::TRANSPARENT
                };
                let edge = if on { egui::Stroke::new(1.5, t.accent) } else { egui::Stroke::new(1.0, t.border) };
                ui.painter().rect(rect, 4.0, fill, edge, egui::StrokeKind::Inside);
                crate::icons::library(ui.painter(), rect.shrink(12.0), icon, t.text);
                let resp = resp.on_hover_text(icon.name);
                if resp.clicked() {
                    *selected = icon.id.to_string();
                }
                if resp.double_clicked() {
                    *selected = icon.id.to_string();
                    insert = true;
                }
            }
        });
    });
    let name = icon_lib::find(selected).map(|i| i.name).unwrap_or("");
    ui.add_space(4.0);
    ui.label(egui::RichText::new(name).color(t.text_dim));
    let (ok, cancel) = buttons(ui, tl!("Insert"));
    if (ok || insert) && !selected.is_empty() {
        let _ = app.run("insert.icon", json!({"id": selected.as_str()}));
        return true;
    }
    cancel
}

/// Enclose Characters' preview: the text in the chosen shape, shrunk or with the shape enlarged
/// (drawn, so it doesn't depend on the interface font having ○ □ △ ◇).
pub(super) fn enclose_preview(ui: &mut Ui, style: &str, text: &str, shape: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(110.0, 64.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, t.input);
    p.rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let ink = ui.visuals().text_color();
    let c = r.center();
    let (box_h, text_px) = match style {
        "none" => (0.0, 26.0),
        "enlarge" => (48.0, 26.0),
        _ => (34.0, 20.0),
    };
    let h = box_h / 2.0;
    let stroke = egui::Stroke::new(1.5, ink);
    match (shape, box_h > 0.0) {
        (_, false) => {}
        ("square", _) => {
            p.rect_stroke(egui::Rect::from_center_size(c, vec2(box_h, box_h)), 0.0, stroke, egui::StrokeKind::Middle);
        }
        ("triangle", _) => {
            let pts = vec![egui::pos2(c.x, c.y - h), egui::pos2(c.x + h * 1.1, c.y + h), egui::pos2(c.x - h * 1.1, c.y + h)];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        ("diamond", _) => {
            let pts = vec![egui::pos2(c.x, c.y - h), egui::pos2(c.x + h, c.y), egui::pos2(c.x, c.y + h), egui::pos2(c.x - h, c.y)];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        _ => {
            p.circle_stroke(c, h, stroke);
        }
    }
    let px = match (style, shape) {
        ("shrink", "triangle" | "diamond") => text_px * 0.75,
        _ => text_px,
    };
    let dy = if shape == "triangle" && box_h > 0.0 { h * 0.3 } else { 0.0 };
    p.text(c + vec2(0.0, dy), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(px), ink);
}
