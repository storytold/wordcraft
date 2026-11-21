use super::{DocLayout, Hash, Hasher, Page, Placed, Pos, Rect, StoryRef, WordApp, json};

/// Fingerprint of everything outside the layout that changes how pages render: the view's
/// marks and markup, dark mode, the page colour and watermark, header/body dimming, the media,
/// and (while a drag lasts) the object left out of the page.
/// Taken once per pass.
// lean: a hash of the view inputs stands in for the page-raster revision; the upgrade is an engine render-inputs
// revision that the pages' pictures key on directly.
pub(super) fn view_key(app: &WordApp, dim_body: bool, hidden: Option<&Pos>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
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

/// Fingerprint of what one placed item draws: everything about it that changes a pixel of the
/// page raster. `None` for the items that draw nothing themselves (`Cell` and `Object` are there
/// for hit testing, handles and wrapping), which no raster can show.
///
/// A paragraph, freeform or graphic is named by its `Arc`'s address, which identifies it only
/// while it lives: [`PageBands`] holds every such `Arc` it fingerprints, so a texture tag can
/// never compare against an address the allocator has since given to other content.
pub(crate) fn item_fingerprint(it: &Placed) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
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
        Placed::Cell { rect, clip, .. } => {
            (rect.x.to_bits(), rect.y.to_bits(), rect.w.to_bits(), rect.h.to_bits(), clip).hash(&mut h);
        }
        Placed::Object { .. } => return None,
    }
    Some(h.finish())
}

/// One drawn item of a page as its texture remembers it ([`PageBands`]): what it draws and the
/// vertical band of the page, in points, its ink can land in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct ItemBand {
    pub(crate) id: u64,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
}

/// How far, in points, a re-rendered band reaches past the changed items' own geometry: an
/// outline, a shadow, an italic's overhang or a glyph drawn past its line's extent all land a
/// little outside the box that laid them out, and the renderer's own culling works from the
/// raster it is handed ([`PageBands::changed_rows`]).
pub(crate) const BAND_PAD_PT: f32 = 32.0;
/// Share of a page's raster rows a changed band may cover before the page is drawn whole
/// instead: a band that big saves nothing.
pub(crate) const BAND_MAX_PAGE_PCT: usize = 75;

/// An `Arc` an item fingerprint names by address, kept alive as long as the snapshot that holds
/// it is. The layout cache may evict a paragraph layout between a page's render and the next
/// edit; without this reference the allocator could hand its address to different content and a
/// texture tag would accept the stale pixels.
// The arcs are never read: holding them until the snapshot drops is the whole job.
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) enum Held {
    Para(std::sync::Arc<wordcraft_layout::ParaLayout>),
    Freeform(std::sync::Arc<wordcraft_doc::freeform::Freeform>),
    Graphic(std::sync::Arc<wordcraft_doc::graphic::Graphic>),
}

/// What a page's texture holds, for re-rendering only the bands an edit changed (#1109): the
/// page size, one band per drawn item in page order, and the layouts those items' fingerprints
/// name by address. Built from the same walk that fingerprints a page's content — once per page
/// and layout, never once a frame (#811).
#[derive(Default, Clone)]
pub(crate) struct PageBands {
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) items: Vec<ItemBand>,
    /// Every address [`item_fingerprint`] can name, so none is freed while this snapshot (and
    /// the texture tagged with it) may still be compared against it. Released on retag.
    #[allow(dead_code)] // Only `Drop` reads it; retaining the layouts is the point.
    pub(crate) held: Vec<Held>,
}

