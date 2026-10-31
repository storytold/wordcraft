//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Close, Options, About.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

/// The side menu: page id, label and icon (#1714).
const PAGES: [(&str, &str, &str); 11] = [
    ("home", "Home", "homeFolder"),
    ("new", "New", "blankPage"),
    ("open", "Open", "open"),
    ("info", "Info", "info"),
    ("recover", "Recover", "recover"),
    ("save", "Save", "save"),
    ("saveAs", "Save As", "saveAs"),
    ("print", "Print", "print"),
    ("export", "Export", "export"),
    ("close", "Close", "close"),
    ("options", "Options", "options"),
];

/// The Backstage back chevron's centre on macOS (#1873): beside the traffic lights in a
/// window (the title bar's content edge), at the plain control inset in full screen, where the
/// lights aren't docked top-left and the reserved offset would leave dead space.
fn back_chevron_c(band_min_x: f32, center_y: f32, fullscreen: bool, content_left: f32) -> egui::Pos2 {
    let x = if fullscreen { 26.0 } else { content_left + 3.0 };
    pos2(band_min_x + x, center_y)
}

/// Draws the Backstage side menu and the active page. Without a document only the Home, New, Open
/// and Options pages work, and it handles Back, Escape, Save, Save As and Close routing.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // No document open (#1129): only the pages that open or create one, and Options.
    let has_document = app.has_document();
    if !has_document && !works_without_document(&app.ui.backstage_page) {
        app.ui.backstage_page = "home".into();
    }
    let mac = app.integrated_titlebar;
    egui::Panel::left("backstage_nav")
        .exact_size(200.0)
        // Fixed width, as in Word: without this egui shows a resize handle that does nothing (#502).
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.nav).inner_margin(egui::Margin { left: 0, right: 0, top: if mac { 0 } else { 12 }, bottom: 12 }))
        .show(ui, |ui| {
            let back = if mac {
                // macOS (#222): the Backstage covers the title bar, so its top band is the title
                // bar: the back button sits beside the traffic lights, centred with them and as far
                // from them as the title bar's own controls, and the rest of the band drags the window.
                let (band, drag) = ui.allocate_exact_size(vec2(200.0, app.mac_titlebar.height), Sense::click_and_drag());
                if drag.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                // The chevron's stroke starts at the content edge.
                let c = back_chevron_c(band.min.x, band.center().y, app.session.view.full_screen, app.mac_titlebar.content_left);
                let resp = ui.interact(Rect::from_center_size(c, vec2(22.0, band.height())), ui.scope_id().with("backstage_back"), Sense::click());
                icons::paint(ui.painter(), Rect::from_center_size(c, vec2(18.0, 18.0)), "chevronLeft", egui::Color32::WHITE, egui::Color32::WHITE);
                resp
            } else {
                // The hit area is scoped to the chevron, not the whole 200px-wide row (#3041):
                // the row reads as plain sidebar background next to it, so clicking there must
                // not also close the File page, same as the macOS chevron fix (#2972) below.
                let (r, _) = ui.allocate_exact_size(vec2(200.0, 40.0), Sense::hover());
                let c = pos2(r.min.x + 26.0, r.center().y);
                let resp = ui.interact(Rect::from_center_size(c, vec2(22.0, r.height())), ui.scope_id().with("backstage_back"), Sense::click());
                icons::paint(ui.painter(), Rect::from_center_size(c, vec2(18.0, 18.0)), "chevronLeft", egui::Color32::WHITE, egui::Color32::WHITE);
                resp
            };
            // Escape in the built-in file browser closes only the browser.
            if back.clicked() || (!app.file_dialog_open() && ui.input(|i| i.key_pressed(egui::Key::Escape))) {
                app.ui.backstage = false;
                app.canvas.want_focus = true;
            }
            ui.add_space(if mac { 12.0 } else { 6.0 });
            for (id, label, icon) in PAGES {
                // No document open (#1129): Info, Save, Save As, Print and Export are greyed out.
                let on = has_document || works_without_document(id);
                let (r, resp) = ui.add_enabled_ui(on, |ui| ui.allocate_exact_size(vec2(200.0, 38.0), Sense::click())).inner;
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tl!(label)));
                let active = app.ui.backstage_page == id;
                if active {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(46));
                } else if on && resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(26));
                }
                let ink = if on { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(110) };
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 31.0, r.center().y), vec2(18.0, 18.0)), icon, ink, ink);
                ui.painter().text(
                    pos2(r.min.x + 52.0, r.center().y),
                    Align2::LEFT_CENTER,
                    tl!(label),
                    if active { semibold(14.0) } else { medium(14.0) },
                    ink,
                );
                if resp.clicked() {
                    match id {
                        // A document without a file goes to the Save As page (as in Word) instead
                        // of `file.save`, which would also open the native Save As dialog.
                        // In the browser a save is a download, so it saves right away.
                        "save" if app.session.path.is_none() && app.services.download.is_none() => {
                            app.ui.backstage_page = "saveAs".into();
                        }
                        "save" => {
                            let r = app.run("file.save", json!({}));
                            if r.is_ok_and(|v| v.get("saved").and_then(serde_json::Value::as_bool) == Some(true) || v.get("downloaded").is_some()) {
                                app.ui.backstage = false;
                                app.canvas.want_focus = true;
                            } else {
                                app.ui.backstage_page = "saveAs".into();
                            }
                        }
                        "saveAs" => {
                            app.ui.backstage_page = "saveAs".into();
                        }
                        // Not a page: `file.close` asks Save / Don't Save / Cancel first when there
                        // are unsaved changes, and Cancel leaves the Backstage where it was.
                        "close" => {
                            let _ = app.run("file.close", json!({}));
                        }
                        _ => app.ui.backstage_page = id.into(),
                    }
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                for (label, id) in [("About", "about"), ("Discord community", "discord")] {
                    let (r, resp) = ui.allocate_exact_size(vec2(200.0, 32.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!(label)));
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(26));
                    }
                    ui.painter().text(pos2(r.min.x + 22.0, r.center().y), Align2::LEFT_CENTER, tl!(label), regular(13.0), egui::Color32::WHITE);
                    if resp.clicked() {
                        if id == "discord" {
                            let _ = app.run("ui.discord", json!({}));
                        } else {
                            app.dialog = crate::dialogs::Dialog::open("about", app);
                        }
                    }
                }
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(egui::Margin::symmetric(40, 30))).show(ui, |ui| {
        // The Print page fills the pane and scrolls its settings and its preview itself (#1156).
        if app.ui.backstage_page == "print" {
            export_page(app, ui);
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| match app.ui.backstage_page.as_str() {
            "home" => home_page(app, ui),
            "new" => new_page(app, ui),
            "open" => open_page(app, ui),
            "saveAs" => save_as_page(app, ui),
            "info" => info_page(app, ui),
            "recover" => recovery_page(app, ui),
            "print" | "export" => export_page(app, ui),
            "options" => crate::options::page(app, ui),
            _ => home_page(app, ui),
        });
    });
}

