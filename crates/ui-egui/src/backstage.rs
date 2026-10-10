//! File tab (Backstage): Home/New, Open, Info, Save As, Print, Export, Options, About.

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{APP_COLOR, Tokens, medium, regular, semibold};
use crate::{WordApp, icons};

const PAGES: [(&str, &str); 9] = [
    ("home", "Home"),
    ("new", "New"),
    ("open", "Open"),
    ("info", "Info"),
    ("save", "Save"),
    ("saveAs", "Save As"),
    ("print", "Print"),
    ("export", "Export"),
    ("options", "Options"),
];

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::left("backstage_nav")
        .exact_size(200.0)
        .frame(egui::Frame::NONE.fill(APP_COLOR).inner_margin(egui::Margin { left: 0, right: 0, top: 12, bottom: 12 }))
        .show(ui, |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(200.0, 40.0), Sense::click());
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(r.min.x + 26.0, r.center().y), vec2(18.0, 18.0)),
                "chevronLeft",
                egui::Color32::WHITE,
                egui::Color32::WHITE,
            );
            if resp.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.ui.backstage = false;
                app.canvas.want_focus = true;
            }
            ui.add_space(6.0);
            for (id, label) in PAGES {
                let (r, resp) = ui.allocate_exact_size(vec2(200.0, 38.0), Sense::click());
                let active = app.ui.backstage_page == id;
                if active {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(46));
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_white_alpha(26));
                }
                ui.painter().text(
                    pos2(r.min.x + 22.0, r.center().y),
                    Align2::LEFT_CENTER,
                    tl!(label),
                    if active { semibold(14.0) } else { medium(14.0) },
                    egui::Color32::WHITE,
                );
                if resp.clicked() {
                    match id {
                        "save" => {
                            let _ = app.run("file.save", json!({}));
                        }
                        "saveAs" => app.save_as_dialog(),
                        "open" => {
                            app.ui.backstage_page = id.into();
                        }
                        _ => app.ui.backstage_page = id.into(),
                    }
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                for (label, id) in [("About", "about"), ("Discord community", "discord")] {
                    let (r, resp) = ui.allocate_exact_size(vec2(200.0, 32.0), Sense::click());
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
        egui::ScrollArea::vertical().show(ui, |ui| match app.ui.backstage_page.as_str() {
            "new" | "home" => new_page(app, ui),
            "open" => open_page(app, ui),
            "info" => info_page(app, ui),
            "print" | "export" => export_page(app, ui),
            "options" => options_page(app, ui),
            _ => new_page(app, ui),
        });
    });
}

fn heading(ui: &mut Ui, s: &str) {
    ui.label(egui::RichText::new(tl!(s)).font(semibold(26.0)));
    ui.add_space(16.0);
}

fn template_tile(ui: &mut Ui, app: &mut WordApp, label: &str, template: &str) {
    let t = Tokens::get(ui.ctx());
    ui.vertical(|ui| {
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
        let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(&key))).or_else(|| {
            let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
            let _ = s.run("file.new", &json!({"template": template}));
            let l = s.export_layout();
            let page = l.pages.first()?;
            let img = wordcraft_render::render_page(&s.doc, page, 150.0 / page.w * ppp, &Default::default());
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
        ui.label(egui::RichText::new(tl!(label)).font(regular(12.5)));
        if resp.clicked() {
            let _ = app.run("file.new", json!({"template": template}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    });
}

fn new_page(app: &mut WordApp, ui: &mut Ui) {
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
        template_tile(ui, app, "Blank document", "blank");
        template_tile(ui, app, "Studio handbook (sample)", "sample");
        template_tile(ui, app, "Letter", "letter");
        template_tile(ui, app, "Résumé", "resume");
        template_tile(ui, app, "Report", "report");
    });
    ui.add_space(28.0);
    open_list(app, ui);
}

fn open_list(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!("Recent")).font(semibold(16.0)));
    ui.add_space(6.0);
    if app.ui.recent.is_empty() {
        ui.label(egui::RichText::new(tl!("Documents you open will show up here.")).color(t.text_dim));
    }
    for p in app.ui.recent.clone() {
        let name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(700.0), 40.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(22.0, 22.0)), "printLayout", t.icon, t.accent);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 13.0), Align2::LEFT_CENTER, &name, medium(13.0), t.text);
        ui.painter().text(pos2(r.min.x + 40.0, r.min.y + 29.0), Align2::LEFT_CENTER, &p, regular(11.0), t.text_dim);
        if resp.clicked() {
            let _ = app.run("file.open", json!({"path": p}));
            app.ui.backstage = false;
            app.canvas.want_focus = true;
        }
    }
}

fn open_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Open");
    if ui.button(egui::RichText::new(tl!("📂  Browse…")).font(medium(14.0))).clicked() {
        let _ = app.run("ui.openFileDialog", json!({}));
    }
    ui.add_space(18.0);
    open_list(app, ui);
}