impl PageBands {
    /// Snapshot a page's drawn items and the bands of the page they can ink.
    pub(crate) fn of(page: &Page) -> PageBands {
        // A text story (a text box's own lines) is drawn through its box's frame: a warped box's
        // letters are stretched across the frame, so the frame's band is the one that changes
        // when its text does.
        let mut frames: std::collections::HashMap<u32, (f32, f32)> = std::collections::HashMap::new();
        for it in page.items.iter().chain(&page.header).chain(&page.footer) {
            if let Placed::Object { text_box: Some(id), rect, spin, .. } = it {
                let band = frame_band(*rect, *spin);
                frames
                    .entry(*id)
                    .and_modify(|f| {
                        f.0 = f.0.min(band.0);
                        f.1 = f.1.max(band.1);
                    })
                    .or_insert(band);
            }
        }
        let mut items = Vec::with_capacity(page.items.len() + page.header.len() + page.footer.len());
        let mut held = Vec::new();
        for it in page.items.iter().chain(&page.header).chain(&page.footer) {
            let Some(id) = item_fingerprint(it) else { continue };
            match it {
                Placed::Lines { para, .. } => held.push(Held::Para(para.clone())),
                Placed::Shape { freeform: Some(freeform), .. } => held.push(Held::Freeform(freeform.clone())),
                Placed::Graphic { graphic, .. } => held.push(Held::Graphic(graphic.clone())),
                _ => {}
            }
            let (mut top, mut bottom) = item_band(it, page.h);
            if let Placed::Lines { story: StoryRef::Part(story), .. } = it
                && let Some((f_top, f_bottom)) = frames.get(story)
            {
                top = top.min(*f_top);
                bottom = bottom.max(*f_bottom);
            }
            items.push(ItemBand { id, top, bottom });
        }
        PageBands { w: page.w, h: page.h, items, held }
    }

    /// Fingerprint of a page's laid-out content. A layout never changes once made, so this is
    /// taken once per page and layout ([`PageKeys`]), not every frame.
    pub(crate) fn content(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.w.to_bits(), self.h.to_bits(), self.items.len()).hash(&mut h);
        for b in &self.items {
            // The band too, not only the id: a text box's frame is not an item of its own, and
            // moving or turning it changes only the band its own lines carry.
            (b.id, b.top.to_bits(), b.bottom.to_bits()).hash(&mut h);
        }
        h.finish()
    }

    /// The raster rows of a texture rendered from `self` that no longer match `new`: the union of
    /// the old and new bands of every item that changed, padded and snapped out to whole pixel
    /// rows. `Some(0..0)` means the fingerprints differ but no item did (a hash collision, a
    /// `Cell` or `Object` change): nothing has to be drawn.
    ///
    /// `None` when the whole page must be drawn again: another page size, another raster scale, a
    /// hostile extent that can't be trusted to the pixel, or a change covering most of the page.
    pub(crate) fn changed_rows(&self, new: &PageBands, scale_px: f32, rows: usize) -> Option<std::ops::Range<usize>> {
        if !(scale_px.is_finite() && scale_px > 0.0) || rows == 0 || self.w != new.w || self.h != new.h {
            return None;
        }
        let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
        for i in 0..self.items.len().max(new.items.len()) {
            let (a, b) = (self.items.get(i), new.items.get(i));
            if a == b {
                continue;
            }
            for band in [a, b].into_iter().flatten() {
                // A NaN or infinite extent is a hostile document, not something to crop to a row.
                if !(band.top.is_finite() && band.bottom.is_finite()) {
                    return None;
                }
                top = top.min(band.top);
                bottom = bottom.max(band.bottom);
            }
        }
        if top > bottom {
            return Some(0..0);
        }
        let to_px = |v: f32| ((v as f64) * (scale_px as f64)).clamp(0.0, rows as f64);
        let first = to_px(top - BAND_PAD_PT).floor() as usize;
        let last = to_px(bottom + BAND_PAD_PT).ceil() as usize;
        if last <= first {
            return Some(0..0);
        }
        if (last - first) * 100 >= rows * BAND_MAX_PAGE_PCT {
            return None;
        }
        Some(first..last)
    }
}

