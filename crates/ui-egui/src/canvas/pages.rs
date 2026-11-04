use super::{DocLayout, Hash, Hasher, Page, Placed, Pos, Pos2, Rect, StoryRef, WordApp, json, pos2};

/// Fingerprint of everything outside the layout that changes how pages render: the view's
/// marks and markup, dark mode, the page colour and watermark, header/body dimming, the media,
/// and (while a drag lasts) the object left out of the page.
/// Taken once per frame.
pub(crate) fn view_key(app: &WordApp, dim_body: bool, hidden: Option<&Pos>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let v = &app.session.view;
    (v.marks, app.session.prefs.marks, v.show_markup, app.page_is_dark(), wordcraft_render::DARK_PAPER, v.hide_ink, v.draw.replay).hash(&mut h);
    (v.read_mode, v.read_page_color).hash(&mut h);
    app.session.prefs.markup.hash(&mut h);
    dim_body.hash(&mut h);
    format!("{:?}{:?}{:?}", app.session.doc.settings.page_color, app.session.doc.settings.page_picture, app.session.doc.settings.watermark)
        .hash(&mut h);
    app.session.doc.media.len().hash(&mut h);
    // A page drawn without a dragged object must not pass for the page with it: the drag starts
    // and ends change the key once each, so the page is rendered again then and only then.
    hidden.hash(&mut h);
    h.finish()
}

/// Fingerprint of a page's laid-out content. A layout never changes once made, so this is
/// taken once per page and layout ([`PageKeys`]), not every frame.
fn page_content_key(page: &Page) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (page.w.to_bits(), page.h.to_bits()).hash(&mut h);
    for list in [&page.items, &page.header, &page.footer] {
        list.len().hash(&mut h);
        for it in list.iter() {
            match it {
                Placed::Lines { para, l0, l1, x, y, turn, .. } => {
                    (std::sync::Arc::as_ptr(para) as usize, l0, l1, x.to_bits(), y.to_bits(), turn).hash(&mut h);
                }
                Placed::Fill { rect, color } => format!("{rect:?}{color:?}").hash(&mut h),
                Placed::Rule { x0, y0, x1, y1, border } => format!("{x0}{y0}{x1}{y1}{border:?}").hash(&mut h),
                Placed::Image { rect, media, spin, .. } => format!("{rect:?}{media}{spin:?}").hash(&mut h),
                Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin, extra, .. } => {
                    format!("{rect:?}{kind:?}{fill:?}{stroke:?}{stroke_width}{effects:?}{spin:?}{extra:?}").hash(&mut h);
                    freeform.as_ref().map(|f| std::sync::Arc::as_ptr(f) as usize).hash(&mut h);
                }
                Placed::Graphic { rect, graphic, spin, .. } => {
                    (std::sync::Arc::as_ptr(graphic) as usize, rect.x.to_bits(), rect.y.to_bits(), rect.w.to_bits(), rect.h.to_bits()).hash(&mut h);
                    format!("{spin:?}").hash(&mut h)
                }
                Placed::Clip { rect, spin, items } => format!("{rect:?}{spin:?}{items:?}").hash(&mut h),
                Placed::Cell { .. } | Placed::Object { .. } => {}
            }
        }
    }
    h.finish()
}

/// The content fingerprints of one layout's pages, taken when a page is first needed (#811).
/// Hashing every item of every page each frame cost tens of milliseconds a frame in a long
/// table (thousands of cell borders per page), idle or scrolling.
#[derive(Default)]
pub(crate) struct PageKeys {
    /// The layout these are for; held so its address can't be reused by another one.
    pub(super) layout: Option<std::sync::Arc<DocLayout>>,
    keys: Vec<Option<u64>>,
    /// Pages fingerprinted since the layout last changed.
    pub(crate) hashed: usize,
}

impl PageKeys {
    fn content(&mut self, layout: &std::sync::Arc<DocLayout>, i: usize) -> u64 {
        if !self.layout.as_ref().is_some_and(|l| std::sync::Arc::ptr_eq(l, layout)) {
            self.layout = Some(layout.clone());
            self.keys = vec![None; layout.pages.len()];
            self.hashed = 0;
        }
        if let Some(Some(k)) = self.keys.get(i) {
            return *k;
        }
        let k = layout.pages.get(i).map_or(0, page_content_key);
        if let Some(slot) = self.keys.get_mut(i) {
            *slot = Some(k);
            self.hashed += 1;
        }
        k
    }
}

