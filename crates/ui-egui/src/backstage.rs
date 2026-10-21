//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Close, Options, About.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{APP_COLOR, Tokens, medium, regular, semibold};
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

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let recovery = app.ui.backstage_page == "recover";
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
        .frame(egui::Frame::NONE.fill(if recovery { egui::Color32::BLACK } else { APP_COLOR }).inner_margin(egui::Margin {
            left: 0,
            right: 0,
            top: if mac { 0 } else { 12 },
            bottom: 12,
        }))
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
                let resp = ui.interact(Rect::from_center_size(c, vec2(22.0, band.height())), ui.id().with("backstage_back"), Sense::click());
                icons::paint(ui.painter(), Rect::from_center_size(c, vec2(18.0, 18.0)), "chevronLeft", egui::Color32::WHITE, egui::Color32::WHITE);
                resp
            } else {
                let (r, resp) = ui.allocate_exact_size(vec2(200.0, 40.0), Sense::click());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(pos2(r.min.x + 26.0, r.center().y), vec2(18.0, 18.0)),
                    "chevronLeft",
                    egui::Color32::WHITE,
                    egui::Color32::WHITE,
                );
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
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tl!(label)));
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
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!(label)));
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
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(if recovery { egui::Color32::BLACK } else { t.ribbon }).inner_margin(egui::Margin::symmetric(40, 30)))
        .show(ui, |ui| {
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
    ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
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
                // Buttons keep the theme's own text colour: the page-wide white override is
                // for labels on the black background and would bleach button labels (#1895).
                ui.visuals_mut().override_text_color = None;
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
        ui.visuals_mut().override_text_color = None;
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

fn template_tile(ui: &mut Ui, app: &mut WordApp, label: &str, template: &str) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
        // The label sits right under the thumbnail whatever row gap the gallery uses.
        ui.spacing_mut().item_spacing.y = 0.0;
        let (r, resp) = ui.allocate_exact_size(vec2(150.0, 194.0), Sense::click());
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
        let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(&key))).or_else(|| {
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
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(&key), h.clone()));
            Some(h)
        });
        if let Some(h) = tex {
            ui.painter().image(h.id(), r.shrink(1.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        // The caption opens the template too, as in Word.
        let caption = ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tc("template", label)).font(regular(12.5))).sense(Sense::click()));
        if resp.clicked() || caption.clicked() {
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

/// New: the whole template gallery with a search box; no Recent list (that is on Home and Open).
fn new_page(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    heading(ui, "New");
    let id = egui::Id::new("backstage_template_search");
    let mut query = ui.ctx().data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let search = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tl!("Search for templates")).desired_width(360.0));
    if search.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, query.clone()));
    }
    ui.add_space(18.0);
    let found = matching_templates(&query);
    if found.is_empty() {
        ui.label(egui::RichText::new(tl!("No templates match your search.")).color(t.text_dim));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 18.0);
        for (label, template) in found {
            template_tile(ui, app, label, template);
        }
    });
}

/// Show clickable recent documents using cached availability, without filesystem work while painting.
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
    let recent: Vec<_> = app.ui.recent.iter().take(app.ui.recent_shown()).cloned().collect();
    #[cfg(not(target_arch = "wasm32"))]
    app.recent_files.refresh(ui.ctx(), &recent, std::time::Instant::now());
    for p in recent {
        let name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let missing = app.recent_files.missing(&p);
        // Browser recent entries are names, not OS paths; there is no filesystem to probe.
        #[cfg(target_arch = "wasm32")]
        let missing = false;
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(700.0), 40.0), Sense::click());
        let status = if missing { tl!("File not found") } else { "" };
        let label = if missing { format!("{name}. {status}") } else { name.clone() };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        let fade = |color: egui::Color32| if missing { color.gamma_multiply(0.55) } else { color };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(22.0, 22.0)),
            "printLayout",
            fade(t.icon),
            fade(t.accent),
        );
        let status = ui.painter().layout_no_wrap(status.into(), regular(11.0), t.text_dim);
        let text_right = if missing { r.max.x - status.size().x - 12.0 } else { r.max.x };
        let text = ui.painter().with_clip_rect(r.intersect(Rect::from_min_max(r.min, pos2(text_right, r.max.y))));
        text.text(pos2(r.min.x + 40.0, r.min.y + 13.0), Align2::LEFT_CENTER, &name, medium(13.0), fade(t.text));
        text.text(pos2(r.min.x + 40.0, r.min.y + 29.0), Align2::LEFT_CENTER, &p, regular(11.0), fade(t.text_dim));
        if missing {
            ui.painter().galley(pos2(r.max.x - status.size().x, r.center().y - status.size().y / 2.0), status, t.text_dim);
        }
        let resp = resp.on_hover_text(if missing { format!("{p}\n{}", tl!("File not found")) } else { p.clone() });
        if resp.clicked() {
            let failed = open_recent(app, &p).err();
            ui.ctx().data_mut(|d| d.insert_temp(open_error_id(), failed.unwrap_or_default()));
        }
    }
}

