//! File › Print (#641): the print settings and a preview of the pages.
//!
//! Page range and parity shape the PDF. Native printer settings and copy collation go to
//! the shared spooler; browser printing and saved print PDFs retain copies in the PDF.

use std::sync::Arc;

use egui::{Rect, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_layout::DocLayout;

use crate::theme::{Tokens, medium, regular};
use crate::{WordApp, icons};

/// The Print page's settings. Not saved with the document or the UI state.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    /// `all`, `current`, `selection` or `custom`.
    pub range: String,
    /// The custom range text, like `1-3, 5`.
    pub pages: String,
    /// `all`, `odd` or `even`.
    pub parity: String,
    pub copies: u32,
    pub collate: bool,
    /// The page shown in the preview (0-based).
    pub preview: usize,
    /// `auto`, `portrait` or `landscape`.
    pub orientation: String,
    /// The preview's zoom (1.0 is the page's real size), or `None` to fit the whole page.
    pub zoom: Option<f32>,
    /// Scale to paper size (#2108): `none` prints the document's page sizes, a Layout › Size
    /// name scales every page onto that paper. The document itself is unchanged.
    pub paper: String,
}

impl Default for PrintSettings {
    fn default() -> Self {
        PrintSettings {
            range: "all".into(),
            pages: String::new(),
            parity: "all".into(),
            copies: 1,
            collate: true,
            preview: 0,
            zoom: None,
            orientation: "auto".into(),
            paper: NO_SCALING.into(),
        }
    }
}

impl PrintSettings {
    /// The params of `ui.print` and `file.printPdf` for these settings.
    pub fn params(&self) -> Value {
        let mut v = json!({
            "range": self.range,
            "parity": self.parity,
            "copies": self.copies,
            "collate": self.collate,
            "orientation": self.orientation,
        });
        if self.range == "custom" {
            v["pages"] = json!(self.pages);
        }
        if self.paper != NO_SCALING {
            v["paper"] = json!(self.paper);
        }
        v
    }

    /// The paper pages are scaled to, as (width, height) in points, or `None` for their own size.
    pub fn paper_size(&self) -> Option<(f32, f32)> {
        wordcraft_geom::paper_by_name(&self.paper).map(|(_, w, h)| (w, h))
    }
}

const RANGES: [(&str, &str); 4] = [("all", "All pages"), ("current", "Current page"), ("selection", "Selected pages"), ("custom", "Custom range")];
const PARITIES: [(&str, &str); 3] = [("all", "All pages in the range"), ("odd", "Odd pages only"), ("even", "Even pages only")];
/// The Scale to paper size choice that keeps each page's own size.
const NO_SCALING: &str = "none";
const ORIENTATIONS: [(&str, &str); 3] = [("auto", "Auto orientation"), ("portrait", "Portrait"), ("landscape", "Landscape")];

/// A choice list over `(id, label)` pairs; true when the choice changed.
fn choice(ui: &mut Ui, salt: &str, current: &mut String, items: &[(&str, &str)], label: &egui::Response) -> bool {
    let shown = items.iter().find(|(id, _)| *id == current.as_str()).map_or("", |(_, l)| *l);
    let mut changed = false;
    let combo = craft_ui::ComboBox::from_id_salt(salt).width(260.0).selected_text(tl!(shown)).show_ui(ui, |ui| {
        for (id, item) in items {
            if ui.selectable_label(current == id, tl!(item)).clicked() && current != id {
                *current = (*id).to_string();
                changed = true;
            }
        }
    });
    combo.response.labelled_by(label.id);
    changed
}

/// A choice list named without a visible label beside it (its heading names a group of choices).
fn choice_named(ui: &mut Ui, salt: &str, current: &mut String, items: &[(&str, &str)], name: &str) -> bool {
    let shown = items.iter().find(|(id, _)| *id == current.as_str()).map_or("", |(_, l)| *l);
    let mut changed = false;
    let combo = craft_ui::ComboBox::from_id_salt(salt).width(260.0).selected_text(tl!(shown)).show_ui(ui, |ui| {
        for (id, item) in items {
            if ui.selectable_label(current == id, tl!(item)).clicked() && current != id {
                *current = (*id).to_string();
                changed = true;
            }
        }
    });
    combo.response.accessible_name(name);
    changed
}

