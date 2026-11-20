use super::{DocLayout, Hash, Hasher, Page, Placed, Pos, Rect, WordApp, json};

/// Fingerprint of everything outside the layout that changes how pages render: the view's
/// marks and markup, dark mode, the page colour and watermark, header/body dimming, the media,
/// and (while a drag lasts) the object left out of the page.
/// Taken once per pass.
// lean: a hash of the view inputs stands in for the page-raster revision; the upgrade is an engine render-inputs
// revision that the pages' pictures key on directly.
pub(super) fn view_key(app: &WordApp, dim_body: bool, hidden: Option<&Pos>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // A page drawn under the system's earlier text settings, or the user's earlier smoothing choice, must not pass
    // for one drawn under these.
    (crate::screen_text::current().key(), app.session.prefs.text_smoothing).hash(&mut h);
    let v = &app.session.view;
    (v.marks, app.session.prefs.marks, v.show_markup, v.dark_mode, wordcraft_render::DARK_PAPER, v.hide_ink, v.draw.replay).hash(&mut h);
    (v.read_mode, v.read_page_color, app.session.prefs.paper_color).hash(&mut h);
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
pub(super) fn page_content_key(page: &Page) -> u64 {
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
                Placed::Image { rect, media, crop, spin, fx, .. } => format!("{rect:?}{media}{crop:?}{spin:?}{fx:?}").hash(&mut h),
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
    pub(super) fn content(&mut self, layout: &std::sync::Arc<DocLayout>, i: usize) -> u64 {
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

/// What page `i` shows, whatever its size: its content and the view (`view_key`). The page's picture is versioned
/// by it.
pub(super) fn page_shows(keys: &mut PageKeys, layout: &std::sync::Arc<DocLayout>, i: usize, view: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (view, keys.content(layout, i)).hash(&mut h);
    h.finish()
}

pub(crate) fn page_screen_scale(rect: Rect, page: &Page, fallback: f32) -> f32 {
    if page.w > 0.0 { rect.width() / page.w } else { fallback }
}

/// The layout the page shows: the document's own, or with the table Insert › Table's grid is
/// hovering over (#3931). The preview is laid out on a copy, so the document and its undo history
/// change only when a square is clicked.
pub(super) fn shown_layout(app: &mut WordApp) -> std::sync::Arc<DocLayout> {
    let table = app.canvas.table_preview.take().map(|(rows, cols)| ("insert.table".to_string(), json!({"rows": rows, "cols": cols})));
    let command = app.canvas.command_preview.take();
    app.canvas.command_previewed = command.clone().filter(|_| table.is_none());
    let Some((id, params)) = table.or(command) else {
        app.canvas.table_preview_layout = None;
        return app.session.layout();
    };
    // The caret can move, or the view change, with the grid still hovered: the table then goes
    // somewhere else, or the page is laid out to another width.
    let key = ((id.clone(), params.clone(), app.session.document_id(), app.session.rev(), app.session.sel.clone()), app.session.layout());
    if let Some((k, _, _, l)) = &app.canvas.table_preview_layout
        && k.0 == key.0
        && std::sync::Arc::ptr_eq(&k.1, &key.1)
    {
        return l.clone();
    }
    match app.session.preview_layout(&id, &params) {
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

/// Whether the pages are drawn dark: View › Switch Modes, or in Read Mode the Page color's
/// Inverse (Switch Modes is ignored there). Drawing over the pages (ink, drag previews) follows it.
pub(crate) fn dark_page(app: &WordApp) -> bool {
    use wordcraft_engine::cmd::view::ReadPageColor;
    let v = &app.session.view;
    if v.read_mode { v.read_page_color == ReadPageColor::Inverse } else { v.dark_mode }
}

/// Render options for this view's page rasters: marks, markup, dark page, header/body dimming.
pub(super) fn page_render_options(app: &WordApp, dim_body: bool) -> wordcraft_render::RenderOptions {
    let mut opts = screen_render_options();
    // Pages, and only pages, follow the system's own text settings once they are known ([`crate::screen_text`]);
    // `view_key` has them, so a change redraws the pages.
    crate::screen_text::current().apply(&mut opts);
    opts.display.marks = app.session.view.marks;
    opts.display.always = app.session.prefs.marks;
    opts.display.placeholders = true;
    opts.display.markup = app.session.view.show_markup;
    // Ink Replay draws the ink itself, stroke by stroke, over pages without it.
    opts.display.hide_ink = app.session.view.hide_ink || app.session.view.draw.replay;
    opts.display.revisions = app.session.prefs.markup.clone();
    opts.dark = dark_page(app);
    opts.dark_paper = wordcraft_render::DARK_PAPER;
    // A screen tint (View › Paper color) paints the page, unless the page is drawn dark.
    if !opts.dark {
        opts.paper = app.session.prefs.paper_color.color();
    }
    // In Read Mode the Page color sets the paper: Sepia and Inverse as before, None the
    // selected screen tint (white without one).
    if app.session.view.read_mode {
        use wordcraft_engine::cmd::view::ReadPageColor;
        opts.read_paper = Some(match app.session.view.read_page_color {
            ReadPageColor::None => app.session.prefs.paper_color.color(),
            ReadPageColor::Sepia => wordcraft_render::SEPIA_PAPER,
            ReadPageColor::Inverse => wordcraft_doc::props::Rgb::WHITE,
        });
    }
    opts.display.dim_header = !dim_body;
    opts.display.dim_body = dim_body;
    opts
}

/// Subpixel (LCD) text for the page rasters of a screen with `ppp` device pixels per point, as File › Options ›
/// General › Screen text smoothing says. "Use system setting" draws it where the system asks for it
/// ([`crate::screen_text`]) and the screen is below two device pixels per point: on a denser screen the stripes are
/// too fine to gain from, and a page has four times the pixels to rasterise three times. "Subpixel (LCD)" draws it
/// on any screen, in the system's stripe order, RGB when it names none (macOS never does).
pub(super) fn lcd_for(
    choice: wordcraft_engine::cmd::view::TextSmoothing,
    asked: Option<crate::screen_text::SubpixelOrder>,
    stripes: Option<crate::screen_text::SubpixelOrder>,
    ppp: f32,
) -> Option<wordcraft_render::LcdOrder> {
    use crate::screen_text::SubpixelOrder;
    use wordcraft_engine::cmd::view::TextSmoothing;
    use wordcraft_render::LcdOrder;
    let order = |stripes: SubpixelOrder| match stripes {
        SubpixelOrder::Rgb => LcdOrder::Rgb,
        SubpixelOrder::Bgr => LcdOrder::Bgr,
    };
    match choice {
        TextSmoothing::Grayscale => None,
        // The user's own choice, in the order of their screen even where the system has smoothing off (a BGR panel
        // with ClearType off must not get RGB edges).
        TextSmoothing::Subpixel => Some(asked.or(stripes).map(order).unwrap_or(LcdOrder::Rgb)),
        // Written so a density that isn't a number is greyscale too.
        TextSmoothing::System => asked.map(order).filter(|_| ppp < 2.0),
    }
}

/// The scale (pixels per document point) to rasterise a page picture at, and whether that is the screen's own.
///
/// The page view asks for a picture by its longest side in pixels, `side`, rounded up to a multiple of `step`, and
/// scales what it gets to the page's place on screen. Rasterised at `side`, a page is up to `step` pixels larger
/// than it shows and every glyph is resampled on the way to the screen, which undoes baselines put on whole pixels
/// and makes subpixel text wrong (each colour's edge off its stripe). `native` is the view's own scale, device
/// pixels per document point, when it is known: where it accounts for the side asked for (the page's `longest`
/// side, in points, at that scale rounds up to `side`), the picture is rasterised at exactly that scale, one raster
/// pixel per device pixel. Otherwise (another view's request, a fit zoom not settled yet, a part of a page shown
/// reduced) it is rasterised at the side asked for, as before, and is not the screen's own.
pub(super) fn raster_scale(side: u32, step: u32, longest: f32, native: Option<f32>) -> (f32, bool) {
    let asked = side as f32 / longest;
    let fits = |scale: f32| {
        let pixels = longest * scale;
        scale.is_finite() && scale > 0.0 && pixels.is_finite() && pixels <= side as f32 + 0.5 && pixels > side as f32 - step as f32 - 0.5
    };
    match native.filter(|scale| longest.is_finite() && longest > 0.0 && step > 0 && fits(*scale)) {
        Some(scale) => (scale, true),
        None => (asked, false),
    }
}

/// Render options for rasters shown on screen other than the pages themselves (style and font previews, a dragged
/// object), so their text looks as it does in the platform's other apps (exports never change): on macOS it is
/// darkened the way the system draws it; on Linux and the BSDs it is hinted lightly, as FreeType's `hintslight`
/// draws it there (not darkened: hinted text already has FreeType's weight, and darkening made it look semi-bold).
/// Baselines sit on whole device pixels (hinting does that; unhinted text is moved there). These are fixed per
/// platform: such rasters are cached without the system's text settings in their keys, so they must not follow
/// them. Pages do ([`page_render_options`]).
pub(crate) fn screen_render_options() -> wordcraft_render::RenderOptions {
    let mut opts = wordcraft_render::RenderOptions { baseline_snap: true, ..Default::default() };
    crate::screen_text::ScreenText::PLATFORM.apply(&mut opts);
    opts
}

/// Render options for the template thumbnails of File › New: the platform's text, as [`screen_render_options`],
/// without the baseline snapping. A thumbnail is a page shrunk to a tile, drawn as it was before pages were
/// sharpened.
pub(crate) fn thumbnail_render_options() -> wordcraft_render::RenderOptions {
    let mut opts = wordcraft_render::RenderOptions::default();
    crate::screen_text::ScreenText::PLATFORM.apply(&mut opts);
    opts
}
