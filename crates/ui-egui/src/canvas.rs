//! The document canvas: pages on a grey background, rendered into cached page textures (or
//! visible full-resolution tiles for exceptionally tall pages), with the caret, selection,
//! rulers and mouse editing drawn by egui on top.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::display::CommentDisplay;
use wordcraft_layout::{DocLayout, Page, Placed};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

/// Points on screen per document point at 100% (96 px per inch, like a word processor).
pub const PX_PER_PT: f32 = 96.0 / 72.0;
const GAP: f32 = 18.0;
const RULER: f32 = 22.0;
/// Maximum device-pixel height of a tile on very tall Web/Draft pages.
const TILE_HEIGHT_PX: usize = 1024;

/// Drag-select auto-scroll (#688): the pointer within this many points of the viewport's edge
/// starts scrolling, like Word.
const EDGE_MARGIN: f32 = 32.0;
/// Auto-scroll speed (points/s) at the margin's inner boundary and at (or past) the edge.
const EDGE_SPEED_MIN: f32 = 45.0;
const EDGE_SPEED_MAX: f32 = 720.0;
/// Longest frame time trusted for auto-scroll (a stall shouldn't jump the page).
const EDGE_MAX_DT: f32 = 0.1;
/// Page textures are kept for pages within this many viewport heights of the view; farther ones
/// are rendered again when they come back.
const TEXTURE_KEEP_SCREENS: f32 = 3.0;

pub struct CanvasState {
    textures: HashMap<usize, (u64, TextureHandle)>,
    /// Visible full-resolution tiles of pages too tall for one GPU texture.
    tiles: HashMap<(usize, usize), (u64, TextureHandle)>,
    /// Page content fingerprints of the current layout, for the texture cache (#811).
    pub(crate) page_keys: PageKeys,
    /// Whether the document has tracked formatting changes, moves and deletions, by document
    /// revision: finding out walks every paragraph, too slow for every frame (#811).
    markup_facts: std::cell::Cell<Option<((u64, u64), [bool; 3])>>,
    pub scroll_to_caret: bool,
    /// A heading navigation click aligns its page against the navigation pane, not just visible.
    pub(crate) nav_to_caret: bool,
    /// Touchpad travel not yet converted to an entire reading-column turn.
    read_wheel_accum: f32,
    /// Last actual viewport seen during a resize, distinct from the committed reading page size.
    read_viewport: Option<egui::Vec2>,
    read_resized_at: f64,
    /// Reading controls (as opposed to window resizes) must reflow without debounce.
    pub(crate) read_force_reflow: bool,
    /// First text position on the column currently leading the reading viewport, captured from
    /// last frame's layout. Survives changing page sizes, even when the caret stays elsewhere.
    pub(crate) read_first_visible: Option<Pos>,
    /// An animated column-turn destination; intermediate offsets are allowed during the motion.
    read_target_page: Option<usize>,
    pub caret_visible_since: f64,
    /// The caret's colour and the fingerprint of where it was worked out ([`caret_color_at`]).
    caret_bg: Option<(u64, Color32)>,
    dragging: bool,
    /// Alt+drag: where the column (block) selection started (page, x, y) and its last end.
    column_drag: Option<((usize, f32, f32), (usize, f32, f32))>,
    /// The last primary press on the page (time, screen point, clicks in a row), to count
    /// double- and triple-presses as they go down (egui reports them on release).
    last_press: Option<(f64, Pos2, u8)>,
    /// A drag after a double-click, triple-click or Ctrl/⌘+click: the unit it extends by, the
    /// unit first selected, and the last position the pointer hit.
    unit_drag: Option<UnitDrag>,
    pub last_highlight: String,
    pub last_font_color: String,
    pub last_shading: String,
    pub open_url: Option<String>,
    /// Screen rects of pages last frame (for the control channel: page ↔ screen coordinates).
    pub page_rects: Vec<Rect>,
    /// Effective scale (screen points per document point).
    pub scale: f32,
    pub canvas_rect: Option<Rect>,
    pub focused: bool,
    /// The page's widget id last frame, so a click elsewhere can hand the keyboard straight back to
    /// it (`WordApp::refocus_after_chrome_click`).
    pub(crate) widget_id: Option<egui::Id>,
    pub render_ms: f64,
    pub ime_preedit: String,
    /// Document faces added to the interface fonts so the input method's composition (#565) and a
    /// file name in the window caption can be drawn.
    pub(crate) ime_faces: Vec<std::sync::Arc<wordcraft_fonts::FontFace>>,
    pub want_focus: bool,
    pub context_issue: Option<serde_json::Value>,
    pub context_synonyms: Option<serde_json::Value>,
    /// The right-click context menu is open (the mini toolbar stands down while it is).
    pub context_menu_open: bool,
    /// Shift+F10 (or `ui.contextMenu`) asked for the context menu at the caret (#821).
    pub context_menu_from_keys: bool,
    /// Where the context menu stands: the right-click point, or below the caret.
    pub context_menu_at: Option<Pos2>,
    /// Screen rect of the selection's first line, for the floating mini toolbar.
    pub mini_anchor: Option<Rect>,
    /// The selection the mini toolbar was put away for by a click outside the document (the
    /// ribbon, a pane, the status bar); it comes back when the selection changes.
    pub mini_dismissed: Option<wordcraft_engine::Selection>,
    /// A picture, shape or text box being dragged by its frame.
    pub(crate) obj_drag: Option<crate::objects::ObjectDrag>,
    /// Crop mode (Picture Format › Crop) is on for the picture here (`crop`).
    pub(crate) crop: Option<wordcraft_doc::Pos>,
    /// A crop handle being dragged.
    pub(crate) crop_drag: Option<crate::crop::CropDrag>,
    /// An ink stroke being drawn with a pen (Draw tab).
    pub(crate) ink: Option<crate::ink::InkDrag>,
    /// The eraser is down: whether it has erased anything in this drag yet.
    pub(crate) erasing: Option<bool>,
    /// The lasso being drawn or dragged, and Ink Replay (Draw tab).
    pub(crate) ink_ui: crate::ink::InkUi,
    /// Wheel/touchpad scrolling (smooth notches, touchpad momentum).
    pub(crate) wheel: crate::scroll::CanvasScroll,
    /// Middle-click scrolling (#1065).
    pub(crate) auto_scroll: crate::autoscroll::AutoScroll,
    /// The scroll offset and its maximum at the end of last frame.
    pub(crate) scroll_offset: egui::Vec2,
    scroll_max: egui::Vec2,
    /// Explicit page turn to apply on the next frame (reading navigation buttons/keys).
    scroll_request: Option<egui::Vec2>,
    /// The viewport (the content's visible rect on screen) at the end of last frame; the
    /// drag-select edge auto-scroll decides this frame's scroll from it (#688).
    pub(crate) viewport: Option<egui::Rect>,
    /// Pages per row last frame; when it changes the caret's page is scrolled back into view.
    pub cols: usize,
    /// The comment whose balloon is selected (its text is editable in place, like Word's).
    pub balloon: Option<u32>,
    /// Give the selected balloon's text field keyboard focus on its next frame.
    pub(crate) balloon_focus: bool,
    /// The selected balloon shows a reply field.
    pub(crate) balloon_reply: bool,
    /// The selected balloon's height last frame (screen points), for its click area.
    balloon_h: f32,
    /// Screen rects of the comment balloons last frame (for the control channel and tests).
    pub balloon_rects: Vec<(u32, Rect)>,
    /// A paste event arrived since the last Mod+V release (see `keys::canvas_events`).
    pub(crate) pasted: bool,
    /// Draw Table or Eraser is on (`table_pen`): presses draw instead of moving the caret.
    pub table_tool: Option<crate::table_pen::TableTool>,
    /// The Draw Table stroke (or eraser press) in progress.
    pub(crate) table_stroke: Option<crate::table_pen::PenStroke>,
    /// A table border or the table's resize handle being dragged (`table_resize`).
    pub(crate) table_resize: Option<crate::table_resize::ResizeDrag>,
    /// The last key ran an Alt shortcut: the text its key sends (it may arrive a frame later)
    /// isn't typed. The next key press clears it.
    pub(crate) swallow_text: bool,
    /// Alt+numpad entry in progress: the digits typed while Alt is held (#961).
    pub(crate) alt_code: String,
    /// The character an Alt code just typed: if the platform also sends its own composed
    /// character as text on the Alt release (Windows does), it isn't typed twice.
    pub(crate) alt_code_echo: Option<crate::keys::AltCodeEcho>,
    /// Create Link is waiting for a click on the text box to link to: the linking box's object.
    pub(crate) link_from: Option<wordcraft_doc::Pos>,
    /// What screen readers were last told about the document (#488).
    pub(crate) a11y: crate::a11y::A11yCache,
    /// A press on a table's selection area or move handle, held (`table_mouse`).
    pub(crate) table_drag: Option<crate::table_mouse::TableDrag>,
    /// The kind of tab stop a click on the ruler sets (the selector in the rulers' corner).
    pub tab_kind: wordcraft_doc::props::TabAlign,
    /// A tab stop being placed or dragged on the ruler (`ruler_tabs`).
    pub(crate) tab_drag: Option<crate::ruler_tabs::TabDrag>,
}

impl CanvasState {
    /// Stop a text-selection drag when the pointer gesture becomes an object drag.
    pub(crate) fn cancel_text_drag(&mut self) {
        self.dragging = false;
        self.column_drag = None;
        self.unit_drag = None;
    }

    /// How many pages have a cached rendering.
    #[cfg(test)]
    pub(crate) fn texture_count(&self) -> usize {
        self.textures.len() + self.tiles.len()
    }
}

impl Default for CanvasState {
    fn default() -> Self {
        CanvasState {
            textures: HashMap::new(),
            tiles: HashMap::new(),
            page_keys: PageKeys::default(),
            markup_facts: std::cell::Cell::new(None),
            scroll_to_caret: true,
            nav_to_caret: false,
            read_wheel_accum: 0.0,
            read_viewport: None,
            read_resized_at: 0.0,
            read_force_reflow: false,
            read_first_visible: None,
            read_target_page: None,
            caret_visible_since: 0.0,
            caret_bg: None,
            dragging: false,
            column_drag: None,
            last_press: None,
            unit_drag: None,
            last_highlight: "yellow".into(),
            last_font_color: "C00000".into(),
            last_shading: "FFF2CC".into(),
            open_url: None,
            page_rects: Vec::new(),
            scale: PX_PER_PT,
            canvas_rect: None,
            focused: false,
            widget_id: None,
            render_ms: 0.0,
            ime_preedit: String::new(),
            ime_faces: Vec::new(),
            want_focus: true,
            context_issue: None,
            context_synonyms: None,
            obj_drag: None,
            crop: None,
            crop_drag: None,
            ink: None,
            erasing: None,
            ink_ui: Default::default(),
            context_menu_open: false,
            context_menu_from_keys: false,
            context_menu_at: None,
            mini_anchor: None,
            mini_dismissed: None,
            pasted: false,
            swallow_text: false,
            alt_code: String::new(),
            alt_code_echo: None,
            wheel: Default::default(),
            auto_scroll: Default::default(),
            scroll_offset: egui::Vec2::ZERO,
            scroll_max: egui::Vec2::ZERO,
            scroll_request: None,
            viewport: None,
            cols: 1,
            balloon: None,
            balloon_focus: false,
            balloon_reply: false,
            balloon_h: 0.0,
            balloon_rects: Vec::new(),
            table_tool: None,
            table_stroke: None,
            table_resize: None,
            link_from: None,
            a11y: Default::default(),
            table_drag: None,
            tab_kind: Default::default(),
            tab_drag: None,
        }
    }
}

/// See [`CanvasState::unit_drag`].
struct UnitDrag {
    unit: &'static str,
    /// Clicks in a row that made it (2 or 3; 1 for Ctrl/⌘+click).
    clicks: u8,
    start: Pos,
    end: Pos,
    last: Pos,
}

/// Clicks in a row for a press at `at` (time `now`) after `last`: within the double-click
/// delay and distance it counts up to 3 (a triple-click), otherwise it starts again at 1.
fn press_count(last: Option<(f64, Pos2, u8)>, now: f64, at: Pos2, delay: f64, dist: f32) -> u8 {
    match last {
        Some((t, p, n)) if now - t >= 0.0 && now - t < delay && p.distance(at) <= dist => n.saturating_add(1).min(3),
        _ => 1,
    }
}

impl CanvasState {
    /// True while a mouse drag-select is in progress (the mini toolbar waits for mouse-up).
    pub fn drag_selecting(&self) -> bool {
        self.dragging
    }
}

/// Page positions in content space (points), for the current zoom.
pub struct Geometry {
    pub rects: Vec<Rect>,
    pub size: egui::Vec2,
    pub scale: f32,
    /// Pages per row.
    pub cols: usize,
    /// Per page, the vertical range (pixels from the page rect's top) that is on screen. The
    /// whole page, except with Hide White Space, which cuts the margins between pages.
    pub bands: Vec<(f32, f32)>,
}

/// Room kept above the first line and below the last line of a page when white space is hidden (points).
const WHITE_SPACE_PAD: f32 = 6.0;
/// The line drawn between two pages when white space is hidden (pixels).
const SEAM: f32 = 1.0;

/// Hide White Space: stack single-column pages (`rects`, full pages) so that only each page's
/// text area (`bodies`: top and bottom of the body in points, plus a little room) shows, with a
/// [`SEAM`] between pages. The first page keeps its top and the last its bottom. Rects keep the
/// full page size, so page coordinates still map as `(p - rect.min) / scale`; neighbouring rects
/// overlap in the hidden margins. Returns the moved rects, the visible band of each, and the
/// content height.
fn collapse_white_space(rects: &[Rect], bodies: &[(f32, f32)], scale: f32, avail_h: f32) -> (Vec<Rect>, Vec<(f32, f32)>, f32) {
    let n = rects.len();
    let scale = if scale.is_finite() { scale.clamp(0.01, 10.0) } else { PX_PER_PT };
    let avail_h = if avail_h.is_finite() { avail_h } else { 0.0 };
    let mut out = Vec::with_capacity(n);
    let mut bands = Vec::with_capacity(n);
    let Some(first) = rects.first() else { return (out, bands, avail_h) };
    let mut cursor = first.min.y;
    for (i, r) in rects.iter().enumerate() {
        let h = if r.height().is_finite() { r.height().max(0.0) } else { 0.0 };
        let (bt, bb) = bodies.get(i).copied().unwrap_or((0.0, h));
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let top = if i == 0 { 0.0 } else { (fin(bt, 0.0) * scale - WHITE_SPACE_PAD * scale).clamp(0.0, h) };
        let bot = if i + 1 == n { h } else { (fin(bb, 0.0) * scale + WHITE_SPACE_PAD * scale).clamp(top, h) };
        let y = cursor - top;
        out.push(Rect::from_min_size(pos2(r.min.x, y), r.size()));
        bands.push((top, bot));
        cursor = y + bot + SEAM;
    }
    (out, bands, (cursor - SEAM + GAP).max(avail_h))
}

/// True when any page is cut to a band (white space is hidden).
fn white_space_hidden(rects: &[Rect], bands: &[(f32, f32)]) -> bool {
    rects.iter().zip(bands).any(|(r, b)| b.0 > 0.5 || b.1 < r.height() - 0.5)
}

/// True when a double-click at `p` is on the space between two pages (or on the seam when white
/// space is hidden): Word's way to toggle Hide White Space.
fn on_page_gap(rects: &[Rect], bands: &[(f32, f32)], p: Pos2) -> bool {
    let hidden = white_space_hidden(rects, bands);
    rects.windows(2).enumerate().any(|(i, w)| {
        let (a, b) = (w[0], w[1]);
        let in_x = p.x >= a.min.x.min(b.min.x) && p.x <= a.max.x.max(b.max.x);
        let (y0, y1) = match bands.get(i) {
            Some(&(_, bot)) if hidden => (a.min.y + bot - 3.0, a.min.y + bot + SEAM + 3.0),
            _ => (a.max.y, b.min.y),
        };
        in_x && p.y >= y0 && p.y <= y1
    })
}

/// Width of the card that floats beside a page when comments show only for the selected text (points).
const FLOAT_WIDTH: f32 = 216.0;

/// Comments show as a floating card rather than in a margin column (#1055): the option is on,
/// balloons are shown at all, and no other markup needs the column.
fn floating_comments(app: &WordApp) -> bool {
    let s = &app.session;
    let m = &s.prefs.markup;
    s.balloons_shown() && m.comments && m.comment_display == CommentDisplay::Selected && !s.doc.comments.is_empty() && markup_width(app) <= 0.0
}

/// The comment shown when only the selected one is: the clicked balloon, else the comment whose
/// range holds the caret.
fn shown_comment(app: &WordApp) -> Option<u32> {
    app.canvas.balloon.or_else(|| wordcraft_engine::cmd::review::comment_range_at_caret(&app.session))
}

/// Width of the markup area beside each page for comment and revision balloons (points), or 0.
pub fn markup_width(app: &WordApp) -> f32 {
    // Word shows comments either in balloons (contextual) or in the Comments pane (list); Track
    // Changes Options can put every revision inline (`Session::balloons_shown`).
    let s = &app.session;
    if !s.balloons_shown() {
        return 0.0;
    }
    let m = &s.prefs.markup;
    let comments = m.comments && !s.doc.comments.is_empty() && m.comment_display == CommentDisplay::Margin;
    let [has_formats, has_moves, has_deletions] = match app.canvas.markup_facts.get() {
        Some((rev, facts)) if rev == (s.document_id(), s.rev()) => facts,
        _ => {
            use wordcraft_engine::cmd::{moves, review};
            let facts = [review::has_format_changes(&s.doc), moves::has_moves(&s.doc), review::has_deletions(&s.doc)];
            app.canvas.markup_facts.set(Some(((s.document_id(), s.rev()), facts)));
            facts
        }
    };
    let formats = m.formatting && has_formats;
    let moves = m.insertions_deletions && has_moves;
    let deletions = s.revisions_in_balloons() && has_deletions;
    if s.balloons_shown() && (comments || formats || moves || deletions) { 216.0 } else { 0.0 }
}

/// A page dimension (points) safe to lay out: finite, at least 1 pt, at most `cap`.
fn sane_dim(v: f32, cap: f32) -> f32 {
    if v.is_finite() { v.clamp(1.0, cap) } else { 72.0 }
}

/// How many pages fit side by side: as many slots of `slot_px` (a page plus its markup area, on
/// screen) as the width holds with a gap around each, like Word's Print Layout when zoomed out.
/// At least `min_cols`, never more than `pages` (or 1 when `auto` is off and `min_cols` is 1).
fn columns_for(avail_w: f32, slot_px: f32, pages: usize, min_cols: usize, auto: bool) -> usize {
    let fit = if auto && avail_w.is_finite() && slot_px.is_finite() && slot_px > 0.0 {
        // Saturating float → int cast; bounded by the page count below.
        ((avail_w - GAP) / (slot_px + GAP)).floor().max(1.0) as usize
    } else {
        1
    };
    fit.max(min_cols).min(pages.max(1)).max(1)
}

/// Place pages (sizes in points) in rows of `cols`, left to right then top to bottom, each in a slot
/// as wide as the widest page plus `markup` and centred in `avail_w`. Returns the screen-space rects
/// (relative to the content's top-left) and the content size.
fn place_pages(sizes: &[(f32, f32)], markup: f32, scale: f32, cols: usize, avail: egui::Vec2) -> (Vec<Rect>, egui::Vec2) {
    let cols = cols.max(1);
    let scale = if scale.is_finite() { scale.clamp(0.01, 10.0) } else { PX_PER_PT };
    let markup = if markup.is_finite() { markup.clamp(0.0, 10_000.0) } else { 0.0 };
    let avail = vec2(if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 }, if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 });
    let maxw = sizes.iter().map(|&(w, _)| sane_dim(w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let slot = maxw * scale;
    let row_w = cols as f32 * slot + (cols as f32 - 1.0) * GAP;
    let content_w = (row_w + 2.0 * GAP).max(avail.x);
    let mut rects = Vec::with_capacity(sizes.len());
    let mut y = GAP;
    for row in sizes.chunks(cols) {
        let mut x = (content_w - row_w) / 2.0;
        let mut h = 0.0f32;
        for &(w, ph) in row {
            let (w, ph) = (sane_dim(w, 1e5), sane_dim(ph, 1e6));
            // Pages narrower than the widest keep their markup area beside them.
            let px = x + (maxw - markup - w).max(0.0) * scale / 2.0;
            rects.push(Rect::from_min_size(pos2(px, y), vec2(w * scale, ph * scale)));
            h = h.max(ph * scale);
            x += slot + GAP;
        }
        y += h + GAP;
    }
    (rects, vec2(content_w, y.max(avail.y)))
}

/// Reading columns use readable text at the user's chosen zoom. The window holds roughly 3–5
/// narrow, full-height columns, and a resize reflows the text, not just the paper scale.
fn reading_size(avail: egui::Vec2, scale: f32, requested: Option<f32>) -> (f32, f32) {
    let width = if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 };
    let height = if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 };
    let px = if let Some(pt) = requested.filter(|v| v.is_finite()) {
        // Keep a selected full-page column usable on a narrow window, not clipped offscreen.
        (pt.clamp(144.0, 612.0) * scale).min((width - 2.0 * GAP).max(144.0))
    } else {
        let count = (width / 295.0).round().clamp(1.0, 5.0);
        ((width - (count + 1.0) * GAP) / count).max(192.0)
    };
    (px / scale, (height - 2.0 * GAP).max(192.0) / scale)
}

/// A single horizontal row of reflowed reading pages; no wrapping at the viewport edge.
fn place_read_pages(sizes: &[(f32, f32)], scale: f32, avail: egui::Vec2) -> (Vec<Rect>, egui::Vec2) {
    let mut x = GAP;
    let mut rects = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        let w = sane_dim(w, 20_000.0) * scale;
        let h = sane_dim(h, 20_000.0) * scale;
        rects.push(Rect::from_min_size(pos2(x, GAP), vec2(w, h)));
        x += w + GAP;
    }
    // Extra trailing canvas allows even the last column to sit directly beside the nav pane.
    let end = rects.last().map_or(avail.x, |p| p.left() + avail.x - GAP);
    (rects, vec2(x.max(end).max(avail.x), avail.y.max(0.0)))
}

/// Text at the start of a reading page. Unlike the editing caret, this records what the reader
/// can currently see, even when the caret is elsewhere in the document.
fn reading_page_start(layout: &DocLayout, page: usize) -> Option<Pos> {
    let p = layout.pages.get(page)?;
    p.items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, para, l0, .. } => Some(Pos::new(StoryRef::Body, path.clone(), para.lines.get(*l0)?.start)),
            _ => None,
        })
        .or_else(|| Some(Pos::body(p.first_block, 0)))
}

/// Resolve a reading anchor after page breaks change. Preserves the first visible text instead
/// of jumping to the editing caret (which may be thousands of columns away).
fn reading_anchor_page(layout: &DocLayout, anchor: &Pos) -> Option<usize> {
    layout.caret_on(anchor, 0).map(|c| c.page)
}

/// Turn one reading column left/right. Multiple ticks during an animation queue additional
/// columns, rather than rounding the partially animated position and losing a tick.
pub(crate) fn turn_read_page(app: &mut WordApp, direction: f32) {
    let step = app.session.view.read_width * app.canvas.scale + GAP;
    if step.is_finite() && step > 0.0 {
        let current = app.canvas.read_target_page.unwrap_or_else(|| (app.canvas.scroll_offset.x / step).round().max(0.0) as usize);
        let next = if direction < 0.0 { current.saturating_sub(1) } else { current.saturating_add(1) };
        app.canvas.read_target_page = Some(next.min(app.canvas.cols.saturating_sub(1)));
    }
}

/// Ease toward an exact column boundary; a late frame never overshoots. The final offset
/// is snapped exactly, so stopping leaves no partially visible column.
fn reading_animation_step(current: f32, target: f32, dt: f32) -> f32 {
    if !current.is_finite() || !target.is_finite() {
        return target.max(0.0);
    }
    let t = 1.0 - (-18.0 * dt.clamp(0.0, 0.1)).exp();
    let next = current + (target - current) * t;
    if (target - next).abs() <= 0.5 { target } else { next }
}

/// One mechanical wheel notch turns one column; precise touchpads accumulate to a turn.
/// Do not feed reading input into the editor's animated pixel scroller.
fn read_wheel_turns(events: &[egui::Event], hovered: bool, accum: &mut f32) -> i32 {
    if !hovered {
        return 0;
    }
    let mut turns = 0i32;
    for event in events {
        if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
            if modifiers.command || modifiers.ctrl || !delta.is_finite() {
                continue;
            }
            let d = delta.x + delta.y;
            match unit {
                egui::MouseWheelUnit::Line => {
                    // Browsers typically emit three lines for one physical notch.
                    let n = if cfg!(target_arch = "wasm32") { (d / 3.0).round() } else { d.round() };
                    turns = turns.saturating_sub(n.clamp(-5.0, 5.0) as i32);
                    *accum = 0.0;
                }
                egui::MouseWheelUnit::Page => {
                    turns -= d.signum() as i32;
                    *accum = 0.0;
                }
                egui::MouseWheelUnit::Point => {
                    *accum = (*accum + d).clamp(-300.0, 300.0);
                    if accum.abs() >= 55.0 {
                        turns -= accum.signum() as i32;
                        // Discard excess pixels: a large wheel tick must not turn twice on
                        // the next event, while small touchpad deltas still add up to a turn.
                        *accum = 0.0;
                    }
                }
            }
        }
    }
    turns.clamp(-5, 5)
}

pub fn geometry(app: &WordApp, l: &DocLayout, avail: egui::Vec2) -> Geometry {
    let v = &app.session.view;
    let markup = markup_width(app);
    let maxw = l.pages.iter().map(|p| sane_dim(p.w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let maxh = l.pages.iter().map(|p| sane_dim(p.h, 20_000.0)).fold(0.0f32, f32::max).max(72.0);
    let web = v.mode != wordcraft_layout::ViewMode::Print;
    let mut scale = v.zoom.clamp(0.1, 5.0) * PX_PER_PT;
    if v.read_mode {
        scale = v.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        let sizes: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.w, p.h)).collect();
        let (rects, size) = place_read_pages(&sizes, scale, avail);
        // Reading columns show whole pages (Hide White Space doesn't apply).
        let bands = rects.iter().map(|r| (0.0, r.height())).collect();
        return Geometry { rects, size, scale, cols: sizes.len().max(1), bands };
    }
    match v.fit.as_str() {
        "pageWidth" => scale = ((avail.x - 2.0 * GAP - 20.0) / maxw).clamp(0.1, 6.0),
        "onePage" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 2.0 * GAP) / maxw).clamp(0.05, 6.0),
        "multiplePages" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 3.0 * GAP) / (2.0 * maxw)).clamp(0.05, 6.0),
        _ => {}
    }
    if web {
        scale = PX_PER_PT * v.zoom.clamp(0.1, 5.0);
    }
    if !scale.is_finite() {
        scale = PX_PER_PT;
    }
    // Pages flow side by side in Print Layout whenever more than one fits across; One Page and
    // Page Width show one per row (and so does a zoom after One Page, #768), Web Layout has no pages. Reading Mode is handled above.
    let (min_cols, auto) = if web || v.fit == "onePage" || v.fit == "pageWidth" || v.single_column {
        (1, false)
    } else if v.multi_page {
        (2, true)
    } else {
        (1, true)
    };
    let cols = columns_for(avail.x, maxw * scale, l.pages.len(), min_cols, auto);
    let sizes: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.w, p.h)).collect();
    let (mut rects, mut size) = place_pages(&sizes, markup, scale, cols, avail);
    let mut bands: Vec<(f32, f32)> = rects.iter().map(|r| (0.0, r.height())).collect();
    let editing_part = matches!(app.session.sel.focus.story, StoryRef::Part(_));
    if v.hide_white_space && v.mode == wordcraft_layout::ViewMode::Print && !v.read_mode && cols == 1 && !editing_part && rects.len() > 1 {
        let bodies: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.body.y, p.body.bottom())).collect();
        let (r, b, h) = collapse_white_space(&rects, &bodies, scale, avail.y);
        (rects, bands) = (r, b);
        size.y = h;
    }
    Geometry { rects, size, scale, cols, bands }
}