/// Recovery is a separate copy. Leaving this page never discards it.
fn recovery_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Recover documents");
    ui.label(tl!("Restore opens an unsaved copy. Your original file stays unchanged."));
    ui.add_space(20.0);
    if app.session.password.is_some() {
        ui.label(tl!("Recovery is disabled while this document has a password."));
    }
    if let Some(error) = app.session.recovery_error() {
        ui.label(error);
    }
    if !app.session.recovery_ready() {
        ui.label(tl!("Local recovery is unavailable or still loading."));
    }
    let documents = app.session.recovery_documents();
    if documents.is_empty() && app.session.recovery_ready() {
        ui.label(tl!("No recovery copies."));
    }
    let busy = app.session.recovery_busy();
    egui::Grid::new("recovery_documents").min_col_width(180.0).spacing(vec2(24.0, 16.0)).show(ui, |ui| {
        if !documents.is_empty() {
            ui.strong(tl!("Document"));
            ui.strong(tl!("Recovery copy"));
            ui.end_row();
        }
        for doc in documents {
            ui.label(&doc.name);
            ui.label(&doc.saved_at);
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(tl!("Restore")).clicked() {
                        let _ = app.run("file.recover", json!({"restore": doc.id}));
                    }
                    if ui.button(tl!("Discard…")).clicked() {
                        let _ = app.run("ui.recoveryDiscard", json!({"id": doc.id}));
                    }
                });
            });
            ui.end_row();
        }
    });
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("Continue without restoring")).clicked() {
            let _ = app.run("ui.backstage", json!({"value": false}));
            app.canvas.want_focus = true;
        }
        if ui.add_enabled(!busy && app.session.recovery_ready(), egui::Button::new(tl!("Refresh"))).clicked() {
            let _ = app.run("file.recover", json!({"refresh": true}));
        }
    });
}