fn info_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Info");
    let info = app.session.run("file.info", &json!({})).unwrap_or_default();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(egui::RichText::new(tl!("Properties")).font(semibold(15.0)));
        let mut props = app.session.doc.core.clone();
        let mut changed = false;
        egui::Grid::new("props").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            for (l, v) in [("Title", &mut props.title), ("Subject", &mut props.subject), ("Author", &mut props.creator), ("Keywords", &mut props.keywords), ("Category", &mut props.category)] {
                ui.label(tl!(l));
                changed |= ui.text_edit_singleline(v).lost_focus();
                ui.end_row();
            }
        });
        if changed {
            let _ = app.run("file.properties", json!({"title": props.title, "subject": props.subject, "author": props.creator, "keywords": props.keywords, "category": props.category}));
        }
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
    let is_print = app.ui.backstage_page == "print";
    heading(ui, if is_print { "Print" } else { "Export" });
    if is_print && app.services.print.is_some() {
        ui.label(tl!("Send the document straight to the system print dialog, or save a copy in another format below."));
        ui.add_space(12.0);
        if ui.add(egui::Button::new(egui::RichText::new(tl!("Print")).font(medium(13.5))).min_size(vec2(320.0, 34.0))).clicked() {
            match wordcraft_engine::io::save_bytes("x.pdf", &app.session.doc) {
                Ok(bytes) => {
                    if let Some(print) = &app.services.print {
                        print(&bytes);
                    }
                    app.status(tl!("Opening print dialog…"));
                }
                Err(e) => app.status(format!("{}: {e}", tl!("Print failed"))),
            }
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        ui.label(tl!("Or save a copy in another format:"));
    } else {
        ui.label(tl!("Save a copy in another format. Printing goes through a PDF you can print from any viewer."));
    }
    ui.add_space(12.0);
    for (label, ext) in [
        ("PDF document (*.pdf)", "pdf"),
        ("Word document (*.docx)", "docx"),
        ("OpenDocument Text (*.odt)", "odt"),
        ("Rich Text Format (*.rtf)", "rtf"),
        ("Web page (*.html)", "html"),
        ("Markdown (*.md)", "md"),
        ("Plain text (*.txt)", "txt"),
        ("Page image (*.png)", "png"),
    ] {
        if ui.add(egui::Button::new(egui::RichText::new(tl!(label)).font(medium(13.5))).min_size(vec2(320.0, 34.0))).clicked() {
            let name = format!("{}.{ext}", app.title_stem());
            let picked = app.services.pick_save.as_ref().and_then(|f| f(&name));
            if let Some(path) = picked {
                let r = if ext == "png" { app.run("file.exportPng", json!({"path": path})) } else { app.run("file.saveAs", json!({"path": path})) };
                if r.is_ok() {
                    app.status(crate::i18n::fmt(tl!("Exported {path}"), &[("path", &path)]));
                }
            }
        }
        ui.add_space(4.0);
    }
}

fn options_page(app: &mut WordApp, ui: &mut Ui) {
    heading(ui, "Options");
    ui.label(egui::RichText::new(tl!("General")).font(semibold(15.0)));
    language_picker(app, ui);
    ui.horizontal(|ui| {
        ui.label(tl!("User name:"));
        let mut n = app.session.author.clone();
        if ui.text_edit_singleline(&mut n).changed() {
            app.session.author = n;
        }
    });
    let mut dark = app.ui.dark;
    if ui.checkbox(&mut dark, tl!("Dark mode")).changed() {
        let _ = app.run("ui.dark", json!({"value": dark}));
    }
    ui.checkbox(&mut app.autosave, tl!("AutoSave documents that have been saved"));
    ui.checkbox(&mut app.ui.show_discord, tl!("Show the community button in the title bar"));
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Display")).font(semibold(15.0)));
    let mut marks = app.session.view.marks;
    if ui.checkbox(&mut marks, tl!("Show all formatting marks")).changed() {
        let _ = app.run("view.marks", json!({"value": marks}));
    }
    let mut ruler = app.session.view.ruler;
    if ui.checkbox(&mut ruler, tl!("Show rulers")).changed() {
        let _ = app.run("view.ruler", json!({"value": ruler}));
    }
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Agents")).font(semibold(15.0)));
    ui.label(tl!("Every command is available to scripts and AI agents: run `wordcraft-cli mcp` for an MCP server, or start the app with `--control <port>` for the JSON control channel."));
}

/// File ▸ Options ▸ Interface language: follow the system (the default) or pick one (#8).
fn language_picker(app: &mut WordApp, ui: &mut Ui) {
    use crate::i18n::{AUTO, Lang};
    let system = crate::i18n::system_lang();
    let auto_label = crate::i18n::fmt(tl!("Automatic ({language})"), &[("language", system.name())]);
    let current = if app.ui.language == AUTO { auto_label.clone() } else { Lang::from_pref(&app.ui.language).name().to_string() };
    ui.horizontal(|ui| {
        ui.label(tl!("Interface language:"));
        egui::ComboBox::from_id_salt("interface_language").selected_text(current).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(app.ui.language == AUTO, auto_label.as_str()).clicked() {
                let _ = app.run("ui.language", json!({"value": AUTO}));
            }
            for lang in Lang::all() {
                if ui.selectable_label(app.ui.language == lang.code(), lang.name()).clicked() {
                    let _ = app.run("ui.language", json!({"value": lang.code()}));
                }
            }
        });
    });
    ui.label(egui::RichText::new(tl!("Automatic follows your system's language. Menus and commands change; your documents don't.")).small().weak());
}