/// The output layout, kept while the document is unchanged (laying out every frame would be slow).
fn layout_of(app: &WordApp, ctx: &egui::Context) -> Arc<DocLayout> {
    let id = egui::Id::unique("print_page_layout");
    let rev = app.session.rev();
    if let Some((r, l)) = ctx.data(|d| d.get_temp::<(u64, Arc<DocLayout>)>(id))
        && r == rev
    {
        return l;
    }
    let l = app.session.export_layout();
    ctx.data_mut(|d| d.insert_temp(id, (rev, l.clone())));
    l
}

/// The texture of one page, re-rendered only when the document, page or size changes.
fn page_texture(app: &WordApp, ctx: &egui::Context, lay: &DocLayout, page: usize, width: f32) -> Option<TextureHandle> {
    let id = egui::Id::unique("print_page_texture");
    let key = (app.session.rev(), page, width as u32, (ctx.pixels_per_point() * 100.0) as u32);
    if let Some((k, t)) = ctx.data(|d| d.get_temp::<((u64, usize, u32, u32), TextureHandle)>(id))
        && k == key
    {
        return Some(t);
    }
    let pg = lay.pages.get(page)?;
    // Zoomed in, the texture stays within what the GPU takes and is stretched beyond that.
    let max_side = ctx.input(|i| i.max_texture_side) as f32 / pg.w.max(pg.h).max(1.0);
    let scale = (width / pg.w.max(1.0) * ctx.pixels_per_point()).clamp(0.05, 4.0).min(max_side);
    let img = wordcraft_render::render_page(&app.session.doc, pg, scale, &Default::default());
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight());
    let tex = ctx.load_texture("print_page_preview", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
    Some(tex)
}

/// Return pressed with nothing else to take it: no field or list open and no control focused (a
/// focused button takes Return itself). Consumes it when so.
fn return_prints(ctx: &egui::Context) -> bool {
    !ctx.text_edit_focused()
        && !egui::Popup::is_any_open(ctx)
        && ctx.memory(|m| m.focused().is_none())
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
}