/// The texture cache key of page `i`: its content, the view, and the raster scale.
pub(super) fn page_key(app: &mut WordApp, layout: &std::sync::Arc<DocLayout>, i: usize, view: u64, scale_px: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (view, scale_px.to_bits(), app.canvas.page_keys.content(layout, i)).hash(&mut h);
    h.finish()
}

pub(super) fn to_screen(origin: Pos2, page_rect: Rect, scale: f32, x: f32, y: f32) -> Pos2 {
    pos2(origin.x + page_rect.min.x + x * scale, origin.y + page_rect.min.y + y * scale)
}

/// A page raster as an egui image. Both hold premultiplied RGBA8, so the pixels go across as they
/// are: un-premultiplying them for egui to premultiply again gave the same image for two more
/// passes over every pixel of the page, on each keystroke.
pub(super) fn page_image(img: &wordcraft_render::Rendered) -> egui::ColorImage {
    egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels)
}

/// Place a raster at a physical-pixel-aligned origin and its exact device-pixel size.
pub(super) fn texel_aligned_rect(layout: Rect, texture_px: egui::Vec2, ppp: f32) -> Rect {
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_size(pos2(snap(layout.min.x), snap(layout.min.y)), texture_px / ppp)
}

/// Tile indices intersecting the view, in a fixed grid of raster pixels. The grid follows the
/// document, not the viewport, so scrolling never changes tile contents or their cache keys.
pub(super) fn visible_tiles(page_rect: Rect, clip: Rect, screen_scale: f32, raster_scale: f32, tile_height_px: usize) -> std::ops::Range<usize> {
    if !page_rect.intersects(clip)
        || !screen_scale.is_finite()
        || !raster_scale.is_finite()
        || screen_scale <= 0.0
        || raster_scale <= 0.0
        || tile_height_px == 0
    {
        return 0..0;
    }
    let total_px = (page_rect.height() / screen_scale * raster_scale).ceil() as usize;
    let count = total_px.div_ceil(tile_height_px);
    let at = |y: f32| ((y - page_rect.min.y).clamp(0.0, page_rect.height()) / screen_scale * raster_scale) / tile_height_px as f32;
    let first = (at(clip.top()).floor() as usize).min(count);
    let last = (at(clip.bottom()).ceil() as usize).min(count);
    first..last.max(first)
}

pub(crate) fn page_screen_scale(rect: Rect, page: &Page, fallback: f32) -> f32 {
    if page.w > 0.0 { rect.width() / page.w } else { fallback }
}

/// Closest the drawing grid's lines get on screen; zoomed far out, every other line is skipped.
pub(super) const DRAWING_GRID_MIN_PX: f32 = 4.0;

/// Most grid lines drawn each way on one page (page sizes come from files).
pub(super) const DRAWING_GRID_MAX_LINES: usize = 2000;

/// The View › Gridlines drawing grid over a page's text area, in document points: the x of each
/// vertical line and the y of each horizontal line, starting at the margins and `step` (across,
/// down) apart, never outside the margins. Drawn on screen only — never printed or exported.
pub(super) fn drawing_grid(body: wordcraft_geom::Rect, step: (f32, f32)) -> (Vec<f32>, Vec<f32>) {
    let axis = |start: f32, len: f32, step: f32| -> Vec<f32> {
        if !(step.is_finite() && step > 0.0 && start.is_finite() && len.is_finite() && len >= 0.0) {
            return Vec::new();
        }
        let end = start + len + 0.01;
        (0..DRAWING_GRID_MAX_LINES).map(|k| start + k as f32 * step).take_while(|p| *p <= end).collect()
    };
    (axis(body.x, body.w, step.0), axis(body.y, body.h, step.1))
}

/// Global View Gridlines outlines every table (#1628). While editing a header/footer, also
/// outline that story's cells (#2197), without adding printable borders or changing the view flag.
pub(super) fn gridline_cells(page: &Page, all: bool, editing: Option<StoryRef>) -> impl Iterator<Item = wordcraft_geom::Rect> + '_ {
    // Automatic header/footer guides need not scan a potentially large body placement list.
    let body = if all { page.items.as_slice() } else { &[] };
    body.iter().chain(&page.header).chain(&page.footer).filter_map(move |it| match it {
        Placed::Cell { rect, covered: false, story, .. } if all || Some(*story) == editing => Some(*rect),
        _ => None,
    })
}