/// While a fit mode (Page Width, One Page, Multiple Pages) sizes the page, keep the session's zoom
/// equal to what is shown, as Word does. Zoom In/Out, the Zoom dialog and `view.state` then start
/// from the visible zoom; before, they stepped from the stale manual zoom, so Zoom In from a 163%
/// Page Width jumped to 110% (issue #67).
pub fn sync_fit_zoom(app: &mut WordApp, scale: f32) {
    let v = &mut app.session.view;
    if v.fit.is_empty() || v.read_mode || v.mode != wordcraft_layout::ViewMode::Print || !scale.is_finite() {
        return;
    }
    v.zoom = (scale / PX_PER_PT).clamp(0.1, 5.0);
}

/// Fingerprint of everything outside the layout that changes how pages render: the view's
/// marks and markup, dark mode, the page colour and watermark, header/body dimming, the media.
/// Taken once per frame.
fn view_key(app: &WordApp, dim_body: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let v = &app.session.view;
    (v.marks, app.session.prefs.marks, v.show_markup, v.dark_mode, wordcraft_render::DARK_PAPER, v.hide_ink, v.draw.replay).hash(&mut h);
    app.session.prefs.markup.hash(&mut h);
    dim_body.hash(&mut h);
    format!("{:?}{:?}{:?}", app.session.doc.settings.page_color, app.session.doc.settings.page_picture, app.session.doc.settings.watermark)
        .hash(&mut h);
    app.session.doc.media.len().hash(&mut h);
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
                Placed::Shape { rect, kind, fill, stroke, stroke_width, effects, freeform, spin, extra } => {
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
    layout: Option<std::sync::Arc<DocLayout>>,
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
fn page_key(app: &mut WordApp, layout: &std::sync::Arc<DocLayout>, i: usize, view: u64, scale_px: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (view, scale_px.to_bits(), app.canvas.page_keys.content(layout, i)).hash(&mut h);
    h.finish()
}

fn to_screen(origin: Pos2, page_rect: Rect, scale: f32, x: f32, y: f32) -> Pos2 {
    pos2(origin.x + page_rect.min.x + x * scale, origin.y + page_rect.min.y + y * scale)
}

/// A page raster as an egui image. Both hold premultiplied RGBA8, so the pixels go across as they
/// are: un-premultiplying them for egui to premultiply again gave the same image for two more
/// passes over every pixel of the page, on each keystroke.
fn page_image(img: &wordcraft_render::Rendered) -> egui::ColorImage {
    egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels)
}

/// Place a raster at a physical-pixel-aligned origin and its exact device-pixel size.
fn texel_aligned_rect(layout: Rect, texture_px: egui::Vec2, ppp: f32) -> Rect {
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_size(pos2(snap(layout.min.x), snap(layout.min.y)), texture_px / ppp)
}

/// Tile indices intersecting the view, in a fixed grid of raster pixels. The grid follows the
/// document, not the viewport, so scrolling never changes tile contents or their cache keys.
fn visible_tiles(page_rect: Rect, clip: Rect, screen_scale: f32, raster_scale: f32, tile_height_px: usize) -> std::ops::Range<usize> {
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
const DRAWING_GRID_MIN_PX: f32 = 4.0;
/// Most grid lines drawn each way on one page (page sizes come from files).
const DRAWING_GRID_MAX_LINES: usize = 2000;

/// The View › Gridlines drawing grid over a page's text area, in document points: the x of each
/// vertical line and the y of each horizontal line, starting at the margins and `step` (across,
/// down) apart, never outside the margins. Drawn on screen only — never printed or exported.
fn drawing_grid(body: wordcraft_geom::Rect, step: (f32, f32)) -> (Vec<f32>, Vec<f32>) {
    let axis = |start: f32, len: f32, step: f32| -> Vec<f32> {
        if !(step.is_finite() && step > 0.0 && start.is_finite() && len.is_finite() && len >= 0.0) {
            return Vec::new();
        }
        let end = start + len + 0.01;
        (0..DRAWING_GRID_MAX_LINES).map(|k| start + k as f32 * step).take_while(|p| *p <= end).collect()
    };
    (axis(body.x, body.w, step.0), axis(body.y, body.h, step.1))
}

/// The cells Table Layout › View Gridlines outlines on a page: the body's tables and the
/// header's and footer's (#1628), skipping cells covered by a merge.
fn gridline_cells(page: &Page) -> impl Iterator<Item = wordcraft_geom::Rect> + '_ {
    page.items.iter().chain(&page.header).chain(&page.footer).filter_map(|it| match it {
        Placed::Cell { rect, covered: false, .. } => Some(*rect),
        _ => None,
    })
}

/// The drawing grid's spacing in points at a screen scale: the document's spacing (`w:drawingGrid*Spacing`;
/// ECMA-376's 1/8 inch when it doesn't say), doubled until the lines are at least
/// [`DRAWING_GRID_MIN_PX`] apart on screen.
fn drawing_grid_step(spacing: f32, scale: f32) -> f32 {
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

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    // Full pagination is expensive. During a resize keep the already laid-out columns and
    // reflow once the viewport has been stable briefly; ordinary reading controls update now.
    const RESIZE_SETTLE: f64 = 0.16;
    let mut reflow_anchor: Option<Pos> = None;
    if app.session.view.read_mode {
        let now = ui.input(|i| i.time);
        let resized = app.canvas.read_viewport.is_none_or(|v| (v - full.size()).length_sq() > 0.5);
        let entering = app.canvas.read_viewport.is_none();
        if resized {
            app.canvas.read_viewport = Some(full.size());
            app.canvas.read_resized_at = now;
        }
        let scale = app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        let (w, h) = reading_size(full.size(), scale, app.session.view.read_column_width);
        let changed = (app.session.view.read_width - w).abs() > 0.5 || (app.session.view.read_height - h).abs() > 0.5;
        let forced = std::mem::take(&mut app.canvas.read_force_reflow) || entering;
        if changed && (forced || now - app.canvas.read_resized_at >= RESIZE_SETTLE) {
            if !entering {
                reflow_anchor = app.canvas.read_first_visible.clone();
            }
            app.session.view.read_width = w;
            app.session.view.read_height = h;
            app.canvas.scroll_to_caret = entering;
        } else if changed {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(170));
        }
        if forced && !entering && reflow_anchor.is_none() {
            // A spacing change can repaginate without changing physical page dimensions.
            reflow_anchor = app.canvas.read_first_visible.clone();
        }
    } else {
        app.canvas.read_viewport = None;
        app.canvas.read_force_reflow = false;
        app.canvas.read_first_visible = None;
        app.canvas.read_target_page = None;
    }
    // Keys first, so this frame lays out and paints what was typed (handled after painting, a
    // keystroke showed one frame later). Focus is the canvas's from the last frame.
    // While "Save changes?" is up, keys answer it rather than edit the document behind it.
    if app.canvas.focused && !matches!(app.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })) {
        crate::keys::canvas_events(app, ui.ctx());
    } else {
        // Alt+numpad entry doesn't survive the canvas losing the keyboard.
        app.canvas.alt_code.clear();
    }
    crate::ink::replay_tick(app, ui.ctx());
    let layout = app.session.layout();
    if let Some(anchor) = reflow_anchor
        && let Some(page) = reading_anchor_page(&layout, &anchor)
    {
        let step = app.session.view.read_width * app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT + GAP;
        app.canvas.read_target_page = None;
        app.canvas.scroll_request = Some(vec2(page as f32 * step, 0.0));
    }
    // View › Switch Modes: dark paper, inverted document colours.
    let dark_page = app.session.view.dark_mode;
    let dark_paper = wordcraft_render::DARK_PAPER;
    let show_ruler = app.session.view.ruler && app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode;
    let (hruler, vruler, area) = if show_ruler {
        let h = Rect::from_min_max(pos2(full.min.x + RULER, full.min.y), pos2(full.max.x, full.min.y + RULER));
        let v = Rect::from_min_max(pos2(full.min.x, full.min.y + RULER), pos2(full.min.x + RULER, full.max.y));
        (Some(h), Some(v), Rect::from_min_max(pos2(full.min.x + RULER, full.min.y + RULER), full.max))
    } else {
        (None, None, full)
    };
    app.canvas.canvas_rect = Some(area);
    // Ctrl/⌘ + wheel (and trackpad pinch) zooms the document, like Word. egui reports both as a
    // zoom factor and doesn't scroll for them. Start from the shown scale so fit modes zoom too.
    // With animation effects off the wheel zooms in whole steps, each at once.
    let animate = app.animations_on();
    let opts = ui.ctx().options(|o| o.input_options);
    let zoom_delta = ui.input(|i| app.canvas.wheel.zoom(i, &opts, animate));
    // The page point under the pointer before the zoom: it is kept under the pointer (#1223).
    let mut zoom_anchor = None;
    if (zoom_delta - 1.0).abs() > 1e-3 && ui.rect_contains_pointer(area) {
        let reading = app.session.view.read_mode;
        if !reading && let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
            zoom_anchor = page_at(&app.canvas.page_rects, &layout, app.canvas.scale, p).map(|at| (p, at));
        }
        let pct = (app.canvas.scale / PX_PER_PT * zoom_delta * 100.0).clamp(if reading { 75.0 } else { 10.0 }, if reading { 150.0 } else { 500.0 });
        let id = if reading { "view.readTextSize" } else { "view.zoom" };
        let _ = app.run(id, json!({"value": pct}));
    }
    let avail = if app.session.view.read_mode { area.size() } else { area.size() - vec2(14.0, 0.0) };
    let geo = geometry(app, &layout, avail);
    // A pane or toolbar resize changes the reachable last column immediately. Navigation must
    // clamp against this frame's geometry, not the previous frame's scroll maximum.
    if app.session.view.read_mode {
        app.canvas.scroll_max = (geo.size - area.size()).max(egui::Vec2::ZERO);
    }
    // The status bar's zoom readout and slider were laid out before this, from the last frame's
    // scale: after a zoom change draw again at once, so they follow in the next frame rather than
    // whenever something else next repaints (the caret's blink).
    if (geo.scale - app.canvas.scale).abs() > 1e-6 {
        ui.ctx().request_repaint();
    }
    app.canvas.scale = geo.scale;
    sync_fit_zoom(app, geo.scale);
    // A pointer zoom scrolls so the same page point is under the pointer at the new scale, also
    // when the pages reflow into rows. Without this the scroll offset stayed in pixels and the page
    // slid away under the pointer.
    let anchored = if let Some((p, (page, x, y))) = zoom_anchor
        && let (Some(pr), Some(lp), Some(vp)) = (geo.rects.get(page), layout.pages.get(page), app.canvas.viewport)
    {
        let s = page_screen_scale(*pr, lp, geo.scale);
        app.canvas.scroll_max = (geo.size - vp.size()).max(egui::Vec2::ZERO);
        app.canvas.scroll_request = Some(pr.min.to_vec2() + vec2(x, y) * s - (p - vp.min));
        true
    } else {
        false
    };
    // Zooming or resizing reflowed the pages into a different number of columns: the caret's page
    // moved, so bring it back into view rather than leave the reader somewhere else. A zoom at the
    // pointer keeps the reader where the pointer is instead.
    if geo.cols != app.canvas.cols {
        app.canvas.cols = geo.cols;
        if !app.session.view.read_mode && !anchored {
            app.canvas.scroll_to_caret = true;
        }
    }
    // Where the selection really ends, also when it took whole table cells.
    let caret = layout.caret_on(&app.session.sel_ends().focus, app.session.page_hint);
    // Editing a header/footer (or a note) dims the body; once per frame, for every page.
    let dim_body = dims_body(app, &layout);
    if let Some(c) = caret
        && !app.session.selects_whole_story()
    {
        app.session.page_hint = c.page;
    }
    let mut scroll_target: Option<Rect> = None;
    if app.canvas.nav_to_caret {
        app.canvas.read_target_page = None;
        if let Some(c) = caret
            && let Some(pr) = geo.rects.get(c.page)
        {
            let x = (pr.left() - GAP).max(0.0);
            let y = if app.session.view.read_mode { 0.0 } else { (pr.top() + c.top * geo.scale - 40.0).max(0.0) };
            app.canvas.scroll_request = Some(vec2(x, y));
        }
        app.canvas.nav_to_caret = false;
    } else if app.canvas.scroll_to_caret && app.session.view.read_mode && app.canvas.scroll_request.is_none() {
        if let Some(c) = caret
            && let Some(pr) = geo.rects.get(c.page)
        {
            // Ordinary caret-follow only reveals an offscreen position, as in Print Layout.
            // A click in a visible column must not align that whole column to the left.
            let x = pr.left() + c.x * geo.scale;
            let left = app.canvas.scroll_offset.x;
            if x < left || x > left + area.width() {
                app.canvas.read_target_page = None;
                app.canvas.scroll_request = Some(vec2((pr.left() - GAP).max(0.0), 0.0));
            }
        }
    } else if app.canvas.scroll_to_caret
        && let Some(c) = caret
        && let Some(pr) = geo.rects.get(c.page)
    {
        let r = Rect::from_min_size(
            pos2(pr.min.x + c.x * geo.scale, pr.min.y + c.top * geo.scale),
            vec2((c.width * geo.scale).max(2.0), c.height * geo.scale),
        );
        scroll_target = Some(r.expand2(vec2(40.0, 60.0)));
    }
    app.canvas.scroll_to_caret = false;
    let mut origin = area.min;
    let mut ui_area = ui.new_child(egui::UiBuilder::new().max_rect(area));
    // The canvas scrolls itself on wheel/touchpad input (crate::scroll): touchpads 1:1 with
    // momentum, wheel notches eased in (or all at once with animation effects off). Applied
    // before drawing, so it shows this frame. In reading mode, wheel and touchpad events turn
    // whole columns instead.
    let hovered = wheel_hovered(ui.rect_contains_pointer(area), ui.ctx().dragged_id().is_some(), app.canvas.drag_selecting());
    let notch = crate::scroll::notch_px(app.canvas.scale / PX_PER_PT);
    let wheel_delta = if app.session.view.read_mode {
        let turns = ui.input(|i| read_wheel_turns(&i.events, hovered, &mut app.canvas.read_wheel_accum));
        for _ in 0..turns.unsigned_abs() {
            turn_read_page(app, turns.signum() as f32);
        }
        if let Some(page) = app.canvas.read_target_page {
            let step = app.session.view.read_width * geo.scale + GAP;
            let x = (page as f32 * step).min(app.canvas.scroll_max.x);
            let dt = ui.input(|i| i.stable_dt);
            let next = if animate { reading_animation_step(app.canvas.scroll_offset.x, x, dt) } else { x };
            app.canvas.scroll_request = Some(vec2(next, 0.0));
            if next == x {
                app.canvas.read_target_page = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        // Discard any pixel-scroll inertia left from editing mode.
        app.canvas.wheel = Default::default();
        egui::Vec2::ZERO
    } else {
        app.canvas.read_wheel_accum = 0.0;
        ui.input(|i| app.canvas.wheel.frame(i, &opts, hovered, notch, area.height(), animate))
    };
    // Dragging a selection near the viewport's edge scrolls on (#688): from a crawl at the margin
    // to full speed at the edge, through the same offset the wheel writes.
    let (down, released, pointer, dt) = ui.input(|i| (i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.latest_pos(), i.stable_dt));
    // A release `mouse` returned before seeing (double-click paths, off the canvas) ends the drag here (#1019).
    if app.canvas.dragging && !down && !released {
        app.canvas.dragging = false;
        app.canvas.column_drag = None;
        app.canvas.unit_drag = None;
    }
    let auto_delta = if !app.session.view.read_mode
        && app.canvas.drag_selecting()
        && down
        && let Some(vp) = app.canvas.viewport
        && let Some(p) = pointer
    {
        vec2(edge_scroll(p.x, vp.min.x, vp.max.x, dt), edge_scroll(p.y, vp.min.y, vp.max.y, dt))
    } else {
        egui::Vec2::ZERO
    };
    // Middle-click scrolling (#1065): Print Layout and the other editing views, not Read Mode.
    let middle_delta = if app.session.view.read_mode || app.dialog.is_some() {
        app.canvas.auto_scroll.cancel();
        egui::Vec2::ZERO
    } else {
        app.canvas.auto_scroll.update(ui, area)
    };
    let delta = wheel_delta + auto_delta + middle_delta;
    let mut scroll_area = egui::ScrollArea::both()
        .id_salt("canvas_scroll")
        .auto_shrink([false, false])
        // Bringing the caret into view glides there, or jumps with animation effects off.
        .animated(animate)
        .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: false, ..Default::default() });
    if app.session.view.read_mode {
        scroll_area = scroll_area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    }
    if delta != egui::Vec2::ZERO || app.canvas.scroll_request.is_some() || app.session.view.read_mode {
        let before = app.canvas.scroll_offset;
        let step = app.session.view.read_width * geo.scale + GAP;
        let snapped = if step.is_finite() && step > 0.0 { vec2((before.x / step).round() * step, 0.0) } else { egui::Vec2::ZERO };
        let after = match app.canvas.scroll_request.take() {
            Some(request) => request.clamp(egui::Vec2::ZERO, app.canvas.scroll_max),
            None if app.session.view.read_mode => snapped.clamp(egui::Vec2::ZERO, app.canvas.scroll_max),
            None => clamped_scroll(before, delta, app.canvas.scroll_max),
        };
        if after == before && wheel_delta != egui::Vec2::ZERO {
            app.canvas.wheel.hit_edge();
        }
        if auto_delta != egui::Vec2::ZERO && after != before {
            // The scroll keeps going without new input.
            ui.ctx().request_repaint();
        }
        scroll_area = scroll_area.scroll_offset(after);
    }
    if app.canvas.wheel.is_animating() {
        ui.ctx().request_repaint();
    }
    let out = scroll_area.show_viewport(&mut ui_area, |ui, viewport| {
        origin = ui.min_rect().min - viewport.min.to_vec2();
        let content = Rect::from_min_size(ui.min_rect().min, geo.size);
        let resp = ui.allocate_rect(content, Sense::click_and_drag());
        if let Some(tr) = scroll_target {
            ui.scroll_to_rect(Rect::from_min_size(ui.min_rect().min + tr.min.to_vec2(), tr.size()), None);
        }
        let painter = ui.painter_at(ui.clip_rect());
        let ppp = ui.ctx().pixels_per_point();
        let max_tex = ui.ctx().input(|i| i.max_texture_side).min(wordcraft_render::MAX_SIDE as usize).max(1) as f32;
        let mut visible_tile_keys = HashSet::new();
        let mut rendered = 0;
        let t0 = crate::now_ms();
        let mut rects = Vec::with_capacity(geo.rects.len());
        let content_origin = ui.min_rect().min;
        let view = view_key(app, dim_body);
        for (i, pr) in geo.rects.iter().enumerate() {
            let sr = pr.translate(content_origin.to_vec2());
            let Some(page) = layout.pages.get(i) else { continue };
            let (band_top, band_bot) = geo.bands.get(i).copied().unwrap_or((0.0, sr.height()));
            // A Web/Draft page can be arbitrarily tall. Shrinking the whole page into one GPU
            // texture makes the text blurry; split those pages into native-resolution tiles.
            let width_scale = (geo.scale * ppp).min(max_tex / page.w.max(1.0)).max(0.05);
            let tiled = page.h.clamp(1.0, 1e6) * width_scale > max_tex;
            let scale_px = if tiled { width_scale } else { width_scale.min(max_tex / page.h.clamp(1.0, 1e6)).max(0.05) };
            let tile_height_px = TILE_HEIGHT_PX.min(max_tex as usize).max(1);
            // Hide White Space shows only a band of the page; everything below paints into it.
            let band_of = |r: Rect| Rect::from_min_max(pos2(r.min.x - 4.0, r.min.y + band_top), pos2(r.max.x + 4.0, r.min.y + band_bot));
            let visible = band_of(sr).intersects(ui.clip_rect().expand(200.0));
            // Only pages on screen or with a texture need their key (#811).
            let cached = !tiled && app.canvas.textures.contains_key(&i);
            let key =
                if visible || cached { page_key(app, &layout, i, view, scale_px) ^ if tiled { (tile_height_px as u64) << 32 } else { 0 } } else { 0 };
            let mut fresh = cached && app.canvas.textures.get(&i).is_some_and(|(k, _)| *k == key);
            // Render before placing the page: a page re-rendered in this frame (a zoom step, an
            // edit) then sits on the pixel grid in this frame too, rather than a frame later.
            if visible && !tiled && !fresh && (rendered < 2 || !app.canvas.textures.contains_key(&i) && rendered < 4) {
                let opts = page_render_options(app, dim_body);
                let img = wordcraft_render::render_page(&app.session.doc, page, scale_px, &opts);
                let ci = page_image(&img);
                match app.canvas.textures.get_mut(&i) {
                    Some((k, tex)) => {
                        tex.set(ci, egui::TextureOptions::LINEAR);
                        *k = key;
                    }
                    None => {
                        let tex = ui.ctx().load_texture(format!("page{i}"), ci, egui::TextureOptions::LINEAR);
                        app.canvas.textures.insert(i, (key, tex));
                    }
                }
                rendered += 1;
                fresh = true;
            } else if visible && !tiled && !fresh {
                ui.ctx().request_repaint();
            }
            let current_scale = geo.scale * ppp;
            let visual_sr = if fresh && (scale_px - current_scale).abs() <= 0.0001_f32.max(current_scale.abs() * 0.0001) {
                app.canvas.textures.get(&i).map_or(sr, |(_, tex)| texel_aligned_rect(sr, tex.size_vec2(), ppp))
            } else {
                sr
            };
            rects.push(visual_sr);
            if !visible {
                continue;
            }
            let band = band_of(visual_sr);
            let painter = if band_top <= 0.0 && band_bot >= sr.height() {
                painter.clone()
            } else {
                painter.with_clip_rect(painter.clip_rect().intersect(band))
            };
            // Shadow and paper.
            painter.rect_filled(visual_sr.translate(vec2(0.0, 2.0)).expand(1.5), 1.0, t.page_shadow);
            painter.rect_filled(visual_sr, 0.0, if dark_page { Color32::from_gray(dark_paper) } else { Color32::WHITE });
            if tiled {
                // Render before painting to keep the frame sharp immediately after an edit or
                // zoom. Tiles are fixed to raster-pixel boundaries, independent of scrolling.
                app.canvas.textures.remove(&i);
                let total_px = (page.h.clamp(1.0, 1e6) * scale_px).ceil() as usize;
                let screen_per_raster_px = geo.scale / scale_px;
                let base = pos2((sr.min.x * ppp).round() / ppp, (sr.min.y * ppp).round() / ppp);
                for tile in visible_tiles(sr, ui.clip_rect().expand(200.0).intersect(band), geo.scale, scale_px, tile_height_px) {
                    let start_px = tile.saturating_mul(tile_height_px);
                    let height_px = total_px.saturating_sub(start_px).min(tile_height_px);
                    if height_px == 0 {
                        continue;
                    }
                    let id = (i, tile);
                    visible_tile_keys.insert(id);
                    let fresh = app.canvas.tiles.get(&id).is_some_and(|(k, _)| *k == key);
                    if !fresh && rendered < 4 {
                        // The tiny subtraction avoids a 1px overshoot from round-tripping f32
                        // pixels through page points and render_area's ceil() conversion.
                        let img = wordcraft_render::render_area(
                            &app.session.doc,
                            page,
                            0.0,
                            start_px as f32 / scale_px,
                            page.w,
                            (height_px as f32 - 0.01).max(0.01) / scale_px,
                            scale_px,
                            &page_render_options(app, dim_body),
                        );
                        let ci = page_image(&img);
                        match app.canvas.tiles.get_mut(&id) {
                            Some((k, tex)) => {
                                tex.set(ci, egui::TextureOptions::LINEAR);
                                *k = key;
                            }
                            None => {
                                let tex = ui.ctx().load_texture(format!("page{i}-tile{tile}"), ci, egui::TextureOptions::LINEAR);
                                app.canvas.tiles.insert(id, (key, tex));
                            }
                        }
                        rendered += 1;
                    } else if !fresh {
                        ui.ctx().request_repaint();
                    }
                    if let Some((_, tex)) = app.canvas.tiles.get(&id) {
                        let rect = Rect::from_min_size(
                            pos2(base.x, base.y + start_px as f32 * screen_per_raster_px),
                            vec2(tex.size_vec2().x * screen_per_raster_px, height_px as f32 * screen_per_raster_px),
                        );
                        painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
            } else if let Some((_, tex)) = app.canvas.textures.get(&i) {
                painter.image(tex.id(), visual_sr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            let page_scale = page_screen_scale(visual_sr, page, geo.scale);
            // Header/footer editing chrome.
            if let StoryRef::Part(id) = app.session.sel.focus.story {
                let sect_body = page.body;
                if page.header_story == Some(id) || page.footer_story == Some(id) {
                    let hy = visual_sr.min.y + sect_body.y * page_scale - 4.0;
                    let fy = visual_sr.min.y + sect_body.bottom() * page_scale + 4.0;
                    for (y, label) in [(hy, "Header"), (fy, "Footer")] {
                        dashed(&painter, pos2(visual_sr.min.x, y), pos2(visual_sr.max.x, y), Stroke::new(1.0, t.accent));
                        let tr = Rect::from_min_size(pos2(visual_sr.min.x + 2.0, if label == "Header" { y } else { y - 18.0 }), vec2(52.0, 18.0));
                        painter.rect_filled(tr, 2.0, t.checked);
                        painter.text(tr.center(), egui::Align2::CENTER_CENTER, tl!(label), regular(11.0), t.accent_text);
                    }
                }
            }
            // View › Gridlines: the drawing grid over the text area (Print Layout, on screen only).
            if app.session.view.gridlines && app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode {
                let settings = &app.session.doc.settings;
                // A section with a document grid (`w:docGrid`) shows that grid: lines at the line
                // pitch and, for a character grid, columns at the character pitch.
                let sections = app.session.doc.sections();
                let sect = sections.get(page.section).map(|(_, s)| *s);
                let step = match sect.and_then(|s| s.grid_line_pitch().map(|lp| (s, lp))) {
                    Some((s, lp)) => {
                        let size = app.session.doc.styles.resolve_char(Some("Normal"), &Default::default()).size;
                        let cp = s.grid_char_pitch(size).map_or(f32::INFINITY, |cp| drawing_grid_step(cp, page_scale));
                        (cp, drawing_grid_step(lp, page_scale))
                    }
                    None => (drawing_grid_step(settings.grid_h, page_scale), drawing_grid_step(settings.grid_v, page_scale)),
                };
                let (xs, ys) = drawing_grid(page.body, step);
                let stroke = Stroke::new(1.0, t.blue.linear_multiply(0.22));
                let (x0, x1) = (visual_sr.min.x + page.body.x * page_scale, visual_sr.min.x + page.body.right() * page_scale);
                let (y0, y1) = (visual_sr.min.y + page.body.y * page_scale, visual_sr.min.y + page.body.bottom() * page_scale);
                for x in xs {
                    let sx = visual_sr.min.x + x * page_scale;
                    painter.line_segment([pos2(sx, y0), pos2(sx, y1)], stroke);
                }
                for y in ys {
                    let sy = visual_sr.min.y + y * page_scale;
                    painter.line_segment([pos2(x0, sy), pos2(x1, sy)], stroke);
                }
            }
            // Table Layout › View Gridlines: table cell outlines (on screen only).
            if app.session.view.table_gridlines {
                for rect in gridline_cells(page) {
                    let r = Rect::from_min_size(
                        pos2(visual_sr.min.x + rect.x * page_scale, visual_sr.min.y + rect.y * page_scale),
                        vec2(rect.w * page_scale, rect.h * page_scale),
                    );
                    painter.rect_stroke(r, 0.0, Stroke::new(0.5, t.blue.linear_multiply(0.6)), egui::StrokeKind::Middle);
                }
            }
        }
        // Hide White Space: a thin line where one page ends and the next begins.
        for (i, pair) in rects.windows(2).enumerate() {
            let Some(&(_, bot)) = geo.bands.get(i) else { continue };
            if white_space_hidden(&geo.rects, &geo.bands) {
                let y = pair[0].min.y + bot;
                painter.rect_filled(Rect::from_min_max(pos2(pair[0].min.x, y), pos2(pair[0].max.x, y + SEAM)), 0.0, t.border_strong);
            }
        }
        if rendered > 0 {
            app.canvas.render_ms = crate::now_ms() - t0;
        }
        // Drop textures of pages that no longer exist, and of pages scrolled well out of view
        // (#811): scrolling through a long document kept every page it had shown in memory.
        let n = layout.pages.len();
        let clip = ui.clip_rect();
        let near = clip.expand2(vec2(clip.width(), clip.height() * TEXTURE_KEEP_SCREENS));
        app.canvas.textures.retain(|k, _| *k < n && geo.rects.get(*k).is_some_and(|r| r.translate(content_origin.to_vec2()).intersects(near)));
        app.canvas.tiles.retain(|id, _| visible_tile_keys.contains(id));
        app.canvas.page_rects = rects.clone();
        let view_rect = ui.clip_rect();
        if floating_comments(app) {
            // Its own layer, so presses on the card never reach the canvas under it.
            let ctx = ui.ctx().clone();
            egui::Area::new(egui::Id::new("floating_comment")).order(egui::Order::Foreground).fixed_pos(Pos2::ZERO).sense(Sense::hover()).show(
                &ctx,
                |aui| {
                    aui.set_clip_rect(view_rect);
                    let ap = aui.painter().clone();
                    balloons(app, aui, &ap, &rects, &layout, geo.scale, view_rect);
                },
            );
        } else {
            balloons(app, ui, &painter, &rects, &layout, geo.scale, view_rect);
        }
        // Find › Reading Highlight.
        let marks: Vec<(Pos, Pos)> = app.session.find_highlights().iter().take(5000).cloned().collect();
        for (a, b) in &marks {
            for (pi, r) in layout.selection_rects(&app.session.doc, a, b, app.session.page_hint) {
                if let Some(pr) = rects.get(pi) {
                    let scale = layout.pages.get(pi).map_or(geo.scale, |page| page_screen_scale(*pr, page, geo.scale));
                    let hr = Rect::from_min_size(pos2(pr.min.x + r.x * scale, pr.min.y + r.y * scale), vec2(r.w * scale, r.h * scale));
                    painter.rect_filled(hr, 0.0, t.find_highlight);
                }
            }
        }
        // Selection (a selected object shows its frame instead): whole table cells shade the
        // cells (an empty one too), a column selection each row's piece, text running into or
        // across a table the whole cells of the rows it takes.
        let selected: Vec<_> = if crate::objects::selected(app).is_some() {
            Vec::new()
        } else if let Some(range) = app.session.cell_range() {
            wordcraft_engine::cmd::table_mouse::cell_rects(&layout, &app.session.doc, range)
        } else if !app.session.sel.is_collapsed() {
            match app.session.column_segments() {
                Some(segs) => segs.iter().flat_map(|(a, b)| layout.selection_rects(&app.session.doc, a, b, app.session.page_hint)).collect(),
                None => {
                    let (a, b) = app.session.sel.ordered();
                    wordcraft_engine::cmd::table_mouse::selection_rects(&layout, &app.session.doc, &a, &b, app.session.page_hint)
                }
            }
        } else {
            Vec::new()
        };
        let visible_clip = ui.clip_rect();
        let mut screen_rects: Vec<Rect> = Vec::with_capacity(selected.len());
        for (pi, r) in selected {
            if let Some(pr) = rects.get(pi) {
                let scale = layout.pages.get(pi).map_or(geo.scale, |page| page_screen_scale(*pr, page, geo.scale));
                let sr = Rect::from_min_size(pos2(pr.min.x + r.x * scale, pr.min.y + r.y * scale), vec2(r.w * scale, r.h * scale));
                painter.rect_filled(sr, 0.0, t.selection);
                screen_rects.push(sr);
            }
        }
        // Anchor only to visible selection geometry. A selection can span pages, and its
        // document-first line may be scrolled offscreen; using it would pin the toolbar to an
        // old screen position unrelated to the visible selection.
        app.canvas.mini_anchor = crate::mini_toolbar::visible_anchor(screen_rects, visible_clip);
        // Content controls: the frame of the one the caret is in (all of them in Design Mode).
        crate::content_controls::paint(app, &painter, &t, &layout, &rects, geo.scale);
        // Read Aloud: the sentence being spoken.
        if let Some((a, b)) = crate::read_aloud::highlight(app) {
            for (pi, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
                if let Some(pr) = rects.get(pi) {
                    let sr =
                        Rect::from_min_size(pos2(pr.min.x + r.x * geo.scale, pr.min.y + r.y * geo.scale), vec2(r.w * geo.scale, r.h * geo.scale));
                    painter.rect_filled(sr, 0.0, t.accent.gamma_multiply(0.16));
                    painter.hline(sr.x_range(), sr.max.y - 0.5, egui::Stroke::new(1.5, t.accent));
                }
            }
        }
        // Caret.
        let focused = resp.has_focus() || app.canvas.focused;
        // Editing an equation: shade it and draw the caret inside it.
        let mut math_caret = false;
        if let Some(m) = app.session.math.clone()
            && let Some((pi, ex, base, ml)) = layout.equation_geom(&m.at, app.session.page_hint)
            && let Some(pr) = rects.get(pi)
        {
            math_caret = true;
            let s = geo.scale;
            let zone = Rect::from_min_max(
                pos2(pr.min.x + ex * s - 2.0, pr.min.y + (base - ml.ascent) * s - 2.0),
                pos2(pr.min.x + (ex + ml.width) * s + 2.0, pr.min.y + (base + ml.descent) * s + 2.0),
            );
            painter.rect_filled(zone, 2.0, Color32::from_black_alpha(16));
            if let Some(slot) = ml.slot(&m.pos.path, m.pos.off) {
                let x = (pr.min.x + (ex + slot.x) * s).round() + 0.5;
                let y0 = pr.min.y + (base - slot.y - slot.a) * s;
                let y1 = pr.min.y + (base - slot.y + slot.d) * s;
                let since = crate::now_ms() - app.canvas.caret_visible_since;
                // The caret glides and fades like Word's (`caret_glide`).
                let on = ((since / 530.0) as u64).is_multiple_of(2);
                let look = crate::caret_glide::look(app, ui, content_origin, &geo, [pos2(x, y0), pos2(x, y1)], focused, since, on);
                if look.opacity > 0.0 && focused {
                    let caret_color = caret_color_at(app, &layout, pi, ex + slot.x, base - slot.y + (slot.d - slot.a) / 2.0, view, dim_body);
                    painter.line_segment(look.bar, Stroke::new(1.5, caret_color.gamma_multiply(look.opacity)));
                }
                if focused {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));
                    let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            rect: cr,
                            cursor_rect: cr,
                            purpose: Default::default(),
                            should_interrupt_composition: false,
                        });
                    });
                }
            }
        }
        if !math_caret
            && let Some(c) = layout.caret_on(&app.session.sel.focus, app.session.page_hint)
            && let Some(pr) = rects.get(c.page)
        {
            let scale = layout.pages.get(c.page).map_or(geo.scale, |page| page_screen_scale(*pr, page, geo.scale));
            let x = pr.min.x + c.x * scale;
            let y0 = pr.min.y + c.top * scale;
            let y1 = y0 + c.height * scale;
            let since = crate::now_ms() - app.canvas.caret_visible_since;
            let on = ((since / 530.0) as u64).is_multiple_of(2);
            // The caret contrasts with what lies under it, like Word's inverted caret (#299):
            // worked out only when something is drawn in it.
            let caret_color = |app: &mut WordApp| caret_color_at(app, &layout, c.page, c.x + c.width / 2.0, c.top + c.height / 2.0, view, dim_body);
            let bar = if c.width > 0.0 {
                // Turned text (a table cell's text direction): the caret lies across the page.
                [pos2(x, y0.round() + 0.5), pos2(x + c.width * scale, y0.round() + 0.5)]
            } else {
                [pos2(x.round() + 0.5, y0), pos2(x.round() + 0.5, y1)]
            };
            // The caret glides and fades like Word's (`caret_glide`).
            let shown = app.session.sel.is_collapsed() && focused;
            let look = crate::caret_glide::look(app, ui, content_origin, &geo, bar, shown, since, on);
            if shown && look.opacity > 0.0 {
                painter.line_segment(look.bar, Stroke::new(1.5, caret_color(app).gamma_multiply(look.opacity)));
            }
            if focused {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));
            }
            // IME candidate window placement.
            let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));
            if focused {
                ui.ctx().output_mut(|o| {
                    o.ime =
                        Some(egui::output::IMEOutput { rect: cr, cursor_rect: cr, purpose: Default::default(), should_interrupt_composition: false });
                });
            }
            if !app.canvas.ime_preedit.is_empty() {
                // The composition is interface text, but it may be in a script the interface fonts lack
                // (Bangla, Thai ...): borrow the document fonts' fallback faces, or it draws as boxes (#565).
                let new = crate::theme::faces_for_preedit(
                    &app.canvas.ime_preedit,
                    &app.canvas.ime_faces,
                    |c| (c as u32) < 0x250,
                    |c| wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX),
                );
                if !new.is_empty() {
                    app.canvas.ime_faces.extend(new);
                    crate::theme::install_fonts_language_extra(
                        ui.ctx(),
                        app.fonts_hans,
                        app.fonts_korean,
                        app.fonts_system_cjk,
                        &app.canvas.ime_faces,
                    );
                    ui.ctx().request_repaint();
                }
                let caret_color = caret_color(app);
                let g = painter.text(pos2(x, y1), egui::Align2::LEFT_BOTTOM, &app.canvas.ime_preedit, regular(c.height * scale * 0.8), caret_color);
                painter.line_segment([pos2(g.min.x, g.max.y), pos2(g.max.x, g.max.y)], Stroke::new(1.0, caret_color));
            }
        }
        crate::objects::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::ink::paint(app, &painter, &rects, &layout, geo.scale);
        crate::table_pen::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::table_resize::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::table_mouse::paint(app, &painter, &t, &layout, &rects, geo.scale);
        (resp, rects)
    });
    app.canvas.scroll_offset = out.state.offset;
    crate::ink::replay_bar(app, ui, out.inner_rect);
    app.canvas.scroll_max = (out.content_size - out.inner_rect.size()).max(egui::Vec2::ZERO);
    if app.session.view.read_mode {
        let step = app.session.view.read_width * geo.scale + GAP;
        if step.is_finite() && step > 0.0 {
            let page = (app.canvas.scroll_offset.x / step).round().max(0.0) as usize;
            app.canvas.read_first_visible = reading_page_start(&layout, page.min(layout.pages.len().saturating_sub(1)));
        }
    }
    // The viewport barely moves between frames; the auto-scroll decides this frame's scroll before
    // the scroll area is built, so it reads last frame's viewport.
    app.canvas.viewport = Some(out.inner_rect);
    let (resp, rects) = out.inner;
    // A click on the page leaves the selected comment balloon.
    if app.canvas.balloon.is_some()
        && ui.input(|i| i.pointer.any_pressed())
        && resp.contains_pointer()
        && !ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| over_balloon(app, p))
    {
        deselect_balloon(app, ui.ctx());
    }
    // Space and Enter also set `Response::clicked` on a focused click-sensing widget
    // (egui's fake primary click). `request_focus` always interrupts IME; on X11 that
    // destroys the XIM context and fcitx then drops every later key (#1169). A pointer
    // click, an accessibility click, or a drag still takes focus, including a click on
    // a canvas that already has it, so a composition commits. `want_focus` focuses only
    // while the canvas does not, and does not interrupt an editor that already has it.
    let keyboard_click = ui.memory(|m| m.has_focus(resp.id)) && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter));
    let pointer_click = resp.clicked_by(egui::PointerButton::Primary);
    let accesskit_click = resp.clicked() && !pointer_click && !keyboard_click;
    let focus_click = pointer_click || accesskit_click || resp.drag_started();
    if focus_click || (app.canvas.want_focus && !resp.has_focus()) {
        resp.request_focus();
    }
    if focus_click || app.canvas.want_focus {
        app.canvas.want_focus = false;
    }
    app.canvas.widget_id = Some(resp.id);
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
    }
    app.canvas.focused = resp.has_focus();
    // Screen readers: the document under the canvas, with the caret (#488).
    crate::a11y::expose(app, ui.ctx(), resp.id, &layout, &rects, area);
    // The buttons that start or stop middle-click scrolling don't also click the page.
    let middle_gesture = app.canvas.auto_scroll.blocks_input();
    // Double-click the space between two pages (or the line between them) to hide or show it.
    if resp.double_clicked()
        && app.session.view.mode == wordcraft_layout::ViewMode::Print
        && !app.session.view.read_mode
        && let Some(p) = resp.interact_pointer_pos()
        && on_page_gap(&rects, &geo.bands, p)
    {
        let _ = app.run("view.hideWhiteSpace", json!({"value": !app.session.view.hide_white_space}));
    } else
    // Draw Table / Eraser own the mouse while on (no caret moves); else the usual editing.
    if !middle_gesture && !crate::table_pen::pointer(app, ui, &resp, &rects, &layout, geo.scale) {
        mouse(app, ui, &resp, &rects, &layout, geo.scale);
    }
    // A click on a check box ticks it; on a list's button opens the list.
    if !middle_gesture {
        crate::content_controls::clicks(app, &resp);
    }
    let secondary_clicked = resp.secondary_clicked() && !middle_gesture;
    // Selected table cells: a right-click off them drops them, even between the selection's ends
    // (beside a selected column), so the menu acts on the cell clicked.
    if secondary_clicked
        && let Some(range) = app.session.cell_range()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, &layout, geo.scale, p)
    {
        let at = wordcraft_geom::Point::new(x, y);
        let on_cells =
            wordcraft_engine::cmd::table_mouse::cell_rects(&layout, &app.session.doc, range).iter().any(|(pi, r)| *pi == page && r.contains(at));
        if !on_cells && let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story) {
            app.session.sel = wordcraft_engine::Selection::caret(pos);
        }
    }
    // Right-click: move the caret there (unless inside the selection), then the context menu.
    // Right-click in an equation puts the caret there and opens the equation menu.
    if secondary_clicked
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, &layout, geo.scale, p)
    {
        match layout.equation_hit(page, x, y, app.session.sel.focus.story) {
            Some((at, inner)) => {
                app.session.sel = wordcraft_engine::Selection::caret(at);
                let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
            }
            None if app.session.math.is_some() => {
                let _ = app.run("equation.exit", json!({}));
            }
            None => {}
        }
    }
    // Right-click on a picture, shape or text box selects it (its menu offers Wrap Text).
    if secondary_clicked
        && app.session.math.is_none()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(&rects, &layout, geo.scale, p)
    {
        if crate::objects::select_at(app, &layout, &rects, geo.scale, p) {
            app.canvas.context_issue = None;
            app.canvas.context_synonyms = None;
        } else if let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story) {
            let (a, b) = app.session.sel.ordered();
            if !(a <= pos && pos <= b) || app.session.sel.is_collapsed() {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            context_suggestions(app);
        }
    }
    // Shift+F10 opens the same menu below the caret, for the selection or the word there (#821).
    let from_keys = std::mem::take(&mut app.canvas.context_menu_from_keys);
    if from_keys {
        context_suggestions(app);
        app.canvas.context_menu_at = caret_screen(app).map(|(p, h)| p + egui::vec2(0.0, h));
    } else if secondary_clicked {
        app.canvas.context_menu_at = resp.interact_pointer_pos();
    }
    let open = if secondary_clicked || from_keys {
        Some(egui::SetOpenCommand::Bool(true))
    } else if resp.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let menu = egui::Popup::menu(&resp).open_memory(open);
    let menu = match app.canvas.context_menu_at {
        Some(p) => menu.at_position(p),
        None => menu.at_pointer_fixed(),
    };
    menu.show(|ui| context_menu(app, ui));
    // Stand down while the context menu owns the pointer so the two never double up; the open
    // state also covers the click that dismisses the menu.
    app.canvas.context_menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if let Some(c) = crate::ink::cursor(app).filter(|_| resp.hovered() || app.canvas.ink.is_some()) {
        ui.ctx().set_cursor_icon(c);
    } else if resp.hovered() || app.canvas.obj_drag.is_some() {
        let latest = ui.input(|i| i.pointer.latest_pos());
        let over_object = latest.and_then(|p| crate::objects::cursor(app, &layout, &rects, geo.scale, p));
        // The selection bar: Word shows a right-pointing arrow there; egui has none, so the
        // plain arrow stands in (#856).
        let in_bar = app.session.sel.focus.story == StoryRef::Body
            && (app.canvas.unit_drag.as_ref().is_some_and(|d| d.unit == "line")
                || latest.and_then(|p| page_at(&rects, &layout, geo.scale, p)).is_some_and(|(pg, x, y)| layout.selection_bar_at(pg, x, y).is_some()));
        let fallback = if in_bar { egui::CursorIcon::Default } else { egui::CursorIcon::Text };
        ui.ctx().set_cursor_icon(over_object.unwrap_or(fallback));
    }
    crate::table_resize::cursor(app, ui, &resp, &layout, &rects, geo.scale);
    crate::table_mouse::cursor(app, ui, &resp, &layout, &rects, geo.scale);
    crate::table_pen::cursor(app, ui, &resp);
    app.canvas.auto_scroll.paint(ui, area);
    let _ = origin;
    if let (Some(h), Some(v)) = (hruler, vruler) {
        rulers(app, ui, h, v, &rects, &layout, geo.scale);
    }
    crate::mini_toolbar::track_dismissal(app, ui.ctx(), area);
    if let Some(sel) = app.canvas.mini_anchor
        && app.ui.mini_toolbar
    {
        crate::mini_toolbar::show(app, ui.ctx(), sel, out.inner_rect);
    }
    if app.session.view.read_mode {
        // Fixed navigation chrome, independent of the horizontally moving columns.
        let step = app.session.view.read_width * app.canvas.scale + GAP;
        let current = if step > 0.0 { (app.canvas.scroll_offset.x / step).round() as usize + 1 } else { 1 };
        let total = layout.pages.len();
        let footer = Rect::from_min_size(pos2(area.max.x - 260.0, area.max.y - 38.0), vec2(250.0, 32.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(app.canvas.scroll_offset.x > 0.5, egui::Button::new("‹")).on_hover_text(tl!("Previous Page")).clicked() {
                        turn_read_page(app, -1.0);
                    }
                    ui.label(format!("{} / {}", current.min(total.max(1)), total));
                    if ui
                        .add_enabled(app.canvas.scroll_offset.x + 0.5 < app.canvas.scroll_max.x, egui::Button::new("›"))
                        .on_hover_text(tl!("Next Page"))
                        .clicked()
                    {
                        turn_read_page(app, 1.0);
                    }
                    if ui.button(tl!("Edit Document")).clicked() {
                        let _ = app.run("view.printLayout", json!({}));
                    }
                });
            });
        });
    }
}

