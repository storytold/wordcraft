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
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    use super::{HOME_TEMPLATES, TEMPLATES, matching_templates};
    use crate::{Services, WordApp};

    /// #1714: every side-menu item has its own icon, a drawn one (an unknown name would fall back
    /// to a tile with its first letter, the only icon here that draws text).
    #[test]
    fn every_side_menu_item_has_a_drawn_icon() {
        let ctx = egui::Context::default();
        let mut names = std::collections::HashSet::new();
        for (id, _, icon) in super::PAGES {
            assert!(names.insert(icon), "{id}: {icon} is used twice");
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(18.0, 18.0));
                crate::icons::paint(ui.painter(), r, icon, egui::Color32::WHITE, egui::Color32::WHITE);
            });
            // The frame's font textures aren't uploaded anywhere here (cleared before any assert,
            // so a failure doesn't panic again while the frame is dropped).
            out.textures_delta.clear();
            fn has_text(s: &egui::Shape) -> bool {
                match s {
                    egui::Shape::Text(_) => true,
                    egui::Shape::Vec(v) => v.iter().any(has_text),
                    _ => false,
                }
            }
            assert!(!out.shapes.is_empty(), "{id}: {icon} draws nothing");
            assert!(!out.shapes.iter().any(|s| has_text(&s.shape)), "{id}: {icon} is not a drawn icon");
        }
    }

    /// Regression test for #504: the side menu is a fixed 200 points wide, but egui drew a resize
    /// line and showed a resize pointer on its right edge that did nothing.
    #[test]
    fn the_side_menu_edge_is_not_a_resize_handle() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        app.ui.language = "en".into();
        app.ui.backstage = true;
        let ctx = egui::Context::default();
        let edge = egui::pos2(200.0, 400.0);
        for frame in 0..8 {
            let mut input = egui::RawInput {
                time: Some(f64::from(frame) * 0.1),
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, 900.0))),
                ..Default::default()
            };
            if frame > 0 {
                input.events.push(egui::Event::PointerMoved(edge));
            }
            let mut out = ctx.run_ui(input, |ui| {
                app.logic(&ctx.clone());
                app.ui(ui);
            });
            out.textures_delta.clear();
            if frame > 2 {
                assert!(app.ui.backstage, "File is open");
                let cursor = format!("{:?}", out.platform_output.cursor_icon);
                assert!(!cursor.starts_with("Resize"), "frame {frame}: the menu edge shows a resize pointer ({cursor})");
            }
        }
    }

    /// #2938: opening a deleted file from Recent keeps File open and reports the error instead of
    /// closing it on a greyed-out window.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn opening_a_missing_recent_file_keeps_file_open_with_the_error() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        app.ui.backstage = true;
        let missing = std::env::temp_dir().join(format!("wordcraft-missing-{}.docx", std::process::id()));
        let err = super::open_recent(&mut app, &missing.to_string_lossy()).unwrap_err();
        assert!(!err.is_empty());
        assert!(app.ui.backstage, "File stays open");
    }

    /// #1873: the macOS back chevron sits beside the traffic lights windowed, at the plain
    /// control inset in full screen.
    #[test]
    fn back_chevron_keeps_clear_of_traffic_lights_only_windowed() {
        let windowed = super::back_chevron_c(0.0, 14.0, false, 70.0);
        assert_eq!((windowed.x, windowed.y), (73.0, 14.0));
        let full = super::back_chevron_c(0.0, 14.0, true, 70.0);
        assert_eq!((full.x, full.y), (26.0, 14.0));
        assert!(full.x < windowed.x);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_fixture() -> (WordApp, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "wordcraft-recovery-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let id = "0123456789abcdef0123456789abcdef";
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("snapshot-00000000000000000001.json"),
            serde_json::to_vec(&json!({
                "version": 1, "generation": 1, "info": {"id": id, "name": "Synthetic draft", "savedAt": "2026-10-11T10:42:00Z"},
                "document": wordcraft_doc::Document::from_text("Recovered synthetic work")
            }))
            .unwrap(),
        )
        .unwrap();
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        app.ui.language = "en".into();
        (app, root)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_app() -> (WordApp, std::path::PathBuf) {
        let (mut app, root) = recovery_fixture();
        app.session.enable_recovery(root.clone(), std::sync::Arc::new(|| {})).unwrap();
        (app, root)
    }

    /// Hold discovery polling until the accessible control has dispatched its command.
    /// Warming up first keeps font installation independent of worker timing.
    #[cfg(not(target_arch = "wasm32"))]
    fn pending_recovery_harness(
        app: WordApp,
        root: &std::path::Path,
    ) -> (egui_kittest::Harness<'static, WordApp>, std::rc::Rc<std::cell::Cell<bool>>) {
        let poll = std::rc::Rc::new(std::cell::Cell::new(true));
        let gate = poll.clone();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                if gate.get() {
                    app.logic(&ui.ctx().clone());
                }
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        poll.set(false);
        h.state_mut().session.enable_recovery(root.to_path_buf(), std::sync::Arc::new(|| {})).unwrap();
        assert!(!h.state().session.recovery_ready());
        (h, poll)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn settle_recovery(h: &mut egui_kittest::Harness<'_, WordApp>) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            h.run_steps(2);
            if h.state().session.recovery_ready() && !h.state().session.recovery_busy() {
                break;
            }
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Worker completion is a state boundary; egui needs subsequent frames to finish
        // font installation/layout and expose the resulting buttons to AccessKit.
        h.run_steps(4);
    }

    /// #3243: the Recover page uses the app theme like every other Backstage page: no black
    /// fill, and its labels are not forced white (unreadable on the light theme).
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_page_follows_the_theme() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let (mut app, root) = recovery_app();
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx, crate::theme::Appearance::System);
            ctx.set_theme(theme);
            let mut shapes = Vec::new();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            for frame in 0..1000 {
                let input = egui::RawInput {
                    time: Some(f64::from(frame) * 0.01),
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, 900.0))),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    app.logic(&ctx.clone());
                    app.ui(ui);
                });
                out.textures_delta.clear();
                shapes = out.shapes;
                if frame > 4 && app.session.recovery_ready() && !app.session.recovery_busy() && !app.session.recovery_documents().is_empty() {
                    break;
                }
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(app.ui.backstage && app.ui.backstage_page == "recover");
            let mut texts = 0;
            for s in &shapes {
                match &s.shape {
                    egui::Shape::Rect(r) => {
                        assert_ne!(r.fill, egui::Color32::BLACK, "{theme:?}: the Recover page is filled black");
                    }
                    egui::Shape::Text(t) if t.galley.job.text == "Document" || t.galley.job.text == "Recovery copy" => {
                        texts += 1;
                        assert_ne!(t.override_text_color, Some(egui::Color32::WHITE), "{theme:?}: header forced white");
                    }
                    _ => {}
                }
            }
            assert_eq!(texts, 2, "{theme:?}: both column headers drawn");
            drop(app);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_does_not_replace_an_explicit_new_document() {
        for template in ["Blank document", "Report"] {
            let (mut app, root) = recovery_fixture();
            app.ui.backstage = true;
            app.ui.backstage_page = "home".into();
            let (mut h, poll) = pending_recovery_harness(app, &root);
            let previous = h.state().session.document_id();
            h.get_by_label("New").click();
            h.run_steps(4);
            h.get_by_label(template).click();
            h.run_steps(4);
            assert!(!h.state().session.recovery_ready());
            let document = h.state().session.document_id();
            let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
            assert_ne!(document, previous);
            poll.set(true);
            settle_recovery(&mut h);
            assert!(!h.state().ui.backstage, "the requested template must stay visible");
            assert_eq!(h.state().session.document_id(), document);
            assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), text);
            assert_eq!(h.state().session.recovery_documents().len(), 1, "older copies remain available");
            h.state_mut().ui.backstage = true;
            h.run_steps(4);
            h.get_by_label("Recover").click();
            h.run_steps(4);
            assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
            drop(h);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_does_not_replace_an_explicitly_opened_file() {
        let (mut app, root) = recovery_fixture();
        let path = root.join("Requested.txt");
        std::fs::write(&path, "Requested synthetic document").unwrap();
        app.services.pick_open = Some(Box::new(move |_| Some(path.to_string_lossy().into_owned())));
        app.ui.backstage = true;
        app.ui.backstage_page = "home".into();
        let (mut h, poll) = pending_recovery_harness(app, &root);
        let previous = h.state().session.document_id();
        h.get_by_label("Open").click();
        h.run_steps(4);
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        assert!(!h.state().session.recovery_ready());
        let document = h.state().session.document_id();
        assert_ne!(document, previous);
        poll.set(true);
        settle_recovery(&mut h);
        assert!(!h.state().ui.backstage);
        assert_eq!(h.state().session.document_id(), document);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Requested synthetic document");
        assert_eq!(h.state().session.recovery_documents().len(), 1);
        h.state_mut().ui.backstage = true;
        h.run_steps(4);
        h.get_by_label("Recover").click();
        h.run_steps(4);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
        drop(h);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_page_keeps_dismissed_copies_and_confirms_discard() {
        let (app, root) = recovery_app();
        let mut h = recovery_harness(app);
        settle_recovery(&mut h);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
        h.get_by_label("Continue without restoring").click();
        h.run_steps(4);
        assert!(!h.state().ui.backstage);
        assert_eq!(h.state().session.recovery_documents().len(), 1);
        h.state_mut().run("file.recover", json!({})).unwrap();
        h.run_steps(4);
        h.get_by_label("Discard…").click();
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::DiscardRecovery { .. })));
        h.get_by_label("Cancel").click();
        h.run_steps(4);
        assert_eq!(h.state().session.recovery_documents().len(), 1);
        h.get_by_label("Discard…").click();
        h.run_steps(4);
        h.get_by_label("Discard").click();
        settle_recovery(&mut h);
        assert!(h.state().session.recovery_documents().is_empty());
        assert!(h.query_by_label("No recovery copies.").is_some());
        drop(h);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn recovery_restore_opens_beside_unsaved_work_without_autosave_permission() {
        let (app, root) = recovery_app();
        let mut h = recovery_harness(app);
        settle_recovery(&mut h);
        h.state_mut().execute("text.insert", json!({"text": "Current unsaved work"})).unwrap();
        let work = h.state().session.document_id();
        h.state_mut().autosave_path = Some(root.join("must-not-write.docx"));
        h.state_mut().run("file.recover", json!({})).unwrap();
        h.run_steps(4);
        assert_eq!(h.state().session.recovery_documents().len(), 1);
        h.get_by_label("Restore").click();
        settle_recovery(&mut h);
        assert!(h.state().dialog.is_none(), "nothing is replaced, so nothing to ask");
        assert_ne!(h.state().session.document_id(), work);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Recovered synthetic work");
        assert!(h.state().session.dirty && h.state().session.path.is_none());
        assert!(h.state().autosave_path.is_none());
        assert!(!h.state().ui.backstage);
        let kept = h.state_mut().session.with_document(work, |s| s.doc.plain_text(wordcraft_doc::StoryRef::Body)).unwrap();
        assert_eq!(kept, "Current unsaved work");
        assert!(h.state().session.recovery_documents().is_empty(), "the restored document keeps the copy");
        assert!(!root.join("must-not-write.docx").exists());
        drop(h);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn template_search_matches_labels_case_insensitively() {
        let ids = |q: &str| matching_templates(q).into_iter().map(|(_, id)| id).collect::<Vec<_>>();
        assert_eq!(ids(""), ["blank", "sample", "letter", "resume", "report"]);
        assert_eq!(ids("  LETTER "), ["letter"]);
        assert_eq!(ids("re"), ["resume", "report"]);
        assert_eq!(ids("résumé"), ["resume"]);
        assert!(ids("no such template").is_empty());
    }

    fn personal_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wc-ui-personal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_template(path: &std::path::Path, text: &str) {
        let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": text})).unwrap();
        wordcraft_engine::io::save_path(path, &s.doc).unwrap();
    }

    fn new_page_harness(templates: Option<std::path::PathBuf>) -> egui_kittest::Harness<'static, WordApp> {
        let services = Services { templates_dir: templates, ..Default::default() };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("ui.backstage", json!({"value": true, "page": "new"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1100.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h
    }

    /// File › New lists the user templates folder's templates under Personal (#2852), by
    /// category; the search covers them; a click makes an untitled document attached to the
    /// template.
    #[test]
    fn new_lists_personal_templates_and_opens_one() {
        let dir = personal_dir("gallery");
        let memo = dir.join("Team memo.dotx");
        write_template(&memo, "Memo from the template");
        std::fs::create_dir_all(dir.join("Legal")).unwrap();
        write_template(&dir.join("Legal").join("Contract.dotx"), "Contract");
        let mut h = new_page_harness(Some(dir.clone()));
        assert!(h.query_by_label("Personal").is_some(), "New has a Personal section");
        assert!(h.query_by_label("Team memo").is_some(), "{:?}", h.state().user_templates_dir());
        assert!(h.query_by_label("Legal").is_some(), "a subfolder is a category");
        assert!(h.query_by_label("Report").is_some(), "the built-in templates stay");

        // The search covers both.
        let search = h.state().window_id("backstage_template_search");
        h.ctx.data_mut(|d| d.insert_temp(search, "memo".to_string()));
        h.run_steps(3);
        assert!(h.query_by_label("Team memo").is_some(), "search finds a personal template");
        assert!(h.query_by_label("Contract").is_none() && h.query_by_label("Report").is_none());
        h.ctx.data_mut(|d| d.insert_temp(search, "zzz".to_string()));
        h.run_steps(3);
        assert!(h.query_by_label("No templates match your search.").is_some());
        h.ctx.data_mut(|d| d.insert_temp(search, String::new()));
        h.run_steps(3);

        h.get_by_label("Team memo").click();
        h.run_steps(4);
        let a = h.state();
        assert!(!a.ui.backstage, "File closes");
        assert_eq!(a.session.path, None, "an untitled document");
        assert_eq!(a.session.doc.settings.attached_template.as_deref(), Some(memo.to_string_lossy().as_ref()));
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Memo from the template"));
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty (or not yet made) templates folder says where templates come from; no folder (the
    /// web) shows no Personal section. Home keeps only the built-ins.
    #[test]
    fn personal_section_empty_state_and_no_folder() {
        let dir = personal_dir("empty").join("Templates");
        let h = new_page_harness(Some(dir.clone()));
        assert!(h.query_by_label("Templates you save in your templates folder show up here.").is_some());
        assert!(!dir.exists(), "listing doesn't make the folder");
        drop(h);
        let h = new_page_harness(None);
        assert!(h.query_by_label("Personal").is_none());
    }

    /// A folder chosen in File › Options › Advanced › File locations replaces the app's own.
    #[test]
    fn the_options_folder_replaces_the_default() {
        let own = personal_dir("own");
        write_template(&own.join("Own.dotx"), "x");
        let chosen = personal_dir("chosen");
        write_template(&chosen.join("Chosen.dotx"), "y");
        let mut h = new_page_harness(Some(own.clone()));
        assert!(h.query_by_label("Own").is_some());
        h.state_mut().run("file.options", json!({"userTemplatesFolder": chosen.to_string_lossy()})).unwrap();
        h.run_steps(3);
        assert!(h.query_by_label("Chosen").is_some(), "the chosen folder is listed");
        assert!(h.query_by_label("Own").is_none());
        drop(h);
        let _ = std::fs::remove_dir_all(&own);
        let _ = std::fs::remove_dir_all(&chosen);
    }

    /// A personal template opens a new document like New does: nothing is asked, and the
    /// document with unsaved changes stays open beside it.
    #[test]
    fn a_personal_template_opens_beside_unsaved_work() {
        let dir = personal_dir("dirty");
        let memo = dir.join("Memo.dotx");
        write_template(&memo, "From the template");
        let mut h = new_page_harness(Some(dir.clone()));
        h.state_mut().run("document.setText", json!({"text": "Unsaved work"})).unwrap();
        let first = h.state().session.document_id();
        h.get_by_label("Memo").click();
        h.run_steps(3);
        let body = |h: &egui_kittest::Harness<'static, WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(h.state().dialog.is_none(), "nothing is asked");
        assert!(body(&h).contains("From the template"));
        assert_eq!(h.state().session.doc.settings.attached_template.as_deref(), Some(memo.to_string_lossy().as_ref()));
        h.state_mut().run("view.switchWindows", json!({"id": first})).unwrap();
        assert_eq!(body(&h), "Unsaved work", "the unsaved document is still open");
        assert!(h.state().session.dirty);
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The templates folder is read when New is shown and on Refresh, not on every frame.
    #[test]
    fn personal_templates_are_listed_once_per_visit_and_on_refresh() {
        let dir = personal_dir("refresh");
        write_template(&dir.join("First.dotx"), "x");
        let count = || super::PERSONAL_LISTINGS.with(std::cell::Cell::get);
        let before = count();
        let mut h = new_page_harness(Some(dir.clone()));
        let after_open = count();
        assert_eq!(after_open, before + 1, "listed once when New is shown");
        write_template(&dir.join("Second.dotx"), "y");
        h.run_steps(20);
        assert_eq!(count(), after_open, "not listed again while New stays open");
        assert!(h.query_by_label("Second").is_none());
        h.get_by_label("Refresh").click();
        h.run_steps(3);
        assert_eq!(count(), after_open + 1, "Refresh lists once");
        assert!(h.query_by_label("Second").is_some(), "Refresh shows the new template");
        // Leaving New and coming back reads the folder again.
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
        h.run_steps(2);
        write_template(&dir.join("Third.dotx"), "z");
        h.state_mut().run("ui.backstage", json!({"value": true, "page": "new"})).unwrap();
        h.run_steps(3);
        assert_eq!(count(), after_open + 2);
        assert!(h.query_by_label("Third").is_some());
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A saved templates folder that is a network path, or gone, gives way to the app's own.
    #[test]
    fn an_unusable_saved_folder_falls_back_to_the_apps_own() {
        let own = personal_dir("fallback");
        let services = Services { templates_dir: Some(own.clone()), ..Default::default() };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        for bad in [r"\\host\share".to_string(), "//host/share".into(), own.join("gone").to_string_lossy().into_owned(), "relative".into()] {
            a.session.prefs.user_templates_folder = Some(bad.clone());
            assert_eq!(a.user_templates_dir(), Some(own.clone()), "{bad}");
        }
        a.session.prefs.user_templates_folder = Some(own.to_string_lossy().into_owned());
        assert_eq!(a.user_templates_dir(), Some(own.clone()));
        let _ = std::fs::remove_dir_all(&own);
    }

    /// Home and New are different pages (#141): Home has the short template row, the link to New
    /// and Recent; New has every template and no Recent list.
    #[test]
    fn home_and_new_show_different_sections() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Recent").is_some(), "Home lists recent documents");
        assert!(h.query_by_label("Letter").is_some(), "Home shows the first templates");
        assert!(h.query_by_label("Report").is_none(), "Home shows only a short row of templates");
        h.get_by_label("More templates →").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "new", "the link opens New");
        assert!(h.query_by_label("Report").is_some(), "New shows every template");
        assert!(h.query_by_label("Recent").is_none(), "New has no Recent list");
    }

    /// #3176: a screen reader finds each template and each recent document as a button named by
    /// its caption or file name (they were unnamed text), and pressing one opens it.
    #[test]
    fn templates_and_recent_documents_are_named_buttons() {
        use egui::accesskit::Role;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        let dir = std::env::temp_dir().join("wordcraft-3176-missing");
        let path = |folder: &str, file: &str| dir.join(folder).join(file).to_string_lossy().into_owned();
        let (spring, autumn) = (path("spring", "Report.docx"), path("autumn", "Report.docx"));
        a.ui.recent = vec![path("notes", "Quarterly notes.docx"), spring.clone(), autumn.clone()];
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = a11y_harness(a);
        for (label, _) in TEMPLATES.into_iter().take(HOME_TEMPLATES) {
            assert!(h.query_by_role_and_label(Role::Button, label).is_some(), "{label} is a named button");
        }
        assert!(h.query_by_role_and_label(Role::Button, "Quarterly notes.docx").is_some(), "a recent document is a named button");
        for p in [&spring, &autumn] {
            let label = format!("Report.docx, {p}");
            assert!(h.query_by_role_and_label(Role::Button, &label).is_some(), "a repeated file name is told apart by its path: {label}");
        }
        h.get_by_role_and_label(Role::Button, "Letter").click();
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "the Letter button opens the template");
        assert!(!h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).is_empty(), "with the template's text");
    }

    /// Clicking the back arrow closes the File page and gives the keyboard back to the page,
    /// without naming as focused a widget the frame didn't draw: with accessibility on (a screen
    /// reader, or any UI Automation client on Windows) AccessKit ends the program over that, and
    /// 0.8.0 vanished after Blank document, File, then the back arrow. The test harness always runs
    /// AccessKit.
    #[test]
    fn back_arrow_returns_to_the_page_with_accessibility_on() {
        let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.state().canvas.widget_id.is_some(), "the page has been drawn");
        h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
        h.run_steps(4);
        // The arrow heads the File page's side bar; it has no label to find it by.
        let arrow = egui::pos2(26.0, 32.0);
        h.event(egui::Event::PointerMoved(arrow));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: arrow, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "the arrow closes the File page");
        assert!(h.state().canvas.focused, "and the page has the keyboard");
    }

    /// #3041: clicking the plain sidebar background next to the back arrow must not also close
    /// the File page — only the chevron itself does, same scoping as the macOS chevron (#2972).
    #[test]
    fn clicking_beside_the_back_arrow_does_not_close_the_file_page() {
        let a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        let mut h = a11y_harness(a);
        h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
        h.run_steps(4);
        // Well clear of the chevron (centred at x=26) but still inside the old 200px-wide row.
        click_at(&mut h, egui::pos2(100.0, 32.0));
        assert!(h.state().ui.backstage, "clicking beside the arrow leaves the File page open");
    }

    /// A harness over the whole app; kittest always runs AccessKit, so a frame that names as
    /// focused a widget it didn't draw panics with "Focused ID … is not in the node list".
    fn a11y_harness(a: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h
    }

    /// A primary click at `pos` (for chrome with no label to find it by).
    fn click_at(h: &mut egui_kittest::Harness<'static, WordApp>, pos: egui::Pos2) {
        h.event(egui::Event::PointerMoved(pos));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(4);
    }

    /// The macOS back chevron beside the traffic lights, in a window.
    fn mac_chevron(a: &WordApp) -> egui::Pos2 {
        super::back_chevron_c(0.0, a.mac_titlebar.height / 2.0, false, a.mac_titlebar.content_left)
    }

    /// #2972: on macOS the File page's back button is the chevron beside the traffic lights (the
    /// "sidebar collapse toggle" of the report; there is no other toggle there). Blank document,
    /// File, then the chevron ended 0.8.0 with AccessKit's "Focused ID … is not in the node
    /// list". The same spot clicked in the editor (the title bar) must not either.
    #[test]
    fn mac_back_chevron_returns_to_the_page_with_accessibility_on() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.integrated_titlebar = true;
        a.mac_titlebar = crate::chrome::MacTitleBar { height: 28.0, content_left: 70.0 };
        let chevron = mac_chevron(&a);
        let mut h = a11y_harness(a);
        assert!(h.state().canvas.widget_id.is_some(), "the page has been drawn");
        for _ in 0..2 {
            h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
            h.run_steps(4);
            assert!(h.state().ui.backstage);
            click_at(&mut h, chevron);
            assert!(!h.state().ui.backstage, "the chevron closes the File page");
            assert!(h.state().canvas.focused, "and the page has the keyboard");
            // The same spot in the editor's title bar.
            click_at(&mut h, chevron);
            assert!(!h.state().ui.backstage);
        }
    }

    /// #2931: File › New › Blank document with a document open returns to the editor without
    /// AccessKit losing the focused node; and so does Home's Blank document tile and Escape.
    #[test]
    fn blank_document_from_the_file_page_with_accessibility_on() {
        let work = |mac: bool| {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Work")), Services::default());
            a.integrated_titlebar = mac;
            a11y_harness(a)
        };
        for mac in [false, true] {
            for page in ["new", "home"] {
                // A fresh "Work" document each time, so each path must really make a blank one.
                let mut h = work(mac);
                h.state_mut().run("ui.backstage", json!({"value": true, "page": page})).unwrap();
                h.run_steps(4);
                h.get_by_label("Blank document").click();
                h.run_steps(4);
                assert!(!h.state().ui.backstage, "{page}: Blank document returns to the editor");
                assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "", "{page}: a new blank document");
                assert!(h.state().dialog.is_none(), "{page}: nothing asks first");
            }
            let mut h = work(mac);
            h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
            h.run_steps(4);
            h.key_press(egui::Key::Escape);
            h.run_steps(4);
            assert!(!h.state().ui.backstage, "Escape closes the File page");
            assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Work", "Escape keeps the document");
        }
    }

    /// #2908: File › Options, then the back arrow (Windows/Linux layout and the macOS chevron),
    /// with a document open and from the Start screen, without AccessKit ending the program.
    #[test]
    fn options_back_arrow_with_accessibility_on() {
        for (mac, start) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
            a.integrated_titlebar = mac;
            if start {
                a.ui.start_screen = true;
                assert!(a.start_screen());
            }
            let back = if mac { mac_chevron(&a) } else { egui::pos2(26.0, 32.0) };
            let mut h = a11y_harness(a);
            if !start {
                h.state_mut().run("ui.backstage", json!({"value": true})).unwrap();
                h.run_steps(4);
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Options").click();
            h.run_steps(4);
            assert_eq!(h.state().ui.backstage_page, "options", "mac {mac} start {start}");
            // Into a pane, so the Options page has a focused widget of its own.
            h.get_by_label("Display").click();
            h.run_steps(4);
            click_at(&mut h, back);
            assert!(!h.state().ui.backstage, "mac {mac} start {start}: the arrow closes the File page");
            if start {
                assert!(!h.state().has_document(), "mac {mac}: from the Start screen, an empty window");
            } else {
                assert!(h.state().canvas.focused, "mac {mac}: and the page has the keyboard");
            }
        }
    }

    /// #260: File › Info's Title and Author fields dropped every keystroke (they edited a copy
    /// re-read each frame and only wrote it back when the field lost focus), so a save had no
    /// title or author. Typing now lands in the document as it happens, one undo step per field,
    /// and the browser's Save downloads it with the save stamps advanced (#262).
    #[test]
    fn info_fields_keep_typed_properties_through_a_web_save() {
        use egui::accesskit::Role;
        let got: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>> = Default::default();
        let sink = got.clone();
        let services = Services {
            download: Some(Box::new(move |n: &str, b: &[u8]| {
                sink.borrow_mut().push((n.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("XYZ")), services);
        a.session.author = "GAMMA".into();
        a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        for (field, text) in [("Title", "ALPHA"), ("Author", "BETA")] {
            h.get_by_role_and_label(Role::TextInput, field).click();
            h.run_steps(2);
            for c in text.chars() {
                h.get_by_role_and_label(Role::TextInput, field).type_text(&c.to_string());
                h.run_steps(2);
            }
            assert_eq!(h.get_by_role_and_label(Role::TextInput, field).value().as_deref(), Some(text), "{field} shows what was typed");
        }
        let core = h.state().session.doc.core.clone();
        assert_eq!((core.title.as_str(), core.creator.as_str()), ("ALPHA", "BETA"));
        assert!(h.state().session.dirty, "the property edits are unsaved changes");

        // Save (web: a download) writes them into docProps/core.xml, with fresh save stamps.
        let a = h.state_mut();
        let created = a.session.doc.core.created.clone();
        a.run("file.save", json!({})).unwrap();
        a.run("file.save", json!({})).unwrap();
        let (name, bytes) = got.borrow_mut().pop().unwrap();
        let back = wordcraft_engine::io::open_bytes(&name, &bytes).unwrap();
        assert_eq!((back.core.title.as_str(), back.core.creator.as_str()), ("ALPHA", "BETA"));
        assert_eq!(back.core.last_modified_by, "GAMMA");
        assert_eq!(back.core.revision, core.revision + 2, "each save advances the revision");
        assert!(created.is_empty() && !back.core.created.is_empty(), "the first save sets the creation time");
        assert!(!back.core.modified.is_empty());
        assert!(!a.session.dirty);

        // Each field's typing is one undo step.
        a.session.run("edit.undo", &json!({})).unwrap();
        assert_eq!((a.session.doc.core.title.as_str(), a.session.doc.core.creator.as_str()), ("ALPHA", ""));
        a.session.run("edit.undo", &json!({})).unwrap();
        assert_eq!(a.session.doc.core.title, "");
    }

    #[test]
    fn save_as_page_shows_browse_and_no_open_list() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.query_by_label("Browse…").is_some(), "Save As has Browse button");
        assert!(h.query_by_label("Recent").is_none(), "Save As has no list that opens documents");
    }

    #[test]
    fn backstage_save_navigates_to_save_as_when_unnamed() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs", "unnamed Save opens Save As page");
        assert!(h.state().ui.backstage, "backstage remains open");
    }

    /// Save on a document without a file shows the Save As page only: no native dialog as well.
    #[test]
    fn backstage_save_when_unnamed_opens_no_file_dialog() {
        let shown = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |_| {
                count.set(count.get() + 1);
                std::sync::mpsc::channel::<crate::FileDialogAnswer>().1
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs");
        assert_eq!(shown.get(), 0, "no native Save As dialog");
        assert!(!h.state().file_dialog_open());
    }

    /// Repro: a brand-new document, typed text, File > Save (unnamed) > Save As page > Browse…,
    /// answered asynchronously by a native dialog (as the desktop host does), should land back on
    /// the document, not the "no document" screen.
    #[test]
    fn backstage_save_browse_native_dialog_lands_on_the_document() {
        use std::sync::mpsc::channel;
        let shown: std::rc::Rc<std::cell::RefCell<Vec<(crate::FileDialogRequest, std::sync::mpsc::Sender<crate::FileDialogAnswer>)>>> =
            Default::default();
        let log = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |req| {
                let (tx, rx) = channel();
                log.borrow_mut().push((req, tx));
                rx
            })),
            ..Default::default()
        };
        let dir = std::env::temp_dir().join(format!("wordcraft-backstage-save-repro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Document1.docx");

        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "Hello from the repro"})).unwrap();
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs", "unnamed Save opens Save As page");
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        assert!(h.state().file_dialog_open(), "Browse opened the native dialog");
        let (_, tx) = shown.borrow_mut().pop().unwrap();
        tx.send(Ok(vec![path.to_string_lossy().to_string()])).unwrap();
        h.run_steps(6);
        assert!(h.state().session.path.as_deref() == Some(path.as_path()), "the document was saved to the picked path");
        assert!(!h.state().ui.backstage, "backstage closes once saved");
        assert!(h.state().has_document(), "the document stays open after Save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Save As page's own render loop skips the status bar while Backstage is open, so a
    /// write failure (no permission, disk full, a bad path) needs its own place to say why, or
    /// the page just sits there with a Browse… button and no clue.
    #[test]
    fn backstage_save_browse_write_failure_shows_on_the_save_as_page() {
        use std::sync::mpsc::channel;
        let shown: std::rc::Rc<std::cell::RefCell<Vec<(crate::FileDialogRequest, std::sync::mpsc::Sender<crate::FileDialogAnswer>)>>> =
            Default::default();
        let log = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |req| {
                let (tx, rx) = channel();
                log.borrow_mut().push((req, tx));
                rx
            })),
            ..Default::default()
        };
        let dir = std::env::temp_dir().join(format!("wordcraft-backstage-save-error-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // `dir` is deliberately never created, so the write fails: its parent doesn't exist.
        let bogus = dir.join("missing").join("out.docx");

        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "unsaved text"})).unwrap();
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        h.get_by_label("Browse…").click();
        h.run_steps(4);
        let (_, tx) = shown.borrow_mut().pop().unwrap();
        tx.send(Ok(vec![bogus.to_string_lossy().to_string()])).unwrap();
        h.run_steps(6);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "saveAs", "nothing was actually saved");
        let msg = h.state().status_msg.as_ref().map(|(m, _)| m.clone()).expect("the failure set a status message");
        assert!(!msg.is_empty());
        // The same text is rendered on the Save As page itself, not just the (invisible, while
        // Backstage is open) status bar.
        h.get_by_label(msg.as_str());
    }

    #[test]
    fn backstage_save_closes_when_saved() {
        let got: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>> = Default::default();
        let sink = got.clone();
        let services = Services {
            download: Some(Box::new(move |n: &str, b: &[u8]| {
                sink.borrow_mut().push((n.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save").click();
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "successful save closes backstage");
    }

    /// #1124: the side menu had no Close. It sits after Export and closes the document, asking
    /// first when there are unsaved changes; Cancel leaves the Backstage as it was.
    #[test]
    fn backstage_close_asks_then_closes() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": "Draft"})).unwrap();
        a.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        let export = h.get_by_label("Export").rect().min.y;
        assert!(h.get_by_label("Close").rect().min.y > export, "Close comes after Export");
        h.get_by_label("Close").click();
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "unsaved changes ask first");
        assert!(h.state().has_document());

        h.state_mut().run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        h.run_steps(4);
        assert!(h.state().has_document(), "Cancel keeps the document open");
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "info", "Cancel leaves the Backstage as it was");

        h.get_by_label("Close").click();
        h.run_steps(4);
        h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(!h.state().has_document() && !h.state().quit_requested, "Don't Save closes the document, not the window (#1712)");
    }

    /// A document without unsaved changes closes without a prompt.
    #[test]
    fn backstage_close_on_a_clean_document_closes_at_once() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Close").click();
        h.run_steps(1);
        assert!(h.state().dialog.is_none(), "no prompt");
        assert!(!h.state().has_document() && !h.state().quit_requested);
    }

    #[test]
    fn backstage_save_as_sidebar_opens_save_as_page() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("ui.backstage", json!({"value": true, "page": "home"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        h.get_by_label("Save As").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.backstage_page, "saveAs");
    }
}