/// The drawing grid's spacing in points at a screen scale: the document's spacing (`w:drawingGrid*Spacing`;
/// ECMA-376's 1/8 inch when it doesn't say), doubled until the lines are at least
/// [`DRAWING_GRID_MIN_PX`] apart on screen.
pub(super) fn drawing_grid_step(spacing: f32, scale: f32) -> f32 {
    let mut step = if spacing.is_finite() && spacing > 0.0 { spacing } else { wordcraft_doc::DEFAULT_GRID };
    if !(scale.is_finite() && scale > 0.0) {
        return step;
    }
    for _ in 0..16 {
        if step * scale >= DRAWING_GRID_MIN_PX {
            break;
        }
        step *= 2.0;
    }
    step
}

/// Draw the document canvas in the space `ui` has left, one frame: the pages, caret, selection
/// and rulers, with the mouse editing and the pointer shape over them.
/// The layout the page shows: the document's own, or with the table Insert › Table's grid is
/// hovering over (#3931). The preview is laid out on a copy, so the document and its undo history
/// change only when a square is clicked.
pub(super) fn shown_layout(app: &mut WordApp) -> std::sync::Arc<DocLayout> {
    let Some((rows, cols)) = app.canvas.table_preview.take() else {
        app.canvas.table_preview_layout = None;
        return app.session.layout();
    };
    // The caret can move, or the view change, with the grid still hovered: the table then goes
    // somewhere else, or the page is laid out to another width.
    let key = ((rows, cols, app.session.document_id(), app.session.rev(), app.session.sel.clone()), app.session.layout());
    if let Some((k, _, _, l)) = &app.canvas.table_preview_layout
        && k.0 == key.0
        && std::sync::Arc::ptr_eq(&k.1, &key.1)
    {
        return l.clone();
    }
    match app.session.preview_layout("insert.table", &json!({"rows": rows, "cols": cols})) {
        Some((doc, sel, l)) => {
            app.canvas.table_preview_layout = Some((key, doc, sel, l.clone()));
            l
        }
        None => {
            app.canvas.table_preview_layout = None;
            app.session.layout()
        }
    }
}

/// The document the pages of [`shown_layout`] are drawn from: the previewed copy while the page
/// shows Insert › Table's preview, whose paragraph paths are the layout's (#3931).
pub(crate) fn page_doc(app: &WordApp) -> &wordcraft_doc::Document {
    app.canvas.table_preview_layout.as_ref().map_or(&app.session.doc, |(_, doc, _, _)| doc)
}

/// The previewed copy's selection while the page shows Insert › Table's preview: the caret and
/// selection are drawn from it, as the document's own name other places in the preview's layout.
pub(crate) fn preview_sel(app: &WordApp) -> Option<&wordcraft_engine::Selection> {
    app.canvas.table_preview_layout.as_ref().map(|(_, _, sel, _)| sel)
}

/// Render options for this view's page rasters: marks, markup, dark page, header/body dimming.
pub(crate) fn page_render_options(app: &WordApp, dim_body: bool) -> wordcraft_render::RenderOptions {
    let mut opts = screen_render_options();
    opts.display.marks = app.session.view.marks;
    opts.display.always = app.session.prefs.marks;
    opts.display.placeholders = true;
    opts.display.markup = app.session.view.show_markup;
    // Ink Replay draws the ink itself, stroke by stroke, over pages without it.
    opts.display.hide_ink = app.session.view.hide_ink || app.session.view.draw.replay;
    opts.display.revisions = app.session.prefs.markup.clone();
    opts.dark = app.page_is_dark();
    opts.dark_paper = wordcraft_render::DARK_PAPER;
    // In Read Mode the Page color sets the paper and the dark page, and Switch Modes is ignored.
    if app.session.view.read_mode {
        use wordcraft_engine::cmd::view::ReadPageColor;
        let color = app.session.view.read_page_color;
        opts.read_paper = Some(if color == ReadPageColor::Sepia { wordcraft_render::SEPIA_PAPER } else { wordcraft_doc::props::Rgb::WHITE });
        opts.dark = color == ReadPageColor::Inverse;
    }
    opts.display.dim_header = !dim_body;
    opts.display.dim_body = dim_body;
    opts
}

/// Render options for page rasters shown on screen: on macOS, text is darkened the way the system
/// draws it, so a page looks as it does in other Mac apps (exports never are).
pub(crate) fn screen_render_options() -> wordcraft_render::RenderOptions {
    wordcraft_render::RenderOptions { text_darkening: cfg!(target_os = "macos"), ..Default::default() }
}