fn open_error_id() -> egui::Id {
    egui::Id::new("backstage_open_error")
}

/// Drop the Recent list's open error once File is closed, so it doesn't come back next time.
pub(crate) fn forget_open_error(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<String>(open_error_id()));
}

/// Open a file from the Recent list. A file that can't be opened (deleted, moved, unreadable)
/// leaves File open so the list can say why, rather than closing it on a greyed-out window.
fn open_recent(app: &mut WordApp, path: &str) -> Result<(), String> {
    app.run("file.open", json!({"path": path}))?;
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
    if ui.button(egui::RichText::new(tl!("Browse…")).font(medium(14.0))).clicked() {
        app.save_as_dialog();
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

    use super::matching_templates;
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

    /// #3314: a missing row is named for assistive technology and remains actionable; clicking
    /// it keeps the existing open error instead of silently disabling or removing the entry.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn missing_recent_row_is_marked_and_still_opens_with_an_error() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default());
        let missing = std::env::temp_dir().join(format!("wordcraft-recent-row-{}-missing.docx", std::process::id()));
        let path = missing.to_string_lossy().into_owned();
        app.ui.recent = vec![path.clone()];
        app.ui.backstage = true;
        app.ui.backstage_page = "open".into();
        let mut h = recovery_harness(app);
        let name = format!("{}. File not found", missing.file_name().unwrap().to_string_lossy());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            h.run_steps(2);
            if h.query_by_label(&name).is_some() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        h.get_by_label(&name).click();
        h.run_steps(4);
        assert!(h.state().ui.backstage);
        assert_eq!(h.state().ui.recent, vec![path]);
        assert!(!h.state().ctx.as_ref().unwrap().data(|d| d.get_temp::<String>(super::open_error_id())).unwrap_or_default().is_empty());
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
    fn recovery_app() -> (WordApp, std::path::PathBuf) {
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
        app.session.enable_recovery(root.clone(), std::sync::Arc::new(|| {})).unwrap();
        (app, root)
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
    fn recovery_restore_uses_save_changes_and_resets_autosave_permission() {
        let (app, root) = recovery_app();
        let mut h = recovery_harness(app);
        settle_recovery(&mut h);
        h.state_mut().execute("text.insert", json!({"text": "Current unsaved work"})).unwrap();
        h.state_mut().autosave_path = Some(root.join("must-not-write.docx"));
        h.state_mut().run("file.recover", json!({})).unwrap();
        h.run_steps(4);
        assert_eq!(h.state().session.recovery_documents().len(), 1);
        h.get_by_label("Restore").click();
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })));
        h.get_by_label("Cancel").click();
        h.run_steps(4);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Current unsaved work");
        h.get_by_label("Restore").click();
        h.run_steps(4);
        h.get_by_label("Don't Save").click();
        settle_recovery(&mut h);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Recovered synthetic work");
        assert!(h.state().session.dirty && h.state().session.path.is_none());
        assert!(h.state().autosave_path.is_none());
        assert!(!h.state().ui.backstage);
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
        let arrow = egui::pos2(100.0, 32.0);
        h.event(egui::Event::PointerMoved(arrow));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: arrow, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(4);
        assert!(!h.state().ui.backstage, "the arrow closes the File page");
        assert!(h.state().canvas.focused, "and the page has the keyboard");
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
            let back = if mac { mac_chevron(&a) } else { egui::pos2(100.0, 32.0) };
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