/// Comment balloons in the markup area right of each page, joined to their anchors, with each
/// comment's replies under it. Clicking a balloon selects it, like Word: its text becomes editable
/// in place, with Reply, Resolve and Delete (the same commands as the Comments pane).
fn balloons(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, rects: &[Rect], layout: &DocLayout, scale: f32, view: Rect) {
    app.canvas.balloon_rects.clear();
    let mw = markup_width(app);
    let float = floating_comments(app);
    if app.canvas.balloon.is_some_and(|id| (mw <= 0.0 && !float) || !app.session.doc.comments.contains_key(&id)) {
        deselect_balloon(app, ui.ctx());
    }
    if mw <= 0.0 && !float {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let clip = view;
    let only_shown = app.session.prefs.markup.comment_display == CommentDisplay::Selected;
    let shown = shown_comment(app);
    // Replies hang under the comment they answer rather than getting balloons of their own.
    let is_reply = |c: &wordcraft_doc::Comment| c.parent.is_some_and(|p| app.session.doc.comments.contains_key(&p));
    let mut replies: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, c) in &app.session.doc.comments {
        if let Some(p) = c.parent.filter(|_| is_reply(c)) {
            replies.entry(p).or_default().push(*id);
        }
    }
    // (page, anchor x, anchor y, item) for each anchored, unresolved-or-not comment, each
    // tracked formatting change (Word's default shows those in balloons too) and, when revisions
    // go in balloons, each tracked deletion.
    let mut by_page: HashMap<usize, Vec<(f32, f32, Balloon, Pos)>> = HashMap::new();
    let m = &app.session.prefs.markup;
    let comments = if m.comments { wordcraft_engine::cmd::review::comment_list(&app.session) } else { Vec::new() };
    for (id, pos) in comments {
        let Some(pos) = pos else { continue };
        if app.session.doc.comments.get(&id).is_some_and(is_reply) || (only_shown && shown != Some(id)) {
            continue;
        }
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Comment(id), pos));
    }
    let formats = if m.formatting { wordcraft_engine::cmd::review::format_changes(&app.session) } else { Vec::new() };
    for (pos, author, props) in formats.into_iter().take(500) {
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        let what = crate::panes::describe_props(&props).join(", ");
        let text = if what.is_empty() { tl!("Formatted").to_string() } else { format!("{}: {what}", tl!("Formatted")) };
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(author, text), pos));
    }
    // Each end of a tracked move.
    let moves = if m.insertions_deletions { wordcraft_engine::cmd::review::move_changes(&app.session) } else { Vec::new() };
    for (pos, author, from) in moves.into_iter().take(500) {
        let Some(c) = layout.caret_on(&pos, app.session.page_hint) else { continue };
        let text = if from { tl!("Moved from") } else { tl!("Moved to") }.to_string();
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(author, text), pos));
    }
    // Show revisions in balloons: each tracked deletion, taken out of the text, at its place.
    let deletions = if app.session.revisions_in_balloons() { wordcraft_engine::cmd::review::deletion_balloons(&app.session) } else { Vec::new() };
    for d in deletions {
        let Some(c) = layout.caret_on(&d.at, app.session.page_hint) else { continue };
        let text = format!("{}: {}", tl!("Deleted"), d.text);
        by_page.entry(c.page).or_default().push((c.x, c.top + c.height, Balloon::Format(d.author, text), d.at));
    }
    let palette = [t.blue, Color32::from_rgb(0xB0, 0x3A, 0x2E), Color32::from_rgb(0x2E, 0x7D, 0x32), Color32::from_rgb(0x8E, 0x44, 0xAD), t.orange];
    let mut authors: Vec<String> = Vec::new();
    let mut clicked: Option<(u32, Pos)> = None;
    // The markup area extends each page.
    for (pi, pr) in rects.iter().enumerate() {
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        let area = Rect::from_min_max(pos2(pr.max.x, pr.min.y), pos2(pr.max.x + mw * page_scale, pr.max.y));
        if mw > 0.0 && area.intersects(clip) {
            painter.rect_filled(area, 0.0, Color32::from_rgb(0xF3, 0xF3, 0xF3));
            painter.line_segment([area.left_top(), area.left_bottom()], Stroke::new(1.0, Color32::from_rgb(0xE0, 0xE0, 0xE0)));
        }
    }
    let mut pages: Vec<usize> = by_page.keys().copied().collect();
    pages.sort_unstable();
    for pi in pages {
        let Some(list) = by_page.get_mut(&pi) else { continue };
        let Some(pr) = rects.get(pi) else { continue };
        if !pr.expand(400.0).intersects(clip) {
            continue;
        }
        list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        // Beside the page in the column; floating, beside it when the view has room, else over its edge.
        let (x0, w, over_page) = if mw > 0.0 {
            (pr.max.x + 10.0, (mw * page_scale - 20.0).max(60.0), false)
        } else {
            let w = (FLOAT_WIDTH * page_scale - 20.0).max(60.0);
            let beside = pr.max.x + 10.0;
            if beside + w <= view.max.x - 8.0 { (beside, w, false) } else { ((view.max.x - w - 8.0).max(view.min.x + 8.0), w, true) }
        };
        let mut next_y = pr.min.y;
        for (ax, ay, item, pos) in list.iter() {
            let id = match item {
                Balloon::Comment(id) => id,
                Balloon::Format(author, text) => {
                    // A read-only card: who formatted, and how.
                    let author = author_name(author);
                    let ai = authors.iter().position(|a| *a == author).unwrap_or_else(|| {
                        authors.push(author.clone());
                        authors.len() - 1
                    });
                    let color = palette.get(ai % palette.len()).copied().unwrap_or(t.blue);
                    let fs = (11.0 * page_scale / PX_PER_PT).clamp(8.0, 16.0);
                    let anchor = pos2(pr.min.x + ax * page_scale, pr.min.y + ay * page_scale);
                    let top = (anchor.y - 12.0).max(next_y);
                    let head = painter.layout(author, semibold(fs), t.text, w - 16.0);
                    let body = painter.layout(text.clone(), regular(fs), t.text_dim, w - 16.0);
                    let card = Rect::from_min_size(pos2(x0, top), vec2(w, head.size().y + body.size().y + 16.0));
                    next_y = card.max.y + 6.0;
                    if !card.intersects(clip) {
                        continue;
                    }
                    let lead = Stroke::new(1.0, color.linear_multiply(0.7));
                    dashed(painter, anchor, pos2(pr.max.x, anchor.y), lead);
                    painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
                    card_shapes(painter, &t, card, color, false, None);
                    let y = card.min.y + 6.0;
                    let hh = head.size().y;
                    painter.galley(pos2(card.min.x + 10.0, y), head, t.text);
                    painter.galley(pos2(card.min.x + 10.0, y + hh + 4.0), body, t.text_dim);
                    continue;
                }
            };
            let Some(c) = app.session.doc.comments.get(id) else { continue };
            let resolved = c.resolved;
            let author = author_name(&c.author);
            let ai = authors.iter().position(|a| *a == author).unwrap_or_else(|| {
                authors.push(author.clone());
                authors.len() - 1
            });
            let color = palette.get(ai % palette.len()).copied().unwrap_or(t.blue);
            let fs = (11.0 * page_scale / PX_PER_PT).clamp(8.0, 16.0);
            let thread: Vec<u32> = replies.get(id).cloned().unwrap_or_default();
            let anchor = pos2(pr.min.x + ax * page_scale, pr.min.y + ay * page_scale);
            // Over the page the card goes under the commented line, not on it.
            let top = (anchor.y + if over_page { 4.0 } else { -12.0 }).max(next_y);
            let active = app.canvas.balloon == Some(*id);
            // Lay out a read-only balloon's text: the comment, then each reply under a rule.
            let mut lines: Vec<(std::sync::Arc<egui::Galley>, f32)> = Vec::new();
            let card = if active {
                Rect::from_min_size(pos2(x0, top), vec2(w, app.canvas.balloon_h.max(24.0)))
            } else {
                let dim = |r: bool| if r { t.text_dim } else { t.text };
                lines.push((balloon_text(painter, author, semibold(fs), t.text, w - 16.0), 0.0));
                let body = app.session.doc.plain_text(StoryRef::Part(c.part));
                lines.push((balloon_text(painter, body.trim().to_string(), regular(fs), dim(resolved), w - 16.0), 4.0));
                for rid in &thread {
                    let Some(r) = app.session.doc.comments.get(rid) else { continue };
                    lines.push((balloon_text(painter, author_name(&r.author), semibold(fs), t.text, w - 16.0), 9.0));
                    let text = app.session.doc.plain_text(StoryRef::Part(r.part));
                    lines.push((balloon_text(painter, text.trim().to_string(), regular(fs), dim(resolved), w - 16.0), 4.0));
                }
                let h = lines.iter().map(|(g, gap)| g.size().y + gap).sum::<f32>() + 12.0;
                Rect::from_min_size(pos2(x0, top), vec2(w, h))
            };
            if !card.intersects(clip) {
                next_y = card.max.y + 6.0;
                continue;
            }
            let selected = active || (app.session.sel.focus.path == pos.path && app.session.sel.focus.story == pos.story);
            // Leader: from the anchor along the text to the page edge, then to the card.
            let lead = Stroke::new(if selected { 1.5 } else { 1.0 }, color.linear_multiply(if selected { 1.0 } else { 0.7 }));
            if !over_page {
                dashed(painter, anchor, pos2(pr.max.x, anchor.y), lead);
                painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
            }
            let card = if active {
                active_balloon(app, ui, painter, *id, &thread, card, fs, color)
            } else {
                card_shapes(painter, &t, card, color, selected, None);
                let mut y = card.min.y + 6.0;
                for (i, (g, gap)) in lines.into_iter().enumerate() {
                    y += gap;
                    if i >= 2 && i % 2 == 0 {
                        // A reply starts: a hairline between it and what it answers.
                        painter.hline(card.min.x + 10.0..=card.max.x - 8.0, y - gap / 2.0, Stroke::new(1.0, t.border));
                    }
                    let h = g.size().y;
                    painter.galley(pos2(card.min.x + 10.0, y), g, t.text);
                    y += h;
                }
                let r = ui.interact(card, ui.id().with(("balloon", *id)), Sense::click());
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    clicked = Some((*id, pos.clone()));
                }
                card
            };
            next_y = card.max.y + 6.0;
            app.canvas.balloon_rects.push((*id, card));
        }
    }
    if let Some((id, p)) = clicked {
        select_balloon(app, ui.ctx(), id);
        app.session.sel = wordcraft_engine::Selection::caret(p);
    }
}