/// The vertical band of the page (top, bottom) a frame's ink can land in, in points. A spun
/// frame's ink lands anywhere inside the circle that holds its corners, whatever its angle.
pub(crate) fn frame_band(rect: wordcraft_geom::Rect, spin: wordcraft_geom::Spin) -> (f32, f32) {
    if spin.is_identity() {
        return (rect.y, rect.y + rect.h);
    }
    let r = (rect.w * rect.w + rect.h * rect.h).max(0.0).sqrt() / 2.0;
    let cy = rect.y + rect.h / 2.0;
    (cy - r, cy + r)
}

fn style_effect_extent(rc: &wordcraft_doc::resolve::ResolvedChar) -> Option<(f32, f32)> {
    if let Some(te) = &rc.text_effects
        && te.reflection.is_some()
    {
        return None;
    }
    let mut e_top = 0.0f32;
    let mut e_bot = 0.0f32;
    if let Some(te) = &rc.text_effects {
        if let Some(g) = te.glow {
            let s = g.sanitized().size;
            e_top = e_top.max(s);
            e_bot = e_bot.max(s);
        }
        if let Some(s) = te.shadow {
            let s_san = s.sanitized();
            let (_dx, dy) = s_san.offset();
            let r = s_san.blur / 2.0;
            e_top = e_top.max((r - dy).max(0.0));
            e_bot = e_bot.max((r + dy).max(0.0));
        }
        if let Some(o) = te.outline {
            let w = if o.width.is_finite() { o.width.max(0.0) } else { 0.0 };
            e_top = e_top.max(w);
            e_bot = e_bot.max(w);
        }
    }
    if rc.shadow || rc.emboss || rc.engrave {
        let d = (rc.size / 20.0).clamp(0.5, 6.0);
        e_top = e_top.max(d);
        e_bot = e_bot.max(d);
    }
    if rc.outline {
        e_top = e_top.max(0.5);
        e_bot = e_bot.max(0.5);
    }
    Some((e_top, e_bot))
}

fn lines_effect_extent(para: &wordcraft_layout::ParaLayout, l0: usize, l1: usize) -> Option<(f32, f32)> {
    let mut e_top = 0.0f32;
    let mut e_bot = 0.0f32;
    for l in l0..l1 {
        let line = para.lines.get(l)?;
        let c0 = line.c0.min(para.clusters.len());
        let c1 = line.c1.min(para.clusters.len());
        for cl in &para.clusters[c0..c1] {
            let Some(style) = para.styles.get(cl.style as usize) else { continue };
            let (top, bot) = style_effect_extent(&style.rc)?;
            e_top = e_top.max(top);
            e_bot = e_bot.max(bot);
        }
    }
    Some((e_top, e_bot))
}

/// The vertical band of the page (top, bottom) an item's ink can land in, in points: its frame
/// grown by what the renderer grows it by (an outline stroke, a shadow, a glow). A measure that
/// isn't there falls back to the whole page, which only costs a band re-render that draws it all
/// ([`PageBands::changed_rows`] draws the page whole for a hostile band anyway).
pub(crate) fn item_band(it: &Placed, page_h: f32) -> (f32, f32) {
    let whole = (0.0, page_h);
    match it {
        Placed::Lines { para, l0, l1, y, turn, .. } => {
            if turn.is_turned() {
                return it.turned_bounds().map_or(whole, |r| (r.y, r.y + r.h));
            }
            let (Some(first), Some(last)) = (para.lines.get(*l0), l1.checked_sub(1).and_then(|l| para.lines.get(l))) else {
                return whole;
            };
            let Some((e_top, e_bot)) = lines_effect_extent(para, *l0, *l1) else {
                return whole;
            };
            (*y - e_top, *y + (last.top + last.height - first.top) + e_bot)
        }
        Placed::Fill { rect, .. } => (rect.y, rect.y + rect.h),
        Placed::Image { rect, spin, .. } | Placed::Graphic { rect, spin, .. } => frame_band(*rect, *spin),
        Placed::Shape { rect, spin, stroke_width, effects, .. } => {
            let (top, bottom) = frame_band(*rect, *spin);
            let stroke = if stroke_width.is_finite() { stroke_width.abs().clamp(0.25, 200.0) / 2.0 } else { 100.0 };
            let [_, e_top, _, e_bottom] = effects.extent();
            (top - stroke - e_top, bottom + stroke + e_bottom)
        }
        Placed::Rule { y0, y1, border, .. } => {
            // A double border draws two lines offset from the rule, each half the width across.
            let w = if border.width.is_finite() { border.width.abs().clamp(0.25, 200.0) } else { 200.0 };
            (y0.min(*y1) - w * 1.5, y0.max(*y1) + w * 1.5)
        }
        Placed::Clip { rect, spin, .. } => frame_band(*rect, *spin),
        Placed::Cell { rect, .. } => (rect.y, rect.y + rect.h),
        Placed::Object { .. } => whole,
    }
}