/// The Print page: settings on the left, the preview on the right.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let lay = layout_of(app, &ctx);
    let total = lay.pages.len();
    let current = app.session.current_pages(&lay);
    app.ui.print.preview = app.ui.print.preview.min(total.saturating_sub(1));

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(380.0);
            // The settings scroll on their own: the preview beside them keeps the whole height (#1156).
            egui::ScrollArea::vertical().id_salt("print_settings").show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let native = app.services.print_native.is_some();
                let hook = native || app.services.print.is_some();
                // #2958: as in Word, the page's action leads the settings, in the accent the app
                // gives a dialog's primary button: the first thing seen, and plainly a button
                // that prints now rather than one more setting in the list below.
                let ready = !native || app.printing.ready();
                // Return prints too, as in Word, whose ⌘P opens on Print as the default button.
                if hook
                    && (ui.add_enabled(ready, crate::widgets::primary_button(ui, tl!("Print")).min_size(vec2(320.0, 34.0))).clicked()
                        || (ready && app.dialog.is_none() && return_prints(ui.ctx())))
                {
                    let params = app.ui.print.params();
                    let _ = app.run("ui.print", params);
                }
                if native {
                    if let Some(discover) = &app.services.printers {
                        crate::printing::show(&mut app.printing, ui, discover, |label| crate::i18n::t(label).to_owned());
                    }
                    app.ui.print.copies = app.printing.settings.copies;
                } else if hook {
                    ui.label(tl!("Send the document straight to the system print dialog, or save a copy in another format below."));
                    ui.label(
                        egui::RichText::new(tl!("The printer, two-sided printing and pages per sheet are chosen in that dialog.")).color(t.text_dim),
                    );
                } else {
                    ui.label(tl!("Save a copy in another format. Printing goes through a PDF you can print from any viewer."));
                }
                if ui
                    .add(egui::Button::new(egui::RichText::new(tl!("Save these pages as PDF…")).font(medium(13.5))).min_size(vec2(320.0, 34.0)))
                    .clicked()
                {
                    app.print_pdf_dialog();
                }
                ui.add_space(6.0);

                if native {
                    if app.ui.print.copies > 1 {
                        ui.add_enabled(!app.printing.busy(), egui::Checkbox::new(&mut app.ui.print.collate, tl!("Collate copies")));
                    }
                } else {
                    let copies_label = ui.label(egui::RichText::new(tl!("Copies")).font(medium(13.0)));
                    ui.horizontal(|ui| {
                        let mut n = app.ui.print.copies;
                        ui.add(egui::DragValue::new(&mut n).range(1..=wordcraft_engine::print::MAX_COPIES).speed(0.2)).labelled_by(copies_label.id);
                        app.ui.print.copies = n.clamp(1, wordcraft_engine::print::MAX_COPIES);
                        if app.ui.print.copies > 1 {
                            ui.checkbox(&mut app.ui.print.collate, tl!("Collate copies"));
                        }
                    });
                }

                let pages_label = ui.label(egui::RichText::new(tl!("Pages")).font(medium(13.0)));
                if choice(ui, "print_range", &mut app.ui.print.range, &RANGES, &pages_label) {
                    // Show the first page that will print.
                    app.ui.print.preview = if app.ui.print.range == "all" { 0 } else { current.0.min(total.saturating_sub(1)) };
                }
                if app.ui.print.range == "custom" {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.ui.print.pages)
                            .hint_text(tl!("Pages like 1-3, 5"))
                            .desired_width(260.0)
                            .char_limit(wordcraft_engine::print::MAX_SPEC_CHARS),
                    )
                    .labelled_by(pages_label.id);
                }
                choice_named(ui, "print_parity", &mut app.ui.print.parity, &PARITIES, tl!("Pages in the range"));
                let orientation_label = ui.label(egui::RichText::new(tl!("Orientation")).font(medium(13.0)));
                choice(ui, "print_orientation", &mut app.ui.print.orientation, &ORIENTATIONS, &orientation_label);
                // Word: Pages per sheet › Scale to paper size. Only the printout changes.
                let paper_label = ui.label(egui::RichText::new(tl!("Scale to paper size")).font(medium(13.0)));
                let papers: Vec<(&str, &str)> =
                    std::iter::once((NO_SCALING, "No Scaling")).chain(wordcraft_geom::PAPER_SIZES.iter().map(|(n, _, _)| (*n, *n))).collect();
                choice(ui, "print_paper", &mut app.ui.print.paper, &papers, &paper_label);

                match wordcraft_engine::print::plan(&app.ui.print.params(), total, current) {
                    Ok(plan) => {
                        let msg = crate::i18n::fmt(
                            tl!("{count} of {total} pages will print"),
                            &[("count", &plan.chosen.to_string()), ("total", &total.to_string())],
                        );
                        ui.label(egui::RichText::new(msg).color(t.text_dim));
                    }
                    Err(e) => {
                        ui.label(egui::RichText::new(e.to_string()).color(t.red));
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                if hook {
                    ui.label(tl!("Or save a copy in another format:"));
                }
                export_formats(app, ui);
            });
        });
        ui.add_space(24.0);
        ui.vertical(|ui| preview(app, ui, &lay, total));
    });
}

/// Room around the page inside the preview pane.
const PREVIEW_MARGIN: f32 = 10.0;
/// The preview's bottom bar: the page indicator on the left, the zoom on the right.
const PREVIEW_BAR: f32 = 34.0;

/// The next preview zoom one step (10%) in or out, from 10% to 500% as in the status bar.
fn zoom_step(z: f32, up: bool) -> f32 {
    let z = (z * 10.0).round() / 10.0;
    (if up { z + 0.1 } else { z - 0.1 }).clamp(0.1, 5.0)
}