/// What a balloon in the markup area shows.
enum Balloon {
    Comment(u32),
    /// A tracked formatting change, move or deletion: its author and "Formatted: …" /
    /// "Moved from" / "Deleted: …".
    Format(String, String),
}

fn author_name(a: &str) -> String {
    if a.is_empty() { tl!("Author").to_string() } else { a.to_string() }
}

/// A balloon card's background, border and author-colour stripe; `slot` paints them under
/// widgets added before the card's size was known.
fn card_shapes(painter: &egui::Painter, t: &Tokens, card: Rect, color: Color32, selected: bool, slot: Option<egui::layers::ShapeIdx>) {
    let shapes = vec![
        egui::Shape::rect_filled(card.translate(vec2(0.0, 1.0)), 4.0, t.page_shadow),
        egui::Shape::rect_filled(card, 4.0, if selected { t.checked } else { Color32::WHITE }),
        egui::Shape::rect_stroke(card, 4.0, Stroke::new(if selected { 1.5 } else { 1.0 }, color), egui::StrokeKind::Inside),
        egui::Shape::rect_filled(Rect::from_min_size(card.min, vec2(4.0, card.height())), egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, color),
    ];
    match slot {
        Some(i) => painter.set(i, egui::Shape::Vec(shapes)),
        None => {
            painter.extend(shapes);
        }
    }
}

/// Does `text` hold right-to-left letters (Arabic, Hebrew, Syriac, Thaana, N'Ko …)?
fn has_rtl(text: &str) -> bool {
    text.chars().any(|c| matches!(unicode_bidi::bidi_class(c), unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL))
}

/// The row `line` of paragraph `p` of `para` in display order, for egui: the bidi runs left to
/// right as they are shown, and inside a right-to-left run its words reversed. The letters of each
/// word stay in logical order, because egui's shaper already runs an Arabic word right to left.
fn visual_row(info: &unicode_bidi::BidiInfo, p: &unicode_bidi::ParagraphInfo, para: &str, line: std::ops::Range<usize>) -> String {
    // `visual_runs` indexes the line's first byte: an empty line (an empty last row) would panic.
    if line.start >= line.end || line.end > p.range.end {
        return String::new();
    }
    let (levels, runs) = info.visual_runs(p, line);
    let mut out = String::new();
    for run in runs {
        let rtl = levels.get(run.start).is_some_and(|l| l.is_rtl());
        // Runs lie on character boundaries; a missing slice is skipped, never a panic.
        let Some(piece) = para.get(run) else { continue };
        if rtl {
            let mut words: Vec<&str> = piece.split_inclusive(char::is_whitespace).collect();
            words.reverse();
            for w in words {
                // The space that followed a word now goes before it.
                let word = w.trim_end_matches(char::is_whitespace);
                out.push_str(w.get(word.len()..).unwrap_or_default());
                out.extend(word.chars().map(mirrored));
            }
        } else {
            out.push_str(piece);
        }
    }
    out
}

/// `c` as drawn in a right-to-left run: brackets face the other way (Unicode's Bidi_Mirrored
/// pairs that interface text uses), since egui draws the character it is given.
fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        _ => c,
    }
}

/// A read-only balloon's text, wrapped to `width`. egui joins Arabic letters but places them left
/// to right, so a right-to-left comment read its words backwards. Each wrapped row is put into
/// display order (Unicode bidi, per paragraph) and a right-to-left paragraph's rows are set against
/// the right edge. Text without right-to-left letters is laid out exactly as before.
fn balloon_text(painter: &egui::Painter, text: String, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    if !has_rtl(&text) {
        return painter.layout(text, font, color, width);
    }
    let format = egui::TextFormat::simple(font.clone(), color);
    let mut job = egui::text::LayoutJob::default();
    let mut rows: Vec<(String, bool)> = Vec::new();
    for para in text.split('\n') {
        let info = unicode_bidi::BidiInfo::new(para, None);
        if info.paragraphs.is_empty() {
            rows.push((String::new(), false));
        }
        for p in &info.paragraphs {
            let rtl = p.level.is_rtl();
            // Wrap in logical order at spaces, by the words' widths (the order doesn't change a word's
            // width), then put each row in display order. egui's own rows can't be used: it draws a
            // lam-alef pair as one glyph, so its glyph count isn't the character count.
            let mut start = p.range.start;
            let mut end = start;
            let mut used = 0.0;
            for word in para.get(p.range.clone()).unwrap_or_default().split_inclusive(char::is_whitespace) {
                let w = painter.layout_no_wrap(word.trim_end().to_string(), font.clone(), color).size().x;
                let gap = painter.layout_no_wrap(word.get(word.trim_end().len()..).unwrap_or_default().to_string(), font.clone(), color).size().x;
                if end > start && used + w > width {
                    rows.push((visual_row(&info, p, para, start..end).trim().to_string(), rtl));
                    start = end;
                    used = 0.0;
                }
                used += w + gap;
                end += word.len();
            }
            rows.push((visual_row(&info, p, para, start..end).trim().to_string(), rtl));
        }
    }
    let last = rows.len().saturating_sub(1);
    for (i, (row, rtl)) in rows.into_iter().enumerate() {
        let lead = if rtl { (width - painter.layout_no_wrap(row.clone(), font.clone(), color).size().x).max(0.0) } else { 0.0 };
        let row = if i < last { row + "\n" } else { row };
        job.append(&row, lead, format.clone());
    }
    painter.layout_job(job)
}

/// The selected balloon: the comment and its replies as editable fields (each edit is one undo
/// step via `review.editComment`), an optional reply field, and Reply / Resolve / Delete.
/// `card` carries last frame's size; returns this frame's.
#[allow(clippy::too_many_arguments)]
fn active_balloon(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, id: u32, thread: &[u32], card: Rect, fs: f32, color: Color32) -> Rect {
    let t = Tokens::get(ui.ctx());
    let slot = painter.add(egui::Shape::Noop);
    // The card's own click area sits under its fields, so a click on its padding stays here.
    let _ = ui.interact(card, ui.id().with(("balloon", id)), Sense::click());
    let inner = Rect::from_min_max(pos2(card.min.x + 10.0, card.min.y + 6.0), pos2(card.max.x - 6.0, card.min.y + 20_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 3.0;
    let (font, bold, small) = (regular(fs), semibold(fs), regular((fs * 0.9).max(8.0)));
    let focus = std::mem::take(&mut app.canvas.balloon_focus);
    let name = |app: &WordApp, cid: u32| app.session.doc.comments.get(&cid).map(|c| author_name(&c.author)).unwrap_or_default();
    let resolved = app.session.doc.comments.get(&id).is_some_and(|c| c.resolved);
    child.horizontal(|ui| {
        ui.label(egui::RichText::new(name(app, id)).font(bold.clone()).color(t.text));
        if resolved {
            ui.label(egui::RichText::new(tl!("Resolved")).font(small.clone()).color(t.green));
        }
    });
    crate::panes::comment_editor(app, &mut child, id, Some(font.clone()), focus && !app.canvas.balloon_reply);
    for rid in thread {
        child.add_space(4.0);
        child.label(egui::RichText::new(name(app, *rid)).font(bold.clone()).color(t.text));
        crate::panes::comment_editor(app, &mut child, *rid, Some(font.clone()), false);
    }
    if app.canvas.balloon_reply && crate::panes::reply_editor(app, &mut child, id, Some(font.clone()), focus) {
        app.canvas.balloon_reply = false;
    }
    let (mut reply, mut resolve, mut delete) = (false, false, false);
    child.add_space(2.0);
    child.horizontal(|ui| {
        let button = |ui: &mut Ui, s: &str| ui.add(egui::Button::new(egui::RichText::new(s).font(small.clone())).small()).clicked();
        reply = button(ui, tl!("Reply"));
        resolve = button(ui, if resolved { tl!("Reopen") } else { tl!("Resolve") });
        delete = button(ui, tl!("Delete"));
    });
    let card = Rect::from_min_max(card.min, pos2(card.max.x, child.min_rect().max.y + 6.0));
    app.canvas.balloon_h = card.height();
    card_shapes(painter, &t, card, color, true, Some(slot));
    if reply {
        app.canvas.balloon_reply = true;
        app.canvas.balloon_focus = true;
    }
    if resolve {
        let _ = app.run("review.resolveComment", json!({"id": id}));
    }
    if delete {
        deselect_balloon(app, ui.ctx());
        let _ = app.run("review.deleteComment", json!({"id": id}));
    }
    card
}

/// Select a comment's balloon (its text gets the keyboard), writing back any edit in the one
/// selected before.
pub fn select_balloon(app: &mut WordApp, ctx: &egui::Context, id: u32) {
    if app.canvas.balloon == Some(id) {
        return;
    }
    deselect_balloon(app, ctx);
    app.canvas.balloon = Some(id);
    app.canvas.balloon_focus = true;
    app.canvas.balloon_h = 0.0;
}

/// A press over a balloon card belongs to the card, though the canvas under it also contains the pointer (#1013).
fn over_balloon(app: &WordApp, p: Pos2) -> bool {
    app.canvas.balloon_rects.iter().any(|(_, r)| r.contains(p))
}

/// Leave the selected balloon, writing back what was typed in it (its fields may not be drawn
/// again to notice they lost focus).
pub fn deselect_balloon(app: &mut WordApp, ctx: &egui::Context) {
    let Some(id) = app.canvas.balloon.take() else { return };
    app.canvas.balloon_reply = false;
    app.canvas.balloon_focus = false;
    let thread: Vec<u32> =
        std::iter::once(id).chain(app.session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k)).collect();
    for c in thread {
        crate::panes::commit_comment(app, ctx, c);
    }
    crate::panes::commit_reply(app, ctx, id);
}

fn dashed(p: &egui::Painter, a: Pos2, b: Pos2, s: Stroke) {
    p.extend(egui::Shape::dashed_line(&[a, b], s, 4.0, 3.0));
}

/// Page index and document coordinates under a screen point.
pub fn page_at(rects: &[Rect], layout: &DocLayout, scale: f32, p: Pos2) -> Option<(usize, f32, f32)> {
    let i = nearest_page(rects, p)?;
    let r = rects.get(i)?;
    let scale = layout.pages.get(i).map_or(scale, |page| page_screen_scale(*r, page, scale));
    Some((i, (p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

/// The page whose screen rect is under (or nearest to) a screen point.
pub fn nearest_page(rects: &[Rect], p: Pos2) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, r) in rects.iter().enumerate() {
        let d = if r.contains(p) {
            // Pages overlap in the margins Hide White Space cuts: the one whose middle is nearer wins.
            1e-3 * (p.y - r.center().y).abs() / r.height().max(1.0)
        } else {
            let dx = (r.min.x - p.x).max(p.x - r.max.x).max(0.0);
            let dy = (r.min.y - p.y).max(p.y - r.max.y).max(0.0);
            dx + dy * 2.0
        };
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Whether wheel input counts as being over the canvas. Another widget's drag (a scrollbar, say)
/// owns the wheel, but a drag-select on the canvas itself keeps scrolling so a long passage can be selected.
fn wheel_hovered(over_canvas: bool, any_drag: bool, drag_selecting: bool) -> bool {
    over_canvas && (!any_drag || drag_selecting)
}

/// The drag auto-scroll along one axis for a pointer at `p` inside the viewport band `lo..hi`, in
/// the wheel's convention (positive = content moves down the screen, toward the document's start):
/// nothing while the pointer is [`EDGE_MARGIN`] or more inside, then ramping from
/// [`EDGE_SPEED_MIN`] at the margin to [`EDGE_SPEED_MAX`] at the edge (and capping beyond it),
/// applied over `dt` seconds. Junk numbers scroll nothing.
fn edge_scroll(p: f32, lo: f32, hi: f32, dt: f32) -> f32 {
    if !(p.is_finite() && lo.is_finite() && hi.is_finite() && dt.is_finite()) || hi <= lo {
        return 0.0;
    }
    let dt = dt.clamp(0.0, EDGE_MAX_DT);
    let speed = |into: f32| egui::emath::lerp(EDGE_SPEED_MIN..=EDGE_SPEED_MAX, (into / EDGE_MARGIN).clamp(0.0, 1.0));
    if p < lo + EDGE_MARGIN {
        speed(lo + EDGE_MARGIN - p) * dt
    } else if p > hi - EDGE_MARGIN {
        -speed(p - (hi - EDGE_MARGIN)) * dt
    } else {
        0.0
    }
}

/// The scroll offset after `delta` (the wheel's convention) reaches `before`: clamped to the
/// scrollable range, so a document shorter than the viewport never moves.
fn clamped_scroll(before: egui::Vec2, delta: egui::Vec2, max: egui::Vec2) -> egui::Vec2 {
    (before - delta).clamp(egui::Vec2::ZERO, max)
}

fn mouse(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) {
    // A pen or the eraser (Draw tab) owns the pointer.
    if crate::ink::pointer(app, ui, resp, rects, layout, scale) {
        return;
    }
    let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
    // An object (or crop handle) drag follows the pointer anywhere until it's released.
    let dragging = app.canvas.obj_drag.is_some() || app.canvas.crop_drag.is_some();
    let object_pointer = pointer.or_else(|| dragging.then(|| ui.input(|i| i.pointer.latest_pos())).flatten());
    if let Some(at) = object_pointer
        && crate::objects::pointer(app, ui, resp, rects, layout, scale, at)
    {
        return;
    }
    // A press on a table border (or the table's resize handle) drags it instead of the caret.
    if crate::table_resize::pointer(app, ui, resp, rects, layout, scale) {
        return;
    }
    // A press on a table's selection area, insert button or move handle.
    if crate::table_mouse::pointer(app, ui, resp, rects, layout, scale) {
        return;
    }
    let Some(p) = pointer else { return };
    // While drag-selecting, the pointer may sit past the viewport's edge (or the window's):
    // hit-test where it would be inside the view, so as the edge auto-scroll moves the content the
    // selection keeps growing under a stationary pointer (#688).
    let p = if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some(vp) = app.canvas.viewport
    {
        vp.clamp(p)
    } else {
        p
    };
    let Some((page, x, y)) = page_at(rects, layout, scale, p) else { return };
    let mods = ui.input(|i| i.modifiers);
    let story = app.session.sel.focus.story;
    // Inside an equation, clicks place the caret in it (and double-clicks don't select words).
    if (resp.double_clicked() || resp.triple_clicked()) && layout.equation_hit(page, x, y, story).is_some() {
        return;
    }
    // Outline view: a paragraph's symbol selects it with what is under it; a double-click
    // expands or collapses the heading.
    if app.session.view.mode == wordcraft_layout::ViewMode::Outline
        && let Some(block) = layout.outline_symbol_at(page, x, y)
    {
        if resp.double_clicked() {
            let _ = app.run("outline.toggle", json!({ "block": block }));
        } else if ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() {
            let _ = app.run("outline.selectSubtree", json!({ "block": block }));
            app.session.page_hint = page;
        }
        return;
    }
    // The press already selected the word or paragraph (and a drag may have grown it).
    let released_clicks = if resp.triple_clicked() {
        3
    } else if resp.double_clicked() {
        2
    } else {
        0
    };
    if released_clicks > 0 && app.canvas.unit_drag.as_ref().is_some_and(|d| d.clicks == released_clicks) {
        app.canvas.unit_drag = None;
        app.canvas.dragging = false;
        return;
    }
    // Double-click in the header/footer area edits it (creating it when the page has none, #960);
    // double-click in the body leaves it. Print Layout only: the other views show no margins.
    let hf_zone = if app.session.view.mode == wordcraft_layout::ViewMode::Print && !app.session.view.read_mode {
        layout.header_footer_zone(page, y)
    } else {
        None
    };
    if resp.double_clicked() {
        if let Some((_, header)) = hf_zone
            && story == StoryRef::Body
        {
            let id = if header { "insert.editHeader" } else { "insert.editFooter" };
            if app.run(id, json!({ "page": page })).is_ok()
                && let Some(pos) = app.session.layout().hit(page, x, y, app.session.sel.focus.story)
            {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return;
        }
        if matches!(story, StoryRef::Part(_)) && !in_text_box(app) && hf_zone.is_none() {
            let _ = app.run("insert.closeHeader", json!({}));
            if let Some(pos) = layout.hit(page, x, y, StoryRef::Body) {
                app.session.sel = wordcraft_engine::Selection::caret(pos);
            }
            return;
        }
        // Double-click a field showing its code: edit the code.
        if let Some(pos) = layout.hit(page, x, y, story)
            && let Some(field) = field_showing_code(app, &pos)
        {
            app.session.sel = wordcraft_engine::Selection::caret(field);
            let _ = app.run("fields.setCode", json!({}));
            return;
        }
        let _ = app.run("select.word", json!({}));
        return;
    }
    if resp.triple_clicked() {
        let _ = app.run("select.paragraph", json!({}));
        return;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() && !over_balloon(app, p);
    if pressed {
        let now = ui.input(|i| i.time);
        let (delay, dist) = ui.ctx().options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_dist));
        let clicks = press_count(app.canvas.last_press, now, p, delay, dist);
        app.canvas.last_press = Some((now, p, clicks));
        app.canvas.unit_drag = None;
        // A double-click that enters or leaves a header/footer is handled on release.
        let hf_switch =
            (hf_zone.is_some() && story == StoryRef::Body) || (matches!(story, StoryRef::Part(_)) && !in_text_box(app) && hf_zone.is_none());
        // Clicking into a footnote/endnote or a text box edits it; clicking outside a text box goes
        // back to where it is. While editing a header/footer, only its own text boxes are in reach.
        let story = {
            let is_note = |s: StoryRef| matches!(part_kind(app, s), Some(PartKind::Footnote | PartKind::Endnote));
            let is_box = |s: StoryRef| part_kind(app, s) == Some(PartKind::TextBox);
            let host = if is_box(story) { text_box_host(app, layout).unwrap_or(StoryRef::Body) } else { story };
            let in_hf = matches!(part_kind(app, host), Some(PartKind::Header | PartKind::Footer));
            let hf_box = if in_hf { layout.header_footer_text_box_at(page, x, y).map(StoryRef::Part) } else { None };
            match (hf_box, layout.story_at(page, x, y)) {
                (Some(b), _) => b,
                (None, Some(s)) if is_note(s) => s,
                (None, Some(s)) if is_box(s) && !in_hf => s,
                (None, Some(StoryRef::Body)) if is_note(story) => StoryRef::Body,
                _ if is_box(story) => host,
                _ => story,
            }
        };
        if !mods.shift
            && let Some((at, inner)) = layout.equation_hit(page, x, y, story)
        {
            app.session.sel = wordcraft_engine::Selection::caret(at);
            let _ = app.run("equation.edit", json!({"pos": serde_json::to_value(&inner).unwrap_or_default()}));
            app.session.page_hint = page;
            return;
        }
        if app.session.math.is_some() {
            let _ = app.run("equation.exit", json!({}));
        }
        // The selection bar (the left margin beside body text, #856): a click selects the line,
        // a double-click the paragraph, a triple-click or Ctrl/⌘+click the document; Shift+click
        // extends by lines; dragging grows the selection by the clicked unit.
        if story == StoryRef::Body
            && !mods.alt
            && let Some(line) = layout.selection_bar_at(page, x, y)
        {
            app.canvas.column_drag = None;
            app.session.page_hint = page;
            let lj = serde_json::to_value(&line).unwrap_or_default();
            let (unit, start, end) = if mods.shift {
                let anchor = app.session.sel.anchor.clone();
                let aj = serde_json::to_value(&anchor).unwrap_or_default();
                let _ = app.run("select.dragUnit", json!({"unit": "line", "start": aj, "end": aj, "pos": lj}));
                ("line", anchor.clone(), anchor)
            } else {
                let unit = match clicks {
                    _ if mods.command => "document",
                    3.. => "document",
                    2 => "paragraph",
                    _ => "line",
                };
                let _ = app.run("caret.set", json!({"pos": lj}));
                let id = match unit {
                    "document" => "select.all",
                    "paragraph" => "select.paragraph",
                    _ => "select.line",
                };
                let params = if id == "select.all" { json!({"whole": true}) } else { json!({}) };
                let _ = app.run(id, params);
                let (start, end) = app.session.sel.ordered();
                (unit, start, end)
            };
            app.canvas.unit_drag = Some(UnitDrag { unit, clicks, start, end, last: line });
            app.canvas.dragging = true;
            return;
        }
        let Some(pos) = layout.hit(page, x, y, story) else { return };
        // Ctrl/⌘+click follows a hyperlink.
        if mods.command
            && let Some(link) = link_under(app, layout, page, x, y, &pos)
        {
            if let Some(name) = link.strip_prefix('#') {
                // The caret goes to the bookmark's start, as in Word (Go To selects its text).
                if app.run("edit.goto", json!({"bookmark": name})).is_ok() {
                    let _ = app.run("select.collapse", json!({}));
                }
            } else if is_followable_link(&link) {
                app.canvas.open_url = Some(link);
            } else {
                app.status(tl!("Only web (http, https) and email (mailto) links can be followed"));
            }
            return;
        }
        let pj = serde_json::to_value(&pos).unwrap_or_default();
        // Alt+drag selects a column (block) of text.
        if mods.alt && !mods.shift && !mods.command {
            let _ = app.run("caret.set", json!({"pos": pj}));
            app.session.page_hint = page;
            app.canvas.column_drag = Some(((page, x, y), (page, x, y)));
            app.canvas.dragging = true;
            return;
        }
        app.canvas.column_drag = None;
        let _ = app.run("caret.set", json!({"pos": pj, "extend": mods.shift}));
        // Double-click selects a word, triple-click a paragraph, Ctrl/⌘+click a sentence; holding
        // the button and dragging then extends by whole units (Word, #549).
        let unit = match clicks {
            _ if mods.shift => None,
            3 if !hf_switch => Some("paragraph"),
            2 if !hf_switch => Some("word"),
            1 if mods.command => Some("sentence"),
            _ => None,
        };
        if let Some(unit) = unit {
            let _ = app.run(&format!("select.{unit}"), json!({}));
            let (start, end) = app.session.sel.ordered();
            app.canvas.unit_drag = Some(UnitDrag { unit, clicks, start, end, last: pos });
        }
        app.session.page_hint = page;
        app.canvas.dragging = true;
        // Format painter applies on mouse up.
    } else if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some((from, last)) = app.canvas.column_drag
    {
        if last != (page, x, y) {
            app.canvas.column_drag = Some((from, (page, x, y)));
            let story = app.session.sel.focus.story;
            let _ = app.run(
                "select.column",
                json!({"from": {"page": from.0, "x": from.1, "y": from.2}, "to": {"page": page, "x": x, "y": y}, "story": story}),
            );
        }
    } else if app.canvas.dragging
        && ui.input(|i| i.pointer.primary_down())
        && let Some(d) = app.canvas.unit_drag.as_mut()
    {
        // Lines grow line by line wherever the pointer is across; the whole document can't grow.
        let target = match d.unit {
            "line" => layout.body_line_at(page, y),
            "document" => None,
            _ => layout.hit(page, x, y, story),
        };
        if let Some(pos) = target
            && pos != d.last
        {
            d.last = pos.clone();
            let params = json!({"unit": d.unit, "start": d.start, "end": d.end, "pos": pos});
            let _ = app.run("select.dragUnit", params);
            app.session.page_hint = page;
            app.canvas.caret_visible_since = crate::now_ms();
        }
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_down()) {
        // Through the command, so from one table cell into another the drag takes whole cells
        // (Word); straight to the session, as the app's `run` would scroll on every frame.
        if let Some(pos) = layout.hit(page, x, y, story)
            && pos != app.session.sel_ends().focus
        {
            let pj = serde_json::to_value(&pos).unwrap_or_default();
            let _ = app.session.run("caret.set", &json!({"pos": pj, "extend": true}));
            app.session.page_hint = page;
            app.canvas.caret_visible_since = crate::now_ms();
        }
        // The pointer was clamped into the viewport above, so when the edge auto-scroll moves the
        // content under it, this hit-test keeps the selection growing (#688).
    } else if app.canvas.dragging && ui.input(|i| i.pointer.primary_released()) {
        app.canvas.dragging = false;
        app.canvas.column_drag = None;
        app.canvas.unit_drag = None;
        if app.session.painter.is_some() && !app.session.sel.is_collapsed() {
            let _ = app.run("edit.pasteFormat", json!({}));
        }
    }
    // Hover a link: show its target.
    if resp.hovered()
        && !app.canvas.dragging
        && let Some(pos) = layout.hit(page, x, y, story)
        && let Some(link) = link_under(app, layout, page, x, y, &pos)
    {
        let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
        let tip = format!("{link}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", key)]));
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::new("link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
}

/// Horizontal and vertical rulers for the caret's page and paragraph.
fn rulers(app: &mut WordApp, ui: &mut Ui, h: Rect, v: Rect, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let corner = Rect::from_min_max(pos2(v.min.x, h.min.y), pos2(v.max.x, h.max.y));
    p.rect_filled(corner, 0.0, t.canvas);
    p.rect_filled(h, 0.0, t.canvas);
    p.rect_filled(v, 0.0, t.canvas);
    crate::ruler_tabs::selector(app, ui, corner);
    let pi = app.session.page_hint.min(layout.pages.len().saturating_sub(1));
    let (Some(page), Some(pr)) = (layout.pages.get(pi), rects.get(pi)) else { return };
    let scale = page_screen_scale(*pr, page, scale);
    let sect = app.session.doc.sections().get(page.section).map(|(_, s)| (*s).clone()).unwrap_or_default();
    // Horizontal.
    let hp = ui.painter_at(h);
    let x0 = pr.min.x;
    let bar = Rect::from_min_max(pos2(x0, h.min.y + 4.0), pos2(x0 + page.w * scale, h.max.y - 3.0));
    hp.rect_filled(bar, 0.0, t.ruler_margin);
    let text = Rect::from_min_max(pos2(x0 + page.body.x * scale, bar.min.y), pos2(x0 + page.body.right() * scale, bar.max.y));
    hp.rect_filled(text, 0.0, t.ruler);
    let rp = app.session.doc.para_at(&app.session.sel.focus).map(|para| app.session.doc.styles.resolve_para(&para.props));
    // Graduations in the File › Options unit.
    let (unit, div, step) = ruler_scale(app.session.units());
    // The caret paragraph's reading direction anchors the whole scale (#692): for a right-to-left
    // paragraph it runs from the page's right edge and the numbers increase right-to-left, so the
    // ruler matches the indent and tab markers, which sit at the paragraph's start edge.
    let rtl = rp.as_ref().is_some_and(|rp| rp.bidi);
    let origin = if rtl { page.w - page.body.right() } else { page.body.x };
    // A distance along the reading direction (points from the page's start edge) and back.
    let sx = |d: f32| ruler_x(d, x0, scale, page.w, rtl);
    let s_at = |x: f32| ruler_d(x, x0, scale, page.w, rtl);
    for (d, mark) in ruler_marks(origin, page.w, (unit, div, step), scale) {
        let x = sx(d);
        match mark {
            RulerMark::Label(n) => {
                hp.text(pos2(x, bar.center().y), egui::Align2::CENTER_CENTER, n.abs().to_string(), regular(9.5), t.ruler_tick);
            }
            RulerMark::Tick(len) => {
                hp.line_segment([pos2(x, bar.center().y - len / 2.0), pos2(x, bar.center().y + len / 2.0)], Stroke::new(1.0, t.ruler_tick));
            }
        }
    }
    // Margins: the edges of the white text area drag the section's margins. Registered before
    // the indent markers, so a marker sitting on an edge keeps the pointer where they overlap.
    let (le, re) = (x0 + page.body.x * scale, x0 + page.body.right() * scale);
    for (edge, zone) in [
        (MarginEdge::Left, Rect::from_min_max(pos2(le - 10.0, bar.min.y), pos2(le + 3.0, bar.max.y))),
        (MarginEdge::Right, Rect::from_min_max(pos2(re - 3.0, bar.min.y), pos2(re + 10.0, bar.max.y))),
    ] {
        margin_handle(app, ui, edge, zone, |p| (p.x - x0) / scale, &sect);
    }
    // Indent markers for the caret's paragraph (relative to its column, or in a table to the
    // cell's text).
    let (col_x, w) = crate::table_ruler::text_area(app, layout, pi).unwrap_or((page.body.x, page.body.w));
    // Where the paragraph starts, from the page's start edge (the right edge when right-to-left).
    let start = if rtl { page.w - (col_x + w) } else { col_x };
    if let Some(rp) = rp {
        // Indents and tabs are measured from the paragraph's start edge: the right margin of a
        // right-to-left paragraph, where the start-indent markers sit.
        let px = |s: f32| sx(start + s);
        let dist_at = |x: f32| s_at(x) - start;
        let first = px(rp.indent_left + rp.indent_first);
        let left = px(rp.indent_left);
        let right = px(w - rp.indent_right);
        let c = t.text_dim;
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(first - 4.5, bar.min.y), pos2(first + 4.5, bar.min.y), pos2(first, bar.min.y + 5.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y - 3.0), pos2(left, bar.max.y - 8.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        hp.rect(
            Rect::from_min_max(pos2(left - 4.5, bar.max.y - 3.0), pos2(left + 4.5, bar.max.y + 1.0)),
            0.0,
            t.ruler,
            Stroke::new(1.0, c),
            egui::StrokeKind::Inside,
        );
        hp.add(egui::Shape::convex_polygon(
            vec![pos2(right - 4.5, bar.max.y - 1.0), pos2(right + 4.5, bar.max.y - 1.0), pos2(right, bar.max.y - 6.0)],
            t.ruler,
            Stroke::new(1.0, c),
        ));
        // Tab stops: drawn and registered before the indent markers' drags, which win where they overlap.
        let step = unit / div as f32 / 2.0;
        let ruler = crate::ruler_tabs::Ruler { bar, text, width: w, step, rtl, x_of: &px, dist_at: &dist_at };
        crate::ruler_tabs::stops(app, ui, &hp, &ruler, &rp.tabs);
        // In a table, its column borders: registered after the tab stops (whose set-a-stop zone
        // spans the text area, so a border marker keeps its drag) and before the indent markers,
        // which keep the pointer where they overlap.
        crate::table_ruler::columns(app, ui, layout, pi, bar, x0, scale);
        // Dragging the left-indent marker.
        let id = ui.id().with("ruler_left");
        let mr = Rect::from_center_size(pos2(left, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(mr, id, Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = dist_at(pp.x).clamp(-col_x, (w - 18.0).max(-col_x));
            let snapped = (pt / 4.5).round() * 4.5;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"left": snapped}));
        }
        let fr = Rect::from_center_size(pos2(first, bar.min.y + 3.0), vec2(12.0, 10.0));
        let r = ui.interact(fr, ui.id().with("ruler_first"), Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = dist_at(pp.x) - rp.indent_left;
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"firstLine": (pt / 4.5).round() * 4.5}));
        }
        let rr = Rect::from_center_size(pos2(right, bar.max.y - 3.0), vec2(12.0, 12.0));
        let r = ui.interact(rr, ui.id().with("ruler_right"), Sense::click_and_drag());
        if r.dragged()
            && let Some(pp) = r.interact_pointer_pos()
        {
            let pt = w - dist_at(pp.x);
            if !r.drag_started() {
                app.session.join_next_undo();
            }
            let _ = app.run("para.indents", json!({"right": (pt / 4.5).round() * 4.5}));
        }
    }
    // Vertical.
    let vp = ui.painter_at(v);
    let y0 = pr.min.y;
    let vbar = Rect::from_min_max(pos2(v.min.x + 4.0, y0), pos2(v.max.x - 3.0, y0 + page.h.min(20_000.0) * scale));
    vp.rect_filled(vbar, 0.0, t.ruler_margin);
    let (te, be) = (y0 + page.body.y * scale, y0 + page.body.bottom() * scale);
    vp.rect_filled(Rect::from_min_max(pos2(vbar.min.x, te), pos2(vbar.max.x, be)), 0.0, t.ruler);
    for (edge, ey) in [(MarginEdge::Top, te), (MarginEdge::Bottom, be)] {
        let zone = Rect::from_min_max(pos2(vbar.min.x, ey - 4.0), pos2(vbar.max.x, ey + 4.0));
        margin_handle(app, ui, edge, zone, |p| (p.y - y0) / scale, &sect);
    }
    let origin = page.body.y;
    let mut k = -((origin / unit).ceil() as i32) * div;
    loop {
        let ypt = origin + k as f32 * unit / div as f32;
        if ypt > page.h.min(20_000.0) {
            break;
        }
        if ypt >= 0.0 && k % div == 0 && k != 0 {
            vp.text(
                pos2(vbar.center().x, y0 + ypt * scale),
                egui::Align2::CENTER_CENTER,
                (k / div * step).abs().to_string(),
                regular(9.5),
                t.ruler_tick,
            );
        } else if ypt >= 0.0 && k % (div / 2) == 0 {
            let sy = y0 + ypt * scale;
            vp.line_segment([pos2(vbar.center().x - 2.5, sy), pos2(vbar.center().x + 2.5, sy)], Stroke::new(1.0, t.ruler_tick));
        }
        k += 1;
        if k > 2000 {
            break;
        }
    }
    // In a table, its rows' bottom borders.
    crate::table_ruler::rows(app, ui, layout, pi, vbar, y0, scale);
}

/// One mark drawn along a horizontal ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RulerMark {
    /// A whole-inch number, `n` inches from the start margin.
    Label(i32),
    /// A tick line, `len` screen points long.
    Tick(f32),
}