/// The Backstage pages that work while no document is open (#1129).
fn works_without_document(page: &str) -> bool {
    matches!(page, "home" | "new" | "open" | "options")
}

fn heading(ui: &mut Ui, s: &str) {
    ui.label(egui::RichText::new(tl!(s)).font(semibold(26.0)));
    ui.add_space(16.0);
}

/// Draws a template thumbnail and caption as one accessible button, lazily renders and caches a
/// region-aware page preview, and creates a document from the template on click.
fn template_tile(ui: &mut Ui, app: &mut WordApp, label: &str, template: &str) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        // The thumbnail and its caption are one button named by the caption, so a screen reader
        // announces "Blank document, button" rather than unnamed text (#3176); the caption opens
        // the template too, as in Word.
        let caption = crate::i18n::tc("template", label);
        let galley = ui.painter().layout_no_wrap(caption.to_string(), regular(12.5), ui.visuals().text_color());
        let (tile, resp) = ui.allocate_exact_size(vec2(galley.size().x.max(150.0), 194.0 + galley.size().y), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), caption));
        let r = Rect::from_min_size(tile.min, vec2(150.0, 194.0));
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
            ui.painter().image(h.id(), r.shrink(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        ui.painter().galley(pos2(r.min.x, r.max.y), galley, ui.visuals().text_color());
        if resp.clicked() {
            let _ = app.run("file.new", json!({"template": template}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    });
}

/// The built-in templates in gallery order: display label and the `file.new` template id.
const TEMPLATES: [(&str, &str); 5] =
    [("Blank document", "blank"), ("Studio handbook (sample)", "sample"), ("Letter", "letter"), ("Résumé", "resume"), ("Report", "report")];

/// How many templates Home shows before "More templates" (the rest are on New).
const HOME_TEMPLATES: usize = 4;

/// The templates whose English or translated label, or id, contains `query` (case-insensitive;
/// the id lets "resume" find "Résumé"); all of them for an empty query.
fn matching_templates(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.trim().to_lowercase();
    TEMPLATES
        .into_iter()
        .filter(|(label, id)| {
            q.is_empty() || id.contains(&q) || label.to_lowercase().contains(&q) || crate::i18n::tc("template", label).to_lowercase().contains(&q)
        })
        .collect()
}

/// Home: a greeting, Blank plus a short row of templates with a link to New, then Recent.
fn home_page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let hour = wordcraft_engine::cmd::now_iso().get(11..13).and_then(|h| h.parse::<u32>().ok()).unwrap_or(9);
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

/// New: the whole template gallery with a search box, then the personal templates (#2852); no
/// Recent list (that is on Home and Open). The search covers both.
fn new_page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    heading(ui, "New");
    let id = app.window_id("backstage_template_search");
    let mut query = ui.ctx().data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let search = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tl!("Search for templates")).desired_width(360.0));
    if search.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, query.clone()));
    }
    ui.add_space(18.0);
    let found = matching_templates(&query);
    let (folder, personal) = personal_templates(app, ui);
    let personal_found = matching_personal(&query, &personal);
    if found.is_empty() && personal_found.is_empty() {
        ui.label(egui::RichText::new(tl!("No templates match your search.")).color(t.text_dim));
        return;
    }
    if !found.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
            for (label, template) in found {
                template_tile(ui, app, label, template);
            }
        });
    }
    // Personal: the user templates folder's templates; a folder per category, as in Word.
    let Some(folder) = folder else { return };
    if !query.trim().is_empty() && personal_found.is_empty() {
        return;
    }
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Personal")).font(semibold(16.0)));
        ui.add_space(8.0);
        if ui.small_button(tl!("Refresh")).on_hover_text(folder.to_string_lossy()).clicked() {
            refresh_personal_templates(ui.ctx());
        }
    });
    ui.add_space(8.0);
    let error = ui.ctx().data(|d| d.get_temp::<String>(personal_error_id())).unwrap_or_default();
    if !error.is_empty() {
        ui.label(egui::RichText::new(error).color(ui.visuals().error_fg_color));
        ui.add_space(6.0);
    }
    if personal.is_empty() {
        ui.label(egui::RichText::new(tl!("Templates you save in your templates folder show up here.")).color(t.text_dim));
        ui.label(egui::RichText::new(folder.to_string_lossy()).font(regular(11.5)).color(t.text_dim));
        return;
    }
    let mut categories: Vec<&str> = Vec::new();
    for p in &personal_found {
        if !categories.contains(&p.category.as_str()) {
            categories.push(&p.category);
        }
    }
    for category in categories {
        if !category.is_empty() {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(category).font(medium(13.5)).color(t.text));
            ui.add_space(6.0);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
            for p in personal_found.iter().filter(|p| p.category == category) {
                personal_tile(ui, app, p);
            }
        });
    }
}