/// The preview (#1156): the page fills the pane, a bar below pages through the document and
/// zooms; zoomed in, the page scrolls both ways.
fn preview(app: &mut WordApp, ui: &mut Ui, lay: &DocLayout, total: usize) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let page = app.ui.print.preview;
    let (page_w, page_h) = lay.pages.get(page).map_or((612.0, 792.0), |p| (p.w.max(1.0), p.h.max(1.0)));
    // Scale to paper size: the sheet is the chosen paper with the page scaled and centred on it.
    let sheet = app.ui.print.paper_size().map(|(w, h)| wordcraft_geom::fit_to_paper(page_w, page_h, w, h));
    let (pw, ph) = sheet.map_or((page_w, page_h), |f| (f.paper_w, f.paper_h));
    let pane = vec2(ui.available_width().max(120.0), (ui.available_height() - PREVIEW_BAR).max(120.0));
    // Screen points per page point.
    let fit = ((pane.x - 2.0 * PREVIEW_MARGIN) / pw).min((pane.y - 2.0 * PREVIEW_MARGIN) / ph).max(0.01);
    let scale = app.ui.print.zoom.map_or(fit, |z| z * crate::canvas::PX_PER_PT);
    let size = vec2(pw * scale, ph * scale);
    let label = crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &(page + 1).to_string()), ("pages", &total.to_string())]);
    egui::ScrollArea::both().id_salt("print_preview").auto_shrink(false).max_width(pane.x).max_height(pane.y).show(ui, |ui| {
        let room = vec2((size.x + 2.0 * PREVIEW_MARGIN).max(ui.available_width()), (size.y + 2.0 * PREVIEW_MARGIN).max(ui.available_height()));
        let (area, _) = ui.allocate_exact_size(room, Sense::hover());
        let r = Rect::from_center_size(area.center(), size);
        let resp = ui.interact(r, ui.scope_id().with("print_preview_page"), Sense::hover());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
        ui.painter().rect_filled(r.translate(vec2(2.0, 3.0)), 0.0, t.page_shadow);
        ui.painter().rect(r, 0.0, egui::Color32::WHITE, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let on_sheet = sheet.map_or(r, |f| Rect::from_min_size(r.min + vec2(f.dx, f.dy) * scale, vec2(page_w, page_h) * f.k * scale));
        if let Some(tex) = page_texture(app, &ctx, lay, page, on_sheet.width()) {
            ui.painter().image(tex.id(), on_sheet, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
    });
    ui.horizontal(|ui| {
        ui.set_height(PREVIEW_BAR);
        let nav = |ui: &mut Ui, icon: &str, tip: &str, enabled: bool| {
            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
            let col = if enabled { t.icon } else { t.text_disabled };
            if enabled && resp.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            icons::paint(ui.painter(), r.shrink(4.0), icon, col, col);
            enabled && resp.on_hover_text(tip).clicked()
        };
        if nav(ui, "chevronLeft", tl!("Previous Page"), page > 0) {
            app.ui.print.preview = page - 1;
        }
        ui.label(egui::RichText::new(&label).font(regular(13.0)));
        if nav(ui, "chevronRight", tl!("Next Page"), page + 1 < total) {
            app.ui.print.preview = page + 1;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let z = scale / crate::canvas::PX_PER_PT;
            if nav(ui, "onePage", tl!("One Page"), true) {
                app.ui.print.zoom = None;
            }
            ui.label(egui::RichText::new(format!("{}%", (z * 100.0).round())).font(regular(13.0)));
            if nav(ui, "plus", tl!("Zoom In"), z < 5.0) {
                app.ui.print.zoom = Some(zoom_step(z, true));
            }
            // A narrow window shortens the slider, then drops it, before it covers the page indicator.
            let room = ui.available_width() - 26.0 - 2.0 * ui.spacing().item_spacing.x - 12.0;
            if room >= 40.0 {
                let mut v = z;
                ui.spacing_mut().slider_width = room.min(110.0);
                let response = ui.add(egui::Slider::new(&mut v, 0.1..=5.0).logarithmic(true).show_value(false)).on_hover_text(tl!("Zoom"));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, ui.is_enabled(), tl!("Zoom")));
                if response.changed() {
                    app.ui.print.zoom = Some(v.clamp(0.1, 5.0));
                }
            }
            if nav(ui, "minus", tl!("Zoom Out"), z > 0.1) {
                app.ui.print.zoom = Some(zoom_step(z, false));
            }
        });
    });
}

/// The export "format" of a Word 2003 XML Document: an `.xml` file saved with `format: "wordml2003"`.
pub(crate) const EXPORT_WORD_2003: &str = "wordml2003";

/// The "save a copy" formats (File › Export, and below the settings on the Print page).
pub fn export_formats(app: &mut WordApp, ui: &mut Ui) {
    for (label, ext) in [
        ("PDF document (*.pdf)", "pdf"),
        ("Word document (*.docx)", "docx"),
        ("Word XML Document (*.xml)", "xml"),
        ("Word 2003 XML Document (*.xml)", EXPORT_WORD_2003),
        ("OpenDocument Text (*.odt)", "odt"),
        ("Rich Text Format (*.rtf)", "rtf"),
        ("Web page (*.html)", "html"),
        ("Markdown (*.md)", "md"),
        ("LaTeX (*.tex)", "tex"),
        ("Plain text (*.txt)", "txt"),
        ("Page image (*.png)", "png"),
    ] {
        if ui.add(egui::Button::new(egui::RichText::new(tl!(label)).font(medium(13.5))).min_size(vec2(320.0, 34.0))).clicked() {
            app.export_dialog(ext);
        }
        ui.add_space(4.0);
    }
}
