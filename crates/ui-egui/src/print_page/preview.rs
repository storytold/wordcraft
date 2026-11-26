//! The Print page's preview pane: the planned sheet over [`PreviewPane`], a bar below pages through the
//! sheets and zooms.

use std::sync::Arc;

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewZoom};
use craft::ui::primitives::textures::{Priority, Revision, TextureState, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use egui::{Color32, ColorImage, Ui};
use wordcraft_layout::DocLayout;

use super::{PrintPage, layout_key, preview_options};
use crate::WordApp;

/// The salt of the preview's thumbnail store.
const SALT: &str = "wordcraft.print_preview";

/// The store the sheets are drawn into. Its side limit is the GPU's, so a sheet never asks for more than a
/// texture can hold (#6284). A store keeps the options it was first made with, so the limit is part of the salt:
/// a window moved to a display with another limit gets the store of that limit.
pub(super) fn thumbnails(ctx: &egui::Context) -> Thumbnails<usize> {
    let limit = ctx.input(|i| i.max_texture_side).max(16) as u32;
    Thumbnails::in_context(ctx, (SALT, limit), ThumbnailOptions::new().max_side(limit))
}

/// What every sheet is drawn from: the output layout's inputs and the print settings that impose it.
pub(super) fn inputs(app: &WordApp) -> Revision {
    Revision::of(&(layout_key(app), app.print.settings.params().to_string()))
}

/// Draws the imposed sheets of the document on the UI thread, as production's preview did.
pub(super) struct SheetSource<'a> {
    app: &'a WordApp,
    lay: &'a Arc<DocLayout>,
    sheets: &'a [wordcraft_engine::print::Sheet],
    inputs: Revision,
}

impl<'a> SheetSource<'a> {
    /// A source over `sheets` of `lay`.
    pub(super) fn new(app: &'a WordApp, lay: &'a Arc<DocLayout>, sheets: &'a [wordcraft_engine::print::Sheet]) -> Self {
        SheetSource { app, lay, sheets, inputs: inputs(app) }
    }
}

impl ThumbSource<usize> for SheetSource<'_> {
    fn revision(&self, _key: &usize, _request: &ThumbRequest) -> Revision {
        self.inputs
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        let Some(sheet) = self.sheets.get(*key) else { return ThumbRender::Blank };
        let side = request.side.max(1) as usize;
        let scale = side as f32 / sheet.width.max(sheet.height).max(1.0);
        let w = ((sheet.width * scale).round() as usize).clamp(1, side);
        let h = ((sheet.height * scale).round() as usize).clamp(1, side);
        let mut image = ColorImage::filled([w, h], Color32::WHITE);
        for slot in &sheet.slots {
            let Some(pg) = self.lay.pages.get(slot.page) else { continue };
            let k = (slot.width / pg.w).min(slot.height / pg.h);
            let page_scale = k * scale;
            if !(page_scale.is_finite() && page_scale > 0.0) {
                continue;
            }
            let x0 = ((slot.x + (slot.width - pg.w * k) / 2.0) * scale).round() as i64;
            let y0 = ((slot.y + (slot.height - pg.h * k) / 2.0) * scale).round() as i64;
            let page = wordcraft_render::render_page(&self.app.session.doc, pg, page_scale, &preview_options(self.app));
            let straight = page.to_straight();
            let row_len = (page.width as usize).max(1) * 4;
            for (row, line) in straight.chunks_exact(row_len).enumerate() {
                let y = y0 + row as i64;
                if y < 0 || y >= h as i64 {
                    continue;
                }
                for (col, px) in line.as_chunks::<4>().0.iter().enumerate() {
                    let x = x0 + col as i64;
                    if x < 0 || x >= w as i64 {
                        continue;
                    }
                    let Some(dst) = image.pixels.get_mut(y as usize * w + x as usize) else { continue };
                    let a = u32::from(px[3]);
                    let mix = |src: u8, under: u8| ((u32::from(src) * a + u32::from(under) * (255 - a) + 127) / 255) as u8;
                    *dst = Color32::from_rgb(mix(px[0], dst.r()), mix(px[1], dst.g()), mix(px[2], dst.b()));
                }
            }
        }
        ThumbRender::Ready(image)
    }
}

/// The preview (#1156): the sheet fills the pane, a bar below pages through the sheets and zooms; zoomed in,
/// the sheet pans.
pub(super) fn show(app: &mut WordApp, ui: &mut Ui, lay: &Arc<DocLayout>) {
    let params = app.print.settings.params();
    let sheets = match wordcraft_engine::print::sheets(&params, lay, app.session.current_pages(lay)) {
        Ok(sheets) => sheets,
        Err(error) => {
            Text::new(error.to_string()).show(ui);
            return;
        }
    };
    let total = sheets.len();
    let page = app.print.settings.preview.min(total.saturating_sub(1));
    app.print.settings.preview = page;
    if sheets.get(page).is_none() {
        return;
    }
    let ctx = ui.ctx().clone();
    let room = ui.available_size();
    // The sheet is drawn for the pane at fit; zoomed in, it is stretched, up to the store's limit.
    let request = ThumbRequest::for_cell(room.max_elem(), ctx.pixels_per_point());
    let state = thumbnails(&ctx).state(&ctx, &SheetSource::new(app, lay, &sheets), &page, &request, Priority::VISIBLE);
    let label = crate::i18n::fmt(tl!("Sheet {page} of {pages}"), &[("page", &(page + 1).to_string()), ("pages", &total.to_string())]);
    let mut view = app.print.view.clone();
    let mut step = None;
    // The pane takes the room the settings column leaves, the bar under it (production: the sheet at the page's
    // height, the bar at its foot).
    Stack::new().fill().show(ui, |ui, _| {
        let shown = match state.shown() {
            Some(texture) => PreviewState::Ready(texture.sized()),
            None => match state {
                TextureState::Failed { .. } => PreviewState::Failed(tl!("The preview could not be drawn").into()),
                _ => PreviewState::Pending(None),
            },
        };
        let pane = PreviewPane::state(label.clone(), shown, &view).sizing(Sizing::Fill).show(ui);
        view = pane.view.unwrap_or(pane.shown);
        Row::new().show(ui, |ui, _| {
            // Production's sheet count between a back and a forward chevron, each named for the sheet it goes to.
            let previous = IconButton::new(IconId::new("chevronLeft"))
                .label(tl!("Previous sheet"))
                .tooltip(tl!("Previous sheet"))
                .enabled(page > 0)
                .id_salt("previous")
                .show(ui);
            if previous.clicked() {
                step = Some(page.saturating_sub(1));
            }
            Text::new(label.clone()).show(ui);
            let next = IconButton::new(IconId::new("chevronRight"))
                .label(tl!("Next sheet"))
                .tooltip(tl!("Next sheet"))
                .enabled(page + 1 < total)
                .id_salt("next")
                .show(ui);
            if next.clicked() {
                step = Some(page + 1);
            }
            if clicked(&Button::new(tl!("One Page")).id_salt("one-page").show(ui)) {
                view = PrintPage::fit_view();
            }
            if let Some(zoomed) = PreviewZoom::new(&view).show(ui).view {
                view = zoomed;
            }
        });
    });
    app.print.view = view;
    if let Some(next) = step {
        app.print.settings.preview = next;
    }
}