/// The marks of a horizontal ruler with their distance from the page's start edge (points along
/// the reading direction): ticks every 1/8 inch past `origin` (the start margin, so the margin is
/// numbered backwards to the page edge) out to `width`, their labels counting whole inches from
/// the start margin. Sub-half-inch ticks drop out when they'd sit closer than ~4 screen points.
fn ruler_marks(origin: f32, width: f32, (unit, div, step): (f32, i32, i32), scale: f32) -> Vec<(f32, RulerMark)> {
    let div = div.max(2);
    let mut marks = Vec::new();
    let mut k = -((origin / unit).ceil() as i32) * div;
    loop {
        let d = origin + k as f32 * unit / div as f32;
        if d > width {
            break;
        }
        if d >= 0.0 {
            let mark = if k % div == 0 {
                (k != 0).then_some(RulerMark::Label(k / div * step))
            } else if scale * unit / div as f32 > 4.0 || k % 2 == 0 {
                Some(RulerMark::Tick(if k % (div / 2) == 0 {
                    5.0
                } else if div >= 8 && k % (div / 4) == 0 {
                    3.0
                } else {
                    1.5
                }))
            } else {
                None
            };
            if let Some(mark) = mark {
                marks.push((d, mark));
            }
        }
        k += 1;
        if k > 400 {
            break;
        }
    }
    marks
}

/// Screen x of a point `d` points from the page's start edge, on a page whose left edge sits at
/// `x0` drawn at `scale`. A right-to-left paragraph reads from the right, so its scale mirrors.
fn ruler_x(d: f32, x0: f32, scale: f32, page_w: f32, rtl: bool) -> f32 {
    if rtl { x0 + (page_w - d) * scale } else { x0 + d * scale }
}

/// The inverse of [`ruler_x`]: the distance from the page's start edge under screen `x`.
fn ruler_d(x: f32, x0: f32, scale: f32, page_w: f32, rtl: bool) -> f32 {
    if rtl { page_w - (x - x0) / scale } else { (x - x0) / scale }
}

/// A ruler's graduation for a unit: points per numbered step, divisions per step, and how much
/// each step counts (millimetre rulers number every centimetre as 10, 20…).
fn ruler_scale(u: wordcraft_geom::Unit) -> (f32, i32, i32) {
    use wordcraft_geom::Unit;
    match u {
        Unit::Centimeters => (wordcraft_geom::PT_PER_CM, 4, 1),
        Unit::Millimeters => (wordcraft_geom::PT_PER_CM, 4, 10),
        Unit::Points => (72.0, 8, 72),
        Unit::Picas => (72.0, 8, 6),
        Unit::Inches => (72.0, 8, 1),
    }
}

/// One edge of a page's text area on a ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MarginEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl MarginEdge {
    fn param(self) -> &'static str {
        match self {
            MarginEdge::Left => "left",
            MarginEdge::Right => "right",
            MarginEdge::Top => "top",
            MarginEdge::Bottom => "bottom",
        }
    }
    fn label(self) -> &'static str {
        match self {
            MarginEdge::Left => "Left Margin",
            MarginEdge::Right => "Right Margin",
            MarginEdge::Top => "Top Margin",
            MarginEdge::Bottom => "Bottom Margin",
        }
    }
}

/// The least text width (or height) a margin drag leaves: one inch, in points.
const MIN_TEXT: f32 = 72.0;

/// The margin (points) after dragging `edge` of the text area on a ruler to `at` points from the
/// page's left (or top) edge: snapped to 1/16 inch, never negative, and leaving at least an inch
/// of text between it and the opposite margin.
pub(crate) fn dragged_margin(edge: MarginEdge, at: f32, s: &wordcraft_doc::section::SectionProps) -> f32 {
    let (raw, room, cur) = match edge {
        MarginEdge::Left => (at - s.gutter, s.page_w - s.margin_right - s.gutter - MIN_TEXT, s.margin_left),
        MarginEdge::Right => (s.page_w - at, s.page_w - s.margin_left - s.gutter - MIN_TEXT, s.margin_right),
        MarginEdge::Top => (at, s.page_h - s.margin_bottom - MIN_TEXT, s.margin_top),
        MarginEdge::Bottom => (s.page_h - at, s.page_h - s.margin_top - MIN_TEXT, s.margin_bottom),
    };
    if !raw.is_finite() || !room.is_finite() {
        return cur;
    }
    ((raw / 4.5).round() * 4.5).clamp(0.0, room.max(0.0))
}

/// A draggable margin edge on a ruler: `to_pt` turns the pointer into points from the page's
/// left (or top) edge. A whole drag is one undo step, through `layout.margins`.
fn margin_handle(app: &mut WordApp, ui: &Ui, edge: MarginEdge, zone: Rect, to_pt: impl Fn(Pos2) -> f32, sect: &wordcraft_doc::section::SectionProps) {
    let cursor =
        if matches!(edge, MarginEdge::Left | MarginEdge::Right) { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical };
    let r = ui.interact(zone, ui.id().with(("ruler_margin", edge)), Sense::drag()).on_hover_cursor(cursor);
    let r = if r.dragged() { r } else { r.on_hover_text(tl!(edge.label())) };
    if r.dragged()
        && let Some(pp) = r.interact_pointer_pos()
    {
        let v = dragged_margin(edge, to_pt(pp), sect);
        if !r.drag_started() {
            app.session.join_next_undo();
        }
        let _ = app.run("layout.margins", json!({ edge.param(): v }));
    }
}

/// Convert a page position (points) to screen coordinates (for agents and tests).
pub fn page_to_screen(app: &mut WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let r = app.canvas.page_rects.get(page)?;
    let layout = app.session.layout();
    let scale = layout.pages.get(page).map_or(app.canvas.scale, |page_data| page_screen_scale(*r, page_data, app.canvas.scale));
    Some(to_screen(pos2(0.0, 0.0), *r, scale, x, y))
}

/// Whether the caret is in a text box's story.
pub fn in_text_box(app: &WordApp) -> bool {
    part_kind(app, app.session.sel.focus.story) == Some(PartKind::TextBox)
}

fn part_kind(app: &WordApp, s: StoryRef) -> Option<PartKind> {
    match s {
        StoryRef::Part(id) => app.session.doc.parts.get(&id).map(|p| p.kind),
        StoryRef::Body => None,
    }
}

/// The story a text box's story sits in (where the box is), when the caret is in a text box.
pub(crate) fn text_box_host(app: &WordApp, layout: &DocLayout) -> Option<StoryRef> {
    match app.session.sel.focus.story {
        StoryRef::Part(id) if in_text_box(app) => layout.text_box(id, app.session.page_hint).map(|o| o.story),
        _ => None,
    }
}

/// Editing a header or footer, or a text box in one.
pub fn editing_header_footer(app: &WordApp, layout: &DocLayout) -> bool {
    let story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
}

/// The body is dimmed while editing anything but the body (or a text box in it).
fn dims_body(app: &WordApp, layout: &DocLayout) -> bool {
    match app.session.sel.focus.story {
        StoryRef::Body => false,
        StoryRef::Part(_) if in_text_box(app) => editing_header_footer(app, layout),
        StoryRef::Part(_) => true,
    }
}

/// The spelling suggestions and synonyms the context menu offers for the selection or the word
/// at the caret.
fn context_suggestions(app: &mut WordApp) {
    app.canvas.context_issue = app.session.run("review.suggestions", &json!({})).ok().filter(|v| !v.is_null());
    app.canvas.context_synonyms = app.session.run("review.thesaurus", &json!({})).ok().and_then(|v| v.get("synonyms").cloned());
}

/// Caret position on screen.
pub fn caret_screen(app: &mut WordApp) -> Option<(Pos2, f32)> {
    let l = app.session.layout();
    let c = l.caret_on(&app.session.sel.focus, app.session.page_hint)?;
    let p = page_to_screen(app, c.page, c.x, c.top)?;
    let scale = app
        .canvas
        .page_rects
        .get(c.page)
        .and_then(|r| l.pages.get(c.page).map(|page| page_screen_scale(*r, page, app.canvas.scale)))
        .unwrap_or(app.canvas.scale);
    Some((p, c.height.max(c.width) * scale))
}

pub fn pos_from_screen(app: &mut WordApp, p: Pos2) -> Option<Pos> {
    let layout = app.session.layout();
    let (page, x, y) = page_at(&app.canvas.page_rects, &layout, app.canvas.scale, p)?;
    let story = app.session.sel.focus.story;
    app.session.layout().hit(page, x, y, story)
}

/// Right-click inside an equation: its structures' actions, display and conversion.
fn equation_menu(app: &mut WordApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    let acts = app.session.run("equation.structureActions", &json!({})).ok().and_then(|v| v.get("actions").cloned()).unwrap_or_default();
    let mut last_level = None;
    for a in acts.as_array().into_iter().flatten() {
        let (Some(action), Some(label), Some(level)) =
            (a.get("action").and_then(|v| v.as_str()), a.get("label").and_then(|v| v.as_str()), a.get("level").and_then(|v| v.as_u64()))
        else {
            continue;
        };
        if last_level.is_some_and(|l| l != level) {
            ui.separator();
        }
        last_level = Some(level);
        if ui.button(tl!(label)).clicked() {
            let _ = app.run("equation.structure", json!({"action": action, "level": level}));
            app.canvas.want_focus = true;
            ui.close();
        }
    }
    if last_level.is_some() {
        ui.separator();
    }
    let display = app.session.run("equation.get", &json!({})).ok().and_then(|v| v.get("display").and_then(|d| d.as_bool())).unwrap_or(false);
    let items: [(&str, &str, serde_json::Value); 5] = [
        (if display { "Change to Inline" } else { "Change to Display" }, "equation.display", json!({"value": !display})),
        ("Professional", "equation.convert", json!({"to": "professional"})),
        ("Linear", "equation.convert", json!({"to": "linear"})),
        ("Equation Number", "equation.number", json!({})),
        ("Close Equation", "equation.exit", json!({})),
    ];
    for (label, id, params) in items {
        if ui.button(tl!(label)).clicked() {
            let _ = app.run(id, params);
            app.canvas.want_focus = true;
            ui.close();
        }
    }
}

