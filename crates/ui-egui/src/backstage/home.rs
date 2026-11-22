//! File › Home and File › Open: the greeting with templates and recent documents, and the Open
//! page's Browse buttons over the same recent list.

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::{HOME_TEMPLATES, TEMPLATES, heading, open_list};
use crate::WordApp;
use crate::theme::{Tokens, medium, regular};
use craft::ui::shell::{RecentFile, Template};

/// The recent documents as the framework lists them: the app's paths, each named by its file name,
/// with timestamps read from disk where available for age display and sorting (#2716).
#[allow(dead_code)]
pub(super) fn recent_files(app: &WordApp) -> Vec<RecentFile> {
    app.ui
        .recent
        .iter()
        .map(|entry| {
            let path = &entry.path;
            let p = std::path::Path::new(path);
            let name = p.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            let mut file = RecentFile::new(path.as_str(), name);
            if let Ok(metadata) = std::fs::metadata(p) {
                if let Ok(modified) = metadata.modified()
                    && let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH)
                {
                    file.opened = Some(d.as_secs() as i64);
                }
                if let Ok(created) = metadata.created()
                    && let Ok(d) = created.duration_since(std::time::UNIX_EPOCH)
                {
                    file.created = Some(d.as_secs() as i64);
                }
            }
            file
        })
        .collect()
}

/// A template card for each of `list` (display label, `file.new` id), named in the interface language.
pub(super) fn template_cards(list: &[(&str, &str)]) -> Vec<Template> {
    list.iter().map(|(name, id)| Template::new(*id, crate::i18n::tc("template", name).to_string(), "")).collect()
}

fn template_tile(ui: &mut Ui, app: &mut WordApp, label: &str, template: &str) {
    let t = Tokens::get(ui.ctx());
    crate::rtl::vertical(ui, |ui| {
        // The thumbnail and its caption are one button named by the caption, so a screen reader
        // announces "Blank document, button" rather than unnamed text (#3176); the caption opens
        // the template too, as in Word.
        let caption = crate::i18n::tc("template", label);
        let galley = ui.painter().layout_no_wrap(caption.to_string(), regular(12.5), ui.visuals().text_color());
        let (tile, resp) = ui.allocate_exact_size(vec2(galley.size().x.max(150.0), 194.0 + galley.size().y), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), caption));
        let flows = crate::rtl::flows(ui);
        let r_min_x = if flows { tile.max.x - 150.0 } else { tile.min.x };
        let r = Rect::from_min_size(pos2(r_min_x, tile.min.y), vec2(150.0, 194.0));
        ui.painter().rect(
            r,
            2.0,
            egui::Color32::WHITE,
            Stroke::new(if resp.hovered() { 2.0 } else { 1.0 }, if resp.hovered() { t.accent } else { t.border_strong }),
            egui::StrokeKind::Inside,
        );
        let ppp = ui.ctx().pixels_per_point();
        let key = format!("tpl:{template}:{ppp}");
        let region = app.session.region();
        let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::unique(&key))).or_else(|| {
            let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
            // The blank tile shows the region's paper (#838).
            s.system_region = region;
            let _ = s.run("file.new", &json!({"template": template}));
            let l = s.export_layout();
            let page = l.pages.first()?;
            let img = wordcraft_render::render_page(&s.doc, page, 150.0 / page.w * ppp, &crate::canvas::screen_render_options());
            let px = img.to_straight();
            let h = ui.ctx().load_texture(
                &key,
                egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &px),
                egui::TextureOptions::LINEAR,
            );
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(&key), h.clone()));
            Some(h)
        });
        if let Some(h) = tex {
            ui.painter().image(h.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let text_x = if flows { tile.max.x } else { tile.min.x };
        ui.painter().galley(pos2(text_x, r.max.y + 6.0), galley, ui.visuals().text_color());
        if resp.clicked() {
            let _ = app.run("file.new", json!({"template": template}));
            app.ui.backstage = false;
            app.canvas.focus_page(ui.ctx());
        }
    });
}

/// Home: a greeting, Blank plus a short row of templates with a link to New, then Recent.
pub(super) fn home(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let hour = wordcraft_engine::cmd::local_now().3.0;
    heading(
        ui,
        if hour < 12 {
            "Good morning"
        } else if hour < 18 {
            "Good afternoon"
        } else {
            "Good evening"
        },
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 0.0);
        for (label, template) in TEMPLATES.into_iter().take(HOME_TEMPLATES) {
            template_tile(ui, app, label, template);
        }
    });
    ui.add_space(8.0);
    let more = ui.add(egui::Label::new(egui::RichText::new(tl!("More templates →")).font(medium(13.5)).color(t.accent)).sense(Sense::click()));
    if more.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.ui.backstage_page = "new".into();
    }
    ui.add_space(24.0);
    open_list(app, ui);
}

/// Open: Browse for a file, then the recent documents.
pub(super) fn open(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Open");
    if ui.button(egui::RichText::new(tl!("Browse…")).font(medium(14.0))).clicked() {
        let _ = app.run("ui.openFileDialog", json!({}));
    }
    // Linux and the BSDs (#576): the system dialog needs a file chooser portal or zenity; when it
    // doesn't show up, WordCraft's own browser is always one click away.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    if ui
        .button(egui::RichText::new(tl!("Browse in WordCraft…")).font(medium(14.0)))
        .on_hover_text(tl!("Open a file with WordCraft's own browser when the system file dialog doesn't appear."))
        .clicked()
    {
        let _ = app.run("ui.fileBrowser", json!({}));
    }
    ui.add_space(18.0);
    open_list(app, ui);
}