/// A personal template: what the gallery needs of a [`PersonalTemplate`] listing entry.
type PersonalTemplate = wordcraft_engine::cmd::user_templates::PersonalTemplate;

/// The templates folder setting and listing File › New shows, and the pass it was last shown in.
#[derive(Clone)]
struct PersonalListing {
    key: (Option<String>, Option<std::path::PathBuf>),
    pass: u64,
    folder: Option<std::path::PathBuf>,
    templates: Vec<PersonalTemplate>,
}

/// Stable storage id for the personal-template listing cache.
fn personal_listing_id() -> egui::Id {
    egui::Id::unique("backstage_personal_templates")
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has listed the templates folder (tests check it isn't every frame).
    static PERSONAL_LISTINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The user templates folder and its templates (#2852). The folder is read when New is shown (so
/// a template saved meanwhile, or a folder chosen in Options, appears), when its setting changes
/// and on Refresh; never on every frame, as reading a big or synced folder would stall the window.
/// A folder that isn't there lists nothing.
fn personal_templates(app: &WordApp, ui: &Ui) -> (Option<std::path::PathBuf>, Vec<PersonalTemplate>) {
    let key = (app.session.prefs.user_templates_folder.clone(), app.services.templates_dir.clone());
    let pass = ui.ctx().cumulative_pass_nr();
    let cached = ui.ctx().data(|d| d.get_temp::<PersonalListing>(personal_listing_id()));
    // Still on New since the last pass: the listing stands.
    if let Some(mut l) = cached
        && l.key == key
        && l.pass.saturating_add(1) >= pass
    {
        l.pass = pass;
        let out = (l.folder.clone(), l.templates.clone());
        ui.ctx().data_mut(|d| d.insert_temp(personal_listing_id(), l));
        return out;
    }
    let folder = app.user_templates_dir();
    let templates = match &folder {
        Some(f) if f.is_dir() => {
            #[cfg(test)]
            PERSONAL_LISTINGS.with(|n| n.set(n.get().saturating_add(1)));
            wordcraft_engine::cmd::user_templates::list_templates(&f.to_string_lossy()).map(|l| l.templates).unwrap_or_else(|e| {
                log::debug!("personal templates: {e}");
                Vec::new()
            })
        }
        _ => Vec::new(),
    };
    let l = PersonalListing { key, pass, folder: folder.clone(), templates: templates.clone() };
    ui.ctx().data_mut(|d| d.insert_temp(personal_listing_id(), l));
    (folder, templates)
}

/// Read the templates folder again on the next pass (File › New › Personal's Refresh).
fn refresh_personal_templates(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<PersonalListing>(personal_listing_id()));
    ctx.request_repaint();
}

/// The personal templates whose name, title or category contains `query` (case-insensitive);
/// all of them for an empty query.
fn matching_personal(query: &str, list: &[PersonalTemplate]) -> Vec<PersonalTemplate> {
    let q = query.trim().to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    list.iter().filter(|p| q.is_empty() || has(&p.label) || has(&p.category) || p.title.as_deref().is_some_and(has)).cloned().collect()
}

/// A personal template's tile: a page with the template mark (no thumbnail) and its name; a
/// click makes a new document from it.
fn personal_tile(ui: &mut Ui, app: &mut WordApp, p: &PersonalTemplate) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_max_width(150.0);
        let (r, resp) = ui.allocate_exact_size(vec2(150.0, 194.0), Sense::click());
        ui.painter().rect(
            r,
            2.0,
            egui::Color32::WHITE,
            Stroke::new(if resp.hovered() { 2.0 } else { 1.0 }, if resp.hovered() { t.accent } else { t.border_strong }),
            egui::StrokeKind::Inside,
        );
        let mark = egui::Color32::from_gray(150);
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(44.0, 44.0)), "documentTemplate", mark, t.accent);
        let resp = resp.on_hover_text(&p.path);
        let caption = ui.add(egui::Label::new(egui::RichText::new(&p.label).font(regular(12.5))).truncate().sense(Sense::click()));
        if resp.clicked() || caption.clicked() {
            let failed = open_personal(app, &p.path).err();
            ui.ctx().data_mut(|d| d.insert_temp(personal_error_id(), failed.unwrap_or_default()));
        }
    });
}