fn context_menu(app: &mut WordApp, ui: &mut Ui) {
    if app.session.math.is_some() {
        return equation_menu(app, ui);
    }
    ui.set_min_width(220.0);
    let item = |ui: &mut Ui, app: &mut WordApp, label: &str, id: &str, params: serde_json::Value| {
        let sc = crate::widgets::shortcut_text(app, id);
        let on = crate::widgets::enabled(app, id);
        if ui.add_enabled(on, egui::Button::new(tl!(label)).shortcut_text(sc)).clicked() {
            let _ = app.run(id, params);
            ui.close();
        }
    };
    if let Some(issue) = app.canvas.context_issue.clone() {
        let sugg: Vec<String> = issue
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        // English rules' messages are catalog keys; Polish rules already explain in Polish.
        ui.label(egui::RichText::new(tl!(issue.get("message").and_then(|m| m.as_str()).unwrap_or(""))).small().weak());
        if sugg.is_empty() {
            ui.label(egui::RichText::new(tl!("(no suggestions)")).italics());
        }
        for sgt in sugg {
            if ui.button(egui::RichText::new(&sgt).strong()).clicked() {
                let _ = app.run("review.applySuggestion", json!({"text": sgt}));
                ui.close();
            }
        }
        if issue.get("kind").and_then(|k| k.as_str()) == Some("spelling") {
            item(ui, app, "Ignore All", "review.ignoreAll", json!({}));
            item(ui, app, "Add to Dictionary", "review.addToDictionary", json!({}));
        }
        ui.separator();
    }
    item(ui, app, "Cut", "edit.cut", json!({}));
    item(ui, app, "Copy", "edit.copy", json!({}));
    item(ui, app, "Paste", "edit.paste", json!({}));
    ui.separator();
    if let Some(syn) = app.canvas.context_synonyms.clone().and_then(|v| v.as_array().cloned()).filter(|a| !a.is_empty()) {
        ui.menu_button(tl!("Synonyms"), |ui| {
            for w in syn.iter().filter_map(|x| x.as_str()) {
                if ui.button(w).clicked() {
                    let _ = app.run("select.word", json!({}));
                    let (a, b) = app.session.sel.ordered();
                    let trimmed = app.session.doc.para_at(&a).and_then(|p| p.text.get(a.off..b.off)).map(|t| t.trim_end().len()).unwrap_or(0);
                    let end = wordcraft_doc::Pos { off: a.off + trimmed, ..b };
                    app.session.sel = wordcraft_engine::Selection { anchor: a, focus: end };
                    let _ = app.run("text.insert", json!({"text": w, "raw": true}));
                    ui.close();
                }
            }
        });
    }
    item(ui, app, "Font…", "ui.dialog", json!({"name": "font"}));
    item(ui, app, "Paragraph…", "ui.dialog", json!({"name": "paragraph"}));
    item(ui, app, "Link…", "ui.dialog", json!({"name": "link"}));
    item(ui, app, "New Comment", "review.newComment", json!({}));
    if !wordcraft_engine::cmd::fields::selected(&app.session).is_empty() {
        ui.separator();
        item(ui, app, tl!("Update Field"), "references.updateFields", json!({}));
        item(ui, app, tl!("Edit Field…"), "fields.setCode", json!({}));
        item(ui, app, tl!("Toggle Field Codes"), "fields.toggleCode", json!({}));
    } else if wordcraft_engine::cmd::references::caret_in_toc(&app.session) {
        // The TOC field sits in the table's heading; its entries are plain paragraphs.
        ui.separator();
        item(ui, app, tl!("Update Field"), "references.updateToc", json!({}));
    }
    let selected_object = wordcraft_engine::cmd::objects::selected(&app.session).is_some();
    if selected_object || app.session.sel.focus.path.cell().is_some() {
        item(ui, app, "View Alt Text…", "view.altTextPane", json!({"value": true}));
    }
    // A selected picture, shape, text box or chart: its own items, then Wrap Text (#513).
    let picture = crate::ribbon::has_picture_selected(&app.session);
    let object = wordcraft_engine::cmd::objects::object_selection(&app.session).is_some();
    if picture || object {
        ui.separator();
    }
    if picture {
        item(ui, app, "Change Picture…", "ui.changePicture", json!({}));
        item(ui, app, "Reset Picture", "picture.reset", json!({}));
        item(ui, app, "Size and Crop…", "ui.tab", json!({"tab": "Picture Format"}));
    }
    if object {
        ui.menu_button(tl!("Wrap Text"), |ui| crate::ribbon::wrap_text_items(ui, app));
    }
    if app.session.sel.focus.path.cell().is_some() {
        // The table's items follow what is selected, as in Word (2019, Windows).
        let pick = table_pick(&app.session);
        ui.separator();
        ui.menu_button(tl!("Insert"), |ui| {
            item(ui, app, "Insert Columns to the Left", "table.insertColumnLeft", json!({}));
            item(ui, app, "Insert Columns to the Right", "table.insertColumnRight", json!({}));
            item(ui, app, "Insert Rows Above", "table.insertRowAbove", json!({}));
            item(ui, app, "Insert Rows Below", "table.insertRowBelow", json!({}));
        });
        // Every Delete is there whatever is selected, as on Word's Delete button: with the caret
        // in a cell, Delete Table is how the table goes (#1198).
        ui.menu_button(tl!("Delete"), |ui| {
            item(ui, app, "Delete Cells", "table.deleteCells", json!({}));
            item(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
            item(ui, app, "Delete Rows", "table.deleteRow", json!({}));
            item(ui, app, "Delete Table", "table.deleteTable", json!({}));
        });
        if pick == TablePick::Cell {
            item(ui, app, "Split Cells", "table.split", json!({}));
        }
        if matches!(pick, TablePick::Cells | TablePick::Rows | TablePick::Columns) {
            item(ui, app, "Merge Cells", "table.merge", json!({}));
        }
        if matches!(pick, TablePick::Rows | TablePick::Columns | TablePick::Table) {
            item(ui, app, "Distribute Rows", "table.distributeRows", json!({}));
            item(ui, app, "Distribute Columns", "table.distributeColumns", json!({}));
        }
        if pick == TablePick::Table {
            ui.menu_button(tl!("AutoFit"), |ui| {
                item(ui, app, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
                item(ui, app, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
                item(ui, app, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
            });
        }
        item(ui, app, "Text Direction", "table.textDirection", json!({}));
        if pick == TablePick::Table {
            item(ui, app, "Insert Caption…", "references.caption", json!({}));
        }
        item(ui, app, "Table Properties…", "table.properties", json!({}));
    }
}

/// What of a table the selection is: Word's context menu offers different table items for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TablePick {
    /// The caret or text in one cell, or that one cell.
    Cell,
    /// Cells of more than one row or column, but not whole rows or columns.
    Cells,
    Rows,
    Columns,
    Table,
}

fn table_pick(s: &wordcraft_engine::Session) -> TablePick {
    if let Some(r) = s.cell_range()
        && let Some(t) = s.doc.table(r.story, &r.table)
    {
        let all_rows = r.rows == (0, t.rows.len().saturating_sub(1));
        let all_cols = r.cols == (0, t.cols().saturating_sub(1));
        return match (all_rows, all_cols) {
            (true, true) => TablePick::Table,
            (true, false) => TablePick::Columns,
            (false, true) => TablePick::Rows,
            _ if r.cells(t).len() > 1 => TablePick::Cells,
            _ => TablePick::Cell,
        };
    }
    // Text running from one cell into another of the same table.
    match (s.sel.anchor.path.cell(), s.sel.focus.path.cell()) {
        (Some((ta, ra, ca)), Some((tb, rb, cb))) if ta == tb && (ra, ca) != (rb, cb) => TablePick::Cells,
        _ => TablePick::Cell,
    }
}

/// The simple field at or just before `pos` when it shows its code (View › Field Codes, or its
/// own toggle): where it is.
fn field_showing_code(app: &WordApp, pos: &wordcraft_doc::Pos) -> Option<wordcraft_doc::Pos> {
    let para = app.session.doc.para_at(pos)?;
    let width = wordcraft_doc::para::OBJ.len_utf8();
    [Some(pos.off), pos.off.checked_sub(width)].into_iter().flatten().find_map(|off| match para.object_at(off) {
        Some(wordcraft_doc::para::InlineObject::Field { code, .. }) if *code != app.session.view.field_codes => {
            Some(wordcraft_doc::Pos { off, ..pos.clone() })
        }
        _ => None,
    })
}

/// Render options for this view's page rasters: marks, markup, dark page, header/body dimming.
fn page_render_options(app: &WordApp, dim_body: bool) -> wordcraft_render::RenderOptions {
    let mut opts = screen_render_options();
    opts.display.marks = app.session.view.marks;
    opts.display.always = app.session.prefs.marks;
    opts.display.placeholders = true;
    opts.display.markup = app.session.view.show_markup;
    // Ink Replay draws the ink itself, stroke by stroke, over pages without it.
    opts.display.hide_ink = app.session.view.hide_ink || app.session.view.draw.replay;
    opts.display.revisions = app.session.prefs.markup.clone();
    opts.dark = app.session.view.dark_mode;
    opts.dark_paper = wordcraft_render::DARK_PAPER;
    opts.display.dim_header = !dim_body;
    opts.display.dim_body = dim_body;
    opts
}

/// The caret colour that shows against `bg`: black or white, whichever contrasts more (WCAG
/// contrast ratio), as Word's caret inverts what lies under it (#299). A white caret on white
/// paper, or a black one on a dark page colour or highlight, would vanish.
pub(crate) fn caret_contrast(bg: wordcraft_doc::props::Rgb) -> Color32 {
    let lin = |v: u8| {
        let c = v as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let l = 0.2126 * lin(bg.0) + 0.7152 * lin(bg.1) + 0.0722 * lin(bg.2);
    // Contrast with black (l + 0.05) / 0.05 against with white 1.05 / (l + 0.05).
    if (l + 0.05) * (l + 0.05) >= 0.05 * 1.05 { Color32::BLACK } else { Color32::WHITE }
}

/// The caret colour at page point (`x`, `y`) of page `page`: [`caret_contrast`] of the paper,
/// shading or highlight there. Cached until the layout, the view or the point changes, so the
/// blink doesn't rebuild the page's draw list every frame.
fn caret_color_at(app: &mut WordApp, layout: &std::sync::Arc<DocLayout>, page: usize, x: f32, y: f32, view: u64, dim_body: bool) -> Color32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (std::sync::Arc::as_ptr(layout) as usize, page, x.to_bits(), y.to_bits(), view).hash(&mut h);
    let key = h.finish();
    if let Some((k, c)) = app.canvas.caret_bg
        && k == key
    {
        return c;
    }
    let Some(p) = layout.pages.get(page) else { return Color32::BLACK };
    let bg = wordcraft_render::background_at(&app.session.doc, p, &page_render_options(app, dim_body), x, y);
    let c = caret_contrast(bg);
    app.canvas.caret_bg = Some((key, c));
    c
}

/// Render options for page rasters shown on screen: on macOS, text is darkened the way the system
/// draws it, so a page looks as it does in other Mac apps (exports never are).
pub(crate) fn screen_render_options() -> wordcraft_render::RenderOptions {
    wordcraft_render::RenderOptions { text_darkening: cfg!(target_os = "macos"), ..Default::default() }
}

/// The hyperlink under the pointer at (x, y) on `page`, where `pos` is the caret position the
/// point hits. The hit snaps to the nearest text, so the link only counts when the point is on
/// the linked character itself, not beside, above or below it (issue #413).
fn link_under(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32, pos: &Pos) -> Option<String> {
    let doc = &app.session.doc;
    let para = doc.para_at(pos)?;
    let at = wordcraft_geom::Point::new(x, y);
    // The caret falls between characters: the one under the point is on either side of it.
    [(pos.off, para.next_boundary(pos.off)), (para.prev_boundary(pos.off), pos.off)].into_iter().filter(|(a, b)| a < b).find_map(|(a, b)| {
        let link = para.props_of_char(a).link.clone().filter(|l| !l.is_empty())?;
        let (from, to) = (Pos { off: a, ..pos.clone() }, Pos { off: b, ..pos.clone() });
        layout.selection_rects(doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(at)).then_some(link)
    })
}

/// Whether Ctrl/⌘+click may hand a document's hyperlink to the system: web and email links only,
/// so a document can't launch `file:` paths, programs or custom-scheme handlers.
fn is_followable_link(url: &str) -> bool {
    wordcraft_doc::link::is_web_or_mail_link(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table_resize::tests::{H, cell, screen, steps, table_window};

    /// What `page_image` replaced: the raster un-premultiplied, for egui to premultiply again.
    fn by_straight_alpha(img: &wordcraft_render::Rendered) -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight())
    }

    #[test]
    fn page_image_keeps_every_pixel_the_straight_alpha_round_trip_gave() {
        // Every premultiplied pixel there is: a channel from 0 up to its alpha, for each alpha.
        let mut pixels = Vec::new();
        for a in 0..=255u8 {
            for c in 0..=a {
                pixels.extend([c, a, a - c, a]);
            }
        }
        let every = wordcraft_render::Rendered { width: (pixels.len() / 4) as u32, height: 1, pixels };
        assert!(page_image(&every).pixels == by_straight_alpha(&every).pixels, "a premultiplied pixel changed");
        // And real pages as the canvas draws them: text, a table, a picture.
        let mut s = wordcraft_engine::Session::new(wordcraft_engine::sample::sample_document());
        let layout = s.layout();
        assert!(!layout.pages.is_empty());
        for page in &layout.pages {
            let img = wordcraft_render::render_page(&s.doc, page, 2.0, &screen_render_options());
            let ours = page_image(&img);
            assert_eq!(ours.size, [img.width as usize, img.height as usize]);
            assert!(ours.pixels == by_straight_alpha(&img).pixels, "a page pixel changed");
        }
    }

    #[test]
    fn wheel_scrolls_while_drag_selecting() {
        assert!(wheel_hovered(true, false, false));
        assert!(wheel_hovered(true, true, true), "wheel keeps scrolling during a drag-select");
        assert!(!wheel_hovered(true, true, false), "another widget's drag still owns the wheel");
        assert!(!wheel_hovered(false, true, true));
    }

    #[test]
    fn reading_animation_eases_then_lands_exactly_on_column_start() {
        let goal = 330.0;
        let one = reading_animation_step(0.0, goal, 1.0 / 60.0);
        assert!(one > 0.0 && one < goal);
        let mut pos = one;
        for _ in 0..60 {
            let next = reading_animation_step(pos, goal, 1.0 / 60.0);
            assert!((pos..=goal).contains(&next));
            pos = next;
        }
        assert_eq!(pos, goal);
        assert!(reading_animation_step(500.0, goal, 0.1).is_finite());
    }

    #[test]
    fn reading_wheel_snaps_one_column_per_tick_and_respects_touchpads() {
        let ev = |unit, y| egui::Event::MouseWheel { unit, delta: vec2(0.0, y), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        let mut partial = 0.0;
        for _ in 0..6 {
            assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -65.0)], true, &mut partial), 1);
            assert_eq!(partial, 0.0, "a large wheel tick cannot spill into the next column");
        }
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, -1.0)], true, &mut partial), 1);
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, 1.0)], true, &mut partial), -1);
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 0);
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 0);
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Point, -20.0)], true, &mut partial), 1);
        assert_eq!(read_wheel_turns(&[ev(egui::MouseWheelUnit::Line, -1.0)], false, &mut partial), 0);
        let (rects, size) = place_read_pages(&[(240.0, 500.0); 10], 1.0, vec2(1100.0, 600.0));
        let last_left = rects.last().unwrap().left();
        assert!((size.x - 1100.0 - (last_left - GAP)).abs() < 1.0, "last column aligns against the pane");
        assert!(reading_size(vec2(1440.0, 750.0), PX_PER_PT, Some(612.0)).0 <= 612.0);
    }

    /// #299: the caret is black on light backgrounds and white on dark ones, whatever the UI
    /// theme: on white paper, a dark page colour, a dark page (Switch Modes) and a highlight.
    #[test]
    fn caret_contrasts_with_what_lies_under_it() {
        use wordcraft_doc::props::Rgb;
        assert_eq!(caret_contrast(Rgb::WHITE), Color32::BLACK);
        assert_eq!(caret_contrast(Rgb::BLACK), Color32::WHITE);
        assert_eq!(caret_contrast(Rgb(0xFF, 0xFF, 0x00)), Color32::BLACK, "yellow highlight");
        assert_eq!(caret_contrast(Rgb(0x00, 0x00, 0x80)), Color32::WHITE, "dark blue");
        assert_eq!(caret_contrast(Rgb(0x33, 0x33, 0x33)), Color32::WHITE, "dark paper");
        assert_eq!(caret_contrast(Rgb(0xC0, 0xC0, 0xC0)), Color32::BLACK, "light grey");

        let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
        let mut a = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
        let at = |a: &mut WordApp, off: usize| {
            let layout = a.session.layout();
            let c = layout.caret_on(&Pos::body(0, off), 0).unwrap();
            let view = view_key(a, false);
            caret_color_at(a, &layout, c.page, c.x, c.top + c.height / 2.0, view, false)
        };
        assert_eq!(at(&mut a, 2), Color32::BLACK, "white paper");
        a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
        assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page colour");
        a.run("design.pageColor", json!({})).unwrap();
        assert_eq!(at(&mut a, 2), Color32::BLACK, "page colour removed");
        a.session.view.dark_mode = true;
        assert_eq!(at(&mut a, 2), Color32::WHITE, "dark page");
        a.session.view.dark_mode = false;
        a.run("select.text", json!({"text": "Hello"})).unwrap();
        a.run("format.highlight", json!({"color": "black"})).unwrap();
        assert_eq!(at(&mut a, 2), Color32::WHITE, "inside a black highlight");
        assert_eq!(at(&mut a, 9), Color32::BLACK, "past it");
        assert_eq!(at(&mut a, 2), Color32::WHITE, "again, from the cache");
    }

    /// #299: the caret's colour is worked out only when the caret is drawn: with text selected
    /// there is no caret, so the page's draw list isn't built for it.
    #[test]
    fn caret_colour_is_not_worked_out_while_the_caret_is_hidden() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
        a.run("select.text", json!({"text": "Hello"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(4);
        assert!(h.state().canvas.caret_bg.is_none(), "no caret, no colour lookup");
    }

    /// #167: dragging a ruler's margin edge sets that margin, snapped, never negative, and
    /// always leaving an inch of text.
    #[test]
    fn margin_drags_are_clamped() {
        let s = wordcraft_doc::section::SectionProps {
            page_w: 612.0,
            page_h: 792.0,
            margin_left: 72.0,
            margin_right: 72.0,
            margin_top: 72.0,
            margin_bottom: 72.0,
            gutter: 0.0,
            ..Default::default()
        };
        assert_eq!(dragged_margin(MarginEdge::Left, 100.0, &s), 99.0, "snapped to 1/16 inch");
        assert_eq!(dragged_margin(MarginEdge::Right, 612.0 - 36.0, &s), 36.0);
        assert_eq!(dragged_margin(MarginEdge::Left, -50.0, &s), 0.0, "never negative");
        assert_eq!(dragged_margin(MarginEdge::Left, 600.0, &s), 612.0 - 72.0 - 72.0, "an inch of text is left");
        assert_eq!(dragged_margin(MarginEdge::Right, 0.0, &s), 612.0 - 72.0 - 72.0);
        assert_eq!(dragged_margin(MarginEdge::Top, 9.0, &s), 9.0);
        assert_eq!(dragged_margin(MarginEdge::Bottom, 100.0, &s), 792.0 - 72.0 - 72.0);
        let g = wordcraft_doc::section::SectionProps { gutter: 36.0, ..s.clone() };
        assert_eq!(dragged_margin(MarginEdge::Left, 144.0, &g), 108.0, "the gutter isn't margin");
        assert_eq!(dragged_margin(MarginEdge::Left, f32::NAN, &s), 72.0, "junk keeps the margin");

        // Through the command: the result is applied and is one undo step.
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let sect = wordcraft_engine::cmd::page::sect(&app.session);
        let v = dragged_margin(MarginEdge::Left, sect.page_w, &sect);
        app.run("layout.margins", json!({"left": v})).unwrap();
        let after = wordcraft_engine::cmd::page::sect(&app.session);
        assert!(after.page_w - after.margin_left - after.margin_right - after.gutter >= MIN_TEXT - 0.01);
        assert!(app.session.undo());
        assert_eq!(wordcraft_engine::cmd::page::sect(&app.session).margin_left, sect.margin_left);
    }

    /// Show revisions in balloons: a document whose only revision is a deletion gets the markup
    /// area for its balloon; the other modes have nothing to show there, and neither does the view
    /// while the Comments pane is open.
    #[test]
    fn a_deletion_alone_opens_the_markup_area_when_revisions_go_in_balloons() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("document.setText", json!({"text": "keep gone"})).unwrap();
        a.run("review.trackChanges", json!({"value": true})).unwrap();
        a.run("select.range", json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 9}})).unwrap();
        a.run("text.delete", json!({})).unwrap();
        assert_eq!(markup_width(&a), 0.0, "comments and formatting only: nothing to show");
        a.run("review.trackingOptions", json!({"balloons": "revisions"})).unwrap();
        assert!(markup_width(&a) > 0.0, "the deletion's balloon needs the markup area");
        a.run("view.commentsPane", json!({"value": true})).unwrap();
        assert_eq!(markup_width(&a), 0.0);
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("review.trackingOptions", json!({"insertionsDeletions": false})).unwrap();
        assert_eq!(markup_width(&a), 0.0, "deletions not shown at all");
    }

    /// #549: presses count as double/triple as they go down; slow or far presses start again.
    #[test]
    fn presses_count_up_to_three() {
        let at = pos2(100.0, 100.0);
        assert_eq!(press_count(None, 5.0, at, 0.3, 6.0), 1);
        assert_eq!(press_count(Some((5.0, at, 1)), 5.1, at, 0.3, 6.0), 2);
        assert_eq!(press_count(Some((5.1, at, 2)), 5.2, at + vec2(3.0, 0.0), 0.3, 6.0), 3);
        assert_eq!(press_count(Some((5.2, at, 3)), 5.3, at, 0.3, 6.0), 3, "stays a triple");
        assert_eq!(press_count(Some((5.0, at, 1)), 5.5, at, 0.3, 6.0), 1, "too slow");
        assert_eq!(press_count(Some((5.0, at, 1)), 5.1, at + vec2(20.0, 0.0), 0.3, 6.0), 1, "too far");
        assert_eq!(press_count(Some((5.0, at, 1)), 4.0, at, 0.3, 6.0), 1, "the clock went back");
    }

    /// #549: double-click a word and drag: the selection grows by whole words; triple-click and
    /// drag: by whole paragraphs (Word's behaviour).
    #[test]
    fn multi_click_drag_extends_by_units() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Alpha beta gamma delta\rSecond para\rThird para"})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let screen = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: Pos| {
            let c = h.state_mut().session.layout().caret(&pos).expect("caret");
            page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen")
        };
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let multi_drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, clicks: usize| {
            h.event(Event::PointerMoved(from));
            for i in 0..clicks {
                h.event(button(from, true));
                h.step();
                if i + 1 < clicks {
                    h.event(button(from, false));
                    h.step();
                }
            }
            for k in 1..=4 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
                h.step();
            }
            h.event(button(to, false));
            for _ in 0..3 {
                h.step();
            }
        };
        // Double-click inside "beta", drag into "gamma": "beta gamma " is selected.
        let (from, to) = (screen(&mut h, Pos::body(0, 8)), screen(&mut h, Pos::body(0, 13)));
        multi_drag(&mut h, from, to, 2);
        let (s, e) = h.state().session.sel.ordered();
        assert_eq!((s, e), (Pos::body(0, 6), Pos::body(0, 17)), "whole words");
        // Let the double-click time pass, then triple-click the second paragraph and drag into the third.
        for _ in 0..40 {
            h.step();
        }
        let (from, to) = (screen(&mut h, Pos::body(1, 3)), screen(&mut h, Pos::body(2, 2)));
        multi_drag(&mut h, from, to, 3);
        let sel = h.state().session.sel.clone();
        assert_eq!((sel.anchor, sel.focus), (Pos::body(1, 0), Pos::body(2, 10)), "whole paragraphs");
        // A plain double-click (no drag) still selects just the word.
        h.state_mut().run("select.collapse", json!({})).unwrap();
        for _ in 0..40 {
            h.step();
        }
        let at = screen(&mut h, Pos::body(0, 1));
        multi_drag(&mut h, at, at, 2);
        let (s, e) = h.state().session.sel.ordered();
        assert_eq!((s, e), (Pos::body(0, 0), Pos::body(0, 6)));
    }

    /// #1014: Ctrl+drag on a shape leaves it and drops a copy; a plain drag moves it.
    #[test]
    fn ctrl_drag_copies_an_object() {
        use egui::{Event, Modifiers, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Some text"})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("arrange.bounds", json!({"width": 100, "height": 60, "page": 0, "x": 200, "y": 300})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let objects = |h: &mut egui_kittest::Harness<'_, WordApp>| {
            let doc = &h.state().session.doc;
            doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).map_or(0, |p| p.objects.len())
        };
        let xs = |h: &mut egui_kittest::Harness<'_, WordApp>| {
            let mut v: Vec<i32> = Vec::new();
            for x in (0..600).step_by(50) {
                if h.state_mut().session.layout().find_object(0, |o| (o.rect.x - x as f32).abs() < 0.5).is_some() {
                    v.push(x);
                }
            }
            v
        };
        let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, mods: Modifiers| {
            let from = page_to_screen(h.state_mut(), 0, 250.0, 330.0).expect("on screen");
            let to = page_to_screen(h.state_mut(), 0, 400.0, 330.0).expect("on screen");
            h.event(Event::ModifiersChanged(mods));
            h.event(Event::PointerMoved(from));
            h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: mods });
            h.step();
            for k in 1..=4 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
                h.step();
            }
            h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: mods });
            for _ in 0..3 {
                h.step();
            }
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.step();
        };
        assert_eq!((objects(&mut h), xs(&mut h)), (1, vec![200]));
        drag(&mut h, Modifiers::COMMAND);
        assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![200, 350]), "the original stays, a copy lands 150pt right");
        // Without Ctrl the object under the pointer (the original) is moved, not copied again:
        // it lands on the copy.
        drag(&mut h, Modifiers::NONE);
        assert_eq!((objects(&mut h), xs(&mut h)), (2, vec![350]));
    }

    /// #856: the left margin beside body text is Word's selection bar. A click selects the line,
    /// a double-click the paragraph, a triple-click (or Ctrl/⌘+click) the document; dragging
    /// selects whole lines and Shift+click extends by lines.
    #[test]
    fn selection_bar_clicks_select_lines() {
        use egui::{Event, Modifiers, PointerButton};
        let long = "The quick brown fox jumps over the lazy dog. ".repeat(8);
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": format!("{}\rSecond para\rThird para", long.trim_end())})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        // A point in the left margin beside the line `li` lines below the one holding `pos`.
        let margin = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: Pos, li: f32| {
            let c = h.state_mut().session.layout().caret(&pos).expect("caret");
            page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height * (li + 0.5)).expect("on screen")
        };
        let line_start = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
            let (rects, scale) = (h.state().canvas.page_rects.clone(), h.state().canvas.scale);
            let layout = h.state_mut().session.layout();
            let (pg, x, y) = page_at(&rects, &layout, scale, at).expect("page");
            layout.selection_bar_at(pg, x, y).expect("in the selection bar")
        };
        let span = |h: &mut egui_kittest::Harness<'_, WordApp>, pos: &Pos| h.state_mut().session.layout().line_span(pos, 0).expect("line");
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let clicks_drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, clicks: usize| {
            h.event(Event::PointerMoved(from));
            for i in 0..clicks {
                h.event(button(from, true));
                h.step();
                if i + 1 < clicks {
                    h.event(button(from, false));
                    h.step();
                }
            }
            for k in 1..=4 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
                h.step();
            }
            h.event(button(to, false));
            for _ in 0..3 {
                h.step();
            }
        };
        let pause = |h: &mut egui_kittest::Harness<'_, WordApp>| {
            for _ in 0..40 {
                h.step();
            }
        };
        // Click beside the second line: that whole line is selected, not a caret at its start.
        let at = margin(&mut h, Pos::body(0, 0), 1.0);
        let l2 = line_start(&mut h, at);
        assert!(l2.off > 0 && l2.path == wordcraft_doc::Path::top(0));
        clicks_drag(&mut h, at, at, 1);
        let (_, l2_end, _) = span(&mut h, &l2);
        assert_eq!(h.state().session.sel.ordered(), (l2.clone(), l2_end.clone()), "the line");
        // Double-click: the paragraph, mark included.
        pause(&mut h);
        clicks_drag(&mut h, at, at, 2);
        assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(1, 0)), "the paragraph");
        // Triple-click: the whole document.
        pause(&mut h);
        clicks_drag(&mut h, at, at, 3);
        assert_eq!(h.state().session.sel.ordered(), (Pos::body(0, 0), Pos::body(2, 10)), "everything");
        // Drag from beside the first line down past the third: three whole lines.
        pause(&mut h);
        let (from, to) = (margin(&mut h, Pos::body(0, 0), 0.0), margin(&mut h, Pos::body(0, 0), 2.0));
        let l3 = line_start(&mut h, to);
        clicks_drag(&mut h, from, to, 1);
        let (_, l3_end, _) = span(&mut h, &l3);
        let sel = h.state().session.sel.clone();
        assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), l3_end), "three lines");
        // Shift+click beside "Second para" extends by lines, to that paragraph's mark.
        pause(&mut h);
        let second = margin(&mut h, Pos::body(1, 0), 0.0);
        h.event(Event::ModifiersChanged(Modifiers::SHIFT));
        h.step();
        clicks_drag(&mut h, second, second, 1);
        h.event(Event::ModifiersChanged(Modifiers::default()));
        h.step();
        let sel = h.state().session.sel.clone();
        assert_eq!((sel.anchor, sel.focus), (Pos::body(0, 0), Pos::body(2, 0)), "extended by lines");
        // Hovering the bar shows the arrow, not the I-beam.
        h.event(Event::PointerMoved(at));
        h.step();
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    }

    /// #1085: a selected balloon's buttons work (pressing them is not a click on the page that
    /// leaves the balloon): a new comment with no text resolves and deletes from its balloon.
    #[test]
    fn an_empty_comment_balloon_deletes() {
        for body in ["", "Some text here"] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
            if !body.is_empty() {
                a.run("text.insert", json!({"text": body})).unwrap();
                a.run("select.text", json!({"text": "text"})).unwrap();
            }
            let id = a.run("review.newComment", json!({})).unwrap()["id"].as_u64().unwrap() as u32;
            a.run("view.commentsPane", json!({"value": false})).unwrap();
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            for _ in 0..6 {
                h.step();
            }
            let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: egui::Pos2| {
                h.event(egui::Event::PointerMoved(at));
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
                h.step();
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
                for _ in 0..3 {
                    h.step();
                }
            };
            let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
            click(&mut h, card.left_top() + vec2(30.0, 8.0));
            assert_eq!(h.state().canvas.balloon, Some(id), "{body:?}: the click selected the balloon");
            use egui_kittest::kittest::Queryable;
            let resolve = h.get_by_label("Resolve").rect().center();
            click(&mut h, resolve);
            assert!(h.state().session.doc.comments[&id].resolved, "{body:?}: Resolve worked");
            assert_eq!(h.state().canvas.balloon, Some(id), "{body:?}: the balloon stays selected");
            let del = h.get_by_label("Delete").rect().center();
            click(&mut h, del);
            assert!(h.state().session.doc.comments.is_empty(), "{body:?}: the empty comment was deleted");
        }
    }

    /// A typed character is laid out and painted in the frame that delivers the key, not the next
    /// one: keys are handled before the canvas takes the layout it paints.
    #[test]
    fn a_keystroke_is_painted_in_its_own_frame() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Some text"})).unwrap();
        a.canvas.want_focus = true;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        assert!(h.state().canvas.focused, "the canvas has the keyboard focus");
        h.event(egui::Event::Text("x".into()));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Some textx");
        let latest = h.state_mut().session.layout();
        let painted = h.state().canvas.page_keys.layout.clone();
        assert!(painted.is_some_and(|l| std::sync::Arc::ptr_eq(&l, &latest)), "the frame painted the layout with the typed character");
    }

    /// #167: a comment balloon edits in place like Word's: click it, type, leave it, and the
    /// comment has the new text as one undo step.
    /// Lays `text` out as a read-only balloon does, with a document face that covers Arabic, and
    /// returns each glyph's character and left edge.
    fn balloon_glyphs(text: &str, width: f32) -> Option<Vec<(char, f32)>> {
        let face = wordcraft_fonts::FontDb::global().fallback_for('ش', u32::MAX)?;
        let ctx = egui::Context::default();
        let mut fonts = crate::theme::font_definitions(false, false);
        crate::theme::add_fallback_faces(&mut fonts, &[face]);
        ctx.set_fonts(fonts);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let mut out = Vec::new();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = balloon_text(ui.painter(), text.to_string(), egui::FontId::proportional(14.0), Color32::BLACK, width);
            for r in &g.rows {
                out.extend(r.row.glyphs.iter().map(|gl| (gl.chr, r.pos.x + gl.pos.x)));
            }
        })
        .drop_without_applying_deltas();
        Some(out)
    }

    /// Where egui draws a right-to-left balloon's words, measured: the first word on the right.
    #[test]
    fn a_right_to_left_balloon_reads_from_the_right() {
        let x = |g: &[(char, f32)], c: char| g.iter().find(|(ch, _)| *ch == c).map(|(_, x)| *x).unwrap();
        let Some(g) = balloon_glyphs("شركة اي", 400.0) else { return };
        assert!(x(&g, 'ش') > x(&g, 'ي'), "the first word is right of the second: {g:?}");
        assert!(x(&g, 'ش') > 300.0, "the line is set against the right edge: {g:?}");
        // A Latin word inside keeps its letters left to right; its brackets face it.
        let g = balloon_glyphs("الربط (APIs) هنا", 400.0).unwrap();
        assert!(x(&g, 'A') < x(&g, 's'));
        assert!(x(&g, 'ط') > x(&g, 'A') && x(&g, 'A') > x(&g, 'ه'), "{g:?}");
        // The opening bracket, on the right in Arabic, is drawn mirrored: `(APIs)` on screen.
        assert!(x(&g, '(') < x(&g, 'A') && x(&g, ')') > x(&g, 's'), "brackets face the word: {g:?}");
        // A paragraph that starts in Latin runs left to right.
        let g = balloon_glyphs("@Ahmed برجاء مراجعة", 400.0).unwrap();
        assert!(x(&g, '@') < x(&g, 'ب') && x(&g, '@') < 50.0, "{g:?}");
        assert!(x(&g, 'ب') > x(&g, 'م'), "the Arabic words still read from the right: {g:?}");
        // Wrapped rows stay inside the width.
        let long = "بيان الأنظمة المطلوب ربطها ومتطلبات التكامل وواجهات الربط البرمجي وآلية تبادل البيانات";
        let g = balloon_glyphs(long, 200.0).unwrap();
        assert!(g.iter().all(|(_, x)| *x <= 200.0), "{g:?}");
    }

    #[test]
    fn balloon_rows_put_right_to_left_words_in_display_order() {
        let row = |text: &str| {
            let info = unicode_bidi::BidiInfo::new(text, None);
            let p = info.paragraphs.first().unwrap();
            visual_row(&info, p, text, p.range.clone())
        };
        // Words swap, each word's letters stay as they are (egui shapes the word).
        assert_eq!(row("شركة اي للكهرباء"), "للكهرباء اي شركة");
        // A Latin name and a number inside Arabic keep their own order.
        assert_eq!(row("يجب ربط Odoo 19 هنا"), "هنا Odoo 19 ربط يجب");
        assert_eq!(row("Should include Meters system API"), "Should include Meters system API");
        assert!(!has_rtl("Should include Meters system API"));
        assert!(has_rtl("נא לבדוק"));
        // An empty row at the end of the text (egui's last row after a wrap) is empty, not a panic.
        let text = "شركة اي";
        let info = unicode_bidi::BidiInfo::new(text, None);
        let p = info.paragraphs.first().unwrap();
        assert_eq!(visual_row(&info, p, text, text.len()..text.len()), "");
        assert_eq!(visual_row(&info, p, text, 0..text.len() + 5), "");
    }

    #[test]
    fn a_balloon_edits_its_comment() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Some text here"})).unwrap();
        a.run("select.text", json!({"text": "text"})).unwrap();
        let id = a.run("review.newComment", json!({"text": "Original"})).unwrap()["id"].as_u64().unwrap() as u32;
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let card = h.state().canvas.balloon_rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r).expect("the comment has a balloon");
        let at = card.left_top() + vec2(30.0, 8.0);
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(h.state().canvas.balloon, Some(id), "the click selected the balloon");
        let depth = h.state().session.undo_depth();
        for piece in ["Edited", " twice"] {
            h.event(egui::Event::Text(piece.into()));
            h.step();
        }
        assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while typing");
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.step();
        }
        let part = h.state().session.doc.comments[&id].part;
        let text = h.state().session.doc.plain_text(StoryRef::Part(part));
        assert!(text.contains("Edited twice") && text.contains("Original"), "{text:?}");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the edit");
        assert!(h.state_mut().session.undo());
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Part(part)), "Original");
    }

    /// #960: double-clicking the top margin of a document without a header creates one and
    /// edits it (the bottom margin: a footer), as in Word; typing goes into it, and
    /// double-clicking the body leaves it. One left empty is taken away again.
    #[test]
    fn double_click_margin_creates_header_and_footer() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Body text"})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 2200.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let double_click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
            h.event(Event::PointerMoved(at));
            for _ in 0..2 {
                h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
                h.step();
                h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
                h.step();
            }
            // Let the double-click time pass before the next one.
            for _ in 0..40 {
                h.step();
            }
        };
        let kind = |h: &egui_kittest::Harness<'_, WordApp>| part_kind(h.state(), h.state().session.sel.focus.story);
        let page = h.state_mut().session.layout().pages[0].clone();
        assert!(page.header_story.is_none() && page.footer_story.is_none());
        let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
        let body = page_to_screen(h.state_mut(), 0, x, page.body.y + 6.0).expect("body on screen");
        // Top margin: a new header, the caret in it, typing goes there.
        let at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
        double_click(&mut h, at);
        assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
        let header = h.state().session.sel.focus.story;
        h.event(Event::Text("Running head".into()));
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(h.state().session.doc.plain_text(header), "Running head");
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "Body text");
        // Double-click the body: back in it, the header kept.
        double_click(&mut h, body);
        assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
        assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(header));
        // Bottom margin: a new footer; left empty, leaving takes it away again.
        let at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
        double_click(&mut h, at);
        assert_eq!(kind(&h), Some(PartKind::Footer), "editing a new footer");
        double_click(&mut h, body);
        assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
        assert!(h.state().session.doc.last_section.footers.default.is_none(), "the empty footer is gone");
        // And a written footer stays.
        double_click(&mut h, at);
        let footer = h.state().session.sel.focus.story;
        assert_eq!(kind(&h), Some(PartKind::Footer));
        h.event(Event::Text("Page foot".into()));
        h.step();
        double_click(&mut h, body);
        assert_eq!(h.state().session.sel.focus.story, StoryRef::Body);
        assert_eq!(h.state().session.doc.plain_text(footer), "Page foot");
    }

    /// #513: right-clicking a picture selects it and offers Wrap Text, as in Word; choosing
    /// Square there wraps text around it.
    #[test]
    fn right_click_on_a_picture_offers_wrap_text() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Text before the picture "})).unwrap();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
        let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let rect = h.state_mut().session.layout().object(&pos, 0).expect("the picture is laid out").rect;
        let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
        assert_eq!(picked, Some(pos.clone()), "the right-click selected the picture");
        h.get_by_label_contains("Wrap Text").hover();
        for _ in 0..3 {
            h.step();
        }
        assert!(h.query_by_label_contains("Fix Position on Page").is_some(), "the anchoring choices are offered");
        h.get_by_label_contains("Square").click();
        for _ in 0..3 {
            h.step();
        }
        let o = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap().1;
        assert_eq!(o.frame().unwrap().2.wrap, wordcraft_doc::para::Wrap::Square);
    }

    /// #1197: the context menu on a table of contents entry offers Update Field, as in Word, and
    /// it brings the table up to date.
    #[test]
    fn context_menu_on_a_toc_entry_updates_the_table() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::{Block, Paragraph, Path, props::CharProps};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
        a.run("caret.docStart", json!({})).unwrap();
        a.run("references.toc", json!({})).unwrap();
        let end = a.session.doc.body.len();
        let heading = Paragraph::with_text("Fresh heading", CharProps::default()).styled("Heading1");
        a.session.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(heading)).unwrap();
        a.session.relayout();
        let entry =
            a.session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1"))).expect("a TOC entry");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let c = h.state_mut().session.layout().caret(&Pos::body(entry, 2)).expect("the entry is laid out");
        let at = page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        assert!(h.query_all_by_label_contains("Paste").next().is_some(), "the context menu is open");
        h.get_by_label_contains("Update Field").click();
        for _ in 0..3 {
            h.step();
        }
        let text = h.state().session.doc.plain_text(StoryRef::Body);
        assert_eq!(text.matches("Fresh heading").count(), 2, "the new heading is listed in the table: {text}");
    }

    /// #2182: after Update Field has refreshed a table of contents once, the context menu on it
    /// still offers Update Field, so a later heading change can be brought in again.
    #[test]
    fn context_menu_on_a_toc_still_offers_update_field_after_an_update() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::{Block, Paragraph, Path, props::CharProps};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
        a.run("caret.docStart", json!({})).unwrap();
        a.run("references.toc", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for round in 0..2 {
            let name = format!("Fresh heading {round}");
            let end = h.state().session.doc.body.len();
            let heading = Paragraph::with_text(&name, CharProps::default()).styled("Heading1");
            h.state_mut().session.doc.insert_block(StoryRef::Body, &Path::top(end), Block::Para(heading)).unwrap();
            h.state_mut().session.relayout();
            for _ in 0..6 {
                h.step();
            }
            let entry = h
                .state()
                .session
                .doc
                .body
                .iter()
                .position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1")))
                .expect("a TOC entry");
            let c = h.state_mut().session.layout().caret(&Pos::body(entry, 2)).expect("the entry is laid out");
            let at = page_to_screen(h.state_mut(), c.page, c.x + 1.0, c.top + c.height / 2.0).expect("on screen");
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
                h.step();
            }
            h.step();
            assert!(h.query_all_by_label_contains("Paste").next().is_some(), "round {round}: the context menu is open");
            assert!(h.query_by_label_contains("Update Field").is_some(), "round {round}: Update Field is offered");
            h.get_by_label_contains("Update Field").click();
            for _ in 0..3 {
                h.step();
            }
            let text = h.state().session.doc.plain_text(StoryRef::Body);
            assert_eq!(text.matches(&name).count(), 2, "round {round}: the new heading is listed: {text}");
        }
    }

    /// #996: on macOS a Control-click is the right-click (the trackpad's and the one-button
    /// mouse's way to it), so it opens the same menu and leaves the caret where the menu needs it.
    #[cfg(target_os = "macos")]
    #[test]
    fn control_click_on_macos_opens_the_context_menu() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "The quick brwn fox"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 12));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let (p, ht) = caret_screen(h.state_mut()).unwrap();
        let at = p + egui::vec2(0.0, ht / 2.0);
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        h.step();
        let ctrl = egui::Modifiers { ctrl: true, ..Default::default() };
        // The events winit delivers for a Control-click, through the app's input hook as eframe runs it.
        let send = |h: &mut egui_kittest::Harness<'_, WordApp>, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput { events, ..Default::default() };
            h.state_mut().raw_input_hook(&mut raw);
            for e in raw.events {
                h.event(e);
            }
            h.step();
        };
        send(&mut h, vec![egui::Event::ModifiersChanged(ctrl), egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            send(&mut h, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: ctrl }]);
        }
        h.event(egui::Event::ModifiersChanged(Default::default()));
        h.step();
        assert!(h.state().canvas.context_menu_open, "the Control-click opened the menu");
        h.get_by_label("brown").click();
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "The quick brown fox");
    }

    #[test]
    fn floating_picture_can_be_dragged_in_front_of_or_behind_text() {
        for wrap in ["inFrontOfText", "behindText"] {
            drag_floating_picture(wrap, false, true);
        }
    }

    #[test]
    fn floating_header_picture_can_be_dragged_in_front_of_or_behind_text() {
        for wrap in ["inFrontOfText", "behindText"] {
            drag_floating_picture(wrap, true, true);
        }
    }

    /// A Behind Text picture pressed over the text it sits behind, with no object selected: the
    /// press becomes a drag (moving it), while a plain click still edits that text.
    #[test]
    fn behind_picture_dragged_over_text_without_being_selected_first() {
        drag_floating_picture("behindText", false, false);
        drag_floating_picture("behindText", true, false);
    }

    fn drag_floating_picture(wrap: &str, header: bool, preselect: bool) {
        use egui::{Event, PointerButton};
        {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
            if header {
                a.run("insert.editHeader", json!({})).unwrap();
            }
            a.run("text.insert", json!({"text": "The picture overlaps this line of text."})).unwrap();
            let mut png = Vec::new();
            let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
            image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            a.run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 160})).unwrap();
            let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
            a.run("arrange.bounds", json!({"x": 120, "y": 80})).unwrap();
            a.run("arrange.wrap", json!({"wrap": wrap})).unwrap();
            let before = a.session.layout().object(&pos, 0).unwrap().rect;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                a,
            );
            for _ in 0..6 {
                h.step();
            }
            let page = h.state().canvas.page_rects[0];
            let scale = h.state().canvas.scale;
            let frame = crate::frame::to_screen(page, scale, before);
            // The top edge crosses the document's first text line, so Behind Text must still be
            // draggable where its pixels overlap body text. Stay away from resize handles.
            let from = pos2(frame.left() + frame.width() * 0.25, frame.top() + 5.0 * scale);
            let to = from + vec2(50.0, 40.0);
            let button = |at, pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            if wrap == "behindText" {
                // A click without drag threshold still reaches the text above the picture.
                h.event(Event::PointerMoved(from));
                h.event(button(from, true));
                h.step();
                h.event(button(from, false));
                for _ in 0..3 {
                    h.step();
                }
                assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_none(), "a click still selects text");
                if preselect {
                    let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
                    h.state_mut().run("select.range", json!({"anchor": pos, "focus": end})).unwrap();
                }
                for _ in 0..40 {
                    h.step();
                }
            }
            h.event(Event::PointerMoved(from));
            h.event(button(from, true));
            h.step();
            assert!(h.state().canvas.obj_drag.is_some(), "{wrap}: pressing the image starts a frame drag");
            for k in 1..=4 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
                h.step();
            }
            h.event(button(to, false));
            for _ in 0..3 {
                h.step();
            }
            let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
            let moved = picked.and_then(|p| h.state_mut().session.layout().object(&p, 0)).map(|o| o.rect);
            assert!(
                moved.is_some_and(|after| (after.x - before.x).abs() > 1.0 || (after.y - before.y).abs() > 1.0),
                "{wrap}: {before:?} -> {moved:?}"
            );
        }
    }

    #[test]
    fn only_web_and_email_links_are_followed() {
        for ok in ["https://example.com/a?b#c", "http://example.com", "HTTPS://EXAMPLE.COM", "mailto:someone@example.com"] {
            assert!(is_followable_link(ok), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "file://C:/Windows/System32/calc.exe",
            "C:\\Windows\\System32\\calc.exe",
            "/usr/bin/xterm",
            "javascript:alert(1)",
            "ms-msdt:/id",
            "smb://host/share",
            " https://example.com",
            "\thttps://example.com",
            "https\u{0}://example.com",
            "",
            "example.com",
        ] {
            assert!(!is_followable_link(bad), "{bad:?}");
        }
    }

    /// Ctrl+click on a link to a bookmark puts the caret at the bookmark's start, as Word does;
    /// only Go To selects the bookmark's text (#1236).
    #[test]
    fn a_link_to_a_bookmark_puts_the_caret_at_its_start() {
        let mut a =
            WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Alpha beta gamma\nSee here")), Default::default());
        a.run("select.text", json!({"text": "beta"})).unwrap();
        a.run("insert.bookmark", json!({"name": "Spot"})).unwrap();
        a.run("select.text", json!({"text": "here"})).unwrap();
        a.run("insert.link", json!({"url": "#Spot"})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        // Between the link's "h" and "e".
        let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(1), off: 5 }).unwrap();
        let at = screen(&mut h, c.x, c.top + c.height / 2.0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
        h.event_modifiers(press, egui::Modifiers::COMMAND);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        steps(&mut h, 3);
        let s = &h.state().session;
        let (start, _) = wordcraft_engine::cmd::insert::bookmark_range(&s.doc, "Spot").unwrap();
        assert!(s.sel.is_collapsed(), "a caret, not a selection: {:?}", s.sel);
        assert_eq!(s.sel.focus, start, "at the bookmark's start");
    }

    /// #688: the drag auto-scroll waits in the middle, ramps in from both edges and never leaves
    /// the scrollable range.
    #[test]
    fn edge_auto_scroll_ramps_from_the_margins_and_clamps() {
        let dt = 1.0 / 60.0;
        let (lo, hi) = (100.0, 500.0);
        // Nothing while the pointer is inside, including just within the margin.
        for y in [(lo + hi) / 2.0, lo + EDGE_MARGIN, hi - EDGE_MARGIN] {
            assert_eq!(edge_scroll(y, lo, hi, dt), 0.0, "y = {y}");
        }
        // Direction: above the top edge scrolls back (positive), below the bottom scrolls on
        // (negative), in the wheel's convention.
        assert!(edge_scroll(lo + 1.0, lo, hi, dt) > 0.0);
        assert!(edge_scroll(hi - 1.0, lo, hi, dt) < 0.0);
        // The ramp grows monotonically from the margin to the edge and caps past it.
        let above: Vec<f32> = (0..=8).map(|k| edge_scroll(lo + EDGE_MARGIN - k as f32 * EDGE_MARGIN / 8.0, lo, hi, dt)).collect();
        assert_eq!(above[0], 0.0);
        assert!(above.windows(2).all(|w| w[0] < w[1] && w[1] > 0.0), "{above:?}");
        assert_eq!(above[8], EDGE_SPEED_MAX * dt, "the hard edge scrolls at full speed");
        assert_eq!(edge_scroll(lo - 3.0 * EDGE_MARGIN, lo, hi, dt), above[8], "past the edge the speed caps");
        let below: Vec<f32> = (0..=4).map(|k| edge_scroll(hi - EDGE_MARGIN + k as f32 * EDGE_MARGIN / 4.0, lo, hi, dt)).collect();
        assert_eq!(below[0], 0.0);
        assert!(below.windows(2).all(|w| w[0] > w[1] && w[1] < 0.0), "{below:?}");
        assert_eq!(below[4], -EDGE_SPEED_MAX * dt);
        assert_eq!(edge_scroll(hi + 3.0 * EDGE_MARGIN, lo, hi, dt), below[4], "past the edge the speed caps");
        // A stalled frame doesn't jump the page.
        assert!(edge_scroll(hi - 1.0, lo, hi, 100.0) <= EDGE_SPEED_MAX * EDGE_MAX_DT + 1e-3);
        // Junk in, nothing out; neither an empty (or inverted) band.
        for (p, lo, hi, dt) in
            [(f32::NAN, lo, hi, dt), (200.0, f32::NAN, hi, dt), (200.0, lo, f32::INFINITY, dt), (200.0, lo, hi, f32::NAN), (200.0, 500.0, 100.0, dt)]
        {
            assert_eq!(edge_scroll(p, lo, hi, dt), 0.0);
        }

        // Applying it clamps to the scrollable range; a document that fits never scrolls.
        let max = vec2(0.0, 400.0);
        assert_eq!(clamped_scroll(egui::Vec2::ZERO, vec2(0.0, 50.0), max).y, 0.0, "never past the top");
        assert_eq!(clamped_scroll(vec2(0.0, 400.0), vec2(0.0, -50.0), max).y, 400.0, "never past the bottom");
        assert_eq!(clamped_scroll(vec2(0.0, 300.0), vec2(0.0, 50.0), max).y, 250.0);
        assert_eq!(clamped_scroll(egui::Vec2::ZERO, vec2(0.0, 50.0), egui::Vec2::ZERO).y, 0.0, "nothing to scroll");
    }

    /// #1065: a middle click starts scrolling, the pointer below the click scrolls down on its
    /// own frame after frame, and a second middle click stops it; a primary click stops it without
    /// moving the caret. Held down and dragged, letting go stops it.
    #[test]
    fn middle_click_scrolls_toward_the_pointer_until_clicked_again() {
        let text = vec!["A line of text in a long document."; 400].join("\n");
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text)), Default::default());
        a.canvas.auto_scroll.supported = true;
        let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        let anchor = h.state().canvas.viewport.expect("the canvas is shown").center();
        let button = |h: &mut H, at: Pos2, button, pressed| {
            h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() });
            h.step();
        };
        let offset = |h: &H| h.state().canvas.scroll_offset.y;
        let below = anchor + vec2(0.0, 150.0);

        h.event(egui::Event::PointerMoved(anchor));
        button(&mut h, anchor, egui::PointerButton::Middle, true);
        button(&mut h, anchor, egui::PointerButton::Middle, false);
        h.run_steps(3);
        assert!(h.state().canvas.auto_scroll.active());
        assert_eq!(offset(&h), 0.0, "the pointer at the click doesn't scroll");
        h.event(egui::Event::PointerMoved(below));
        h.run_steps(10);
        let scrolled = offset(&h);
        assert!(scrolled > 50.0, "the pointer below the click scrolls down: {scrolled}");
        button(&mut h, below, egui::PointerButton::Middle, true);
        button(&mut h, below, egui::PointerButton::Middle, false);
        let stopped = offset(&h);
        h.run_steps(10);
        assert!(!h.state().canvas.auto_scroll.active());
        assert_eq!(offset(&h), stopped, "a second click stops it");

        // A primary click stops it too, and isn't a click on the page.
        let caret = h.state().session.sel.clone();
        button(&mut h, anchor, egui::PointerButton::Middle, true);
        button(&mut h, anchor, egui::PointerButton::Middle, false);
        assert!(h.state().canvas.auto_scroll.active());
        button(&mut h, anchor, egui::PointerButton::Primary, true);
        button(&mut h, anchor, egui::PointerButton::Primary, false);
        h.run_steps(2);
        assert!(!h.state().canvas.auto_scroll.active());
        assert_eq!(h.state().session.sel, caret, "the stopping click leaves the caret");

        // Held down and dragged: scrolls while held, stops on release.
        h.event(egui::Event::PointerMoved(anchor));
        button(&mut h, anchor, egui::PointerButton::Middle, true);
        h.event(egui::Event::PointerMoved(below));
        h.run_steps(5);
        let held = offset(&h);
        assert!(held > stopped, "scrolls while the wheel is held");
        button(&mut h, below, egui::PointerButton::Middle, false);
        let released = offset(&h);
        h.run_steps(10);
        assert!(!h.state().canvas.auto_scroll.active());
        assert_eq!(offset(&h), released, "letting go after a drag stops it");
    }

    /// A right-click at `at`, the button pressed and released a frame apart.
    fn right_click(h: &mut H, at: Pos2) {
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        steps(h, 2);
    }

    /// Where the open context menu's entry holding `label` is (an entry's label also has its
    /// shortcut or submenu arrow, and the ribbon above the page may show the same name).
    fn menu_entry(h: &H, label: &str) -> Option<Pos2> {
        use egui_kittest::kittest::Queryable;
        let top = h.state().canvas.page_rects.first().map_or(0.0, |r| r.min.y);
        h.query_all_by_label_contains(label).map(|n| n.rect()).find(|r| r.min.y >= top).map(|r| r.center())
    }

    /// Picks `path` in the open context menu with the mouse: over each submenu on the way, then a
    /// click on the last entry.
    fn pick(h: &mut H, path: &[&str]) {
        let Some((last, submenus)) = path.split_last() else { return };
        for label in submenus {
            let at = menu_entry(h, label).expect(label);
            h.event(egui::Event::PointerMoved(at));
            steps(h, 2);
        }
        let at = menu_entry(h, last).expect(last);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
        steps(h, 2);
    }

    /// Whether the open context menu has an entry holding each of `labels` (`true`) or none
    /// (`false`).
    fn assert_menu(h: &H, labels: &[&str], there: bool) {
        for label in labels {
            assert_eq!(menu_entry(h, label).is_some(), there, "{label} in the menu: {there}");
        }
    }

    /// Right-clicking a table cell puts the caret there, and the table's items follow what is
    /// selected, as in Word: with the caret in a cell, Split Cells; with the whole table,
    /// Distribute, AutoFit and Insert Caption… (never Select or Cell Alignment). Delete is there
    /// for both.
    #[test]
    fn the_context_menu_in_a_table_follows_the_selection() {
        let mut h = table_window();
        let b = cell(&mut h, 1, 2);
        let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
        right_click(&mut h, at);
        assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 2)), "the caret moved to the cell");
        assert_menu(&h, &["Insert", "Delete", "Split Cells", "Text Direction", "Table Properties…"], true);
        assert_menu(&h, &["Merge Cells", "Distribute Rows", "AutoFit", "Select", "Cell Alignment"], false);
        let depth = h.state().session.undo_depth();
        pick(&mut h, &["Insert", "Insert Rows Below"]);
        assert_eq!(h.state().session.doc.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.len()), Some(3));
        assert_eq!(h.state().session.undo_depth(), depth + 1);
        h.state_mut().run("table.selectTable", json!({})).unwrap();
        right_click(&mut h, at);
        assert!(h.state().session.cell_range().is_some(), "a right-click on the selected table keeps it");
        assert_menu(&h, &["Delete", "Distribute Rows", "Distribute Columns", "AutoFit", "Insert Caption…", "Table Properties…"], true);
        assert_menu(&h, &["Split Cells", "Merge Cells"], false);
        pick(&mut h, &["Delete", "Delete Table"]);
        assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()));
    }

    /// With just the caret in a cell, right-click › Delete › Delete Table removes the table, in
    /// one undo step (#1198: the menu offered only Delete Cells there).
    #[test]
    fn a_right_click_in_a_cell_deletes_the_table() {
        let mut h = table_window();
        let b = cell(&mut h, 0, 1);
        let at = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
        right_click(&mut h, at);
        assert!(h.state().session.cell_range().is_none(), "just the caret");
        let depth = h.state().session.undo_depth();
        pick(&mut h, &["Delete", "Delete Table"]);
        assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()));
        assert_eq!(h.state().session.undo_depth(), depth + 1);
    }

    /// With a column's cells selected, a right-click on one of them keeps them: the menu is a
    /// column's (Merge Cells, Distribute) and its Text Direction turns just the
    /// column, in one undo step; one on a cell beside them drops them, though that cell lies
    /// between the selection's ends in the text, and the menu is a cell's again.
    #[test]
    fn a_right_click_keeps_selected_cells_only_on_them() {
        use wordcraft_doc::props::TextDirection;
        let mut h = table_window();
        h.state_mut().run("table.selectColumn", json!({})).unwrap();
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 0))));
        let on = cell(&mut h, 1, 0);
        let at = screen(&mut h, on.x + on.w / 2.0, on.y + on.h / 2.0);
        right_click(&mut h, at);
        assert_eq!(h.state().session.cell_range().map(|r| r.cols), Some((0, 0)), "still the column");
        assert_menu(&h, &["Merge Cells", "Distribute Rows", "Distribute Columns", "Text Direction"], true);
        assert_menu(&h, &["Split Cells", "AutoFit"], false);
        let depth = h.state().session.undo_depth();
        pick(&mut h, &["Text Direction"]);
        assert_eq!(h.state().session.undo_depth(), depth + 1);
        let t = h.state().session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
        let dir = |r: usize, c: usize| t.rows[r].cells[c].props.text_direction;
        assert_eq!(
            [dir(0, 0), dir(1, 0), dir(0, 1), dir(1, 2)],
            [TextDirection::Down, TextDirection::Down, TextDirection::Horizontal, TextDirection::Horizontal]
        );
        let beside = cell(&mut h, 0, 2);
        let at = screen(&mut h, beside.x + beside.w / 2.0, beside.y + beside.h / 2.0);
        right_click(&mut h, at);
        assert!(h.state().session.cell_range().is_none(), "the column is no longer selected");
        assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((0, 2)));
        assert_menu(&h, &["Split Cells"], true);
        assert_menu(&h, &["Merge Cells"], false);
    }

    /// Issue #123: zoomed out, pages sit side by side in rows when the width allows; at 100% a
    /// letter page in an ordinary window stays one per row.
    #[test]
    fn pages_flow_into_rows_when_they_fit() {
        let letter = 612.0 * PX_PER_PT;
        assert_eq!(columns_for(1200.0, letter, 10, 1, true), 1, "100%: one page per row");
        assert_eq!(columns_for(1200.0, letter * 0.6, 10, 1, true), 2, "60%: two per row");
        assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, true), 5, "25%: five per row");
        assert_eq!(columns_for(1200.0, letter * 0.25, 3, 1, true), 3, "never more columns than pages");
        assert_eq!(columns_for(1200.0, letter * 0.25, 10, 1, false), 1, "fit modes that show one page");
        assert_eq!(columns_for(300.0, letter, 10, 2, true), 2, "Multiple Pages keeps two");
        // Hostile numbers never panic and give at least one column.
        for (w, slot, n) in
            [(f32::NAN, letter, 5), (1200.0, f32::NAN, 5), (f32::INFINITY, 1.0, 5), (1200.0, 0.0, 5), (-5.0, -1.0, 5), (1e30, 1e-30, 0)]
        {
            let c = columns_for(w, slot, n, 1, true);
            assert!((1..=n.max(1)).contains(&c), "{w} {slot} {n}: {c}");
        }

        // Mixed sizes: a landscape page among portrait ones, three per row.
        let sizes = [(612.0, 792.0), (792.0, 612.0), (612.0, 792.0), (612.0, 792.0)];
        let scale = 0.25;
        let (rects, size) = place_pages(&sizes, 0.0, scale, 3, vec2(1000.0, 600.0));
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0].top(), rects[1].top());
        assert_eq!(rects[1].top(), rects[2].top());
        assert!(rects[0].right() < rects[1].left() && rects[1].right() < rects[2].left(), "left to right: {rects:?}");
        assert!(rects[3].top() > rects[0].bottom(), "next row below the tallest page");
        assert_eq!(rects[3].left(), rects[0].left(), "rows share columns");
        assert!((rects[1].width() - 792.0 * scale).abs() < 1e-3);
        assert!(size.x >= 1000.0 && size.y >= rects[3].bottom());
        // Each page's centre maps back to that page.
        for (i, r) in rects.iter().enumerate() {
            assert_eq!(nearest_page(&rects, r.center()), Some(i));
        }
        // Hostile page sizes and scales stay finite.
        let bad = [(f32::NAN, f32::INFINITY), (-1.0, 0.0), (1e30, 1e30)];
        for scale in [f32::NAN, 0.0, -1.0, 1e9] {
            let (rects, size) = place_pages(&bad, f32::NAN, scale, 0, vec2(f32::NAN, -1.0));
            assert_eq!(rects.len(), 3);
            assert!(size.x.is_finite() && size.y.is_finite());
            assert!(rects.iter().all(|r| r.min.x.is_finite() && r.max.y.is_finite()), "{rects:?}");
        }
    }

    #[test]
    fn long_web_page_uses_only_visible_native_resolution_tiles() {
        // A 17,843 pt Web Layout page at 100% on a 2x display would need ~47k raster
        // rows. One 8192px texture would upscale the page over five times, blurring text.
        let screen_scale = PX_PER_PT;
        let raster_scale = screen_scale * 2.0;
        let page = Rect::from_min_size(pos2(100.0, -10_000.0), vec2(800.0 * screen_scale, 17_843.0 * screen_scale));
        let viewport = Rect::from_min_size(pos2(0.0, 100.0), vec2(1440.0, 700.0));
        assert!(17_843.0 * raster_scale > 8192.0);
        assert!(800.0 * raster_scale < 8192.0);
        let tiles = visible_tiles(page, viewport, screen_scale, raster_scale, TILE_HEIGHT_PX);
        assert!(!tiles.is_empty());
        assert!(tiles.len() <= 3, "only the viewport should be rasterized: {tiles:?}");
        let total_px = (17_843.0 * raster_scale).ceil() as usize;
        for tile in tiles {
            let start = tile * TILE_HEIGHT_PX;
            assert!(start < total_px);
            assert!(total_px.saturating_sub(start).min(TILE_HEIGHT_PX) <= TILE_HEIGHT_PX);
        }
        assert_eq!(visible_tiles(page, Rect::from_min_size(pos2(1500.0, 0.0), vec2(300.0, 500.0)), screen_scale, raster_scale, TILE_HEIGHT_PX), 0..0);
        assert_eq!(visible_tiles(page, viewport, 0.0, raster_scale, TILE_HEIGHT_PX), 0..0);
        assert_eq!(visible_tiles(page, viewport, screen_scale, f32::NAN, TILE_HEIGHT_PX), 0..0);
        assert_eq!(visible_tiles(page, viewport, screen_scale, raster_scale, 0), 0..0);
        assert!(visible_tiles(page, viewport, screen_scale, raster_scale, 512).len() <= 5);
    }

    #[test]
    fn texel_aligned_rect_snaps_the_origin_and_sizes_from_texture_pixels() {
        let layout = Rect::from_min_size(pos2(10.25, 20.75), vec2(80.0, 120.0));
        let visual = texel_aligned_rect(layout, vec2(161.0, 241.0), 2.0);

        assert_eq!(visual.min, pos2(10.5, 21.0));
        assert_eq!(visual.size(), vec2(80.5, 120.5));
    }

    /// 0.5 cm in points.
    const HALF_CM: f32 = 72.0 * 0.5 / 2.54;

    #[test]
    fn table_gridlines_include_header_and_footer_tables() {
        let cell = |y: f32, covered: bool, story: StoryRef| Placed::Cell {
            rect: wordcraft_geom::Rect::new(0.0, y, 10.0, 10.0),
            covered,
            table: Default::default(),
            row: 0,
            cell: 0,
            story,
            clip: false,
        };
        let page = Page {
            items: vec![cell(100.0, false, StoryRef::Body), cell(110.0, true, StoryRef::Body)],
            header: vec![cell(10.0, false, StoryRef::Part(1))],
            footer: vec![cell(200.0, false, StoryRef::Part(2))],
            ..Default::default()
        };
        let ys: Vec<f32> = gridline_cells(&page).map(|r| r.y).collect();
        assert_eq!(ys, [100.0, 10.0, 200.0]);
    }

    #[test]
    fn drawing_grid_starts_at_the_margins_and_stays_inside_them() {
        // Letter page with 1" margins: the text area is 468 x 648 pt from (72, 72).
        let body = wordcraft_geom::Rect::new(72.0, 72.0, 468.0, 648.0);
        let (xs, ys) = drawing_grid(body, (HALF_CM, HALF_CM));
        assert_eq!(xs.first().copied(), Some(72.0));
        assert_eq!(ys.first().copied(), Some(72.0));
        assert!(xs.iter().all(|x| (72.0..=540.0).contains(x)), "{xs:?}");
        assert!(ys.iter().all(|y| (72.0..=720.0).contains(y)), "{ys:?}");
        // 0.5 cm apart, the same both ways: 468 pt holds 33 steps, 648 pt holds 45.
        assert_eq!(xs.len(), 34);
        assert_eq!(ys.len(), 46);
        for w in xs.windows(2).chain(ys.windows(2)) {
            assert!((w[1] - w[0] - HALF_CM).abs() < 0.01, "{w:?}");
        }
        // The document's own spacing, different across and down: 1/8 inch across, 1/4 inch down.
        let (xs, ys) = drawing_grid(body, (9.0, 18.0));
        assert_eq!((xs.len(), ys.len()), (53, 37));
        // A grid line that lands on the margin is kept.
        let (xs, _) = drawing_grid(wordcraft_geom::Rect::new(0.0, 0.0, 100.0, 10.0), (25.0, 25.0));
        assert_eq!(xs, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
    }

    #[test]
    fn drawing_grid_survives_hostile_sizes() {
        let r = wordcraft_geom::Rect::new;
        for (body, step) in [
            (r(72.0, 72.0, 468.0, 648.0), 0.0),
            (r(72.0, 72.0, 468.0, 648.0), -5.0),
            (r(72.0, 72.0, 468.0, 648.0), f32::NAN),
            (wordcraft_geom::Rect { x: f32::NAN, y: 0.0, w: f32::INFINITY, h: -10.0 }, HALF_CM),
        ] {
            let (xs, ys) = drawing_grid(body, (step, step));
            assert!(xs.is_empty() && ys.is_empty(), "{body:?} {step}");
        }
        let (xs, ys) = drawing_grid(r(0.0, 0.0, 1.0e9, 1.0e9), (HALF_CM, HALF_CM));
        assert_eq!((xs.len(), ys.len()), (DRAWING_GRID_MAX_LINES, DRAWING_GRID_MAX_LINES));
        // Zoomed far out, lines thin out instead of filling the page.
        let grid = wordcraft_doc::DEFAULT_GRID;
        assert_eq!(drawing_grid_step(grid, PX_PER_PT), grid);
        assert_eq!(drawing_grid_step(HALF_CM, PX_PER_PT), HALF_CM);
        assert!(drawing_grid_step(grid, 0.1) * 0.1 >= DRAWING_GRID_MIN_PX);
        assert_eq!(drawing_grid_step(grid, 0.0), grid);
        assert_eq!(drawing_grid_step(grid, f32::NAN), grid);
        // A missing or broken spacing falls back to the spec's 1/8 inch.
        for bad in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            assert_eq!(drawing_grid_step(bad, PX_PER_PT), grid);
        }
    }

    #[test]
    fn texel_aligned_rect_keeps_integer_pixel_pages_unchanged() {
        let layout = Rect::from_min_size(pos2(10.0, 20.0), vec2(80.0, 120.0));
        let visual = texel_aligned_rect(layout, vec2(160.0, 240.0), 2.0);

        assert_eq!(visual, layout);
    }

    /// #692: a right-to-left paragraph mirrors the horizontal ruler's scale: the inch numbers
    /// count from the right margin, and reading a pointer position back inverts the mapping
    /// exactly, so the indent drags keep editing the same indents.
    #[test]
    fn the_ruler_scale_mirrors_for_right_to_left() {
        let (x0, scale, page_w, unit) = (100.0, 1.25, 612.0, 72.0);
        // 0 sits on the page's right edge and the inches rise leftward.
        assert_eq!(ruler_x(0.0, x0, scale, page_w, true), x0 + page_w * scale);
        assert_eq!(ruler_x(page_w, x0, scale, page_w, true), x0);
        // The inches count from the start margin, which is the right one: on an asymmetric page
        // (body from 72 to 468) 1 sits an inch left of the right body edge, mirroring left-to-right.
        let (body_x, body_right) = (72.0, 468.0);
        assert_eq!(ruler_x(body_x + unit, x0, scale, page_w, false), x0 + (body_x + unit) * scale);
        assert_eq!(ruler_x(page_w - body_right + unit, x0, scale, page_w, true), x0 + (body_right - unit) * scale);
        // Reading the pointer back inverts placing exactly, both ways: drags see the same indents.
        for rtl in [false, true] {
            for d in [0.0, 9.0, 72.0, 306.0, 612.0] {
                let x = ruler_x(d, x0, scale, page_w, rtl);
                assert!((ruler_d(x, x0, scale, page_w, rtl) - d).abs() < 1e-3, "rtl={rtl}: {x} reads back near {d}");
            }
        }

        // The marks: inches labelled from the start margin (the page edge counts backwards into
        // the margin, the margin itself has no mark), half inches the long ticks, ending on the
        // far page edge.
        let marks = ruler_marks(72.0, page_w, (unit, 8, 1), scale);
        let at = |d: f32| marks.iter().find(|(td, _)| (*td - d).abs() < 1e-4).map(|(_, m)| *m);
        assert_eq!(marks.first().map(|(d, _)| *d), Some(0.0));
        assert_eq!(marks.last().map(|(d, _)| *d), Some(page_w));
        assert_eq!(at(0.0), Some(RulerMark::Label(-1)));
        assert_eq!(at(72.0), None);
        assert_eq!(at(72.0 + unit), Some(RulerMark::Label(1)));
        assert_eq!(at(72.0 + 2.0 * unit), Some(RulerMark::Label(2)));
        assert_eq!(at(72.0 + unit / 2.0), Some(RulerMark::Tick(5.0)));
        assert_eq!(at(72.0 + unit / 8.0), Some(RulerMark::Tick(1.5)));
        // Zoomed out, sub-half-inch ticks drop out rather than smudge together.
        let coarse = ruler_marks(72.0, page_w, (unit, 8, 1), 0.3);
        assert!(!coarse.iter().any(|(_, m)| matches!(m, RulerMark::Tick(1.5))));
        assert!(coarse.iter().any(|(_, m)| matches!(m, RulerMark::Tick(3.0))));
        // Laid on screen, the same ticks run the other way when mirrored.
        let xs = |rtl| marks.iter().map(|(d, _)| ruler_x(*d, x0, scale, page_w, rtl)).collect::<Vec<_>>();
        assert!(xs(false).windows(2).all(|w| w[0] < w[1]));
        assert!(xs(true).windows(2).all(|w| w[0] > w[1]));
    }

    /// #692: Home › Paragraph › Right-to-Left Text Direction flips the ruler on screen: the
    /// inch numbers mirror about the page's centre and rise right-to-left.
    #[test]
    fn setting_a_paragraph_rtl_mirrors_the_ruler_numbers() {
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        let mut t = 0.0;
        // The horizontal ruler's inch labels: (text, centre x).
        let mut labels = |a: &mut WordApp| {
            t += 1.0 / 60.0;
            let input = egui::RawInput { time: Some(t), screen_rect: Some(screen), ..Default::default() };
            let out = ctx.run_ui(input, |ui| {
                a.logic(ui.ctx());
                a.ui(ui);
            });
            // The first frames only install fonts; the canvas appears after them.
            let found = a.canvas.canvas_rect.as_ref().map_or_else(Vec::new, |area| {
                // Ruler labels sit in the strip above the canvas, right of the vertical ruler.
                out.shapes
                    .iter()
                    .filter_map(|clipped| match &clipped.shape {
                        egui::Shape::Text(text) if text.pos.y >= area.min.y - RULER && text.pos.y < area.min.y && text.pos.x >= area.min.x => {
                            Some((text.galley.text().to_owned(), text.pos.x + text.galley.size().x / 2.0))
                        }
                        _ => None,
                    })
                    .collect::<Vec<(String, f32)>>()
            });
            out.drop_without_applying_deltas();
            found
        };
        let inch =
            |labels: &[(String, f32)], n: &str| labels.iter().find(|(s, _)| s == n).map(|(_, x)| *x).unwrap_or_else(|| panic!("the ruler shows {n}"));
        // The first frames install fonts and lay the page out.
        for _ in 0..4 {
            labels(&mut a);
        }
        let centre = a.canvas.page_rects[a.session.page_hint].center().x;
        let before = labels(&mut a);
        let (before1, before2) = (inch(&before, "1"), inch(&before, "2"));
        assert!(before1 < centre && before1 < before2, "left to right: the numbers rise away from the left margin");

        a.run("para.rtl", json!({})).unwrap();
        let after = labels(&mut a);
        let (after1, after2) = (inch(&after, "1"), inch(&after, "2"));
        assert!(after1 > centre && after1 > after2, "right to left: the numbers rise away from the right margin");
        assert!((after1 - (2.0 * centre - before1)).abs() < 1.0, "1 mirrors about the page centre: {before1} → {after1}");
        assert!((after2 - (2.0 * centre - before2)).abs() < 1.0, "2 mirrors about the page centre: {before2} → {after2}");
    }

    /// Ruler tab gestures are safe: a click on an indent marker places nothing, only the
    /// primary button places or moves stops, Escape cancels a drag, and a drag whose paragraph
    /// the caret leaves is abandoned instead of applied there.
    #[test]
    fn ruler_tab_gestures_are_primary_only_and_cancellable() {
        use egui::{Event, Key, PointerButton};
        use wordcraft_doc::props::TabAlign;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Name\tScore\rSecond"})).unwrap();
        a.run("view.ruler", json!({"value": true})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let area = h.state().canvas.canvas_rect.expect("the canvas is laid out");
        let ruler_x = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| {
            let body_x = h.state_mut().session.layout().pages[0].body.x;
            page_to_screen(h.state_mut(), 0, body_x + d, 0.0).expect("on screen").x
        };
        let on_ruler = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| pos2(ruler_x(h, d), area.min.y - 7.0);
        let button = |at: Pos2, b: PointerButton, pressed: bool| Event::PointerButton { pos: at, button: b, pressed, modifiers: Default::default() };
        let drag_events = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2, b: PointerButton| {
            h.event(Event::PointerMoved(from));
            h.event(button(from, b, true));
            h.step();
            for k in 1..=4 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
                h.step();
            }
            h.event(button(to, b, false));
            for _ in 0..3 {
                h.step();
            }
        };
        let stops_of = |h: &egui_kittest::Harness<'_, WordApp>, block: usize| {
            let s = &h.state().session;
            let para = s.doc.para_at(&Pos::body(block, 0)).expect("a paragraph");
            s.doc.styles.resolve_para(&para.props).tabs.iter().map(|t| (t.pos, t.align)).collect::<Vec<_>>()
        };

        // A plain click on the left-indent marker (at the paragraph's start edge) places nothing.
        let marker = pos2(ruler_x(&mut h, 0.0), area.min.y - 6.0);
        h.event(Event::PointerMoved(marker));
        h.event(button(marker, PointerButton::Primary, true));
        h.step();
        h.event(button(marker, PointerButton::Primary, false));
        for _ in 0..3 {
            h.step();
        }
        assert!(stops_of(&h, 1).is_empty(), "an indent-marker click placed a stop: {:?}", stops_of(&h, 1));

        // A right-button drag across the ruler places nothing.
        let (from, to) = (on_ruler(&mut h, 72.0), on_ruler(&mut h, 144.0));
        drag_events(&mut h, from, to, PointerButton::Secondary);
        assert!(stops_of(&h, 1).is_empty(), "a right-drag placed a stop: {:?}", stops_of(&h, 1));

        // Set a stop at 2" the intended way, then prove Escape cancels a move.
        let at = on_ruler(&mut h, 144.0);
        h.event(Event::PointerMoved(at));
        h.event(button(at, PointerButton::Primary, true));
        h.step();
        h.event(button(at, PointerButton::Primary, false));
        for _ in 0..32 {
            h.step();
        }
        assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)]);
        let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
        h.event(Event::PointerMoved(from));
        h.event(button(from, PointerButton::Primary, true));
        h.step();
        h.event(Event::PointerMoved(from + (to - from) * 0.5));
        h.step();
        h.event(Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        h.event(button(to, PointerButton::Primary, false));
        for _ in 0..32 {
            h.step();
        }
        assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)], "Escape left the stop where it was");

        // A drag abandoned by moving the caret to another paragraph applies nowhere.
        let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
        h.event(Event::PointerMoved(from));
        h.event(button(from, PointerButton::Primary, true));
        h.step();
        h.event(Event::PointerMoved(from + (to - from) * 0.5));
        h.step();
        h.state_mut().run("caret.docStart", json!({})).unwrap();
        h.step();
        h.event(button(to, PointerButton::Primary, false));
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(stops_of(&h, 1), vec![(144.0, TabAlign::Left)], "the old paragraph keeps its stop");
        assert!(stops_of(&h, 0).is_empty(), "the caret's new paragraph gained nothing: {:?}", stops_of(&h, 0));
    }

    /// Tab stops on the ruler: the corner selector picks the kind, a click sets a stop, dragging
    /// moves it, dragging it off clears it, and each gesture is one undo step.
    #[test]
    fn ruler_tab_stops_are_set_moved_and_cleared_with_the_pointer() {
        use egui::{Event, PointerButton};
        use wordcraft_doc::props::TabAlign;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Name\tScore"})).unwrap();
        a.run("view.ruler", json!({"value": true})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let area = h.state().canvas.canvas_rect.expect("the canvas is laid out");
        // The middle of the ruler's strip, over a point `d` points from the paragraph's start.
        let on_ruler = |h: &mut egui_kittest::Harness<'_, WordApp>, d: f32| {
            let body_x = h.state_mut().session.layout().pages[0].body.x;
            let x = page_to_screen(h.state_mut(), 0, body_x + d, 0.0).expect("on screen").x;
            pos2(x, area.min.y - 7.0)
        };
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
            h.event(Event::PointerMoved(at));
            h.event(button(at, true));
            h.step();
            h.event(button(at, false));
            for _ in 0..3 {
                h.step();
            }
            // Let the double-click time pass before the next press.
            for _ in 0..30 {
                h.step();
            }
        };
        let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, from: Pos2, to: Pos2| {
            h.event(Event::PointerMoved(from));
            h.event(button(from, true));
            h.step();
            for k in 1..=6 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
                h.step();
            }
            h.event(button(to, false));
            for _ in 0..3 {
                h.step();
            }
        };
        let stops = |h: &egui_kittest::Harness<'_, WordApp>| {
            let s = &h.state().session;
            let para = s.doc.para_at(&s.sel.focus).expect("a paragraph");
            s.doc.styles.resolve_para(&para.props).tabs.iter().map(|t| (t.pos, t.align)).collect::<Vec<_>>()
        };

        // The selector starts at Left; a click moves it to Center.
        assert_eq!(h.state().canvas.tab_kind, TabAlign::Left);
        click(&mut h, pos2(area.min.x - RULER / 2.0, area.min.y - RULER / 2.0));
        assert_eq!(h.state().canvas.tab_kind, TabAlign::Center);

        // A click on the ruler sets a centre stop at 2 inches (on the 1/16 inch grid).
        let at = on_ruler(&mut h, 144.0);
        click(&mut h, at);
        assert_eq!(stops(&h), vec![(144.0, TabAlign::Center)]);

        // Dragging it to 3 inches moves it, keeping its kind.
        let (from, to) = (on_ruler(&mut h, 144.0), on_ruler(&mut h, 216.0));
        drag(&mut h, from, to);
        assert_eq!(stops(&h), vec![(216.0, TabAlign::Center)]);

        // Dragging it down into the page clears it.
        let from = on_ruler(&mut h, 216.0);
        drag(&mut h, from, from + vec2(0.0, 80.0));
        assert!(stops(&h).is_empty(), "the stop came off: {:?}", stops(&h));

        // Each gesture was one step: undo brings back the moved stop, then the first one.
        assert!(h.state_mut().session.undo());
        assert_eq!(stops(&h), vec![(216.0, TabAlign::Center)]);
        assert!(h.state_mut().session.undo());
        assert_eq!(stops(&h), vec![(144.0, TabAlign::Center)]);
    }

    #[test]
    fn hiding_white_space_stacks_page_bands_and_keeps_hit_testing() {
        let scale = 1.0;
        let sizes = [(612.0, 792.0); 3];
        let (rects, _) = place_pages(&sizes, 0.0, scale, 1, vec2(800.0, 600.0));
        // Body from 72 to 720 pt on every page.
        let bodies = [(72.0, 720.0); 3];
        let (moved, bands, h) = collapse_white_space(&rects, &bodies, scale, 600.0);
        assert_eq!(bands[0].0, 0.0, "first page keeps its top");
        assert_eq!(bands[2].1, 792.0, "last page keeps its bottom");
        assert_eq!(bands[1], (66.0, 726.0));
        // Bands abut with one seam, so the content is much shorter than three full pages.
        assert_eq!(moved[1].min.y + bands[1].0, moved[0].min.y + bands[0].1 + SEAM);
        assert!(h < rects[2].bottom() - 2.0 * 66.0 * 2.0);
        // A point inside page 2's band maps to that page and the right page coordinates.
        let p = pos2(moved[1].center().x, moved[1].min.y + 300.0);
        assert_eq!(page_at_index(&moved, p), Some(1));
        // Just above the seam is page 1's text area, just below is page 2's, though the rects overlap.
        let seam = moved[0].min.y + bands[0].1;
        assert_eq!(nearest_page(&moved, pos2(moved[0].center().x, seam - 2.0)), Some(0));
        assert_eq!(nearest_page(&moved, pos2(moved[0].center().x, seam + SEAM + 2.0)), Some(1));
        assert!(on_page_gap(&moved, &bands, pos2(moved[0].center().x, seam + 0.5)));
        assert!(!on_page_gap(&moved, &bands, pos2(moved[0].center().x, seam - 40.0)));
        // Hostile numbers never panic.
        let _ = collapse_white_space(&rects, &[(f32::NAN, f32::INFINITY), (-1e9, 1e9)], f32::NAN, f32::NAN);
        let _ = collapse_white_space(&[], &[], 1.0, 1.0);
    }

    /// #1169: Space and Enter count as `Response::clicked` on the focused canvas, and
    /// `request_focus` always interrupts IME. On X11 that recreates the XIM context and
    /// fcitx drops every later key. Those keys still type, and they must not set
    /// `should_interrupt_composition`. A real pointer click still does, so a composition
    /// commits when the caret moves.
    #[test]
    fn space_and_enter_on_the_canvas_do_not_interrupt_ime() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("document.setText", json!({"text": "The"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..8 {
            h.step();
        }
        assert!(h.state().canvas.focused, "the canvas takes focus when the window opens");
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let ime_interrupted =
            |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|ime| ime.should_interrupt_composition);
        h.event(key(egui::Key::Space));
        h.event(egui::Event::Text(" ".into()));
        h.step();
        assert_eq!(ime_interrupted(&h), Some(false), "space must not recreate the XIM context");
        let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(text.contains("The") && text.contains(' '), "the space is still inserted: {text:?}");
        assert!(h.state().canvas.focused, "the canvas keeps the keyboard");
        h.event(key(egui::Key::Enter));
        h.step();
        assert_eq!(ime_interrupted(&h), Some(false), "enter must not recreate the XIM context");
        assert!(h.state().canvas.focused, "enter leaves the canvas focused");

        let at = h.state().canvas.page_rects.first().map(|r| r.center()).expect("a page");
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        assert_eq!(ime_interrupted(&h), Some(true), "a click still interrupts composition");
    }

    fn page_at_index(rects: &[Rect], p: Pos2) -> Option<usize> {
        nearest_page(rects, p)
    }
}