/// A rendered page's tracking info for in-place region updates.
#[derive(Clone)]
pub(crate) struct RenderedPage {
    pub(crate) revision: craft::ui::primitives::textures::Revision,
    pub(crate) view: u64,
    pub(crate) scale: f32,
    pub(crate) bands: std::sync::Arc<PageBands>,
    pub(crate) generation: u64,
}

/// Fingerprint of a page's laid-out content. A layout never changes once made, so this is
/// taken once per page and layout ([`PageKeys`]), not every frame.
#[allow(dead_code)]
pub(super) fn page_content_key(page: &Page) -> u64 {
    PageBands::of(page).content()
}

/// The content fingerprints and band snapshots of one layout's pages, taken when a page is first
/// needed (#811). Hashing every item of every page each frame cost tens of milliseconds a frame
/// in a long table (thousands of cell borders per page), idle or scrolling.
#[derive(Default)]
pub(crate) struct PageKeys {
    /// The layout these are for; held so its address can't be reused by another one.
    pub(super) layout: Option<std::sync::Arc<DocLayout>>,
    /// Each page's content fingerprint and item bands, taken once per layout.
    pages: Vec<Option<(u64, std::sync::Arc<PageBands>)>>,
    /// Pages snapshotted since the layout last changed.
    pub(crate) hashed: usize,
}

impl PageKeys {
    /// Page `i`'s content fingerprint and item bands, taken once per layout.
    pub(crate) fn snapshot(&mut self, layout: &std::sync::Arc<DocLayout>, i: usize) -> (u64, std::sync::Arc<PageBands>) {
        if !self.layout.as_ref().is_some_and(|l| std::sync::Arc::ptr_eq(l, layout)) {
            self.layout = Some(layout.clone());
            self.pages = vec![None; layout.pages.len()];
            self.hashed = 0;
        }
        if let Some(Some(page)) = self.pages.get(i) {
            return page.clone();
        }
        let bands = std::sync::Arc::new(layout.pages.get(i).map_or_else(PageBands::default, PageBands::of));
        let page = (bands.content(), bands);
        if let Some(slot) = self.pages.get_mut(i) {
            *slot = Some(page.clone());
            self.hashed += 1;
        }
        page
    }

    pub(super) fn content(&mut self, layout: &std::sync::Arc<DocLayout>, i: usize) -> u64 {
        self.snapshot(layout, i).0
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

/// Render options for page rasters shown on screen, so a page looks as text does in the
/// platform's other apps (exports never change): on macOS, text is darkened the way the system
/// draws it; on Linux and the BSDs it is hinted lightly, as FreeType's `hintslight` draws it
/// there (not darkened: hinted text already has FreeType's weight, and darkening made it
/// look semi-bold).
pub(crate) fn screen_render_options() -> wordcraft_render::RenderOptions {
    wordcraft_render::RenderOptions {
        text_darkening: cfg!(target_os = "macos"),
        hinting: cfg!(all(unix, not(target_os = "macos"))),
        ..Default::default()
    }
}