/// Make a new document from the personal template at `path` and close File; a template that
/// can't be read keeps File open so the gallery can say why.
fn open_personal(app: &mut WordApp, path: &str) -> Result<(), String> {
    app.run("file.newFromTemplate", json!({"path": path}))?;
    app.ui.backstage = false;
    app.canvas.want_focus = true;
    Ok(())
}

/// Stable storage id for the personal-template listing error.
fn personal_error_id() -> egui::Id {
    egui::Id::unique("backstage_personal_error")
}

/// Draws the Recent documents list, distinguishes duplicate names by path for screen readers, and
/// stores an open failure in this page's temporary state.
fn open_list(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Recent")).font(semibold(16.0)));
    ui.add_space(6.0);
    let error = ui.ctx().data(|d| d.get_temp::<String>(open_error_id())).unwrap_or_default();
    if !error.is_empty() {
        ui.label(egui::RichText::new(error).color(ui.visuals().error_fg_color));
        ui.add_space(6.0);
    }
    if app.ui.recent.is_empty() {
        ui.label(egui::RichText::new(tl!("Documents you open will show up here.")).color(t.text_dim));
    }
    let shown = app.ui.recent.iter().take(app.ui.recent_shown()).cloned().collect::<Vec<_>>();
    let file_name = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.to_string());
    let names = shown.iter().map(|p| file_name(p)).collect::<Vec<_>>();
    for (p, name) in shown.iter().zip(&names) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(700.0), 40.0), Sense::click());
        // Two documents with the same file name are told apart by their path, as on screen.
        let spoken = if names.iter().filter(|n| *n == name).count() > 1 { format!("{name}, {p}") } else { name.clone() };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &spoken));
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(22.0, 22.0)), "printLayout", t.icon, t.accent);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 13.0), Align2::LEFT_CENTER, name, medium(13.0), t.text);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 29.0), Align2::LEFT_CENTER, p, regular(11.0), t.text_dim);
        if resp.clicked() {
            let failed = open_recent(app, p).err();
            ui.ctx().data_mut(|d| d.insert_temp(open_error_id(), failed.unwrap_or_default()));
        }
    }
}

/// Stable storage id for the Recent list's open error.
fn open_error_id() -> egui::Id {
    egui::Id::unique("backstage_open_error")
}

/// Drop the Recent list's open error once File is closed, so it doesn't come back next time.
pub(crate) fn forget_open_error(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<String>(open_error_id());
        d.remove::<String>(personal_error_id());
    });
}

/// The Save As page's write-failure message ([`WordApp::fail_save_as`]): this page's own
/// render loop skips the status bar, so a failed write (no permission, disk full, a bad path)
/// needs its own place to say why, or the page just sits there with a Browse… button and no clue.
pub(crate) fn save_as_error_id() -> egui::Id {
    egui::Id::unique("backstage_save_as_error")
}

/// Drop the Save As page's error once File is closed or a later save succeeds, so it doesn't
/// come back next time.
pub(crate) fn forget_save_as_error(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<String>(save_as_error_id()));
}

/// Open a file from the Recent list. A file that can't be opened (deleted, moved, unreadable)
/// leaves File open so the list can say why, rather than closing it on a greyed-out window.
pub(crate) fn open_recent(app: &mut WordApp, path: &str) -> Result<(), String> {
    app.open_file(path, None)?;
    app.ui.backstage = false;
    app.canvas.want_focus = true;
    Ok(())
}

fn open_page(app: &mut WordApp, ui: &mut Ui) {
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

/// Save As: Browse… asks where to save. It has no recent-documents list: clicking one of those
/// opens it, which is not what Save As is for.
fn save_as_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Save As");
    let error = ui.ctx().data(|d| d.get_temp::<String>(save_as_error_id())).unwrap_or_default();
    if !error.is_empty() {
        ui.label(egui::RichText::new(&error).color(ui.visuals().error_fg_color));
        ui.add_space(6.0);
    }
    if ui.button(egui::RichText::new(tl!("Browse…")).font(medium(14.0))).clicked() {
        app.save_as_dialog();
    }
    // A Word template in the user templates folder, which File › New lists under Personal (#2852).
    ui.add_space(8.0);
    if ui
        .button(egui::RichText::new(tl!("Save as Word Template…")).font(medium(14.0)))
        .on_hover_text(tl!("Saves a .dotx template in your templates folder. It shows up in File › New under Personal."))
        .clicked()
    {
        let _ = app.run("ui.saveAsTemplate", json!({}));
    }
}

