//! The template thumbnails of the File page: the first page of each built-in template, drawn in a
//! throwaway session so the user's document is never touched.

use craft::ui::primitives::{ContentPainterCx, Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::shell::ThumbKey;
use craft::ui::style::{Corners, SurfaceKind};
use serde_json::json;

/// The salt every reader of the File page's thumbnails asks the store with, so New, Home and the
/// recent rows share one loader and one set of pictures.
const SALT: &str = "wordcraft.templates";

/// The File page's thumbnail store: one per context, found by its salt (no app field, no static).
pub(crate) fn store(ctx: &egui::Context) -> Thumbnails<ThumbKey> {
    Thumbnails::<ThumbKey>::in_context(ctx, SALT, ThumbnailOptions::new())
}

/// Paints the thumbnail of `key` in `rect`, asked for at the size it is painted: the store's
/// placeholder or failure mark while the picture is not there, the sunken tile when it has none.
pub(crate) fn paint(cx: &ContentPainterCx, rect: egui::Rect, store: &Thumbnails<ThumbKey>, source: &dyn ThumbSource<ThumbKey>, key: &ThumbKey) {
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
    let state = store.state(cx.painter.ctx(), source, key, &request, Priority::VISIBLE);
    if !store.paint(cx, rect, &state, &ThumbPaint::default()) {
        cx.surface(rect, SurfaceKind::Sunken, Corners::None);
    }
}

/// Renders the first page of a built-in template for [`Thumbnails`]. A recent file has no picture
/// here (the row shows the file icon), so only `ThumbKey::Template` renders.
pub(crate) struct TemplateThumbs {
    /// The region whose paper the blank template shows, so the preview matches the document a click
    /// makes; it is part of every revision.
    pub(crate) region: wordcraft_geom::Region,
}

impl ThumbSource<ThumbKey> for TemplateThumbs {
    fn revision(&self, key: &ThumbKey, _request: &ThumbRequest) -> Revision {
        match key {
            ThumbKey::Template(id) => Revision::of(&(id, format!("{:?}", self.region))),
            _ => Revision::new(0),
        }
    }

    fn render(&self, key: &ThumbKey, request: &ThumbRequest) -> ThumbRender {
        let ThumbKey::Template(id) = key else { return ThumbRender::Blank };
        let mut session = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
        session.system_region = self.region.clone();
        if session.run("file.new", &json!({"template": id})).is_err() {
            return ThumbRender::Blank;
        }
        let layout = session.export_layout();
        let Some(page) = layout.pages.first() else { return ThumbRender::Blank };
        // The longest side of the page is `request.side` pixels; the store never asks for zero.
        let scale = request.side as f32 / page.w.max(page.h).max(1.0);
        let image = wordcraft_render::render_page(&session.doc, page, scale, &crate::canvas::screen_render_options());
        let pixels = image.to_straight();
        ThumbRender::Ready(egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &pixels))
    }
}
