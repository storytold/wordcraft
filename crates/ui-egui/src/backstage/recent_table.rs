use egui::{Rect, Sense, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{Tokens, fit_text, medium, regular, semibold};
use crate::{RecentDocument, WordApp};

pub(crate) fn open_list(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.menu_button(egui::RichText::new(tl!("Recent")).font(semibold(16.0)), |ui| {
        if ui.button(tl!("Clear Recent")).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("recent_clear_confirm"), true));
            ui.close();
        }
    });
    let confirm_id = egui::Id::unique("recent_clear_confirm");
    let is_confirm = ui.ctx().data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
    if is_confirm {
        let mut close = false;
        let modal = egui::Modal::new(confirm_id).show(ui.ctx(), |ui| {
            ui.label(tl!("Clear Recent"));
            ui.horizontal(|ui| {
                if ui.button(tl!("Clear")).clicked() {
                    let result = app.run("ui.recentClear", json!({}));
                    set_recent_save_error(
                        ui.ctx(),
                        result.as_ref().ok().and_then(|v| v.get("saved")).and_then(serde_json::Value::as_bool) == Some(false),
                    );
                    close = true;
                }
                close |= ui.button(tl!("Cancel")).clicked();
            });
        });
        let intent_cancel =
            craft::ui::runtime::keys::with_window_intents(ui.ctx(), false, |window| window.take(craft::ui::runtime::keys::Intent::Cancel));
        if close || modal.should_close() || intent_cancel {
            ui.ctx().data_mut(|d| {
                d.remove::<bool>(confirm_id);
            });
        }
    }
    let save_error = ui.ctx().data(|d| d.get_temp::<bool>(recent_save_error_id())).unwrap_or(false);
    if save_error {
        ui.label(
            egui::RichText::new(tl!("Recent documents changed, but the change could not be saved. It may return after restarting WordCraft."))
                .color(ui.visuals().error_fg_color),
        );
        ui.add_space(6.0);
    }
    let error =
        app.backstage_state.open_error.clone().unwrap_or_else(|| ui.ctx().data(|d| d.get_temp::<String>(open_error_id())).unwrap_or_default());
    if !error.is_empty() {
        ui.label(egui::RichText::new(&error).color(ui.visuals().error_fg_color));
        ui.add_space(6.0);
    }
    let shown = app.ui.recent.iter().take(app.ui.recent_shown()).cloned().collect::<Vec<_>>();
    if shown.is_empty() {
        ui.label(egui::RichText::new(tl!("Documents you open will show up here.")).color(t.text_dim));
        return;
    }
    let file_name = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string());
    let names = shown.iter().map(|r| file_name(&r.path)).collect::<Vec<_>>();
    recent_header(ui);
    for (entry, name) in shown.iter().zip(&names) {
        recent_row(app, ui, entry, name, &t);
    }
}

/// The Recent table's responsive column widths: a narrow icon column, a flexible name column,
/// and two fixed date columns. The same geometry is used by the header and every row.
pub(crate) fn recent_columns(width: f32) -> (f32, f32, f32, f32) {
    // Keep the logo near the leading edge while reserving a wider cell after it for visual breathing
    // room before the document name.
    let icon = 52.0;
    let gap = 12.0;
    let date = ((width - icon - gap * 2.0 - 120.0) / 2.0).clamp(72.0, 120.0);
    let name = (width - icon - date * 2.0 - gap * 2.0).max(0.0);
    (icon, name, date, gap)
}

/// A bounded column measured from the interface's reading edge.
pub(crate) fn recent_cell(row: Rect, rtl: bool, inset: f32, width: f32) -> Rect {
    let inset = inset.clamp(0.0, row.width().max(0.0));
    let width = width.clamp(0.0, (row.width() - inset).max(0.0));
    Rect::from_min_size(pos2(crate::rtl::lead_left(rtl, row, inset, width), row.min.y), vec2(width, row.height()))
}