fn info_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Info");
    let info = app.session.run("file.info", &json!({})).unwrap_or_default();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(egui::RichText::new(tl!("Properties")).font(semibold(15.0)));
        // The fields edit a copy of the document's properties that is read afresh every frame,
        // so each change goes into the document as it is typed (#260): waiting for the field to
        // lose focus dropped the keystrokes, and leaving File with Escape never loses it at all.
        // The keystrokes of one visit to a field are a single undo step.
        let mut props = app.session.doc.core.clone();
        let mut edited = None;
        let mut focused = None;
        egui::Grid::new("props").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            for (l, v) in [("Title", &mut props.title), ("Subject", &mut props.subject), ("Author", &mut props.creator), ("Keywords", &mut props.keywords), ("Category", &mut props.category)] {
                let label = ui.label(tl!(l));
                let r = ui.add(egui::TextEdit::singleline(v).id_salt(("info-prop", l))).labelled_by(label.id);
                if r.changed() {
                    edited = Some(l);
                }
                if r.has_focus() {
                    focused = Some(l);
                }
                ui.end_row();
            }
        });
        if let Some(field) = edited {
            if app.info_editing == Some((field, app.session.rev())) {
                app.session.join_next_undo();
            }
            let ok = app.run("file.properties", json!({"title": props.title, "subject": props.subject, "author": props.creator, "keywords": props.keywords, "category": props.category})).is_ok();
            app.info_editing = ok.then(|| (field, app.session.rev()));
        }
        if focused != app.info_editing.map(|(f, _)| f) {
            app.info_editing = None;
        }
        // Advanced Properties › Custom (#2982): view and edit the document's custom properties.
        ui.add_space(6.0);
        if ui
            .button(egui::RichText::new(tl!("Advanced Properties…")).font(medium(13.5)))
            .on_hover_text(tl!("View and edit the document's custom properties."))
            .clicked()
        {
            let _ = app.run("ui.dialog", json!({"name": "customProperties"}));
        }
        // Protect Document › Encrypt with Password (#55).
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tl!("Protect Document")).font(semibold(15.0)));
        let encrypted = info.get("encrypted").and_then(|v| v.as_bool()).unwrap_or(false);
        ui.label(if encrypted {
            tl!("A password is required to open this document.")
        } else {
            tl!("Anyone can open this document. Encrypt it with a password to keep it private.")
        });
        ui.horizontal(|ui| {
            let label = if encrypted { tl!("Change Password…") } else { tl!("Encrypt with Password…") };
            if ui.button(egui::RichText::new(label).font(medium(13.5))).clicked() {
                let _ = app.run("file.encrypt", json!({}));
            }
            if encrypted && ui.button(egui::RichText::new(tl!("Remove Password")).font(medium(13.5))).clicked() {
                let _ = app.run("file.encrypt", json!({"password": null}));
            }
        });
        let ui = &mut cols[1];
        ui.label(egui::RichText::new(tl!("Statistics")).font(semibold(15.0)));
        for (l, k) in [("Pages", "pages"), ("Words", "words"), ("Paragraphs", "paragraphs"), ("Sections", "sections"), ("Comments", "comments")] {
            ui.label(format!("{}: {}", tl!(l), info.get(k).map(|v| v.to_string()).unwrap_or_default()));
        }
        let path = info.get("path").and_then(|v| v.as_str()).unwrap_or(tl!("Not saved yet"));
        ui.label(crate::i18n::fmt(tl!("Location: {path}"), &[("path", path)]));
    });
}

fn export_page(app: &mut WordApp, ui: &mut Ui) {
    if app.ui.backstage_page == "print" {
        heading(ui, "Print");
        // Print Markup (a print setting, as in Word): the comments beside each page.
        let mut markup = app.session.prefs.print_markup;
        if ui
            .checkbox(&mut markup, tl!("Print Markup"))
            .on_hover_text(tl!(
                "Comments beside each page and tracked changes in PDFs, prints and images; a comment list at the end of web pages, Markdown and text."
            ))
            .changed()
        {
            let _ = app.run("file.printMarkup", json!({"value": markup}));
        }
        ui.add_space(12.0);
        crate::print_page::show(app, ui);
        return;
    }
    heading(ui, "Export");
    ui.label(tl!("Save a copy in another format. Printing goes through a PDF you can print from any viewer."));
    ui.add_space(12.0);
    crate::print_page::export_formats(app, ui);
}

#[cfg(test)]
mod tests;