/// Draw the column headings using real labels so their names are available to accessibility
/// clients as well as visually aligning with the rows below.
fn recent_header(ui: &mut Ui) {
    let width = ui.available_width().min(900.0);
    let (icon, name, date, gap) = recent_columns(width);
    let (r, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let rtl = crate::rtl::on();
    let cells = [
        (tl!("Name"), recent_cell(r, rtl, icon, name)),
        (tl!("Last opened"), recent_cell(r, rtl, icon + name + gap, date)),
        (tl!("Last edited"), recent_cell(r, rtl, icon + name + gap + date + gap, date)),
    ];
    for (index, (label, cell)) in cells.into_iter().enumerate() {
        let response = ui.interact(cell, ui.scope_id().with(("recent_header", index)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
        ui.painter().text(
            pos2(crate::rtl::lead_x(rtl, cell, 0.0), cell.center().y),
            crate::rtl::lead_center(rtl),
            label,
            semibold(12.0),
            Tokens::get(ui.ctx()).text,
        );
    }
}

/// Draw one Recent entry and its context menu. The entry is cloned by the caller so removing it
/// from the persisted list cannot invalidate the rows still being painted this frame.
fn recent_row(app: &mut WordApp, ui: &mut Ui, entry: &RecentDocument, name: &str, t: &Tokens) {
    let rtl = crate::rtl::on();
    let side = crate::rtl::lead_center(rtl);
    let path = entry.path.clone();
    let opened = crate::local_time::local_display_datetime(&entry.last_opened).unwrap_or_else(|| tl!("Unknown").to_string());
    let edited = crate::local_time::local_display_datetime(&entry.modified).unwrap_or_else(|| tl!("Unknown").to_string());
    let opened_label = format!("{} {opened}", tl!("Last opened:"));
    let edited_label = format!("{} {edited}", tl!("Last edited:"));
    let width = ui.available_width().min(900.0);
    let (icon, name_width, date, gap) = recent_columns(width);
    let (r, response) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
    let resp = response.on_hover_text(crate::i18n::isolate_path(&path));
    // Include everything painted in the row so screen readers can distinguish locations and hear
    // the modification date even when the filename is unique.
    let spoken = format!("{name}, {path}, {opened_label}, {edited_label}");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &spoken));
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    if let Some(texture) = wordcraft_logo(ui.ctx()) {
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(pos2(crate::rtl::lead_x(rtl, r, 18.0), r.center().y), vec2(24.0, 24.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let name_rect = recent_cell(r, rtl, icon, name_width);
    let name_x = crate::rtl::lead_x(rtl, name_rect, 0.0);
    let name_y = r.center().y - 9.0;
    let path_y = r.center().y + 9.0;
    let name_painter = ui.painter().with_clip_rect(name_rect);
    let name_shown = fit_text(&name_painter, &crate::i18n::isolate(name), &medium(13.0), 0.0, t.text, name_width);
    let location_shown = fit_text(&name_painter, &crate::i18n::isolate_path(&path), &regular(11.0), 0.0, t.text_dim, name_width);
    name_painter.text(pos2(name_x, name_y), side, name_shown, medium(13.0), t.text);
    name_painter.text(pos2(name_x, path_y), side, location_shown, regular(11.0), t.text_dim);
    let opened_rect = recent_cell(r, rtl, icon + name_width + gap, date);
    let edited_rect = recent_cell(r, rtl, icon + name_width + gap + date + gap, date);
    let opened_painter = ui.painter().with_clip_rect(opened_rect);
    let edited_painter = ui.painter().with_clip_rect(edited_rect);
    let opened_shown = fit_text(&opened_painter, &opened, &regular(11.0), 0.0, t.text_dim, date);
    let edited_shown = fit_text(&edited_painter, &edited, &regular(11.0), 0.0, t.text_dim, date);
    opened_painter.text(pos2(crate::rtl::lead_x(rtl, opened_rect, 0.0), r.center().y), side, opened_shown, regular(11.0), t.text_dim);
    edited_painter.text(pos2(crate::rtl::lead_x(rtl, edited_rect, 0.0), r.center().y), side, edited_shown, regular(11.0), t.text_dim);
    let activated = resp.clicked() || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)));
    if activated {
        let failed = super::open_recent(app, &path).err();
        app.backstage_state.open_error = failed.clone();
        ui.ctx().data_mut(|d| d.insert_temp(open_error_id(), failed.unwrap_or_default()));
    }
    let mut remove = false;
    let from_keys = resp.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
    let popup = egui::Popup::context_menu(&resp).at_position(pos2(crate::rtl::lead_x(rtl, resp.rect, 0.0), resp.rect.bottom()));
    let popup = if from_keys { popup.open_memory(egui::SetOpenCommand::Bool(true)) } else { popup };
    popup.show(|ui| {
        if ui.button(tl!("Open")).clicked() {
            let failed = super::open_recent(app, &path).err();
            app.backstage_state.open_error = failed.clone();
            ui.ctx().data_mut(|d| d.insert_temp(open_error_id(), failed.unwrap_or_default()));
            ui.close();
        }
        if app.services.show_in_folder.is_some() && ui.button(tl!("Show in folder")).clicked() {
            if let Err(error) = app.run("ui.recentShowInFolder", json!({"path": path})) {
                log::warn!("couldn't show the Recent document in its folder: {error}");
                ui.ctx().data_mut(|d| d.insert_temp(open_error_id(), tl!("Couldn't show this file in its folder.").to_string()));
            }
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Remove from Recent")).clicked() {
            remove = true;
            ui.close();
        }
    });
    if remove {
        let result = app.run("ui.recentRemove", json!({"path": path}));
        set_recent_save_error(ui.ctx(), result.as_ref().ok().and_then(|v| v.get("saved")).and_then(serde_json::Value::as_bool) == Some(false));
    }
}

fn recent_save_error_id() -> egui::Id {
    egui::Id::unique("backstage_recent_save_error")
}

fn set_recent_save_error(ctx: &egui::Context, failed: bool) {
    ctx.data_mut(|data| {
        if failed {
            data.insert_temp(recent_save_error_id(), true);
        } else {
            data.remove::<bool>(recent_save_error_id());
        }
    });
}

/// Load the existing attributed WordCraft application icon once per egui context.
fn wordcraft_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique("backstage_wordcraft_logo");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let bytes = include_bytes!("../../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.wordcraft.png");
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
    let texture = ctx.load_texture("backstage_wordcraft_logo", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// Stable storage id for the Recent list's open error.
pub(crate) fn open_error_id() -> egui::Id {
    egui::Id::unique("backstage_open_error")
}

/// Drop the Recent list's open error once File is closed, so it doesn't come back next time.
#[allow(dead_code)]
pub(crate) fn forget_open_error(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<String>(open_error_id());
    });
}
