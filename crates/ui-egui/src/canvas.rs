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
/// Width reserved for the vertical scrollbar beside the canvas.
const SCROLLBAR: f32 = 14.0;
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
    /// Set while a command runs for a mouse drag on a table: the pointer is already
    /// on the target, so the view stays put instead of following the caret (#1738).
    pub keep_view: bool,
    /// A heading navigation click aligns its page against the navigation pane, not just visible.
    pub(crate) nav_to_caret: bool,
    /// Touchpad travel not yet converted to an entire reading-column turn.
    read_wheel_accum: f32,
    /// Last actual viewport seen during a resize, distinct from the committed reading page size.
    read_viewport: Settle,
    /// The same for Web Layout's window-wide text width (#1876).
    web_viewport: Settle,
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
    pub(crate) last_press: Option<(f64, Pos2, u8)>,
    /// The first press of the current run of presses moved header/footer editing into another
    /// header/footer; the double-click it starts doesn't select a word there.
    hf_press_switched: bool,
    /// A drag after a double-click, triple-click or Ctrl/⌘+click: the unit it extends by, the
    /// unit first selected, and the last position the pointer hit.
    unit_drag: Option<UnitDrag>,
    pub last_highlight: String,
    pub last_font_color: String,
    pub last_shading: String,
    pub open_url: Option<String>,
    /// Screen rects of pages last frame (for the control channel: page ↔ screen coordinates).
    pub page_rects: Vec<Rect>,
    /// While the vertical scrollbar is dragged: the page at the top of the window (0-based),
    /// which a tip beside the pointer names (#2551).
    pub scrollbar_page: Option<usize>,
    /// The page being looked at (0-based), for the status bar (#3880): the caret's page while the
    /// caret is on screen, else the page at the top of the window. None until the canvas is drawn,
    /// and in Read Mode.
    pub viewed_page: Option<usize>,
    /// Effective scale (screen points per document point).
    pub scale: f32,
    pub canvas_rect: Option<Rect>,
    pub focused: bool,
    /// The page's widget id and the pass that last drew it, so a click elsewhere can hand the
    /// keyboard straight back to it (`WordApp::refocus_after_chrome_click`).
    pub(crate) widget_id: Option<(egui::Id, u64)>,
    pub render_ms: f64,
    pub ime_preedit: String,
    /// The chars of `ime_preedit` the input method marks as its cursor or selection, as it reports
    /// them (unchecked).
    pub(crate) ime_preedit_range: Option<std::ops::Range<usize>>,
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
    /// The frame drawn around the equation being edited, on screen (its options strip opens the
    /// equation's menu).
    pub eq_options: Option<EquationFrame>,
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
    /// A V (or Paste key) press reached the canvas since its last release. egui-winit keeps Mod+V
    /// presses, so a release without one was Mod+V even when Mod went up first.
    pub(crate) v_pressed: bool,
    /// The modifiers last held with Mod, for a Mod+V whose Mod is released before V.
    pub(crate) paste_mods: Option<egui::Modifiers>,
    /// Draw Table or Eraser is on (`table_pen`): presses draw instead of moving the caret.
    pub table_tool: Option<crate::table_pen::TableTool>,
    /// The Draw Table stroke (or eraser press) in progress.
    pub(crate) table_stroke: Option<crate::table_pen::PenStroke>,
    /// A table border or the table's resize handle being dragged (`table_resize`).
    pub(crate) table_resize: Option<crate::table_resize::ResizeDrag>,
    /// The last key ran an Alt shortcut: the text its key sends (it may arrive a frame later)
    /// isn't typed. The next key press clears it.
    pub(crate) swallow_text: bool,
    /// An input method's `DeleteSurrounding` just erased text with this formatting: the text it
    /// sends next replaces it, keeps the formatting and joins the same undo step
    /// (`keys::delete_surrounding`, #1480).
    pub(crate) ime_join_text: Option<wordcraft_doc::props::CharProps>,
    /// An automatic change (AutoCorrect, AutoFormat) altered the text around the caret: the
    /// input method is asked to start afresh (#1480).
    pub(crate) ime_reset: bool,
    /// A replacement or an unsupported IME mutation means the next observation must interrupt
    /// the live composition, whatever the surrounding text looks like. Unlike [`Self::ime_reset`]
    /// this is not limited to the web text agent.
    pub(crate) ime_interrupt_now: bool,
    /// A nonempty preedit lost its body/equation target. The next canvas event batch must
    /// reject that old composition's queued replacement before dispatching any mutation.
    pub(crate) ime_interrupted_preedit: bool,
    /// The last bounded surrounding window and the `(revision, cursor, anchor)` it was built
    /// from. An idle pass reuses it instead of walking a long paragraph again.
    pub(crate) ime_window_cache: Option<(u64, wordcraft_doc::Pos, wordcraft_doc::Pos, Option<wordcraft_doc::ime_text::ImeWindow>)>,
    /// Per-viewport IME owner, generation and change-cause state.
    pub(crate) ime_tracker: egui::ImeContextTracker,
    /// The viewport whose IME state [`Self::ime_tracker`] holds, so closing one releases it.
    pub(crate) ime_viewport: Option<egui::ViewportId>,
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
    /// Whether the I-beam over the page is drawn rather than the system's (`ibeam`).
    pub(crate) drawn_ibeam: bool,
    /// The rows and columns hovered in Insert › Table's grid this frame: the page shows that
    /// table where it would go (#3931). Set by the grid each frame, taken by [`show`].
    pub(crate) table_preview: Option<(usize, usize)>,
    /// The last previewed table's layout, the copy of the document it was laid out from and the
    /// copy's selection, with
    /// what it was laid out for (rows, columns, document id, revision, selection, and the
    /// document's own layout, which a view change replaces), so a still pointer doesn't lay the
    /// document out every frame. Set only while the page shows the preview.
    table_preview_layout: Option<(TablePreviewKey, wordcraft_doc::Document, wordcraft_engine::Selection, std::sync::Arc<DocLayout>)>,
}

type TablePreviewKey = ((usize, usize, u64, u64, wordcraft_engine::Selection), std::sync::Arc<DocLayout>);

impl CanvasState {
    /// Stop a text-selection drag when the pointer gesture becomes an object drag.
    pub(crate) fn cancel_text_drag(&mut self) {
        self.dragging = false;
        self.column_drag = None;
        self.unit_drag = None;
    }

    /// The object a drag has taken off its page: which page, and which object. `None` unless an
    /// object is being **moved** (resizing and turning show the object itself where it is).
    ///
    /// A dragged object's page is not laid out again while it is dragged (that would relayout the
    /// whole document every frame), so the page's raster still shows it where it was — the user
    /// sees it twice: once in place, once under the pointer. Rendering the page without that one
    /// object fixes it (see [`wordcraft_layout::display::DisplayOptions::hide_object`]); erasing
    /// the page would make the other objects blink.
    fn dragged_off_page(&self) -> Option<(usize, Pos)> {
        crate::objects::dragged_off_page(self.obj_drag.as_ref())
    }

    /// How many pages have a cached rendering.
    #[cfg(test)]
    pub(crate) fn texture_count(&self) -> usize {
        self.textures.len() + self.tiles.len()
    }
}

impl Default for CanvasState {
    /// Creates empty canvas state: texture and tile caches, page keys, settle and scroll flags, caret
    /// visibility, and inactive drag and column-selection state.
    fn default() -> Self {
        CanvasState {
            textures: HashMap::new(),
            tiles: HashMap::new(),
            page_keys: PageKeys::default(),
            markup_facts: std::cell::Cell::new(None),
            scroll_to_caret: true,
            keep_view: false,
            nav_to_caret: false,
            read_wheel_accum: 0.0,
            read_viewport: Settle::default(),
            web_viewport: Settle::default(),
            read_force_reflow: false,
            read_first_visible: None,
            read_target_page: None,
            caret_visible_since: 0.0,
            caret_bg: None,
            dragging: false,
            column_drag: None,
            last_press: None,
            hf_press_switched: false,
            unit_drag: None,
            last_highlight: "yellow".into(),
            last_font_color: "C00000".into(),
            last_shading: "FFF2CC".into(),
            open_url: None,
            page_rects: Vec::new(),
            scrollbar_page: None,
            viewed_page: None,
            scale: PX_PER_PT,
            canvas_rect: None,
            focused: false,
            widget_id: None,
            render_ms: 0.0,
            ime_preedit: String::new(),
            ime_preedit_range: None,
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
            eq_options: None,
            mini_anchor: None,
            mini_dismissed: None,
            pasted: false,
            v_pressed: false,
            paste_mods: None,
            swallow_text: false,
            ime_join_text: None,
            ime_reset: false,
            ime_interrupt_now: false,
            ime_interrupted_preedit: false,
            ime_window_cache: None,
            ime_tracker: egui::ImeContextTracker::new(),
            ime_viewport: None,
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
            drawn_ibeam: crate::ibeam::DRAWN,
            table_preview: None,
            table_preview_layout: None,
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

/// After a whole-document selection from a press (not a line, so it can't grow): holds the press
/// so its release isn't a caret click.
pub(crate) fn hold_document_selection(app: &mut WordApp, clicks: u8) {
    let (start, end) = app.session.sel.ordered();
    app.canvas.unit_drag = Some(UnitDrag { unit: "document", clicks, start, last: end.clone(), end });
    app.canvas.dragging = true;
}

/// Clicks in a row for a press at `at` (time `now`) after `last`: within the double-click
/// delay and distance it counts up to 3 (a triple-click), otherwise it starts again at 1.
pub(crate) fn press_count(last: Option<(f64, Pos2, u8)>, now: f64, at: Pos2, delay: f64, dist: f32) -> u8 {
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
pub(crate) const WHITE_SPACE_PAD: f32 = 6.0;
/// The line drawn between two pages when white space is hidden (pixels).
pub(crate) const SEAM: f32 = 1.0;

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

/// True when `p` is on the shown part of a page, not the canvas around or between pages (nor a
/// margin Hide White Space cuts).
fn on_visible_page(rects: &[Rect], bands: &[(f32, f32)], p: Pos2) -> bool {
    rects.iter().zip(bands).any(|(r, &(top, bot))| Rect::from_min_max(pos2(r.min.x, r.min.y + top), pos2(r.max.x, r.min.y + bot)).contains(p))
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

/// Full pagination is expensive. During a resize keep the already laid-out pages and reflow
/// once the viewport has been stable this long (seconds).
const RESIZE_SETTLE: f64 = 0.16;

/// A layout size that follows the window (Read Mode's columns, Web Layout's width): the last
/// viewport seen and when it last changed, so a window drag reflows once it settles, not per frame.
#[derive(Clone, Copy, Debug, Default)]
struct Settle {
    viewport: Option<egui::Vec2>,
    resized_at: f64,
}

impl Settle {
    /// Whether nothing has been seen since the view was entered (or [`Settle::leave`]).
    fn entering(&self) -> bool {
        self.viewport.is_none()
    }

    /// Notes this frame's viewport `size` at time `now`. Returns whether a layout size that
    /// `changed` should be applied now: at once when entering or `forced`, otherwise only after
    /// the viewport has held still for [`RESIZE_SETTLE`].
    fn apply(&mut self, size: egui::Vec2, now: f64, changed: bool, forced: bool) -> bool {
        let entering = self.entering();
        if self.viewport.is_none_or(|v| (v - size).length_sq() > 0.5) {
            self.viewport = Some(size);
            self.resized_at = now;
        }
        changed && (entering || forced || now - self.resized_at >= RESIZE_SETTLE)
    }

    fn leave(&mut self) {
        self.viewport = None;
    }
}

/// Web Layout's text width in points for a work area `avail_px` wide at `scale`: the whole width,
/// clamped to what the layout accepts so a tiny or huge window never gives zero or a giant page.
fn web_layout_width(avail_px: f32, scale: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { PX_PER_PT };
    let w = avail_px / scale;
    if w.is_finite() { w.clamp(144.0, 20_000.0) } else { 800.0 }
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
    // The columns end above the page-turn bar, which used to cover their last lines (#1772).
    (px / scale, (height - GAP - READ_NAV_BAR).max(192.0) / scale)
}

/// Height kept free under Read Mode's columns for the page-turn bar at the bottom right.
const READ_NAV_BAR: f32 = 44.0;

/// The part of the canvas Read Mode scrolls in: as many whole columns as fit, centred. A chosen
/// column width (or a resize) otherwise left the last column cut off at the right edge (#1772).
/// `scale` is pixels per point: the column width is only re-laid out when it moves by more than
/// half a point, so each column may overrun by that much before one is dropped.
fn reading_viewport(full: Rect, column_px: f32, scale: f32) -> Rect {
    if !column_px.is_finite() || column_px <= 0.0 || !full.width().is_finite() {
        return full;
    }
    let step = column_px + GAP;
    let slack = if scale.is_finite() { (0.5 * scale).clamp(0.0, 4.0) } else { 0.0 };
    // A hair of tolerance too: columns sized to fill the canvas exactly come back from points
    // as 2.9999998 of them, and flooring that dropped a whole column.
    let whole = ((full.width() - GAP) / (step - slack) + 1e-3).floor().max(1.0);
    let width = (whole * step + GAP).min(full.width());
    Rect::from_min_size(pos2(full.center().x - width / 2.0, full.min.y), vec2(width, full.height()))
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
    if v.mode == wordcraft_layout::ViewMode::Web {
        // The sheet runs from side to side of the work area, no centred page with grey margins.
        let rects: Vec<Rect> = sizes
            .iter()
            .scan(0.0f32, |y, &(w, h)| {
                let r = Rect::from_min_size(pos2(0.0, *y), vec2(sane_dim(w, 1e5) * scale, sane_dim(h, 1e6) * scale));
                *y = r.bottom();
                Some(r)
            })
            .collect();
        let size = vec2(rects.iter().map(|r| r.right()).fold(avail.x.max(0.0), f32::max), rects.last().map_or(0.0, |r| r.bottom()).max(avail.y));
        let bands = rects.iter().map(|r| (0.0, r.height())).collect();
        return Geometry { rects, size, scale, cols: 1, bands };
    }
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
/// marks and markup, dark mode, the page colour and watermark, header/body dimming, the media,
/// and (while a drag lasts) the object left out of the page.
/// Taken once per frame.
fn view_key(app: &WordApp, dim_body: bool, hidden: Option<&Pos>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let v = &app.session.view;
    (v.marks, app.session.prefs.marks, v.show_markup, v.dark_mode, wordcraft_render::DARK_PAPER, v.hide_ink, v.draw.replay).hash(&mut h);
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

/// Global View Gridlines outlines every table (#1628). While editing a header/footer, also
/// outline that story's cells (#2197), without adding printable borders or changing the view flag.
fn gridline_cells(page: &Page, all: bool, editing: Option<StoryRef>) -> impl Iterator<Item = wordcraft_geom::Rect> + '_ {
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

/// Draw the document canvas in the space `ui` has left, one frame: the pages, caret, selection
/// and rulers, with the mouse editing and the pointer shape over them.
/// The layout the page shows: the document's own, or with the table Insert › Table's grid is
/// hovering over (#3931). The preview is laid out on a copy, so the document and its undo history
/// change only when a square is clicked.
fn shown_layout(app: &mut WordApp) -> std::sync::Arc<DocLayout> {
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

/// Lays out and paints the document pages for print, Read and Web views, settles viewports, draw
/// rulers and overlays, anchors zoom and scrolling, and routes pointer, selection, IME and caret
/// input.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    // During a resize keep the already laid-out pages ([`Settle`]); ordinary reading controls
    // and zoom update now.
    let mut reflow_anchor: Option<Pos> = None;
    let now = ui.input(|i| i.time);
    if app.session.view.read_mode {
        app.canvas.web_viewport.leave();
        let entering = app.canvas.read_viewport.entering();
        let scale = app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        let (w, h) = reading_size(full.size(), scale, app.session.view.read_column_width);
        let changed = (app.session.view.read_width - w).abs() > 0.5 || (app.session.view.read_height - h).abs() > 0.5;
        let forced = std::mem::take(&mut app.canvas.read_force_reflow) || entering;
        if app.canvas.read_viewport.apply(full.size(), now, changed, forced) {
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
        if app.session.view.mode == wordcraft_layout::ViewMode::Web {
            // Web Layout wraps to the work area (#1876): at once on entry or a zoom (the viewport
            // holds still), after a window drag has settled, and only on a real change.
            let scale = app.session.view.zoom.clamp(0.1, 5.0) * PX_PER_PT;
            let w = web_layout_width(full.width() - SCROLLBAR, scale);
            let changed = (app.session.view.web_width - w).abs() > 0.5;
            let entering = app.canvas.web_viewport.entering();
            if app.canvas.web_viewport.apply(full.size(), now, changed, false) {
                app.session.view.web_width = w;
                // The pixel offset no longer points at the same text: keep the caret's line in view.
                app.canvas.scroll_to_caret |= !entering;
            } else if changed {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(170));
            }
        } else {
            app.canvas.web_viewport.leave();
        }
        app.canvas.read_viewport.leave();
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
    let layout = shown_layout(app);
    if let Some(anchor) = reflow_anchor
        && let Some(page) = reading_anchor_page(&layout, &anchor)
    {
        let step = app.session.view.read_width * app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT + GAP;
        app.canvas.read_target_page = None;
        app.canvas.scroll_request = Some(vec2(page as f32 * step, 0.0));
    }
    // View › Switch Modes: dark paper, inverted document colours.
    // In Read Mode the Page color setting is used instead: Inverse draws the dark page, Sepia
    // draws sepia paper and None draws white paper.
    use wordcraft_engine::cmd::view::ReadPageColor;
    let read_mode = app.session.view.read_mode;
    let dark_page = if read_mode { app.session.view.read_page_color == ReadPageColor::Inverse } else { app.session.view.dark_mode };
    let sepia_page = read_mode && app.session.view.read_page_color == ReadPageColor::Sepia;
    let dark_paper = wordcraft_render::DARK_PAPER;
    let show_ruler = app.session.view.ruler
        && app.session.view.mode == wordcraft_layout::ViewMode::Print
        && !app.session.view.read_mode
        && crate::chrome::Screen::of(&app.session.view) == crate::chrome::Screen::Normal;
    let (hruler, vruler, area) = if show_ruler {
        let h = Rect::from_min_max(pos2(full.min.x + RULER, full.min.y), pos2(full.max.x, full.min.y + RULER));
        let v = Rect::from_min_max(pos2(full.min.x, full.min.y + RULER), pos2(full.min.x + RULER, full.max.y));
        (Some(h), Some(v), Rect::from_min_max(pos2(full.min.x + RULER, full.min.y + RULER), full.max))
    } else {
        (None, None, full)
    };
    let area = if app.session.view.read_mode {
        let column = app.session.view.read_width * app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        reading_viewport(area, column, app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT)
    } else {
        area
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
        app.canvas.wheel.cancel();
        let _ = app.run(id, json!({"value": pct}));
    }
    let avail = if app.session.view.read_mode { area.size() } else { area.size() - vec2(SCROLLBAR, 0.0) };
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
    let caret = layout.caret_on(&preview_sel(app).unwrap_or(app.session.sel_ends()).focus, app.session.page_hint);
    // Editing a header/footer (or a note) dims the body; once per frame, for every page.
    let dim_body = dims_body(app, &layout);
    let grid_story = text_box_host(app, &layout).unwrap_or(app.session.sel.focus.story);
    let grid_story = matches!(part_kind(app, grid_story), Some(PartKind::Header | PartKind::Footer)).then_some(grid_story);
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
    // Read Mode's centred columns leave margins at the sides; the wheel turns pages over them too.
    let wheel_area = if app.session.view.read_mode { full } else { area };
    let hovered = wheel_hovered(ui.rect_contains_pointer(wheel_area), ui.ctx().dragged_id().is_some(), app.canvas.drag_selecting());
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
    let mut ime_geometry: Option<(Rect, Rect)> = None;
    let mut ime_interrupt = false;
    let out = scroll_area.show_viewport(&mut ui_area, |ui, viewport| {
        origin = ui.min_rect().min - viewport.min.to_vec2();
        let content = Rect::from_min_size(ui.min_rect().min, geo.size);
        // A fixed id: an automatic one follows how many widgets came before, which Read Mode's
        // chrome changes, and a new id dropped the keyboard focus, so the arrows, Page Up/Down
        // and Escape did nothing in Read Mode until the reader clicked a page (#1772).
        // It also keeps the focus when Focus hides the rulers (#1678).
        ui.advance_cursor_after_rect(content);
        let resp = ui.interact(content, egui::Id::unique("wordcraft_canvas"), Sense::click_and_drag());
        if let Some(tr) = scroll_target {
            ui.scroll_to_rect(Rect::from_min_size(ui.min_rect().min + tr.min.to_vec2(), tr.size()), None);
        }
        // Read Mode's neighbouring columns end right at the viewport's edges: keep their
        // shadows from showing there as hairlines.
        let clip = if app.session.view.read_mode { ui.clip_rect().shrink2(vec2(3.0, 0.0)) } else { ui.clip_rect() };
        let painter = ui.painter_at(clip);
        let ppp = ui.ctx().pixels_per_point();
        let max_tex = ui.ctx().input(|i| i.max_texture_side).min(wordcraft_render::MAX_SIDE as usize).max(1) as f32;
        let mut visible_tile_keys = HashSet::new();
        let mut rendered = 0;
        let t0 = crate::now_ms();
        let mut rects = Vec::with_capacity(geo.rects.len());
        let content_origin = ui.min_rect().min;
        // The object a move has taken off its page: the page is drawn again without it (or it
        // would be on the page where it was and under the pointer at once).
        let hidden = app.canvas.dragged_off_page();
        let view = view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p));
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
            // The page a drag took an object off: render it this frame whatever the budget, or the
            // raster with the object still on it shows for a frame.
            let forced = hidden.as_ref().is_some_and(|(page, _)| *page == i);
            // Render before placing the page: a page re-rendered in this frame (a zoom step, an
            // edit) then sits on the pixel grid in this frame too, rather than a frame later.
            if visible && !tiled && !fresh && (forced || rendered < 2 || !app.canvas.textures.contains_key(&i) && rendered < 4) {
                let mut opts = page_render_options(app, dim_body);
                // Only on the page it is on: the `Pos` can't match an object of another page.
                opts.display.hide_object = hidden.as_ref().filter(|(page, _)| *page == i).map(|(_, p)| p.clone());
                let img = wordcraft_render::render_page(page_doc(app), page, scale_px, &opts);
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
            let paper = if dark_page {
                Color32::from_gray(dark_paper)
            } else if sepia_page {
                let s = wordcraft_render::SEPIA_PAPER;
                Color32::from_rgb(s.0, s.1, s.2)
            } else {
                Color32::WHITE
            };
            painter.rect_filled(visual_sr, 0.0, paper);
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
                        let mut opts = page_render_options(app, dim_body);
                        // Only on the page it is on: the `Pos` can't match an object of another page.
                        opts.display.hide_object = hidden.as_ref().filter(|(page, _)| *page == i).map(|(_, p)| p.clone());
                        // The tiny subtraction avoids a 1px overshoot from round-tripping f32
                        // pixels through page points and render_area's ceil() conversion.
                        let img = wordcraft_render::render_area(
                            page_doc(app),
                            page,
                            0.0,
                            start_px as f32 / scale_px,
                            page.w,
                            (height_px as f32 - 0.01).max(0.01) / scale_px,
                            scale_px,
                            &opts,
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
            // The current header/footer's cells have editing guides; explicit View Gridlines
            // still outlines every table. Neither kind of guide is part of the document.
            if app.session.view.table_gridlines || grid_story.is_some() {
                for rect in gridline_cells(page, app.session.view.table_gridlines, grid_story) {
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
            egui::Area::new(egui::Id::unique("floating_comment")).order(egui::Order::Foreground).fixed_pos(Pos2::ZERO).sense(Sense::hover()).show(
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
        // Find › Reading Highlight. Its places are the document's, not the preview's.
        let marks: Vec<(Pos, Pos)> =
            if preview_sel(app).is_some() { Vec::new() } else { app.session.find_highlights().iter().take(5000).cloned().collect() };
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
        let selected: Vec<_> = if let Some(sel) = preview_sel(app) {
            let (a, b) = sel.ordered();
            if a == b {
                Vec::new()
            } else {
                wordcraft_engine::cmd::table_mouse::selection_rects(&layout, page_doc(app), &a, &b, app.session.page_hint)
            }
        } else if crate::objects::selected(app).is_some() {
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
        // After an automatic change the web's text agent starts afresh
        // (`keys::note_automatic_change`) — only there, because on X11 an interrupt recreates the
        // input context and fcitx drops later keys (#1169). A body replacement interrupts on every
        // platform: the composition belonged to the text that was swapped out.
        let ime_reset = std::mem::take(&mut app.canvas.ime_reset) && cfg!(target_arch = "wasm32") && app.canvas.ime_preedit.is_empty();
        let replaced = std::mem::take(&mut app.canvas.ime_interrupt_now);
        ime_interrupt = ime_reset || replaced;
        // Editing an equation: frame it as Word does (a box as tall as its line, a grip at its
        // top left, an options strip at its right) and draw the caret inside it.
        let mut math_caret = false;
        app.canvas.eq_options = None;
        if let Some(m) = app.session.math.clone()
            && let Some(bx) = layout.equation_box(&m.at, app.session.page_hint)
            && let Some(pr) = rects.get(bx.page)
        {
            math_caret = true;
            let (pi, ex, base, ml) = (bx.page, bx.x, bx.base, bx.math.clone());
            let s = geo.scale;
            let frame = equation_frame(*pr, &bx, s);
            // The argument the caret is in (a numerator, a radicand…) is shaded.
            if !m.pos.path.is_empty()
                && let Some((x0, x1, up, down)) = ml.arg_extent(&m.pos.path)
            {
                let arg = Rect::from_min_max(
                    pos2(pr.min.x + (ex + x0) * s, pr.min.y + (base - up) * s),
                    pos2(pr.min.x + (ex + x1) * s, pr.min.y + (base + down) * s),
                );
                painter.rect_filled(arg.expand(1.0), 0.0, Color32::from_black_alpha(26));
            }
            painter.rect_filled(frame.strip, 0.0, t.equation_box.gamma_multiply(0.35));
            let (c, w) = (frame.strip.center(), frame.strip.width() * 0.3);
            painter.add(egui::Shape::convex_polygon(
                vec![pos2(c.x - w, c.y - w * 0.6), pos2(c.x + w, c.y - w * 0.6), pos2(c.x, c.y + w * 0.6)],
                t.equation_box,
                Stroke::NONE,
            ));
            painter.rect_stroke(frame.outline, 2.0, Stroke::new(1.0, t.equation_box), egui::StrokeKind::Inside);
            painter.rect_filled(frame.grip, 1.0, Color32::WHITE);
            painter.rect_stroke(frame.grip, 1.0, Stroke::new(1.0, t.equation_box), egui::StrokeKind::Inside);
            for k in 0..3 {
                painter.circle_filled(
                    pos2(frame.grip.center().x, frame.grip.min.y + frame.grip.height() * (k as f32 + 1.0) / 4.0),
                    0.9,
                    t.equation_box,
                );
            }
            app.canvas.eq_options = Some(frame);
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
                    ime_geometry = Some((cr, cr));
                }
            }
        }
        if !math_caret
            && let Some(c) = layout.caret_on(&preview_sel(app).unwrap_or(&app.session.sel).focus, app.session.page_hint)
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
            // An input method's composition is drawn from the caret, and the caret and the candidate
            // window move to the input method's cursor inside it.
            let mut preedit_color = None;
            let mut ime_dx = 0.0;
            if focused && !app.canvas.ime_preedit.is_empty() {
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
                let color = caret_color(app);
                let g = painter.layout_no_wrap(app.canvas.ime_preedit.clone(), regular(c.height * scale * 0.8), Color32::PLACEHOLDER);
                let at = preedit_cursor_chars(&app.canvas.ime_preedit, app.canvas.ime_preedit_range.as_ref());
                ime_dx = g.pos_from_cursor(egui::text::CCursor::new(at)).min.x;
                let r = Rect::from_min_size(pos2(x, y1 - g.size().y), g.size());
                painter.galley(r.min, g, color);
                painter.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, color));
                preedit_color = Some(color);
            }
            let x = x + ime_dx;
            let bar = if c.width > 0.0 {
                // Turned text (a table cell's text direction): the caret lies across the page.
                [pos2(x, y0.round() + 0.5), pos2(x + c.width * scale, y0.round() + 0.5)]
            } else {
                [pos2(x.round() + 0.5, y0), pos2(x.round() + 0.5, y1)]
            };
            // The caret glides and fades like Word's (`caret_glide`).
            let shown = preview_sel(app).unwrap_or(&app.session.sel).is_collapsed() && focused;
            let look = crate::caret_glide::look(app, ui, content_origin, &geo, bar, shown, since, on);
            if shown && look.opacity > 0.0 {
                let color = preedit_color.unwrap_or_else(|| caret_color(app));
                painter.line_segment(look.bar, Stroke::new(1.5, color.gamma_multiply(look.opacity)));
            }
            if focused {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(530 - (since as u64 % 530)));
            }
            // IME candidate window placement.
            let cr = Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1));
            if focused {
                ime_geometry = Some((cr, cr));
            }
        }
        crate::objects::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::dialogs_smart_art::frame_button(app, ui.ctx(), &layout, &rects, geo.scale, ui.clip_rect());
        crate::ink::paint(app, &painter, &rects, &layout, geo.scale);
        crate::table_pen::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::table_resize::paint(app, &painter, &t, &layout, &rects, geo.scale);
        crate::table_mouse::paint(app, &painter, &t, &layout, &rects, geo.scale);
        (resp, rects)
    });
    app.canvas.scroll_offset = out.state.clamped_offset();
    scrollbar_page_tip(app, ui, out.id, &geo.rects, &geo.bands, layout.pages.len());
    // The page the status bar names (#3880): the caret's, or a whole-story selection's page, while
    // it is on screen; else the page at the top of the window.
    app.canvas.viewed_page = (!app.session.view.read_mode && !geo.rects.is_empty()).then(|| {
        let extent = if app.session.selects_whole_story() {
            let page = app.session.page_hint.min(geo.rects.len() - 1);
            geo.rects.get(page).map(|r| (page, r.top(), r.bottom()))
        } else {
            caret.and_then(|c| geo.rects.get(c.page).map(|r| (c.page, r.top() + c.top * geo.scale, r.top() + (c.top + c.height) * geo.scale)))
        };
        viewed_page(extent, &geo.rects, &geo.bands, app.canvas.scroll_offset.y, out.inner_rect.height()).min(layout.pages.len().saturating_sub(1))
    });
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
        deselect_balloon(app);
    }
    // Space and Enter also set `Response::clicked` on a focused click-sensing widget
    // (egui's fake primary click). Re-requesting an already focused widget no longer
    // interrupts IME in 0.37. A pointer
    // click, an accessibility click, or a drag still takes focus, including a click on
    // a canvas that already has it, so a composition commits. `want_focus` focuses only
    // while the canvas does not, and does not interrupt an editor that already has it.
    let keyboard_click = ui.memory(|m| m.has_focus(resp.id)) && ui.input(|i| i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter));
    let pointer_click = resp.clicked_by(egui::PointerButton::Primary);
    let accesskit_click = resp.clicked() && !pointer_click && !keyboard_click;
    let focus_click = pointer_click || accesskit_click || resp.drag_started();
    if focus_click {
        ui.memory_mut(|memory| memory.interrupt_ime());
    }
    if focus_click || (app.canvas.want_focus && !resp.has_focus()) {
        resp.request_focus();
    }
    if focus_click || app.canvas.want_focus {
        app.canvas.want_focus = false;
    }
    app.canvas.widget_id = Some((resp.id, ui.ctx().cumulative_pass_nr()));
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
    }
    app.canvas.focused = resp.has_focus();
    // One observation per viewport pass, after every event and command for the pass has been
    // applied: the owner, generation, bounded surrounding text and change cause all come from
    // this same settled state.
    crate::keys::observe_ime(app, ui.ctx(), ime_geometry, ime_interrupt);
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
    // A click on the equation frame's options strip opens the equation's menu below the strip.
    let eq_strip = app.canvas.eq_options.map(|f| f.strip).filter(|r| resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| r.contains(p)));
    if from_keys {
        context_suggestions(app);
        app.canvas.context_menu_at = caret_screen(app).map(|(p, h)| p + egui::vec2(0.0, h));
    } else if let Some(r) = eq_strip {
        app.canvas.context_menu_at = Some(r.left_bottom());
    } else if secondary_clicked {
        app.canvas.context_menu_at = resp.interact_pointer_pos();
    }
    // Keep the parent independently of egui's one memory popup: the toolbar's font, size and
    // colour controls use that slot for their own lists (as in a collapsed ribbon group).
    let mut open = app.canvas.context_menu_open;
    if secondary_clicked || from_keys || eq_strip.is_some() {
        egui::Popup::close_all(ui.ctx());
        open = true;
    } else if open && ui.input(|i| i.pointer.any_click()) {
        let layer = ui.input(|i| i.pointer.interact_pos()).and_then(|p| ui.ctx().layer_id_at(p));
        if !layer.is_some_and(|l| matches!(l.order, egui::Order::Foreground | egui::Order::Tooltip)) {
            open = false;
        }
    }
    let child_open = egui::Popup::is_any_open(ui.ctx());
    let mut shown = open;
    if open {
        app.escape_free = false;
    }
    let menu = egui::Popup::menu(&resp).open_bool(&mut shown);
    let menu = menu.at_position(app.canvas.context_menu_at.unwrap_or(out.inner_rect.left_top()));
    let with_toolbar = crate::mini_toolbar::context_menu_visible(app);
    let menu = if with_toolbar { menu.frame(egui::Frame::NONE).close_behavior(egui::PopupCloseBehavior::IgnoreClicks) } else { menu };
    menu.show(|ui| {
        if with_toolbar {
            crate::mini_toolbar::show_in_context_menu(app, ui);
            ui.add_space(4.0);
            // The command panel keeps its own width, not the toolbar's: the popup's justified
            // layout would stretch it, so it takes the width egui measures in its sizing pass.
            let width_id = ui.scope_id().with("context_menu_width");
            let width = ui.data(|d| d.get_temp::<f32>(width_id)).filter(|_| !ui.is_sizing_pass());
            let size = egui::vec2(width.unwrap_or(ui.available_width()), ui.available_height());
            let rect = egui::Rect::from_min_size(ui.cursor().min, size);
            let panel = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| context_menu(app, ui)).response.rect.width()
            });
            if ui.is_sizing_pass() {
                ui.data_mut(|d| d.insert_temp(width_id, panel.inner));
            }
        } else {
            context_menu(app, ui);
        }
    });
    // Escape closes the child list first; a second Escape closes the context menu.
    if open && !shown && child_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        shown = true;
    }
    // The standalone selection toolbar stands down while the context popup owns the pointer.
    app.canvas.context_menu_open = shown;
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
        // An armed Format Painter shows over the text, as Word's brush does; egui has no brush,
        // so the copy pointer stands in (#3453). The selection bar keeps its arrow.
        let fallback = if in_bar {
            egui::CursorIcon::Default
        } else if app.session.painter.is_some() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Text
        };
        ui.ctx().set_cursor_icon(over_object.unwrap_or(fallback));
    }
    crate::table_resize::cursor(app, ui, &resp, &layout, &rects, geo.scale);
    crate::table_mouse::cursor(app, ui, &resp, &layout, &rects, geo.scale);
    crate::table_pen::cursor(app, ui, &resp);
    // Windows' I-beam is lost on white paper there: draw one that shows (#1746).
    if app.canvas.drawn_ibeam
        && (resp.hovered() || resp.dragged())
        && let Some(p) = ui.input(|i| i.pointer.latest_pos())
    {
        let opts = page_render_options(app, dim_body);
        let bg = if on_visible_page(&rects, &geo.bands, p) {
            opts.paper_of(&app.session.doc)
        } else {
            let c = Tokens::get(ui.ctx()).canvas;
            wordcraft_doc::props::Rgb(c.r(), c.g(), c.b())
        };
        crate::ibeam::replace(ui.ctx(), p, bg);
    }
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
        deselect_balloon(app);
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
    let palette = t.author_colors;
    let leaders = t.leader_colors;
    let mut authors: Vec<String> = Vec::new();
    let mut clicked: Option<(u32, Pos)> = None;
    // The markup area extends each page.
    for (pi, pr) in rects.iter().enumerate() {
        let page_scale = layout.pages.get(pi).map_or(scale, |page| page_screen_scale(*pr, page, scale));
        let area = Rect::from_min_max(pos2(pr.max.x, pr.min.y), pos2(pr.max.x + mw * page_scale, pr.max.y));
        if mw > 0.0 && area.intersects(clip) {
            painter.rect_filled(area, 0.0, t.markup_area);
            painter.line_segment([area.left_top(), area.left_bottom()], Stroke::new(1.0, t.markup_edge));
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
                    let on_page = Stroke::new(1.0, leaders.get(ai % leaders.len()).copied().unwrap_or(color).linear_multiply(0.7));
                    dashed(painter, anchor, pos2(pr.max.x, anchor.y), on_page);
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
            // Lay out a read-only balloon's text: the comment, then each reply under a rule,
            // each with who wrote it and when. (galley, gap above, a rule above)
            let mut lines: Vec<(craft_ui::text::AlignedGalley, f32, bool)> = Vec::new();
            let card = if active {
                Rect::from_min_size(pos2(x0, top), vec2(w, app.canvas.balloon_h.max(24.0)))
            } else {
                let dim = |r: bool| if r { t.text_dim } else { t.text };
                let small = regular((fs * 0.9).max(8.0));
                for (i, cid) in std::iter::once(*id).chain(thread.iter().copied()).enumerate() {
                    let Some(r) = app.session.doc.comments.get(&cid) else { continue };
                    let gap = if i == 0 { 0.0 } else { 9.0 };
                    lines.push((balloon_text(painter, author_name(&r.author), semibold(fs), t.text, w - 16.0), gap, i > 0));
                    let when = crate::local_time::comment_time(r);
                    if !when.is_empty() {
                        lines.push((balloon_text(painter, when, small.clone(), t.text_dim, w - 16.0), 1.0, false));
                    }
                    let text = app.session.doc.plain_text(StoryRef::Part(r.part));
                    lines.push((balloon_text(painter, text.trim().to_string(), regular(fs), dim(resolved), w - 16.0), 4.0, false));
                }
                let h = lines.iter().map(|(g, gap, _)| g.height() + gap).sum::<f32>() + 12.0;
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
                let on_page = Stroke::new(
                    lead.width,
                    leaders.get(ai % leaders.len()).copied().unwrap_or(color).linear_multiply(if selected { 1.0 } else { 0.7 }),
                );
                dashed(painter, anchor, pos2(pr.max.x, anchor.y), on_page);
                painter.line_segment([pos2(pr.max.x, anchor.y), pos2(x0, top + 10.0)], lead);
            }
            let card = if active {
                active_balloon(app, ui, painter, *id, &thread, card, fs, color)
            } else {
                card_shapes(painter, &t, card, color, selected, None);
                let text_painter = painter.with_clip_rect(card.intersect(clip));
                let mut y = card.min.y + 6.0;
                for (g, gap, rule) in lines {
                    y += gap;
                    if rule {
                        // A reply starts: a hairline between it and what it answers.
                        painter.hline(card.min.x + 10.0..=card.max.x - 8.0, y - gap / 2.0, Stroke::new(1.0, t.border));
                    }
                    let h = g.height();
                    g.paint(&text_painter, pos2(card.min.x + 10.0, y), t.text);
                    y += h;
                }
                let r = ui.interact(card, ui.scope_id().with(("balloon", *id)), Sense::click());
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
        select_balloon(app, id);
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
        egui::Shape::rect_filled(card, 4.0, if selected { t.checked } else { t.balloon }),
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

/// A read-only balloon's text, wrapped to `width`. The fork's layout orders right-to-left and
/// mixed-direction paragraphs itself; `TextDirection::Auto` takes each paragraph's first strong
/// character. Place each already-shaped row against the card's start edge, including neutral
/// paragraphs inheriting their predecessor's direction. Logical text and glyph order stay intact.
fn balloon_text(painter: &egui::Painter, text: String, font: egui::FontId, color: Color32, width: f32) -> craft_ui::text::AlignedGalley {
    let mut job = egui::text::LayoutJob::simple(text, font, color, width);
    job.direction = egui::epaint::text::TextDirection::Auto;
    job.halign = egui::Align::Min;
    // Row placement below supplies the reading edge, including empty RTL rows.
    job.halign_min_is_start = false;
    craft_ui::text::align_galley_rows(painter.layout_job(job), width, |row| if row.rtl_paragraph { egui::Align::Max } else { egui::Align::Min })
}

/// The selected balloon: the comment and its replies, each with who wrote it and when, as
/// editable fields (each edit is one undo step via `review.editComment`), an optional reply
/// field, and Reply / Resolve / Delete.
/// `card` carries last frame's size; returns this frame's.
#[allow(clippy::too_many_arguments)]
fn active_balloon(app: &mut WordApp, ui: &mut Ui, painter: &egui::Painter, id: u32, thread: &[u32], card: Rect, fs: f32, color: Color32) -> Rect {
    let t = Tokens::get(ui.ctx());
    let slot = painter.add(egui::Shape::Noop);
    // The card's own click area sits under its fields, so a click on its padding stays here.
    let _ = ui.interact(card, ui.scope_id().with(("balloon", id)), Sense::click());
    let inner = Rect::from_min_max(pos2(card.min.x + 10.0, card.min.y + 6.0), pos2(card.max.x - 6.0, card.min.y + 20_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 3.0;
    let (font, bold, small) = (regular(fs), semibold(fs), regular((fs * 0.9).max(8.0)));
    let focus = std::mem::take(&mut app.canvas.balloon_focus);
    let name = |app: &WordApp, cid: u32| app.session.doc.comments.get(&cid).map(|c| author_name(&c.author)).unwrap_or_default();
    // When it was written, in local time, under the author.
    let when = |app: &WordApp, ui: &mut Ui, cid: u32| {
        let when = app.session.doc.comments.get(&cid).map(crate::local_time::comment_time).unwrap_or_default();
        if !when.is_empty() {
            ui.label(egui::RichText::new(when).font(small.clone()).color(t.text_dim));
        }
    };
    let resolved = app.session.doc.comments.get(&id).is_some_and(|c| c.resolved);
    child.horizontal(|ui| {
        ui.label(egui::RichText::new(name(app, id)).font(bold.clone()).color(t.text));
        if resolved {
            ui.label(egui::RichText::new(tl!("Resolved")).font(small.clone()).color(t.resolved_text));
        }
    });
    when(app, &mut child, id);
    crate::panes::comment_editor(app, &mut child, id, Some(font.clone()), focus && !app.canvas.balloon_reply);
    for rid in thread {
        child.add_space(4.0);
        child.label(egui::RichText::new(name(app, *rid)).font(bold.clone()).color(t.text));
        when(app, &mut child, *rid);
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
        deselect_balloon(app);
        let _ = app.run("review.deleteComment", json!({"id": id}));
    }
    card
}

/// Select a comment's balloon (its text gets the keyboard), writing back any edit in the one
/// selected before.
pub fn select_balloon(app: &mut WordApp, id: u32) {
    if app.canvas.balloon == Some(id) {
        return;
    }
    deselect_balloon(app);
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
pub fn deselect_balloon(app: &mut WordApp) {
    let Some(id) = app.canvas.balloon.take() else { return };
    app.canvas.balloon_reply = false;
    app.canvas.balloon_focus = false;
    let thread: Vec<u32> =
        std::iter::once(id).chain(app.session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k)).collect();
    for c in thread {
        // A refusal is on the status line; the draft stays for when the balloon is selected again.
        let _ = crate::panes::commit_comment(app, c);
    }
    let _ = crate::panes::commit_reply(app, id);
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

/// While the vertical scrollbar of the canvas scroll area `scroll` is dragged, a tip beside the
/// pointer says which page is at the top of the window, as the status bar says it ("Page 15 of
/// 42"): dragging through a long document, you see where you'll land (#2551). `rects` are the
/// pages in content coordinates, `bands` the part of each on screen ([`Geometry::bands`]).
fn scrollbar_page_tip(app: &mut WordApp, ui: &Ui, scroll: egui::Id, rects: &[Rect], bands: &[(f32, f32)], pages: usize) {
    // egui's scroll bars sense under the scroll area's id with the axis (1: vertical).
    let dragged = ui.ctx().is_being_dragged(scroll.with(1usize));
    app.canvas.scrollbar_page =
        (dragged && !rects.is_empty() && pages > 0).then(|| top_page(rects, bands, app.canvas.scroll_offset.y).min(pages - 1));
    let (Some(page), Some(at)) = (app.canvas.scrollbar_page, ui.ctx().pointer_latest_pos()) else { return };
    let text = crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &(page + 1).to_string()), ("pages", &pages.to_string())]);
    egui::Area::new(egui::Id::unique("scrollbar_page_tip"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .pivot(egui::Align2::RIGHT_CENTER)
        .fixed_pos(at - vec2(16.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                // Unwrapped: the area keeps last frame's width, too narrow once the number grows (#3341).
                ui.add(egui::Label::new(text).extend());
            });
        });
}

/// The page shown at height `y` (content coordinates), or the first one below it: the page at
/// the top of a window scrolled to `y`. The last page past the end. A page ends where its band
/// (`bands`, the part on screen) does: with Hide White Space the rects overlap in the hidden
/// margins, and the next page's text already shows there.
fn top_page(rects: &[Rect], bands: &[(f32, f32)], y: f32) -> usize {
    let bottom = |i: usize, r: &Rect| r.min.y + bands.get(i).map_or(r.height(), |b| b.1);
    rects.iter().enumerate().position(|(i, r)| bottom(i, r) > y).unwrap_or(rects.len().saturating_sub(1))
}

/// The page being looked at (#3880): the page of `caret` (page, top, bottom in content
/// coordinates) while any of it is inside the window (`top` .. `top + height`), else the page at
/// the window's top ([`top_page`]).
fn viewed_page(caret: Option<(usize, f32, f32)>, rects: &[Rect], bands: &[(f32, f32)], top: f32, height: f32) -> usize {
    match caret {
        Some((page, from, to)) if to >= top && from <= top + height => page,
        _ => top_page(rects, bands, top),
    }
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

/// Routes pointer input: pen or eraser ink, object and crop drags, table border, selection and move
/// handles, then page hit testing, outlines, headers and footers, equations, selection and caret
/// interaction.
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
    // Inside an equation, clicks place the caret in it (and double-clicks don't select words);
    // so do clicks on the grip and options strip of its frame.
    let on_frame = app.canvas.eq_options.is_some_and(|f| f.strip.contains(p) || f.grip.contains(p));
    if (resp.double_clicked() || resp.triple_clicked()) && (on_frame || layout.equation_hit(page, x, y, story).is_some()) {
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
    let hf_enter = enters_header_footer(app, story, hf_zone);
    if resp.double_clicked() {
        // Its first press already moved into the header/footer and placed the caret.
        if app.canvas.hf_press_switched {
            return;
        }
        if let Some((_, header)) = hf_zone
            && hf_enter
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
        let _ = app.run("select.paragraph", json!({"cell": true}));
        return;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() && !over_balloon(app, p);
    // The equation frame's grip and options strip stay in the equation (the strip's click opens
    // its menu, in `show`).
    if pressed && on_frame {
        return;
    }
    if pressed {
        let now = ui.input(|i| i.time);
        let (delay, dist) = ui.ctx().options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_dist));
        let clicks = press_count(app.canvas.last_press, now, p, delay, dist);
        app.canvas.last_press = Some((now, p, clicks));
        app.canvas.unit_drag = None;
        if clicks <= 1 {
            app.canvas.hf_press_switched = false;
        }
        // A double-click that enters or leaves a header/footer is handled on release.
        let hf_switch = hf_enter || app.canvas.hf_press_switched || (matches!(story, StoryRef::Part(_)) && !in_text_box(app) && hf_zone.is_none());
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
        // While editing a header/footer, a press in another one (the footer while editing the
        // header, or another page's own) edits that one, with the caret at the press. Hit-tested
        // against the story being edited, the press put the caret back in it, scrolling the view
        // away from the band that was clicked.
        if !mods.shift
            && matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
            && let Some((_, header)) = hf_zone
            && enters_header_footer(app, story, hf_zone)
        {
            app.canvas.column_drag = None;
            app.canvas.hf_press_switched = true;
            let id = if header { "insert.editHeader" } else { "insert.editFooter" };
            if app.run(id, json!({ "page": page })).is_ok() {
                let to = app.session.sel.focus.story;
                if let Some(pos) = app.session.layout().hit(page, x, y, to) {
                    app.session.sel = wordcraft_engine::Selection::caret(pos);
                }
                app.session.page_hint = page;
            }
            return;
        }
        // A press in the header/footer area from the body leaves the caret where it is, as in
        // Word (#2103): moving it to the nearest body text scrolled the view there, away from the
        // footer the double-click was aiming at.
        if hf_switch && !mods.shift && hf_zone.is_some() && story == StoryRef::Body && app.session.sel.focus.story == StoryRef::Body {
            app.canvas.column_drag = None;
            return;
        }
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
    // Hover a link: show its target, unless the context menu is opening or open: a tooltip draws
    // above the menu and would cover its first items (#3368).
    if resp.hovered()
        && !app.canvas.dragging
        && !app.canvas.context_menu_open
        && !resp.secondary_clicked()
        && !ui.input(|i| i.pointer.secondary_down())
        && let Some(pos) = layout.hit(page, x, y, story)
        && let Some(link) = link_under(app, layout, page, x, y, &pos)
    {
        let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
        let target = if is_generated_bookmark_link(&link) { tl!("Current Document").to_string() } else { link };
        let tip = format!("{target}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", key)]));
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
}

/// Horizontal and vertical rulers for the caret's page and paragraph. Each spans that page's
/// rect on screen at the page's own scale and is clipped to its strip, whatever the layout
/// (one column, One Page, Multiple Pages), and follows the caret to another page (#1878).
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
    // The caret paragraph's column (in a table, the cell's text) and where it starts, from the
    // page's start edge (the right edge when right-to-left).
    let (col_x, w) = crate::table_ruler::text_area(app, layout, pi).unwrap_or((page.body.x, page.body.w));
    let start = if rtl { page.w - (col_x + w) } else { col_x };
    // Indents and tabs are measured from the paragraph's start edge: the right margin of a
    // right-to-left paragraph, where the start-indent markers sit.
    let px = |s: f32| sx(start + s);
    let dist_at = |x: f32| s_at(x) - start;
    // The markers the numbers keep clear of (#1878): the indents, the tab stops, a table's columns.
    let mut keep_clear: Vec<Rect> = Vec::new();
    if let Some(rp) = &rp {
        let [first, left, right] = [rp.indent_left + rp.indent_first, rp.indent_left, w - rp.indent_right].map(px);
        keep_clear.extend(indent_marker_rects(first, left, right, bar));
        keep_clear.extend(crate::ruler_tabs::marker_rects(app, bar, &px, &rp.tabs));
    }
    keep_clear.extend(crate::table_ruler::column_markers(app, ui.ctx(), layout, pi, bar, x0, scale));
    for (d, mark) in ruler_marks(origin, page.body.w, page.w, (unit, div, step), scale) {
        ruler_mark(&hp, pos2(sx(d), bar.center().y), mark, true, &keep_clear, &t);
    }
    // Margins: the edges of the white text area drag the section's margins. Registered before
    // the indent markers, so a marker sitting on an edge keeps the pointer where they overlap.
    let (le, re) = (x0 + page.body.x * scale, x0 + page.body.right() * scale);
    // An even page of a mirrored layout is the odd page reflected: its left edge is the stored right margin.
    let mirrored = app.session.doc.settings.mirror_margins && page.number.is_multiple_of(2);
    for (edge, zone) in [
        (MarginEdge::Left, Rect::from_min_max(pos2(le - 10.0, bar.min.y), pos2(le + 3.0, bar.max.y))),
        (MarginEdge::Right, Rect::from_min_max(pos2(re - 3.0, bar.min.y), pos2(re + 10.0, bar.max.y))),
    ] {
        margin_handle(app, ui, edge, zone, |p| (p.x - x0) / scale, &sect, mirrored);
    }
    // Indent markers for the caret's paragraph.
    if let Some(rp) = rp {
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
        let id = ui.scope_id().with("ruler_left");
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
        let r = ui.interact(fr, ui.scope_id().with("ruler_first"), Sense::click_and_drag());
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
        let r = ui.interact(rr, ui.scope_id().with("ruler_right"), Sense::click_and_drag());
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
    let vbar = Rect::from_min_max(pos2(v.min.x + 4.0, y0), pos2(v.max.x - 3.0, pr.max.y));
    vp.rect_filled(vbar, 0.0, t.ruler_margin);
    let (te, be) = (y0 + page.body.y * scale, y0 + page.body.bottom() * scale);
    vp.rect_filled(Rect::from_min_max(pos2(vbar.min.x, te), pos2(vbar.max.x, be)), 0.0, t.ruler);
    for (edge, ey) in [(MarginEdge::Top, te), (MarginEdge::Bottom, be)] {
        let zone = Rect::from_min_max(pos2(vbar.min.x, ey - 4.0), pos2(vbar.max.x, ey + 4.0));
        margin_handle(app, ui, edge, zone, |p| (p.y - y0) / scale, &sect, false);
    }
    // The same graduations down the page; the numbers keep clear of a table's row markers.
    let keep_clear = crate::table_ruler::row_markers(app, ui.ctx(), layout, pi, vbar, y0, scale);
    for (d, mark) in ruler_marks(page.body.y, page.body.h, page.h.min(20_000.0), (unit, div, step), scale) {
        ruler_mark(&vp, pos2(vbar.center().x, y0 + d * scale), mark, false, &keep_clear, &t);
    }
    // In a table, its rows' bottom borders.
    crate::table_ruler::rows(app, ui, layout, pi, vbar, y0, scale);
}

/// The indent markers' outlines on the horizontal ruler `bar`, each centred on its screen x:
/// first line (a flag at the top), left and right (at the bottom).
fn indent_marker_rects(first: f32, left: f32, right: f32, bar: Rect) -> [Rect; 3] {
    [
        Rect::from_min_max(pos2(first - 4.5, bar.min.y), pos2(first + 4.5, bar.min.y + 5.0)),
        Rect::from_min_max(pos2(left - 4.5, bar.max.y - 8.0), pos2(left + 4.5, bar.max.y + 1.0)),
        Rect::from_min_max(pos2(right - 4.5, bar.max.y - 6.0), pos2(right + 4.5, bar.max.y - 1.0)),
    ]
}

/// One mark drawn along a ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RulerMark {
    /// A number, `n` units from the start margin.
    Label(i32),
    /// A tick line, `len` screen points long.
    Tick(f32),
}

/// A whole unit's tick where it has no number.
const UNIT_TICK: f32 = 7.0;
/// The least room between two numbers on a ruler (screen points).
const RULER_LABEL_PX: f32 = 30.0;
/// The least room between two ticks on a ruler (screen points).
const RULER_TICK_PX: f32 = 4.0;
/// The sparsest a ruler gets: past this it is left blank.
const RULER_SPARSEST: i32 = 1 << 20;
/// Most marks on one ruler (page sizes come from files).
const RULER_MAX_MARKS: usize = 2000;

/// How a ruler thins out as the page shrinks on screen (#1878).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RulerDensity {
    /// A number every this many units: 1, 2, 4, 8…
    label_every: i32,
    /// A tick every this many of the unit's subdivisions: 1, 2, 4…
    tick_every: i32,
}

/// The density of a ruler whose unit takes `px_per_unit` screen points and has `div`
/// subdivisions: the fewest units between numbers (doubling from 1) that keep them at least
/// `RULER_LABEL_PX` apart, and the fewest subdivisions between ticks that keep those at least
/// `RULER_TICK_PX` apart. The numbers' text keeps its size.
fn ruler_density(px_per_unit: f32, div: i32) -> RulerDensity {
    let px = if px_per_unit.is_finite() { px_per_unit.max(0.0) } else { 0.0 };
    let every = |px: f32, room: f32| {
        let mut n = 1;
        while px * (n as f32) < room && n < RULER_SPARSEST {
            n *= 2;
        }
        n
    };
    RulerDensity { label_every: every(px, RULER_LABEL_PX), tick_every: every(px / div.max(1) as f32, RULER_TICK_PX) }
}

/// The marks of a ruler with their distance from the page's start edge (points along the
/// reading direction): ticks from the page edge out to `width` on the grid of the start margin
/// `origin`, numbered in whole units from that margin across the `text` area only (the margins
/// get ticks, #1878). Whole units, halves, quarters and the rest get shorter ticks in turn, and
/// zoomed out the numbers and ticks thin out (`ruler_density`).
fn ruler_marks(origin: f32, text: f32, width: f32, (unit, div, step): (f32, i32, i32), scale: f32) -> Vec<(f32, RulerMark)> {
    let div = div.max(2);
    let mut marks = Vec::new();
    if !(unit.is_finite() && unit > 0.0 && origin.is_finite() && text.is_finite() && width.is_finite() && scale.is_finite()) {
        return marks;
    }
    let density = ruler_density(scale * unit, div);
    let per_label = div.saturating_mul(density.label_every);
    // The first subdivision at or before the page edge, on the ticks' grid.
    let edge = ((origin / unit).ceil().clamp(-1e6, 1e6) as i32).saturating_mul(div).saturating_neg();
    let mut k = edge.div_euclid(density.tick_every) * density.tick_every;
    for _ in 0..RULER_MAX_MARKS {
        let d = origin + k as f32 * unit / div as f32;
        if d > width + 1e-3 {
            break;
        }
        if d >= -1e-3 && k != 0 {
            let in_text = k > 0 && d <= origin + text + 1e-3;
            let mark = if k % per_label == 0 && in_text {
                RulerMark::Label(k / div * step)
            } else if k % div == 0 {
                RulerMark::Tick(UNIT_TICK)
            } else if k % (div / 2) == 0 {
                RulerMark::Tick(5.0)
            } else if div >= 8 && k % (div / 4) == 0 {
                RulerMark::Tick(3.0)
            } else {
                RulerMark::Tick(1.5)
            };
            marks.push((d, mark));
        }
        k = k.saturating_add(density.tick_every);
    }
    marks
}

/// Draw one graduation at `at` on a ruler, across it when `horizontal`: a tick, or a number,
/// unless its text would sit on a marker in `keep_clear`, where a whole unit's tick stands in.
fn ruler_mark(p: &egui::Painter, at: Pos2, mark: RulerMark, horizontal: bool, keep_clear: &[Rect], t: &Tokens) {
    let len = match mark {
        RulerMark::Label(n) => {
            let galley = p.layout_no_wrap(n.to_string(), regular(9.5), t.ruler_tick);
            let r = Rect::from_center_size(at, galley.size());
            if !keep_clear.iter().any(|o| o.intersects(r)) {
                p.galley(r.min, galley, t.ruler_tick);
                return;
            }
            UNIT_TICK
        }
        RulerMark::Tick(len) => len,
    };
    let (a, b) = if horizontal {
        (pos2(at.x, at.y - len / 2.0), pos2(at.x, at.y + len / 2.0))
    } else {
        (pos2(at.x - len / 2.0, at.y), pos2(at.x + len / 2.0, at.y))
    };
    p.line_segment([a, b], Stroke::new(1.0, t.ruler_tick));
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

/// The stored margin a drag of the visible `edge` to `at` points edits, and its new value. On an
/// even page of a mirrored layout (`mirrored`) the text area is the odd page's reflected, so the
/// visible left edge is the stored right margin and the other way round: the pointer is reflected
/// across the page and the edge swapped (#2914). Top and bottom are the same on every page.
pub(crate) fn margin_drag(edge: MarginEdge, at: f32, s: &wordcraft_doc::section::SectionProps, mirrored: bool) -> (MarginEdge, f32) {
    let (edge, at) = match edge {
        MarginEdge::Left if mirrored => (MarginEdge::Right, s.page_w - at),
        MarginEdge::Right if mirrored => (MarginEdge::Left, s.page_w - at),
        e => (e, at),
    };
    (edge, dragged_margin(edge, at, s))
}

/// A draggable margin edge on a ruler: `to_pt` turns the pointer into points from the page's
/// left (or top) edge, and `mirrored` is an even page of a mirrored layout ([`margin_drag`]). A
/// whole drag is one undo step, through `layout.margins`. The view stays put: following the caret
/// mid-drag would move the page under the pointer and the margin with it.
fn margin_handle(
    app: &mut WordApp,
    ui: &Ui,
    edge: MarginEdge,
    zone: Rect,
    to_pt: impl Fn(Pos2) -> f32,
    sect: &wordcraft_doc::section::SectionProps,
    mirrored: bool,
) {
    let cursor =
        if matches!(edge, MarginEdge::Left | MarginEdge::Right) { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical };
    let r = ui.interact(zone, ui.scope_id().with(("ruler_margin", edge)), Sense::drag()).on_hover_cursor(cursor);
    let r = if r.dragged() { r } else { r.on_hover_text(tl!(edge.label())) };
    if r.dragged()
        && let Some(pp) = r.interact_pointer_pos()
    {
        let (stored, v) = margin_drag(edge, to_pt(pp), sect, mirrored);
        if !r.drag_started() {
            app.session.join_next_undo();
        }
        let _ = app.run_keep_view("layout.margins", json!({ stored.param(): v }));
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

pub(crate) fn part_kind(app: &WordApp, s: StoryRef) -> Option<PartKind> {
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

/// Whether a press in a page's header/footer area (`zone`, from `header_footer_zone`) goes into
/// another header/footer than `story`: from the body, the other one of the two (#1026), or with
/// Different First Page on, page 1's header while editing page 2's (#2826).
fn enters_header_footer(app: &WordApp, story: StoryRef, zone: Option<(Option<StoryRef>, bool)>) -> bool {
    zone.is_some_and(|(shown, header)| match part_kind(app, story) {
        _ if story == StoryRef::Body => true,
        Some(PartKind::Header) => !header || shown != Some(story),
        Some(PartKind::Footer) => header || shown != Some(story),
        _ => false,
    })
}

/// Editing a header or footer, or a text box in one.
pub fn editing_header_footer(app: &WordApp, layout: &DocLayout) -> bool {
    let story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    matches!(part_kind(app, story), Some(PartKind::Header | PartKind::Footer))
}

/// Notes share the body editing context; other non-body stories dim the body.
fn dims_body(app: &WordApp, layout: &DocLayout) -> bool {
    match app.session.sel.focus.story {
        StoryRef::Body => false,
        StoryRef::Part(_) if in_text_box(app) => editing_header_footer(app, layout),
        StoryRef::Part(_) => !matches!(part_kind(app, app.session.sel.focus.story), Some(PartKind::Footnote | PartKind::Endnote)),
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
    // How to paste, chosen before pasting (#2583): the ribbon's Paste menu, item for item. Paste
    // keeps the clipboard's formatting (Word's Keep Source Formatting).
    item(ui, app, "Paste", "edit.paste", json!({}));
    item(ui, app, "Keep Text Only", "edit.pasteText", json!({}));
    item(ui, app, "Merge Formatting", "edit.pasteMerge", json!({}));
    item(ui, app, "Paste Special…", "edit.pasteSpecial", json!({}));
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
        item(ui, app, tl!("Remove Table of Contents"), "references.removeToc", json!({}));
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
        item(ui, app, "Crop", "ui.crop", json!({}));
    }
    if picture || object {
        // A numbered caption under (or over) the picture, shape, text box or chart (#1230).
        item(ui, app, "Insert Caption…", "references.caption", json!({}));
    }
    if object {
        ui.menu_button(tl!("Wrap Text"), |ui| crate::ribbon::wrap_text_items(ui, app));
    }
    if crate::widgets::enabled(app, "shape.fill") {
        item(ui, app, "Format Shape…", "view.formatShapePane", json!({"value": true}));
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

/// The frame around the equation being edited, on screen: Word's outline, the grip at its top
/// left and the options strip inside its right edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquationFrame {
    pub outline: Rect,
    pub grip: Rect,
    pub strip: Rect,
}

/// Lay the frame out around `bx` on the page drawn at `page` (`scale` screen pixels a point):
/// the equation's width and its line's height, plus the strip; the frame's pieces keep their
/// screen size at every zoom.
fn equation_frame(page: Rect, bx: &wordcraft_layout::hit::EquationBox, scale: f32) -> EquationFrame {
    let (pad, strip_w) = (2.0, 10.0);
    let x0 = page.min.x + bx.x * scale - pad;
    let x1 = page.min.x + (bx.x + bx.math.width) * scale + pad;
    let (top, bottom) = (page.min.y + bx.top * scale - pad, page.min.y + bx.bottom * scale + pad);
    let outline = Rect::from_min_max(pos2(x0, top), pos2(x1 + strip_w, bottom));
    let strip = Rect::from_min_max(pos2(x1, top), pos2(x1 + strip_w, bottom));
    let grip = Rect::from_min_size(pos2(x0 - 5.0, top - 3.0), vec2(7.0, 14.0));
    EquationFrame { outline, grip, strip }
}

/// The input method's cursor in its composition, in chars: the end of the range it reports, or the
/// end of the composition when it reports none. The range comes from the OS: it is clamped to the text.
fn preedit_cursor_chars(text: &str, range: Option<&std::ops::Range<usize>>) -> usize {
    let n = text.chars().count();
    range.map_or(n, |r| r.end.min(n))
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

/// The caret colour at page point (`x`, `y`) of page `page`, uncached: for a caret drawn while
/// dragging, such as an inline picture's insertion cursor.
pub(crate) fn caret_color_on(app: &WordApp, layout: &DocLayout, page: usize, x: f32, y: f32) -> Color32 {
    let Some(p) = layout.pages.get(page) else { return Color32::BLACK };
    caret_contrast(wordcraft_render::background_at(&app.session.doc, p, &page_render_options(app, dims_body(app, layout)), x, y))
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
        let link = para.props_of_char(a).link.clone().filter(|l| !l.is_empty()).or_else(|| field_link(para.object_at(a)?))?;
        let (from, to) = (Pos { off: a, ..pos.clone() }, Pos { off: b, ..pos.clone() });
        layout.selection_rects(doc, &from, &to, page).into_iter().any(|(pi, r)| pi == page && r.contains(at)).then_some(link)
    })
}

/// The link a cross-reference field makes: `REF name \h` and `PAGEREF name \h` lead to the
/// bookmark `name` (`#name`), as in Word; without `\h` the result is plain text (#2945).
fn field_link(obj: &wordcraft_doc::InlineObject) -> Option<String> {
    let wordcraft_doc::InlineObject::Field { instr, .. } = obj else { return None };
    let code = wordcraft_engine::cmd::references::FieldCode::parse(instr);
    let name = code.args.first().filter(|n| !n.is_empty())?;
    (matches!(code.name.as_str(), "REF" | "PAGEREF") && code.has('h')).then(|| format!("#{name}"))
}

/// A link to a hidden bookmark (`#_Toc…`, `#_Ref…`): a name the program made, which nobody
/// chose or sees, so the hover tip says where it goes instead of showing it (#2177).
fn is_generated_bookmark_link(link: &str) -> bool {
    link.strip_prefix("#_").is_some()
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

    /// #3931: while Insert › Table's grid asks for a preview the page is laid out with that
    /// table; the document, its revision and undo history stay as they were, and without a
    /// request the page is the document's own again.
    #[test]
    fn the_page_shows_the_table_the_grid_previews() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
        let cells = |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).count();
        let (doc, rev) = (app.session.doc.clone(), app.session.rev());
        app.canvas.table_preview = Some((2, 3));
        assert_eq!(cells(&shown_layout(&mut app)), 6);
        assert_eq!(app.canvas.table_preview, None, "a request lasts one frame");
        assert_eq!((&app.session.doc, app.session.rev(), app.session.can_undo()), (&doc, rev, false));
        app.canvas.table_preview = Some((2, 3));
        let again = shown_layout(&mut app);
        app.canvas.table_preview = Some((2, 3));
        assert!(std::sync::Arc::ptr_eq(&again, &shown_layout(&mut app)), "a still pointer reuses the preview");
        assert_eq!(cells(&shown_layout(&mut app)), 0, "no request: the document's own page");
    }

    /// #3931 review: the preview's pages are drawn from the copy they were laid out from. The
    /// table moves "Kites" down a block, so the document's own block there is not its paragraph.
    #[test]
    fn the_previewed_pages_are_drawn_from_the_previewed_document() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
        app.session.run("caret.end", &json!({})).unwrap();
        app.canvas.table_preview = Some((2, 2));
        let layout = shown_layout(&mut app);
        let text = |doc: &wordcraft_doc::Document, story, path| doc.para(story, path).map(|p| p.text.clone());
        let mut kites = 0;
        for it in layout.pages.iter().flat_map(|p| &p.items) {
            let Placed::Lines { story, path, .. } = it else { continue };
            let shown = text(page_doc(&app), *story, path);
            assert!(shown.is_some(), "{path:?} names a paragraph of the drawn document");
            kites += usize::from(shown.as_deref() == Some("Kites"));
        }
        assert_eq!(kites, 1, "Kites is drawn as itself");
        assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
        assert!(std::ptr::eq(page_doc(&app), &app.session.doc), "no preview: the document's own pages");
    }

    /// #3931 review: with a selection the table replaces it in the copy, so the page draws the
    /// copy's caret, in the table, and no selection: the document's selection names text that has
    /// moved in the preview's layout.
    #[test]
    fn the_preview_draws_the_copy_caret_and_selection() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
        app.session.sel = wordcraft_engine::Selection { anchor: Pos::body(0, 3), focus: Pos::body(1, 2) };
        app.canvas.table_preview = Some((2, 2));
        let layout = shown_layout(&mut app);
        let sel = preview_sel(&app).expect("the preview's selection").clone();
        assert!(sel.is_collapsed(), "the table took the selection's place");
        assert_ne!(sel, app.session.sel);
        let c = layout.caret_on(&sel.focus, 0).expect("the caret is on the preview's page");
        assert!(layout.pages[c.page].items.iter().any(|it| matches!(it, Placed::Lines { path, .. } if path.0.len() == 4 && *path == sel.focus.path)));
        assert!(sel.focus.path.cell().is_some(), "the caret is in the table");
        assert_eq!(shown_layout(&mut app).index, app.session.layout().index);
        assert!(preview_sel(&app).is_none(), "no preview: the document's own selection");
    }

    /// #3931 review: the preview is laid out again when the caret moves or the view changes with
    /// the grid still hovered, so the page shows the table where a click would put it, at the
    /// view's width.
    #[test]
    fn the_preview_follows_the_caret_and_the_view() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")), crate::Services::default());
        let tables =
            |l: &DocLayout| l.index.keys().filter(|(st, p)| *st == StoryRef::Body && p.0.len() == 4).map(|(_, p)| p.0[0]).collect::<HashSet<u32>>();
        app.session.run("caret.docEnd", &json!({})).unwrap();
        app.canvas.table_preview = Some((2, 2));
        let at_end = tables(&shown_layout(&mut app));
        app.session.run("caret.docStart", &json!({})).unwrap();
        app.canvas.table_preview = Some((2, 2));
        let at_start = tables(&shown_layout(&mut app));
        assert_ne!(at_start, at_end, "the caret moved: so did the table");
        let (_, _, fresh) = app.session.preview_layout("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        assert_eq!(at_start, tables(&fresh));
        app.session.view.mode = wordcraft_layout::ViewMode::Web;
        app.session.view.web_width = 300.0;
        app.canvas.table_preview = Some((2, 2));
        let web = shown_layout(&mut app);
        assert_eq!(web.pages[0].w, app.session.layout().pages[0].w, "laid out at the Web view's width");
    }

    /// A window-following layout size (#1876, Read Mode): applied at once on entry or when forced,
    /// held while the window is dragged, applied once the size has been still for the settle time.
    #[test]
    fn settle_waits_for_a_drag_to_stop() {
        let mut s = Settle::default();
        assert!(s.entering());
        assert!(s.apply(vec2(1400.0, 700.0), 1.0, true, false), "entering applies at once");
        assert!(!s.apply(vec2(1400.0, 700.0), 1.01, false, false), "nothing changed");
        assert!(!s.apply(vec2(1300.0, 700.0), 1.02, true, false), "a drag frame waits");
        assert!(!s.apply(vec2(1200.0, 700.0), 1.10, true, false), "still dragging");
        assert!(!s.apply(vec2(1200.0, 700.0), 1.20, true, false), "not settled yet");
        assert!(s.apply(vec2(1200.0, 700.0), 1.11 + RESIZE_SETTLE, true, false), "settled");
        assert!(s.apply(vec2(1100.0, 700.0), 2.0, true, true), "forced applies mid-drag");
        assert!(s.apply(vec2(1100.0, 700.0), 5.0, true, false), "a zoom with a still window applies now");
        s.leave();
        assert!(s.entering());
    }

    /// #2545: notes belong to the body editing context, not the faded header/footer context.
    #[test]
    fn editing_notes_keeps_the_body_and_active_note_opaque() {
        use wordcraft_layout::display::{Draw, page_display};
        for command in ["references.footnote", "references.endnote"] {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Body marker.")), Default::default());
            app.run(command, json!({"text": "Note marker."})).unwrap();
            let note = app.session.sel.focus.story;
            assert!(matches!(part_kind(&app, note), Some(PartKind::Footnote | PartKind::Endnote)));
            app.run("insert.header", json!({"text": "Header marker."})).unwrap();
            let header = app.session.sel.focus.story;
            for story in [note, StoryRef::Body, header] {
                app.session.sel = wordcraft_engine::Selection::caret(app.session.doc.end_of(story));
                let layout = app.session.layout();
                for dark in [false, true] {
                    app.session.view.dark_mode = dark;
                    let opts = page_render_options(&app, dims_body(&app, &layout));
                    let draws = page_display(&app.session.doc, &layout.pages[0], &opts.display);
                    for marker in ["Body", "Note", "Header"] {
                        let alphas: Vec<f32> = draws
                            .iter()
                            .filter_map(|d| match d {
                                Draw::Glyphs { text, alpha, .. } if text.contains(marker) => Some(*alpha),
                                _ => None,
                            })
                            .collect();
                        assert!(!alphas.is_empty(), "{command}: {marker} is drawn");
                        let active = if marker == "Header" { story == header } else { story != header };
                        assert!(
                            alphas.iter().all(|a| if active { *a == 1.0 } else { *a > 0.0 && *a < 1.0 }),
                            "{command}, {story:?}, dark={dark}, {marker}: {alphas:?}"
                        );
                    }
                }
            }
        }
    }

    /// What `page_image` replaced: the raster un-premultiplied, for egui to premultiply again.
    fn by_straight_alpha(img: &wordcraft_render::Rendered) -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight())
    }

    /// #2177: only a link to a program-made hidden bookmark hides its name in the hover tip.
    #[test]
    fn hover_tip_hides_generated_bookmark_names_only() {
        assert!(is_generated_bookmark_link("#_Toc123456"));
        assert!(is_generated_bookmark_link("#_Ref42"));
        assert!(!is_generated_bookmark_link("#Chapter1"));
        assert!(!is_generated_bookmark_link("https://example.com/_x"));
        assert!(!is_generated_bookmark_link(""));
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

    /// #1772: the page-turn bar at the bottom right covered the last lines of the column under it.
    #[test]
    fn reading_columns_end_above_the_page_turn_bar() {
        let scale = PX_PER_PT;
        for (w, h) in [(1280.0, 740.0), (900.0, 700.0), (2000.0, 1200.0)] {
            for requested in [None, Some(180.0), Some(350.0), Some(612.0)] {
                let (_, ph) = reading_size(vec2(w, h), scale, requested);
                let bottom = GAP + ph * scale;
                assert!(bottom <= h - READ_NAV_BAR + 0.5, "{w}×{h} {requested:?}: column ends at {bottom}, bar starts at {}", h - READ_NAV_BAR);
            }
        }
    }

    /// #1772: a chosen column width (or a resized window) left the last visible column cut off at
    /// the right edge. The reading viewport shows only whole columns, centred.
    #[test]
    fn reading_viewport_shows_whole_columns() {
        let scale = PX_PER_PT;
        for (w, h) in [(1280.0, 740.0), (900.0, 700.0), (1440.0, 820.0), (500.0, 600.0)] {
            let full = Rect::from_min_size(pos2(10.0, 20.0), vec2(w, h));
            for requested in [None, Some(180.0), Some(240.0), Some(350.0), Some(612.0)] {
                for zoom in [0.75, 1.0, 1.5] {
                    let (cw, _) = reading_size(full.size(), scale * zoom, requested);
                    let column = cw * scale * zoom;
                    let area = reading_viewport(full, column, scale * zoom);
                    assert!(full.contains_rect(area), "inside the canvas");
                    assert_eq!(area.height(), full.height());
                    assert!((area.center().x - full.center().x).abs() < 0.5, "centred");
                    // Columns at rest start at GAP + k·step from the viewport's left edge. Past
                    // the whole ones, only a column short by under half a point each may show.
                    let step = column + GAP;
                    let whole = ((area.width() - GAP) / step + 1e-3).floor().max(1.0);
                    let used = whole * step + GAP;
                    let short = used + step - area.width();
                    assert!(
                        area.width() <= used + 0.5 || whole == 1.0 || short <= (whole + 1.0) * 0.5 * scale * zoom + 0.5,
                        "{w}×{h} {requested:?} {zoom}: {} wide shows a cut column (columns use {used})",
                        area.width()
                    );
                }
            }
        }
        // With no width chosen the columns are sized to fill the canvas exactly, so none may be
        // dropped: the stored width (pt) and back to px must still fit every one of them.
        for zoom in [0.75, 1.0, 1.25, 1.5] {
            let scale = PX_PER_PT * zoom;
            for w in (300..=4000).step_by(7) {
                let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(w as f32, 700.0));
                let count = (full.width() / 295.0).round().clamp(1.0, 5.0);
                let px = (full.width() - (count + 1.0) * GAP) / count;
                if px < 192.0 {
                    continue; // the minimum width overflows; whole columns then shrink the view
                }
                let (cw, _) = reading_size(full.size(), scale, None);
                let column = cw * scale;
                let area = reading_viewport(full, column, scale);
                let needed = count * column + (count + 1.0) * GAP;
                assert!(area.width() >= needed - 0.5, "{w} at {zoom}: {} wide drops a column of {count} ({needed})", area.width());
            }
        }
        // Junk sizes never panic and fall back to the whole canvas.
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        assert_eq!(reading_viewport(full, f32::NAN, 1.0), full);
        assert_eq!(reading_viewport(full, 0.0, f32::NAN), full);
    }

    #[test]
    fn reading_viewport_keeps_columns_when_the_window_shrinks_a_little() {
        // The column width is only re-laid out when it changes by more than half a point, so a
        // window a few pixels narrower keeps slightly too wide columns: none may drop out.
        for (zoom, shrink) in [(1.0, 1.0), (1.0, 3.0), (1.5, 1.0), (1.5, 3.0), (0.75, 1.0)] {
            let scale = PX_PER_PT * zoom;
            for w in (300..=4000).step_by(7) {
                let before = vec2(w as f32, 700.0);
                let count = (before.x / 295.0).round().clamp(1.0, 5.0);
                if (before.x - (count + 1.0) * GAP) / count < 192.0 {
                    continue;
                }
                let after = before - vec2(shrink, 0.0);
                if (after.x / 295.0).round() != count {
                    continue; // a different column count reflows anyway
                }
                let (kept, _) = reading_size(before, scale, None);
                let (fresh, _) = reading_size(after, scale, None);
                if (kept - fresh).abs() > 0.5 {
                    continue; // re-laid out at once
                }
                let full = Rect::from_min_size(pos2(0.0, 0.0), after);
                let area = reading_viewport(full, kept * scale, scale);
                assert!(
                    area.width() >= full.width() - 0.5,
                    "{w}-{shrink} at {zoom}: kept {kept}pt columns, view narrowed to {} of {}",
                    area.width(),
                    full.width()
                );
            }
        }
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

    /// A large point-unit wheel tick turns exactly one reading column with no spill; line ticks turn
    /// one column either way, small touchpad deltas accumulate, and columns align against the pane.
    #[test]
    fn reading_wheel_snaps_one_column_per_tick_and_respects_touchpads() {
        let ev = |unit, y| egui::Event::MouseWheel {
            unit,
            delta: vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        };
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
            let view = view_key(a, false, None);
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

    /// Read Mode › View › Page Color: the reading columns are drawn on white, sepia or a dark
    /// page, in place of the document's page color; other views keep the page color.
    #[test]
    fn read_mode_page_color_sets_the_reading_paper() {
        use wordcraft_doc::props::Rgb;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Hello WordCraft")), Default::default());
        a.run("design.pageColor", json!({"color": "#102040"})).unwrap();
        let paper = |a: &WordApp| page_render_options(a, false).paper_of(&a.session.doc);
        assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "Print Layout shows the page color");
        let print_key = view_key(&a, false, None);
        a.run("view.readMode", json!({})).unwrap();
        assert_eq!(paper(&a), Rgb::WHITE, "None: white, the page color ignored");
        let none_key = view_key(&a, false, None);
        a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
        assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
        assert!(!page_render_options(&a, false).dark, "Sepia keeps the text's colors");
        let sepia_key = view_key(&a, false, None);
        a.run("view.readPageColor", json!({"value": "inverse"})).unwrap();
        assert!(page_render_options(&a, false).dark);
        let d = wordcraft_render::DARK_PAPER;
        assert_eq!(paper(&a), Rgb(d, d, d));
        let inverse_key = view_key(&a, false, None);
        let keys = [print_key, none_key, sepia_key, inverse_key];
        for (i, k) in keys.iter().enumerate() {
            assert!(!keys[..i].contains(k), "each paper has its own view key");
        }
        a.run("view.printLayout", json!({})).unwrap();
        assert_eq!(paper(&a), Rgb(0x10, 0x20, 0x40), "leaving Read Mode brings the page color back");
        assert!(!page_render_options(&a, false).dark);
        // With Switch Modes on, Read Mode's paper still follows the Page color: None is white and Sepia is sepia.
        a.session.view.dark_mode = true;
        a.run("view.readMode", json!({})).unwrap();
        assert!(!page_render_options(&a, false).dark);
        assert_eq!(paper(&a), Rgb::WHITE);
        a.run("view.readPageColor", json!({"value": "sepia"})).unwrap();
        assert_eq!(paper(&a), wordcraft_render::SEPIA_PAPER);
        a.run("view.printLayout", json!({})).unwrap();
        assert!(page_render_options(&a, false).dark, "Print Layout keeps Switch Modes");
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

    /// #2914: on an even page of a mirrored layout the page is the odd page reflected, so dragging
    /// its visible left edge edits the stored right margin and the visible right edge the left one.
    #[test]
    fn ruler_margin_drags_on_even_mirrored_pages_move_the_visible_edge() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one")), Default::default());
        app.session.doc.settings.mirror_margins = true;
        app.run("layout.margins", json!({"left": 72.0, "right": 36.0})).unwrap();
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("insert.pageBreak", json!({})).unwrap();
        let sect = wordcraft_engine::cmd::page::sect(&app.session);
        let body = |app: &mut WordApp, page: usize| {
            let l = app.session.layout();
            let b = l.pages.get(page).expect("the page exists").body;
            (b.x, b.right(), l.pages[page].number)
        };
        let (x, right, number) = body(&mut app, 1);
        assert_eq!(number, 2, "the second page is even");
        assert!((x - 36.0).abs() < 0.01 && (right - (sect.page_w - 72.0)).abs() < 0.01, "mirrored: outside margin on the left ({x}, {right})");
        // Odd pages and unmirrored layouts keep the plain mapping.
        assert_eq!(margin_drag(MarginEdge::Left, 90.0, &sect, false), (MarginEdge::Left, 90.0));
        assert_eq!(margin_drag(MarginEdge::Right, sect.page_w - 90.0, &sect, false), (MarginEdge::Right, 90.0));

        // The visible left edge of the even page, dragged to 90 pt, becomes the right margin.
        let (edge, v) = margin_drag(MarginEdge::Left, 90.0, &sect, true);
        assert_eq!(edge, MarginEdge::Right);
        app.run("layout.margins", json!({edge.param(): v})).unwrap();
        let after = wordcraft_engine::cmd::page::sect(&app.session);
        assert_eq!((after.margin_left, after.margin_right), (72.0, 90.0), "the left margin was not touched");
        let (x, right, _) = body(&mut app, 1);
        assert!((x - 90.0).abs() < 0.01 && (right - (sect.page_w - 72.0)).abs() < 0.01, "the visible left edge moved to the pointer ({x}, {right})");
        let (x, right, _) = body(&mut app, 0);
        assert!((x - 72.0).abs() < 0.01 && (right - (sect.page_w - 90.0)).abs() < 0.01, "the odd page follows ({x}, {right})");

        // The visible right edge, dragged to 500 pt, becomes the left margin (snapped to 1/16 inch).
        let sect = after;
        let (edge, v) = margin_drag(MarginEdge::Right, 500.0, &sect, true);
        assert_eq!(edge, MarginEdge::Left);
        app.run("layout.margins", json!({edge.param(): v})).unwrap();
        let (_, right, _) = body(&mut app, 1);
        assert!((right - 500.0).abs() <= 2.25, "the visible right edge is at the pointer ({right})");
        // Top and bottom are not mirrored.
        assert_eq!(margin_drag(MarginEdge::Top, 50.0, &sect, true), (MarginEdge::Top, dragged_margin(MarginEdge::Top, 50.0, &sect)));
    }

    /// Dragging a margin edge on the vertical ruler leaves the view where it is. Each step of the
    /// drag runs `layout.margins`, which brought the caret back into view: with the caret off
    /// screen (the bottom margin is far from the text at the top) the page slid under the pointer
    /// mid-drag, and the margin ran far past it (a drag of about 1 cm set it about 6 cm larger).
    #[test]
    fn dragging_the_bottom_margin_on_the_ruler_keeps_the_view_and_follows_the_pointer() {
        let text = vec!["A line of text."; 20].join("\n");
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&text)), Default::default());
        a.run("view.ruler", json!({"value": true})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
        let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        h.run_steps(6);
        let body = h.state_mut().session.layout().pages[0].body;
        let area = h.state().canvas.canvas_rect.expect("the canvas is shown");
        // Scroll the first page's bottom margin to the middle of the window: the caret, at the
        // top of the page, is off screen.
        let bottom = screen(&mut h, 0.0, body.bottom()).y;
        h.state_mut().canvas.scroll_request = Some(vec2(0.0, bottom - area.center().y));
        h.run_steps(4);
        assert!(screen(&mut h, 0.0, body.y).y < area.min.y, "the caret's line is above the view");
        let offset = h.state().canvas.scroll_offset;
        let sect = wordcraft_engine::cmd::page::sect(&h.state().session);
        let depth = h.state().session.undo_depth();

        // The bottom margin's edge on the vertical ruler, dragged up 36 points in a few moves.
        let x = area.min.x - RULER / 2.0;
        let start = pos2(x, screen(&mut h, 0.0, body.bottom()).y);
        let end = pos2(x, screen(&mut h, 0.0, body.bottom() - 36.0).y);
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
            assert_eq!(h.state().canvas.scroll_offset, offset, "the view moved during the drag (step {k})");
        }
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        assert_eq!(h.state().canvas.scroll_offset, offset, "the view moved after the drag");
        let after = wordcraft_engine::cmd::page::sect(&h.state().session);
        assert!((after.margin_bottom - (sect.margin_bottom + 36.0)).abs() <= 4.5, "{} → {}", sect.margin_bottom, after.margin_bottom);
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
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

    /// #2551: dragging the vertical scrollbar shows the page at the top of the window beside the
    /// pointer ("Page 7 of 12"), on one line as the number grows (#3341), and the tip goes when the
    /// button is let go.
    #[test]
    fn dragging_the_scrollbar_names_the_page_at_the_top() {
        use egui::accesskit::Role;
        use egui::{Event, PointerButton};
        use egui_kittest::kittest::Queryable;
        for hide_white_space in [false, true] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
            for _ in 0..11 {
                a.run("text.insert", json!({"text": "Page text"})).unwrap();
                a.run("layout.break", json!({"kind": "page"})).unwrap();
            }
            a.run("view.commentsPane", json!({"value": false})).unwrap();
            a.run("view.hideWhiteSpace", json!({"value": hide_white_space})).unwrap();
            let (rev, sel) = (a.session.rev(), a.session.sel.clone());
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
            assert_eq!(h.state_mut().session.layout().pages.len(), 12);
            let bar = h
                .query_all_by_role(Role::ScrollBar)
                .map(|n| n.rect())
                .filter(|r| r.height() > r.width())
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .expect("the canvas's vertical scrollbar");
            let button =
                |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            let from = pos2(bar.center().x, bar.top() + 8.0);
            h.event(Event::PointerMoved(from));
            h.step();
            assert_eq!(h.state().canvas.scrollbar_page, None, "no tip before a drag");
            h.event(button(from, true));
            h.step();
            let to = pos2(from.x, bar.top() + bar.height() * 0.65);
            for k in 1..=6 {
                h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
                h.step();
            }
            let page = h.state().canvas.scrollbar_page.expect("a tip while the scrollbar is dragged");
            assert!((4..9).contains(&page), "a page well down the document: {page}");
            assert!(h.query_by_label(&format!("Page {} of 12", page + 1)).is_some(), "the tip names page {}", page + 1);
            let end = pos2(from.x, bar.bottom() - 4.0);
            for k in 1..=6 {
                h.event(Event::PointerMoved(to + (end - to) * (k as f32 / 6.0)));
                h.step();
            }
            let last = h.state().canvas.scrollbar_page.expect("still dragging");
            assert!(last >= 9, "a two-digit page: {last}");
            let label = format!("Page {} of 12", last + 1);
            // The status bar (at the left) says the same page (#3880); the tip is the one beside the pointer.
            let tip = h.query_all_by_label(&label).map(|n| n.rect()).find(|r| r.center().x > 700.0).expect("the tip names the last page");
            let body = egui::TextStyle::Body.resolve(&h.ctx.global_style());
            assert!(tip.height() < 1.5 * h.ctx.fonts_mut(|f| f.row_height(&body)), "one line as the number grows: {tip:?}");
            h.event(button(end, false));
            h.step();
            h.step();
            assert_eq!(h.state().canvas.scrollbar_page, None, "the tip goes with the button");
            assert!(h.query_all_by_label(&label).all(|n| n.rect().center().x < 700.0), "and its text (the status bar still names the page)");
            assert_eq!((h.state().session.rev(), &h.state().session.sel), (rev, &sel), "scrolling changes neither the document nor the selection");
        }
    }

    /// #3880: the status bar names the page being looked at. With the caret scrolled out of view, that
    /// is the page at the top of the window; while the caret is on screen, its page.
    #[test]
    fn the_status_bar_names_the_page_in_view_not_only_the_caret_page() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        for _ in 0..11 {
            a.run("text.insert", json!({"text": "Page text"})).unwrap();
            a.run("layout.break", json!({"kind": "page"})).unwrap();
        }
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("caret.docStart", json!({})).unwrap();
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
        assert_eq!(h.state_mut().session.layout().pages.len(), 12);
        assert!(h.query_by_label("Page 1 of 12").is_some(), "the caret is on page 1 and in view");
        // Scroll to page 9 without touching the caret.
        let (rev, sel) = (h.state().session.rev(), h.state().session.sel.clone());
        let target = h.state().canvas.page_rects.get(8).map(|r| r.min.y - h.state().canvas.page_rects[0].min.y).expect("page 9");
        let at = h.state().canvas.scroll_offset.y;
        h.state_mut().canvas.scroll_request = Some(vec2(0.0, at + target));
        for _ in 0..4 {
            h.step();
        }
        assert!(h.query_by_label("Page 9 of 12").is_some(), "scrolled to page 9, the status bar says page 9");
        assert!(h.query_by_label("Page 1 of 12").is_none());
        assert_eq!((h.state().session.rev(), &h.state().session.sel), (rev, &sel), "scrolling changes neither the document nor the selection");
    }

    /// `viewed_page`: the caret's page while any of its line is on screen, else the window's top page.
    #[test]
    fn the_page_in_view_follows_the_caret_while_it_is_on_screen() {
        let rects: Vec<Rect> = (0..4).map(|i| Rect::from_min_size(pos2(0.0, i as f32 * 110.0), vec2(80.0, 100.0))).collect();
        let whole = vec![(0.0, 100.0); 4];
        let caret = |page: usize, y: f32| Some((page, rects[page].min.y + y, rects[page].min.y + y + 10.0));
        // Window 0..150 shows page 0 and the top of page 1.
        assert_eq!(viewed_page(caret(1, 5.0), &rects, &whole, 0.0, 150.0), 1, "the caret is on screen, on page 1");
        assert_eq!(viewed_page(caret(0, 5.0), &rects, &whole, 0.0, 150.0), 0);
        // Scrolled to page 3: the caret on page 0 is off screen.
        assert_eq!(viewed_page(caret(0, 5.0), &rects, &whole, 330.0, 100.0), 3);
        assert_eq!(viewed_page(None, &rects, &whole, 220.0, 100.0), 2, "no caret: the top page");
        assert_eq!(viewed_page(caret(2, 90.0), &rects, &whole, 330.0, 100.0), 3, "a caret just above the window is not in view");
    }

    /// `top_page`: the page at a window's top, the one below in a gap, the last one past the end.
    #[test]
    fn the_page_at_the_top_of_the_window() {
        let rects: Vec<Rect> = (0..3).map(|i| Rect::from_min_size(pos2(0.0, i as f32 * 110.0), vec2(80.0, 100.0))).collect();
        let whole: Vec<(f32, f32)> = rects.iter().map(|r| (0.0, r.height())).collect();
        assert_eq!(top_page(&rects, &whole, 0.0), 0);
        assert_eq!(top_page(&rects, &whole, 99.0), 0);
        assert_eq!(top_page(&rects, &whole, 105.0), 1, "in the gap: the page below");
        assert_eq!(top_page(&rects, &whole, 250.0), 2);
        assert_eq!(top_page(&rects, &whole, 1e6), 2, "past the end: the last page");
        assert_eq!(top_page(&[], &[], 0.0), 0);

        // Hide White Space: the rects overlap in the hidden margins. Just past the seam the next
        // page's text shows, so that page is at the top, though the previous rect reaches further.
        let sizes = [(612.0, 792.0); 3];
        let (full, _) = place_pages(&sizes, 0.0, 1.0, 1, vec2(800.0, 600.0));
        let (moved, bands, _) = collapse_white_space(&full, &[(72.0, 720.0); 3], 1.0, 600.0);
        let seam = moved[0].min.y + bands[0].1;
        assert!(moved[0].max.y > seam + SEAM + 2.0, "the rects overlap past the seam");
        assert_eq!(top_page(&moved, &bands, seam - 2.0), 0);
        assert_eq!(top_page(&moved, &bands, seam + SEAM + 2.0), 1, "across the seam: the next page");
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

    /// A right-to-left paragraph's selection bar is the right margin; the left margin beside it
    /// is nothing special (a click there is the caret's).
    #[test]
    fn selection_bar_of_a_right_to_left_paragraph_is_on_the_right() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "First para\r\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e9}\u{5dc}\u{5d5}\u{5dd}\rThird para"})).unwrap();
        a.run("caret.set", json!({"pos": serde_json::to_value(Pos::body(1, 0)).unwrap()})).unwrap();
        a.run("para.rtl", json!({})).unwrap();
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
        let c = h.state_mut().session.layout().caret(&Pos::body(1, 0)).expect("caret");
        let (body_x, page_w) = {
            let l = h.state_mut().session.layout();
            (l.pages[c.page].body.right(), l.pages[c.page].w)
        };
        let y = c.top + c.height / 2.0;
        let right = page_to_screen(h.state_mut(), c.page, (body_x + page_w) / 2.0, y).expect("on screen");
        let left = page_to_screen(h.state_mut(), c.page, 30.0, y).expect("on screen");
        let click = |h: &mut egui_kittest::Harness<'_, WordApp>, at: Pos2| {
            h.event(Event::PointerMoved(at));
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            for _ in 0..3 {
                h.step();
            }
        };
        h.event(Event::PointerMoved(right));
        h.step();
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the bar's arrow on the right");
        click(&mut h, right);
        let (s, e) = h.state().session.sel.ordered();
        assert_eq!(s, Pos::body(1, 0), "the line is selected");
        assert!(e > s && (e.path == wordcraft_doc::Path::top(1) || e == Pos::body(2, 0)), "{s:?}..{e:?}");
        // Beside it on the left: no bar (not the bar's arrow, and a caret, not a line selection).
        // The cursor there is the text cursor, or none at all on some platforms' test windows.
        h.event(Event::PointerMoved(left));
        h.step();
        assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
        click(&mut h, left);
        assert!(h.state().session.sel.is_collapsed(), "a click in the left margin beside it is a caret, not a line");
    }

    /// #3453: while the Format Painter is armed the pointer over the text says so, and it goes
    /// back once the painter is off. The selection bar keeps its arrow while it is armed.
    #[test]
    fn an_armed_format_painter_changes_the_pointer() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Paint this formatting"})).unwrap();
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
        let c = h.state_mut().session.layout().caret(&Pos::body(0, 6)).expect("caret");
        let over_text = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
        let over_bar = page_to_screen(h.state_mut(), c.page, 30.0, c.top + c.height / 2.0).expect("on screen");
        let pointer_after = |h: &mut egui_kittest::Harness<'_, WordApp>, cmd: Option<&str>| {
            if let Some(cmd) = cmd {
                h.state_mut().run(cmd, json!({})).unwrap();
            }
            h.event(egui::Event::PointerMoved(over_text));
            h.step();
            h.output().platform_output.cursor_icon
        };
        // Where the I-beam is drawn (Windows, #1746) the system pointer over the text is hidden.
        let ibeam = if h.state().canvas.drawn_ibeam { egui::CursorIcon::None } else { egui::CursorIcon::Text };
        assert_eq!(pointer_after(&mut h, None), ibeam, "no painter yet");
        assert_eq!(pointer_after(&mut h, Some("edit.formatPainter")), egui::CursorIcon::Copy, "armed");
        assert_eq!(pointer_after(&mut h, Some("edit.formatPainter")), ibeam, "turned off");
        h.state_mut().run("edit.formatPainter", json!({})).unwrap();
        h.event(egui::Event::PointerMoved(over_bar));
        h.step();
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default, "the selection bar keeps its arrow");
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
        balloon_glyphs_at_scale(text, width, 1.0)
    }

    /// Lays `text` out for a balloon at `ppp` with the document fallback face and returns each glyph's
    /// character and physical left edge, asserting that wrapping stays within `width`.
    fn balloon_glyphs_at_scale(text: &str, width: f32, ppp: f32) -> Option<Vec<(char, f32)>> {
        let face = wordcraft_fonts::FontDb::global().fallback_for('ش', u32::MAX)?;
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        let mut fonts = crate::theme::font_definitions(false, false);
        crate::theme::add_fallback_faces(&mut fonts, &[face]);
        ctx.set_fonts(fonts);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let mut out = Vec::new();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = balloon_text(ui.painter(), text.to_string(), egui::FontId::proportional(14.0), Color32::BLACK, width);
            let shape = g.text_shape(Pos2::ZERO, Color32::BLACK);
            let atlas = ui.fonts(|fonts| fonts.image());
            for r in &shape.galley.rows {
                out.extend(r.row.glyphs.iter().map(|gl| (gl.chr, shape.pos.x + r.pos.x + gl.pos.x)));
                for glyph in &r.row.glyphs {
                    // epaint rounds glyph origins individually while row advance stays fractional.
                    // The two edge roundings can differ by one physical pixel.
                    assert!(
                        shape.pos.x + r.pos.x + glyph.max_x() <= width + 1.0 / ppp,
                        "wrapped glyph {:?}: origin {}, max {}, row width {}, limit {} (text {:?})",
                        glyph.chr,
                        shape.pos.x + r.pos.x,
                        glyph.max_x(),
                        r.size.x,
                        width,
                        text
                    );
                    if !matches!(glyph.chr, '(' | ')') {
                        continue;
                    }
                    // Glyph.chr remains logical in 0.37. Check the actual mirrored bitmap,
                    // not a manually reversed display string, to verify bracket orientation.
                    let [x0, y0] = glyph.uv_rect.min.map(usize::from);
                    let [x1, y1] = glyph.uv_rect.max.map(usize::from);
                    let y = usize::midpoint(y0, y1);
                    let (mut ink, mut moment) = (0.0, 0.0);
                    for x in x0..x1 {
                        let alpha = f32::from(atlas[(x, y)].a());
                        ink += alpha;
                        moment += (x - x0) as f32 * alpha;
                    }
                    assert!(ink > 0.0 && x1 > x0, "bracket must have a visible bitmap");
                    let drawn = if moment / ink < (x1 - x0 - 1) as f32 / 2.0 { '(' } else { ')' };
                    let expected = if glyph.is_rtl() { if glyph.chr == '(' { ')' } else { '(' } } else { glyph.chr };
                    assert_eq!(drawn, expected, "logical bracket {} at bidi level {}", glyph.chr, glyph.bidi_level);
                }
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
        // The logical opening bracket is on the right and is drawn as a closing bracket.
        // balloon_glyphs also checks the atlas bitmap, so this verifies actual `(APIs)` output.
        assert!(x(&g, ')') < x(&g, 'A') && x(&g, '(') > x(&g, 's'), "brackets surround the word: {g:?}");
        // A paragraph that starts in Latin runs left to right.
        let g = balloon_glyphs("@Ahmed برجاء مراجعة", 400.0).unwrap();
        assert!(x(&g, '@') < x(&g, 'ب') && x(&g, '@') < 50.0, "{g:?}");
        assert!(x(&g, 'ب') > x(&g, 'م'), "the Arabic words still read from the right: {g:?}");
        // Wrapped rows stay inside the width.
        let long = "بيان الأنظمة المطلوب ربطها ومتطلبات التكامل وواجهات الربط البرمجي وآلية تبادل البيانات";
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let g = balloon_glyphs_at_scale(long, 200.0, ppp).unwrap();
            assert!(!g.is_empty()); // Check real glyph right edges at each physical scale.
        }
        // Alignment follows each paragraph, including a neutral paragraph's inherited direction.
        let g = balloon_glyphs("شركة\n123\nLatin", 400.0).unwrap();
        assert!(x(&g, 'ش') > 300.0 && x(&g, '1') > 300.0 && x(&g, 'L') < 1.0, "{g:?}");
    }

    struct BalloonFontFixture {
        family: &'static str,
        probe: &'static str,
        text: &'static str,
    }

    /// Explicit embedded bytes only: no OS font discovery or implicit default-font fallback.
    /// Optional-input builds always exercise Latin; qualification must exercise both RTL faces.
    fn balloon_font_fixtures() -> (egui::FontDefinitions, Vec<BalloonFontFixture>) {
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert(
            "Balloon Latin".into(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"))),
        );
        let mut names = vec!["Balloon Latin".to_string()];
        let mut fixtures =
            vec![BalloonFontFixture { family: "Balloon Latin", probe: "Reading", text: "Reading long_unbreakable_text\n123\nLatin\n" }];
        for (family, script, probe, text) in [
            ("Noto Sans Arabic", "Arab", "شركة", "شركة اي\n123\nالربط (APIs) هنا\nLatin\n"),
            ("Noto Sans Hebrew", "Hebr", "שלום", "שלום עולם\n123\nשלום (APIs) עולם\nLatin\n"),
        ] {
            if let Some(face) = wordcraft_fonts::CRAFT_FONTS.iter().find(|face| face.family == family && face.style == "Regular") {
                assert!(face.scripts.contains(&script));
                assert!(!face.bytes.is_empty());
                fonts.font_data.insert(family.into(), std::sync::Arc::new(egui::FontData::from_static(face.bytes)));
                names.push(family.to_string());
                fixtures.push(BalloonFontFixture { family, probe, text });
            }
        }
        if option_env!("CRAFT_FONTS_REQUIRED").is_some() || std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() {
            assert_eq!(fixtures.len(), 3, "qualification requires both exact embedded Arabic and Hebrew faces");
        }
        for name in &names {
            fonts.families.insert(egui::FontFamily::Name(name.clone().into()), vec![name.clone()]);
        }
        fonts.families.insert(egui::FontFamily::Proportional, names.clone());
        fonts.families.insert(egui::FontFamily::Monospace, names);
        (fonts, fixtures)
    }

    /// Independent pre-normalization reading-space oracle: position the original shaped rows
    /// using their paragraph direction, then recompute complete bounds. Returns the reference
    /// galley and whether this fixture actually exercised RTL shaping.
    fn balloon_reading_reference(
        fixture: &BalloonFontFixture,
        original: &egui::Galley,
        width: f32,
        ppp: f32,
    ) -> (std::sync::Arc<egui::Galley>, bool) {
        let mut reference = (*original).clone();
        reference.rect = Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height()));
        reference.mesh_bounds = Rect::NOTHING;
        for row in &mut reference.rows {
            let factor = if row.rtl_paragraph { 1.0 } else { 0.0 };
            row.pos.x = ((f64::from(width) - f64::from(row.size.x)) * factor * f64::from(ppp)).floor() as f32 / ppp;
            reference.rect |= row.rect();
            reference.mesh_bounds |= row.visuals.mesh_bounds.translate(row.pos.to_vec2());
        }
        let mut rtl = false;
        if fixture.family != "Balloon Latin" {
            assert!(original.rows.iter().any(|row| row.rtl_paragraph), "actual RTL shaping is required");
            if width == 0.0 {
                assert!(
                    reference.rows.iter().any(|row| row.rtl_paragraph && row.pos.x < 0.0 && row.size.x > width),
                    "actual negative reading-space overflow is required"
                );
            }
            if width == 200.0 {
                let row = &reference.rows[0];
                let (first, second) = if fixture.family == "Noto Sans Arabic" { ('ش', 'ي') } else { ('ש', 'ע') };
                let x = |c| row.glyphs.iter().find(|glyph| glyph.chr == c).unwrap().pos.x;
                assert!(x(first) > x(second), "the first RTL word is physically right of the next");
                assert!(reference.rows.iter().find(|row| row.text() == "123").unwrap().rtl_paragraph, "neutral paragraph inherits RTL");
                assert!(!reference.rows.iter().find(|row| row.text() == "Latin").unwrap().rtl_paragraph);
            }
            rtl = true;
        }
        (std::sync::Arc::new(reference), rtl)
    }

    /// Every aligned row keeps its logical position, shared glyph meshes and finite ink bounds.
    fn assert_balloon_aligned_rows(galley: &egui::Galley, paint_offset: egui::Vec2, original: &egui::Galley, reference: &egui::Galley, case: &str) {
        let ppp = original.pixels_per_point;
        let (mut empty_mesh_rows, mut drawn_mesh_rows) = (0, 0);
        for (row_index, ((row, expected), old)) in galley.rows.iter().zip(&reference.rows).zip(&original.rows).enumerate() {
            assert!(std::sync::Arc::ptr_eq(&row.row, &old.row), "glyph order/meshes remain shared: {case}, row={row_index}");
            assert_eq!(row.pos.y, old.pos.y, "row Y: {case}, row={row_index}");
            assert!((row.pos + paint_offset - expected.pos).length() * ppp < 1e-4, "absolute row position: {case}, row={row_index}");
            assert!(
                galley.rect.contains_rect(row.rect()),
                "logical row containment: {case}, row={row_index} text={:?} logical={:?} galley={:?}",
                row.text(),
                row.rect(),
                galley.rect
            );
            if row.visuals.mesh.vertices.is_empty() {
                // Empty/whitespace rows still have logical height, but no
                // finite ink rectangle: translating NOTHING yields NaNs.
                empty_mesh_rows += 1;
                assert!(row.visuals.mesh.indices.is_empty(), "empty mesh indices: {case}, row={row_index} text={:?}", row.text());
                assert_eq!(row.visuals.mesh_bounds, Rect::NOTHING, "empty ink sentinel: {case}, row={row_index} text={:?}", row.text());
            } else {
                drawn_mesh_rows += 1;
                let ink = row.visuals.mesh_bounds.translate(row.pos.to_vec2());
                assert!(ink.is_finite(), "finite row ink: {case}, row={row_index} text={:?} ink={ink:?}", row.text());
                assert!(galley.mesh_bounds.is_finite(), "finite galley ink: {case}, row={row_index} bounds={:?}", galley.mesh_bounds);
                assert!(
                    galley.mesh_bounds.contains_rect(ink),
                    "finite ink containment: {case}, row={row_index} text={:?} ink={ink:?} galley={:?}",
                    row.text(),
                    galley.mesh_bounds
                );
                for (vertex_index, vertex) in row.visuals.mesh.vertices.iter().enumerate() {
                    let pos = row.pos + vertex.pos.to_vec2();
                    assert!(
                        galley.mesh_bounds.contains(pos),
                        "ink vertex containment: {case}, row={row_index} vertex={vertex_index} text={:?} pos={pos:?} galley={:?}",
                        row.text(),
                        galley.mesh_bounds
                    );
                }
            }
        }
        assert!(
            empty_mesh_rows > 0 && drawn_mesh_rows > 0,
            "fixture must observe empty and nonempty meshes: {case}, empty={empty_mesh_rows} drawn={drawn_mesh_rows}"
        );
    }

    /// Run `balloon_text` for one width/origin and prove its galley and painted ink match the
    /// independent reference, with the caller's clip retained.
    fn assert_balloon_paint_case(
        ctx: &egui::Context,
        fixture: &BalloonFontFixture,
        font: &egui::FontId,
        original: &egui::Galley,
        reference: &std::sync::Arc<egui::Galley>,
        width: f32,
        origin: Pos2,
    ) {
        let ppp = original.pixels_per_point;
        let rounding = ctx.tessellation_options(|options| options.round_text_to_pixels);
        let case = format!("family={} ppp={ppp} width={width} rounding={rounding} origin={origin:?}", fixture.family);
        let mut clip = Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let aligned = balloon_text(ui.painter(), fixture.text.into(), (*font).clone(), Color32::BLACK, width);
            let craft_ui::text::AlignedGalley::Aligned { galley, paint_offset, reading_rect } = &aligned else { panic!("valid width must align") };
            assert_eq!(aligned.height(), original.rect.height());
            assert_eq!(*reading_rect, Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height())));
            assert_eq!(galley.job.halign, egui::Align::Min);
            assert_eq!(galley.rect.left_top(), Pos2::ZERO);
            assert_eq!(galley.text(), fixture.text);
            assert_eq!(galley.rows.len(), original.rows.len());
            assert_eq!(galley.num_vertices, original.num_vertices);
            assert_eq!(galley.num_indices, original.num_indices);
            assert_balloon_aligned_rows(galley, *paint_offset, original, reference, &case);
            clip = ui.painter().clip_rect();
            aligned.paint(ui.painter(), origin, Color32::BLACK);
        });
        let shapes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) => Some((clipped.clip_rect, shape)),
                _ => None,
            })
            .collect();
        assert_eq!(shapes.len(), 1, "the actual helper paint emits one shape");
        assert_eq!(shapes[0].0, clip, "painting retains the caller's clip");
        let options = ctx.tessellation_options(|options| *options);
        let atlas_size = ctx.fonts(|fonts| fonts.image().size);
        let tessellate = |shape: &egui::epaint::TextShape| {
            let mut tessellator = egui::epaint::Tessellator::new(ppp, options, atlas_size, Vec::new());
            let mut mesh = egui::epaint::Mesh::default();
            tessellator.tessellate_text(shape, &mut mesh);
            mesh
        };
        let actual = tessellate(shapes[0].1);
        let expected = tessellate(&egui::epaint::TextShape::new(origin, std::sync::Arc::clone(reference), Color32::BLACK));
        assert!(!expected.vertices.is_empty(), "real font ink must be present");
        assert_eq!(actual.indices, expected.indices);
        assert_eq!(actual.vertices.len(), expected.vertices.len());
        for (actual, expected) in actual.vertices.iter().zip(&expected.vertices) {
            assert_eq!(actual.uv, expected.uv);
            assert_eq!(actual.color, expected.color);
            assert!((actual.pos - expected.pos).length() * ppp < 1e-4, "absolute physical ink must remain unchanged");
        }
        drop(shapes);
        output.drop_without_applying_deltas();
    }

    /// At four scales and three widths, balloon wrapping keeps the original font rows and glyph
    /// meshes, preserves logical and physical ink, and keeps RTL reading origins, requiring a real
    /// covering face for every fixture.
    #[test]
    fn balloon_wrapper_keeps_real_font_rows_ink_and_reading_origins() {
        let (fonts, fixtures) = balloon_font_fixtures();
        let mut rtl_cases = 0;
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            ctx.set_fonts(fonts.clone());
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            for fixture in &fixtures {
                let probe_font = egui::FontId::new(14.0, egui::FontFamily::Name(fixture.family.into()));
                assert!(
                    ctx.fonts_mut(|fonts| fonts.has_glyphs(&probe_font, fixture.probe)),
                    "the selected face must actually cover {}",
                    fixture.family
                );
                for width in [0.0, 23.3, 200.0] {
                    let font = egui::FontId::proportional(14.0);
                    assert!(
                        ctx.fonts_mut(|fonts| fixture.text.chars().filter(|c| !c.is_control()).all(|c| fonts.has_glyph(&font, c))),
                        "no missing/tofu fixture characters"
                    );
                    let mut job = egui::text::LayoutJob::simple(fixture.text.into(), font.clone(), Color32::BLACK, width);
                    job.direction = egui::epaint::text::TextDirection::Auto;
                    job.halign = egui::Align::Min;
                    job.halign_min_is_start = false;
                    let original = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                    let (reference, rtl) = balloon_reading_reference(fixture, &original, width, ppp);
                    if rtl {
                        rtl_cases += 1;
                    }
                    for rounding in [false, true] {
                        ctx.tessellation_options_mut(|options| options.round_text_to_pixels = rounding);
                        for origin in [pos2(37.25, 19.375), pos2(0.5 / ppp - 1e-5, 0.5 / ppp)] {
                            assert_balloon_paint_case(&ctx, fixture, &font, &original, &reference, width, origin);
                        }
                    }
                }
            }
        }
        assert_eq!(rtl_cases, (fixtures.len() - 1) * 12);
        eprintln!("balloon fixture coverage: {} exact embedded faces, {rtl_cases} RTL geometry cases", fixtures.len());
    }

    fn assert_balloon_timestamp(when: &str) {
        assert_eq!(when.len(), 16, "WordCraft comments carry a local YYYY-MM-DD HH:MM timestamp: {when:?}");
        let b = when.as_bytes();
        assert_eq!(
            (b.get(4), b.get(7), b.get(10), b.get(13)),
            (Some(&b'-'), Some(&b'-'), Some(&b' '), Some(&b':')),
            "timestamp separators: {when:?}"
        );
        assert!(b.iter().enumerate().all(|(i, c)| matches!(i, 4 | 7 | 10 | 13) || c.is_ascii_digit()), "timestamp fields are digits: {when:?}");
    }

    /// One expected read-only card line: find its text shape, prove its origin, glyph coverage and
    /// paragraph reading edge, and return the next line's expected top.
    fn assert_balloon_line(
        ctx: &egui::Context,
        output: &egui::FullOutput,
        card: Rect,
        family: &str,
        expected_lines: &[(&str, f32)],
        line_index: usize,
        mut y: f32,
    ) -> (f32, (String, f32, f32, Pos2, f32)) {
        let (text, gap) = expected_lines[line_index];
        y += gap;
        let shapes: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => Some((clipped.clip_rect, shape)),
                _ => None,
            })
            .collect();
        let occurrence = expected_lines[..line_index].iter().filter(|entry| entry.0 == text).count();
        let expected_occurrences = expected_lines.iter().filter(|entry| entry.0 == text).count();
        assert_eq!(
            shapes.len(),
            expected_occurrences,
            "the actual read-only line must be painted once per occurrence: text={text:?} line={line_index}"
        );
        let (clip, shape) = shapes[occurrence];
        assert_eq!(shape.galley.pixels_per_point, 1.5, "the actual card galley uses the fixture's physical scale: {text}");
        // Recreate the original shaping job independently of the wrapper.
        // Complete normalized bounds may exceed its preserved reading height.
        let original = ctx.fonts_mut(|fonts| fonts.layout_job((*shape.galley.job).clone()));
        assert_eq!(original.rows.len(), shape.galley.rows.len(), "the wrapper preserves original row count: {text}");
        for (row_index, (row, original_row)) in shape.galley.rows.iter().zip(&original.rows).enumerate() {
            assert_eq!(row.pos.y, original_row.pos.y, "the wrapper preserves original row Y: text={text:?} row={row_index}");
        }
        let reading_height = original.rect.height();
        let geometry = (text.to_string(), reading_height, shape.galley.rect.height(), shape.pos, y);
        assert!(card.contains_rect(clip) && clip.is_positive(), "bodies, timestamps and replies keep the actual card clip");
        assert!(
            (shape.pos.y - y).abs() * 1.5 <= 0.501,
            "same reading-origin Y and vertical gaps: text={text:?} line={line_index} expected={y} actual={} gap={gap}",
            shape.pos.y
        );
        assert!(shape.galley.rows.iter().any(|row| !row.visuals.mesh.vertices.is_empty()), "real glyph ink is painted");
        assert!(
            ctx.fonts_mut(|fonts| text.chars().filter(|c| !c.is_control()).all(|c| fonts.has_glyph(&egui::FontId::proportional(14.0), c))),
            "actual app fonts cover text without tofu"
        );
        let width = card.width() - 16.0;
        let origin_x = ((card.min.x + 10.0) * 1.5).round() / 1.5;
        for (row_index, row) in shape.galley.rows.iter().enumerate() {
            let expected_x = if row.rtl_paragraph { ((f64::from(width) - f64::from(row.size.x)) * 1.5).floor() as f32 / 1.5 } else { 0.0 };
            assert!(
                (shape.pos.x + row.pos.x - origin_x - expected_x).abs() * 1.5 < 1e-3,
                "same absolute paragraph reading edge: family={} text={text:?} row={row_index} ppp={} card={card:?} shape_pos={:?} row_pos={:?} origin_x={origin_x} expected_x={expected_x}",
                family,
                shape.galley.pixels_per_point,
                shape.pos,
                row.pos
            );
            assert!(shape.galley.rect.contains_rect(row.rect()));
        }
        (y + reading_height, geometry)
    }

    /// The read-only card paints exactly the author, local timestamp, body, reply author,
    /// timestamp and reply lines, each on the real font's card geometry.
    fn assert_balloon_card(
        app: &WordApp,
        ctx: &egui::Context,
        output: &egui::FullOutput,
        id: u32,
        reply_id: u32,
        fixture: &BalloonFontFixture,
    ) -> bool {
        let Some((_, card)) = app.canvas.balloon_rects.iter().find(|(i, _)| *i == id) else {
            return false;
        };
        assert_eq!(ctx.native_pixels_per_point(), Some(1.5), "the real app receives the fixture's monitor scale");
        assert_eq!(ctx.pixels_per_point(), 1.5, "Interface Auto retains the fixture's physical scale");
        assert_eq!(app.canvas.balloon_rects.len(), 1, "the reply stays in its parent's actual hit card");
        // Current main paints each comment's author, its local timestamp, then its body;
        // a reply repeats that under a 9 pt rule. Model that sequence from the
        // independently shaped originals, not from production's painted Y.
        let comment_when = crate::local_time::comment_time(&app.session.doc.comments[&id]);
        let reply_when = crate::local_time::comment_time(&app.session.doc.comments[&reply_id]);
        for when in [&comment_when, &reply_when] {
            assert_balloon_timestamp(when);
        }
        let body = fixture.text.trim();
        let reply = format!("{} (reply)", fixture.probe);
        let expected_lines = [
            ("Balloon author", 0.0),
            (comment_when.as_str(), 1.0),
            (body, 4.0),
            ("Reply author", 9.0),
            (reply_when.as_str(), 1.0),
            (reply.as_str(), 4.0),
        ];
        let mut y = card.min.y + 6.0;
        let mut line_geometry = Vec::new();
        for line_index in 0..expected_lines.len() {
            let (next_y, geometry) = assert_balloon_line(ctx, output, *card, fixture.family, &expected_lines, line_index, y);
            y = next_y;
            line_geometry.push(geometry);
        }
        assert!(
            (y + 6.0 - card.bottom()).abs() < 1e-3,
            "card height still sums original author, timestamp, body and reply heights and gaps: family={} expected_bottom={} card={card:?} delta={} lines={line_geometry:?}",
            fixture.family,
            y + 6.0,
            y + 6.0 - card.bottom()
        );
        true
    }

    /// Read-only comment and reply cards use the exact embedded faces and keep author, timestamp, body
    /// and reply line geometry, including RTL cases, without editing the document.
    #[test]
    fn read_only_balloon_bodies_and_replies_keep_real_font_card_geometry() {
        let (_, fixtures) = balloon_font_fixtures();
        let db = wordcraft_fonts::FontDb::with_font_dirs(Vec::new());
        let mut rtl_cases = 0;
        for fixture in &fixtures {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
            if fixture.family != "Balloon Latin" {
                let face = db.face(fixture.family, "Regular");
                assert_eq!(face.family, fixture.family, "do not accept a substitute fixture face");
                let embedded = wordcraft_fonts::CRAFT_FONTS.iter().find(|font| font.family == fixture.family && font.style == "Regular").unwrap();
                assert_eq!(face.data(), embedded.bytes);
                assert!(fixture.probe.chars().all(|c| face.covers(c)));
                app.canvas.ime_faces.push(face);
                rtl_cases += 1;
            }
            app.session.author = "Balloon author".into();
            app.run("text.insert", json!({"text": "Some text here"})).unwrap();
            app.run("select.text", json!({"text": "text"})).unwrap();
            let body = fixture.text.trim();
            let id = app.run("review.newComment", json!({"text": body})).unwrap()["id"].as_u64().unwrap() as u32;
            app.session.author = "Reply author".into();
            let reply = format!("{} (reply)", fixture.probe);
            let reply_id = app.run("review.reply", json!({"id": id, "text": reply})).unwrap()["id"].as_u64().unwrap() as u32;
            app.run("view.commentsPane", json!({"value": false})).unwrap();
            app.canvas.balloon = None;
            let rev = app.session.rev();
            let ctx = egui::Context::default();
            ctx.tessellation_options_mut(|options| options.round_text_to_pixels = true);
            let mut checked = false;
            for frame in 0..6 {
                let mut input = egui::RawInput {
                    time: Some(f64::from(frame) / 60.0),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                    ..Default::default()
                };
                // Interface Auto follows the monitor scale; setting only the context's
                // UI zoom would be reset by the real app before these settling frames.
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(1.5);
                let output = ctx.run_ui(input, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                });
                if frame >= 3 {
                    checked |= assert_balloon_card(&app, &ctx, &output, id, reply_id, fixture);
                }
                output.drop_without_applying_deltas();
            }
            assert!(checked, "actual author, timestamp, body and reply shapes must be observed");
            assert_eq!(app.session.rev(), rev, "painting never edits the document");
            assert_eq!(app.session.doc.plain_text(StoryRef::Part(app.session.doc.comments[&id].part)), body);
            assert_eq!(app.session.doc.plain_text(StoryRef::Part(app.session.doc.comments[&reply_id].part)), reply);
        }
        assert_eq!(rtl_cases, fixtures.len() - 1);
        eprintln!("actual balloon card coverage: {} faces, {rtl_cases} RTL author/timestamp/body/reply cases", fixtures.len());
    }

    /// Painted balloon text is clipped inside its comment card.
    #[test]
    fn read_only_balloon_text_is_clipped_to_its_card() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("text.insert", json!({"text": "Some text here"})).unwrap();
        app.run("select.text", json!({"text": "text"})).unwrap();
        let body = "Balloon ink stays within its own card";
        let id = app.run("review.newComment", json!({"text": body})).unwrap()["id"].as_u64().unwrap() as u32;
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        app.canvas.balloon = None;
        let ctx = egui::Context::default();
        let mut checked = false;
        for frame in 0..6 {
            let input = egui::RawInput {
                time: Some(f64::from(frame) / 60.0),
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            if let Some((_, card)) = app.canvas.balloon_rects.iter().find(|(i, _)| *i == id) {
                for clipped in &out.shapes {
                    if let egui::Shape::Text(text) = &clipped.shape
                        && text.galley.text() == body
                    {
                        assert!(card.contains_rect(clipped.clip_rect), "comment ink must not cross onto the page");
                        assert!(clipped.clip_rect.is_positive());
                        checked = true;
                    }
                }
            }
            out.drop_without_applying_deltas();
        }
        assert!(checked, "a read-only comment body must actually be painted");
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
        leave_field(&mut h);
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

    /// #2826: editing the default header, then turning on Different First Page: a double-click in
    /// page 1's top margin goes into page 1's own (new) header, and typing lands there.
    #[test]
    fn double_click_reaches_the_first_page_header_from_another_header() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Body text"})).unwrap();
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("insert.header", json!({"text": "Head"})).unwrap();
        let head = a.session.sel.focus.story;
        a.run("layout.differentFirstPage", json!({"value": true})).unwrap();
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
        let page = h.state_mut().session.layout().pages[0].clone();
        assert!(page.header_story.is_none());
        let at = page_to_screen(h.state_mut(), 0, page.body.x + 20.0, page.body.y / 2.0).expect("top margin on screen");
        h.event(Event::PointerMoved(at));
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            h.step();
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            h.step();
        }
        let first = h.state().session.sel.focus.story;
        assert_eq!(part_kind(h.state(), first), Some(PartKind::Header));
        assert_ne!(first, head, "page 1's own header, not the default one");
        h.event(Event::Text("Cover".into()));
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(h.state().session.doc.plain_text(first), "Cover");
        assert_eq!(h.state_mut().session.layout().pages[0].header_story.map(StoryRef::Part), Some(first));
    }

    /// #1026: while editing the header, double-clicking the bottom margin switches to the
    /// footer (and back), as in Word, instead of selecting a word in the header.
    #[test]
    fn double_click_switches_between_header_and_footer() {
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
            for _ in 0..40 {
                h.step();
            }
        };
        let kind = |h: &egui_kittest::Harness<'_, WordApp>| part_kind(h.state(), h.state().session.sel.focus.story);
        let page = h.state_mut().session.layout().pages[0].clone();
        let (x, top, bottom) = (page.body.x + 20.0, page.body.y / 2.0, (page.body.bottom() + page.h) / 2.0);
        let head_at = page_to_screen(h.state_mut(), 0, x, top).expect("top margin on screen");
        let foot_at = page_to_screen(h.state_mut(), 0, x, bottom).expect("bottom margin on screen");
        double_click(&mut h, head_at);
        assert_eq!(kind(&h), Some(PartKind::Header), "editing a new header");
        let header = h.state().session.sel.focus.story;
        h.event(Event::Text("Head".into()));
        for _ in 0..3 {
            h.step();
        }
        // Header -> footer.
        double_click(&mut h, foot_at);
        assert_eq!(kind(&h), Some(PartKind::Footer), "header -> footer band double-click switches");
        assert!(h.state().session.sel.is_collapsed(), "no word selected");
        assert_eq!(h.state().session.doc.plain_text(header), "Head");
        h.event(Event::Text("Foot".into()));
        for _ in 0..3 {
            h.step();
        }
        let footer = h.state().session.sel.focus.story;
        // Footer -> header.
        double_click(&mut h, head_at);
        assert_eq!(h.state().session.sel.focus.story, header, "footer -> header band double-click switches");
        assert!(h.state().session.sel.is_collapsed(), "no word selected");
        assert_eq!(h.state().session.doc.plain_text(footer), "Foot");
        assert_eq!(h.state().session.doc.plain_text(header), "Head");
    }

    /// #513: right-clicking a picture selects it and offers Wrap Text, as in Word; choosing
    /// Square there wraps text around it.
    /// #1348: an app with one comment ("Original") on some text, its balloon on screen.
    fn comment_harness() -> (egui_kittest::Harness<'static, WordApp>, u32) {
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
        (h, id)
    }

    /// Click comment `id`'s balloon (from no selected balloon), so its text has the keyboard.
    fn click_balloon(h: &mut egui_kittest::Harness<'static, WordApp>, id: u32) {
        h.state_mut().canvas.balloon = None;
        for _ in 0..2 {
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
    }

    fn type_in(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
        h.event(egui::Event::Text(text.into()));
        h.step();
    }

    /// Leave the comment field by clicking on the page (which also leaves the balloon).
    fn leave_field(h: &mut egui_kittest::Harness<'static, WordApp>) {
        let at = h.state().canvas.page_rects[0].center();
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            h.step();
        }
        for _ in 0..3 {
            h.step();
        }
    }

    fn press_escape(h: &mut egui_kittest::Harness<'static, WordApp>) {
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        for _ in 0..3 {
            h.step();
        }
    }

    fn comment_text(app: &WordApp, id: u32) -> String {
        let part = app.session.doc.comments[&id].part;
        app.session.doc.plain_text(StoryRef::Part(part))
    }

    /// #1348: an edit the document refuses (here read-only protection) keeps what was typed and
    /// says why; once the edit is allowed, leaving the field again writes it.
    #[test]
    fn a_refused_comment_edit_keeps_the_draft() {
        let (mut h, id) = comment_harness();
        click_balloon(&mut h, id);
        type_in(&mut h, "Edited");
        h.state_mut().session.doc.settings.protection = Some("readOnly".into());
        leave_field(&mut h);
        assert_eq!(comment_text(h.state(), id), "Original", "the protected document refused the edit");
        let status = h.state().status_msg.as_ref().map(|(s, _)| s.clone()).unwrap_or_default();
        assert!(status.contains("protected"), "the refusal is shown: {status:?}");
        h.state_mut().session.doc.settings.protection = None;
        click_balloon(&mut h, id);
        leave_field(&mut h);
        let text = comment_text(h.state(), id);
        assert!(text.contains("Edited"), "the typed text survived the refusal: {text:?}");
    }

    /// #3268: clicking a balloon's Reply button, typing a reply and clicking away posts it as a
    /// reply comment under the original, not a draft that silently disappears.
    #[test]
    fn replying_to_a_comment_from_its_balloon_posts_the_reply() {
        use egui_kittest::kittest::Queryable;
        let (mut h, id) = comment_harness();
        click_balloon(&mut h, id);
        h.get_by_label("Reply").click();
        for _ in 0..3 {
            h.step();
        }
        type_in(&mut h, "A reply");
        leave_field(&mut h);
        let replies: Vec<u32> = h.state().session.doc.comments.iter().filter(|(_, c)| c.parent == Some(id)).map(|(k, _)| *k).collect();
        assert_eq!(replies.len(), 1, "{:?}", h.state().session.doc.comments);
        assert_eq!(comment_text(h.state(), replies[0]), "A reply");
    }

    /// #1348: a draft typed in one document never lands in another document's comment that happens
    /// to have the same id.
    #[test]
    fn a_comment_draft_stays_with_its_document() {
        let (mut h, id) = comment_harness();
        click_balloon(&mut h, id);
        type_in(&mut h, " stale");
        // An agent replaces the document while the balloon is still being typed in.
        h.state_mut().session.set_document(wordcraft_doc::Document::new());
        let a = h.state_mut();
        a.run("text.insert", json!({"text": "Other words here"})).unwrap();
        a.run("select.text", json!({"text": "words"})).unwrap();
        let fresh = a.run("review.newComment", json!({"text": "Fresh"})).unwrap()["id"].as_u64().unwrap() as u32;
        assert_eq!(fresh, id, "the new document reuses the comment id");
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        for _ in 0..3 {
            h.step();
        }
        click_balloon(&mut h, fresh);
        leave_field(&mut h);
        assert_eq!(comment_text(h.state(), fresh), "Fresh", "the old document's draft didn't leak in");
    }

    /// #1348: saving while a comment is still being typed in saves what was typed.
    #[test]
    fn saving_writes_a_pending_comment_draft_first() {
        let (mut h, id) = comment_harness();
        click_balloon(&mut h, id);
        type_in(&mut h, "Typed ");
        let path = std::env::temp_dir().join(format!("wordcraft-1348-save-{}.docx", std::process::id()));
        h.state_mut().run("file.save", json!({"path": path.to_string_lossy()})).unwrap();
        let text = comment_text(h.state(), id);
        assert!(text.contains("Typed"), "the draft was written before saving: {text:?}");
        assert!(!h.state().session.dirty, "and saved");
        let saved = wordcraft_engine::io::open_path(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let part = saved.comments.values().next().expect("the comment was saved").part;
        assert!(saved.plain_text(StoryRef::Part(part)).contains("Typed"), "the file has the typed text");
    }

    /// #1348: closing a document whose only change is a comment still being typed asks to save.
    #[test]
    fn closing_with_a_pending_comment_draft_asks_to_save() {
        let (mut h, id) = comment_harness();
        h.state_mut().session.dirty = false;
        click_balloon(&mut h, id);
        type_in(&mut h, "Typed ");
        assert!(!h.state_mut().close_requested(), "the typed comment is an unsaved change");
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })));
    }

    /// #1348: an agent can't replace the document while comment text is still being typed
    /// unless it passes `force`, as with unsaved changes.
    #[test]
    fn an_agent_cannot_replace_the_document_over_a_pending_comment_draft() {
        let (mut h, id) = comment_harness();
        h.state_mut().session.dirty = false;
        click_balloon(&mut h, id);
        type_in(&mut h, "Typed ");
        let open = json!({"path": "/nonexistent/other.docx"});
        assert!(crate::control::close_refused(h.state(), "file.open", &open).is_some());
        assert!(crate::control::close_refused(h.state(), "file.new", &json!({})).is_some());
        assert!(crate::control::close_refused(h.state(), "file.close", &json!({})).is_some());
        assert!(crate::control::close_refused(h.state(), "file.close", &json!({"force": true})).is_none());
        assert!(crate::control::close_refused(h.state(), "text.insert", &json!({"text": "x"})).is_none());
    }

    /// #1348: a comment edit typed, then refused (the document became read-only).
    fn refused_draft() -> (egui_kittest::Harness<'static, WordApp>, u32) {
        let (mut h, id) = comment_harness();
        h.state_mut().session.dirty = false;
        click_balloon(&mut h, id);
        type_in(&mut h, "Refused ");
        h.state_mut().session.doc.settings.protection = Some("readOnly".into());
        (h, id)
    }

    /// #1348: a draft the document refuses doesn't keep the window open: closing asks to save,
    /// and Don't Save drops the draft and closes.
    #[test]
    fn a_refused_comment_draft_can_be_dropped_on_close() {
        let (mut h, _) = refused_draft();
        assert!(!h.state_mut().close_requested(), "the refused draft is an unsaved change");
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "the save prompt asks about it");
        h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(h.state_mut().close_requested(), "Don't Save closes");
    }

    /// #1348: New with a refused draft asks to save; Don't Save makes the new document.
    #[test]
    fn a_refused_comment_draft_can_be_dropped_on_new() {
        let (mut h, _) = refused_draft();
        let document = h.state().session.document_id();
        h.state_mut().run("file.new", json!({})).unwrap();
        assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::SaveChanges { .. })), "the save prompt asks about it");
        h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_ne!(h.state().session.document_id(), document, "Don't Save made the new document");
    }

    /// #1348: Save with a refused draft saves the rest of the document, keeps the draft and says why.
    #[test]
    fn saving_with_a_refused_comment_draft_saves_the_document() {
        let (mut h, id) = refused_draft();
        h.state_mut().session.dirty = true;
        let path = std::env::temp_dir().join(format!("wordcraft-1348-refused-{}.docx", std::process::id()));
        let r = h.state_mut().run("file.save", json!({"path": path.to_string_lossy()}));
        let saved = wordcraft_engine::io::open_path(&path);
        let _ = std::fs::remove_file(&path);
        assert!(r.is_ok(), "{r:?}");
        assert!(saved.is_ok(), "the document was saved");
        assert!(!h.state().session.dirty);
        assert!(crate::panes::comment_draft(h.state(), id).is_some_and(|d| d.contains("Refused")), "the draft stays");
        let status = h.state().status_msg.as_ref().map(|(s, _)| s.clone()).unwrap_or_default();
        assert!(status.contains("protected"), "the refusal is shown: {status:?}");
    }

    /// #1348: in a read-only document the comment fields can't be typed in.
    #[test]
    fn comment_fields_are_read_only_where_comments_cant_change() {
        let (mut h, id) = comment_harness();
        h.state_mut().session.doc.settings.protection = Some("readOnly".into());
        h.step();
        click_balloon(&mut h, id);
        type_in(&mut h, "Typed ");
        assert_eq!(crate::panes::comment_draft(h.state(), id), None, "nothing was typed");
        assert_eq!(comment_text(h.state(), id), "Original");
    }

    /// #1348: Escape in a comment field throws the draft away and shows the comment's text again.
    #[test]
    fn escape_reverts_a_comment_draft() {
        let (mut h, id) = comment_harness();
        click_balloon(&mut h, id);
        type_in(&mut h, "Discarded ");
        press_escape(&mut h);
        assert_eq!(crate::panes::comment_draft(h.state(), id), None, "the draft is gone");
        assert_eq!(comment_text(h.state(), id), "Original", "nothing was written");
        leave_field(&mut h);
        assert_eq!(comment_text(h.state(), id), "Original", "and nothing is written later");
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

    /// #1500: right-clicking a selected shape offers Format Shape…, which opens the Format Shape
    /// pane and leaves that shape selected.
    #[test]
    fn right_click_on_a_shape_opens_the_format_shape_pane() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Text before the shape "})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        let pos = wordcraft_engine::cmd::objects::selected(&a.session).unwrap().0;
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
        assert!(crate::widgets::enabled(h.state(), "shape.fill"), "Shape Fill applies to the selected shape");
        assert!(!h.state().session.view.format_shape_pane, "the pane starts closed");
        let rect = h.state_mut().session.layout().object(&pos, 0).expect("the shape is laid out").rect;
        let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
        // A menu item's label ends with its (empty) shortcut column; the ribbon's own Format
        // Shape… launcher has none, so this names the context menu's entry only.
        let entry = "Format Shape… ";
        assert!(h.query_by_label(entry).is_none(), "no context menu yet");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        h.get_by_label(entry).click();
        for _ in 0..3 {
            h.step();
        }
        assert!(h.state().session.view.format_shape_pane, "Format Shape… opened the pane");
        let picked = wordcraft_engine::cmd::objects::object_selection(&h.state().session).map(|(p, _)| p);
        assert_eq!(picked, Some(pos), "the same shape stays selected");
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

    /// #2179: the context menu on a table of contents entry also offers Remove Table of Contents.
    #[test]
    fn context_menu_on_a_toc_entry_removes_the_table() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_engine::sample::report()), Default::default());
        a.run("caret.docStart", json!({})).unwrap();
        a.run("references.toc", json!({})).unwrap();
        let is_entry = |b: &wordcraft_doc::Block| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("TOC1"));
        let entry = a.session.doc.body.iter().position(|b| is_entry(b)).expect("a TOC entry");
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
        h.get_by_label_contains("Remove Table of Contents").click();
        for _ in 0..3 {
            h.step();
        }
        assert!(!h.state().session.doc.body.iter().any(|b| is_entry(b)), "the table is gone");
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

    /// #2583: the context menu offers the ribbon's Paste menu, item for item, and each pastes its
    /// own way: Paste keeps the copied bold and colour (Word's Keep Source Formatting), Merge
    /// Formatting keeps only the bold, Keep Text Only neither; Paste Special… opens its dialog.
    #[test]
    fn context_menu_offers_the_ribbons_paste_items() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::props::{Rgb, TextColor};
        let red = Some(TextColor::Rgb(Rgb(0xC0, 0, 0)));
        let items = [
            ("Paste", "edit.paste"),
            ("Keep Text Only", "edit.pasteText"),
            ("Merge Formatting", "edit.pasteMerge"),
            ("Paste Special…", "edit.pasteSpecial"),
        ];
        for (choice, pasted) in [
            ("Paste", Some((true, red))),
            ("Merge Formatting", Some((true, None))),
            ("Keep Text Only", Some((false, None))),
            ("Paste Special…", None),
        ] {
            let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Bold\nTarget")), Default::default());
            a.session
                .doc
                .format_range(&Pos::body(0, 0), &Pos::body(0, 4), &|c| {
                    c.bold = Some(true);
                    c.color = red;
                })
                .unwrap();
            a.session.relayout();
            a.run("select.text", json!({"text": "Bold"})).unwrap();
            a.run("edit.copy", json!({})).unwrap();
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
            let c = h.state_mut().session.layout().caret(&Pos::body(1, 6)).expect("laid out");
            let at = page_to_screen(h.state_mut(), c.page, c.x, c.top + c.height / 2.0).expect("on screen");
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
                h.step();
            }
            h.step();
            // Each item reads as its name and its shortcut (none for Merge Formatting).
            let labels: Vec<String> = items.iter().map(|(name, id)| format!("{name} {}", crate::widgets::shortcut_text(h.state(), id))).collect();
            for l in &labels {
                assert!(h.query_by_label(l).is_some(), "{choice}: {l:?} is offered");
            }
            let k = items.iter().position(|(name, _)| *name == choice).unwrap();
            h.get_by_label(&labels[k]).click();
            for _ in 0..3 {
                h.step();
            }
            match pasted {
                Some((bold, colour)) => {
                    let doc = &h.state().session.doc;
                    assert_eq!(doc.plain_text(StoryRef::Body), "Bold\nTargetBold", "{choice}: pasted at the end of the second paragraph");
                    let p = doc.para_at(&Pos::body(1, 0)).expect("the target paragraph");
                    assert_eq!(p.props_of_char(7).bold == Some(true), bold, "{choice}: bold");
                    assert_eq!(p.props_of_char(7).color, colour, "{choice}: colour");
                }
                None => {
                    assert!(matches!(h.state().dialog, Some(crate::dialogs::Dialog::PasteSpecial { .. })), "{choice}: the dialog opens");
                }
            }
        }
    }

    /// Word's frame around the equation being edited: as wide as the equation plus the options
    /// strip at its right, as tall as its line, with the grip over its top-left corner. The
    /// strip and grip keep their screen size at every zoom.
    #[test]
    fn the_equation_frame_holds_the_equation_and_its_strip() {
        let math = wordcraft_layout::math::MathLayout { width: 40.0, ascent: 10.0, descent: 5.0, ..Default::default() };
        let bx = wordcraft_layout::hit::EquationBox { page: 0, x: 100.0, base: 90.0, top: 76.0, bottom: 98.0, math: std::sync::Arc::new(math) };
        let page = Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 800.0));
        for scale in [1.0, 2.5] {
            let f = equation_frame(page, &bx, scale);
            let (left, right) = (10.0 + 100.0 * scale, 10.0 + 140.0 * scale);
            assert!(f.outline.min.x < left && f.strip.min.x > right && f.strip.max.x == f.outline.max.x, "{f:?}");
            assert!(f.outline.min.y < 20.0 + 76.0 * scale && f.outline.max.y > 20.0 + 98.0 * scale, "{f:?}");
            assert_eq!((f.strip.min.y, f.strip.max.y), (f.outline.min.y, f.outline.max.y));
            assert!(f.grip.contains(f.outline.left_top() + vec2(1.0, 1.0)), "{f:?}");
            assert_eq!(f.strip.width(), 10.0);
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
            let ctx = h.ctx.clone();
            h.state_mut().raw_input_hook(&ctx, &mut raw);
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

    /// #3340: review.spelling (the Review tab's Spelling & Grammar button and the Home tab's
    /// Editor button both run it) only moved the selection to the next issue — with no way to see
    /// or apply a suggestion from there, just the generic selection toolbar, it looked broken.
    /// Running it now also opens the same suggestions menu Shift+F10 and a Control-click already
    /// open for the word at the caret.
    #[test]
    fn spelling_and_grammar_opens_suggestions_for_the_issue_it_selects() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "The quick brwn fox"})).unwrap();
        a.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        a.run("review.spelling", json!({})).unwrap();
        assert!(a.canvas.context_menu_from_keys, "queues the open the context menu already uses for Shift+F10");
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
        assert!(h.state().canvas.context_menu_open, "Spelling & Grammar opens the suggestions menu");
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

    /// #2945: only a `REF` or `PAGEREF` with `\h` and a bookmark makes a link.
    #[test]
    fn only_hyperlinked_reference_fields_make_links() {
        let field = |instr: &str| wordcraft_doc::InlineObject::Field {
            instr: instr.into(),
            result: "x".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        };
        assert_eq!(field_link(&field(r"REF _Ref000000001 \h")).as_deref(), Some("#_Ref000000001"));
        assert_eq!(field_link(&field(r"PAGEREF Spot \h")).as_deref(), Some("#Spot"));
        assert_eq!(field_link(&field(r"ref Spot \* MERGEFORMAT \H")).as_deref(), Some("#Spot"));
        for plain in [r"REF Spot", r"REF \h", "REF", "", r"PAGE \h", r"TOC \h", r"SEQ Figure \h"] {
            assert_eq!(field_link(&field(plain)), None, "{plain:?}");
        }
    }

    /// #2945: a cross-reference (`REF … \h`) is a link to its target: Ctrl+click on it goes there,
    /// and its hover tip names no bookmark.
    #[test]
    fn ctrl_click_on_a_cross_reference_goes_to_its_target() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Rock\nSee ")), Default::default());
        a.run("select.text", json!({"text": "Rock"})).unwrap();
        a.run("references.caption", json!({"label": "Figure", "text": "A rock"})).unwrap();
        a.run("caret.docEnd", json!({})).unwrap();
        a.run("insert.crossReference", json!({"to": "figure", "target": "Figure 1", "show": "number"})).unwrap();
        let caption = a.session.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.props.style.as_deref() == Some("Caption"))).unwrap();
        let last = a.session.doc.body.len() - 1;
        assert_ne!(caption, last);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        // The field sits after "See ": click between its two edges.
        let obj = wordcraft_doc::para::OBJ.len_utf8();
        let (before, after) = {
            let l = h.state_mut().session.layout();
            let pos = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(last), off };
            (l.caret(&pos(4)).unwrap(), l.caret(&pos(4 + obj)).unwrap())
        };
        let at = screen(&mut h, (before.x + after.x) / 2.0, before.top + before.height / 2.0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND };
        h.event_modifiers(press, egui::Modifiers::COMMAND);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        steps(&mut h, 3);
        let s = &h.state().session;
        assert_eq!(s.sel.focus.path.last(), caption, "the caret went to the caption: {:?}", s.sel);
    }

    /// #3368: right-clicking a link dismisses the hover tooltip. A tooltip draws above the
    /// context menu and covered its first items (Cut, Copy, Paste, Font…), so the menu looked
    /// truncated; the tooltip must stand down while the menu owns the pointer.
    #[test]
    fn right_clicking_a_link_hides_the_hover_tooltip() {
        use egui_kittest::kittest::Queryable;
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("See here")), Default::default());
        a.run("select.text", json!({"text": "here"})).unwrap();
        a.run("insert.link", json!({"url": "https://example.com/very/long/tracking/url?with=parameters&and=still-more"})).unwrap();
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
        let c = h.state_mut().session.layout().caret(&Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 }).unwrap();
        let at = screen(&mut h, c.x, c.top + c.height / 2.0);
        h.event(egui::Event::PointerMoved(at));
        steps(&mut h, 2);
        let tips = |h: &egui_kittest::Harness<'_, WordApp>| h.query_all_by_label_contains("Click to follow link").count();
        assert!(tips(&h) > 0, "hovering the link shows its target");
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        steps(&mut h, 2);
        assert!(h.state().canvas.context_menu_open, "the context menu is open");
        assert_eq!(tips(&h), 0, "the link tooltip is gone, so it can't cover the menu");
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

    /// #1230: right-clicking a picture offers Insert Caption…, as in Word. For a picture in a
    /// table cell the dialog proposes a Figure caption (not Table), and the caption goes under
    /// the picture in that cell.
    #[test]
    fn right_click_on_a_picture_offers_insert_caption() {
        let mut h = table_window();
        h.state_mut().run("caret.end", json!({})).unwrap();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([30, 90, 200, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        h.state_mut().run("insert.picture", json!({"data": wordcraft_engine::cmd::insert::base64_encode(&png), "width": 60})).unwrap();
        let pos = wordcraft_engine::cmd::objects::selected(&h.state().session).unwrap().0;
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(Pos::body(1, 0));
        steps(&mut h, 4);
        let rect = h.state_mut().session.layout().object(&pos, 0).expect("the picture is laid out").rect;
        let at = crate::frame::to_screen(h.state().canvas.page_rects[0], h.state().canvas.scale, rect).center();
        right_click(&mut h, at);
        assert!(wordcraft_engine::cmd::objects::object_selection(&h.state().session).is_some(), "the right-click selected the picture");
        assert_menu(&h, &["Insert Caption…", "Wrap Text"], true);
        pick(&mut h, &["Insert Caption…"]);
        let form = match &h.state().dialog {
            Some(crate::dialogs::Dialog::Refs(d)) => match &**d {
                crate::dialogs_refs::RefsDialog::Caption(f) => f.clone(),
                d => panic!("not the caption dialog: {d:?}"),
            },
            d => panic!("no caption dialog: {d:?}"),
        };
        assert_eq!(form.label, "Figure", "a picture in a table gets a figure caption");
        h.state_mut().dialog = None;
        h.state_mut().run("references.caption", form.params()).unwrap();
        let (table, r, c) = pos.path.cell().unwrap();
        let cell: Vec<String> = h.state().session.doc.table(StoryRef::Body, &table).unwrap().rows[r].cells[c]
            .blocks
            .iter()
            .filter_map(|b| b.as_para())
            .map(|p| p.plain_text())
            .collect();
        assert_eq!(cell.last().map(String::as_str), Some("Figure 1"), "{cell:?}");
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
            header: vec![cell(10.0, false, StoryRef::Part(1)), cell(20.0, true, StoryRef::Part(1))],
            footer: vec![cell(200.0, false, StoryRef::Part(2))],
            ..Default::default()
        };
        let ys: Vec<f32> = gridline_cells(&page, true, None).map(|r| r.y).collect();
        assert_eq!(ys, [100.0, 10.0, 200.0]);
        let ys: Vec<f32> = gridline_cells(&page, true, Some(StoryRef::Part(2))).map(|r| r.y).collect();
        assert_eq!(ys, [100.0, 10.0, 200.0], "explicit View Gridlines still shows the other stories");
        let ys = |editing| gridline_cells(&page, false, editing).map(|r| r.y).collect::<Vec<_>>();
        assert_eq!(ys(Some(StoryRef::Part(1))), [10.0]);
        assert_eq!(ys(Some(StoryRef::Part(2))), [200.0]);
        assert!(ys(None).is_empty(), "closing header/footer editing removes the automatic guides");
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

        // The marks: inches labelled from the start margin (the page edge an inch before it gets
        // a whole-inch tick, not a number, #1878; the margin itself has no mark), half inches the
        // long ticks, ending on the far page edge.
        let marks = ruler_marks(72.0, body_right - body_x, page_w, (unit, 8, 1), scale);
        let at = |d: f32| marks.iter().find(|(td, _)| (*td - d).abs() < 1e-4).map(|(_, m)| *m);
        assert_eq!(marks.first().map(|(d, _)| *d), Some(0.0));
        assert_eq!(marks.last().map(|(d, _)| *d), Some(page_w));
        assert_eq!(at(0.0), Some(RulerMark::Tick(UNIT_TICK)));
        assert_eq!(at(72.0), None);
        assert_eq!(at(72.0 + unit), Some(RulerMark::Label(1)));
        assert_eq!(at(72.0 + 2.0 * unit), Some(RulerMark::Label(2)));
        assert_eq!(at(72.0 + unit / 2.0), Some(RulerMark::Tick(5.0)));
        assert_eq!(at(72.0 + unit / 8.0), Some(RulerMark::Tick(1.5)));
        // Zoomed out, sub-half-inch ticks drop out rather than smudge together.
        let coarse = ruler_marks(72.0, body_right - body_x, page_w, (unit, 8, 1), 0.3);
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

    /// #1878: a ruler's numbers and ticks thin out as the page shrinks on screen, so they never
    /// pile up: the fewest units between numbers (1, 2, 4, 8…) that keep them 30 px apart, and
    /// the fewest subdivisions between ticks that keep those 4 px apart.
    #[test]
    fn ruler_density_thins_out_as_the_page_shrinks() {
        let d = |px, div| {
            let d = ruler_density(px, div);
            (d.label_every, d.tick_every)
        };
        // Inches (8 subdivisions) at 100%, 50%, 25% and 10%: 96, 48, 24 and 9.6 px per inch.
        assert_eq!(d(96.0, 8), (1, 1));
        assert_eq!(d(48.0, 8), (1, 1));
        assert_eq!(d(24.0, 8), (2, 2));
        assert_eq!(d(9.6, 8), (4, 4));
        // Centimetres (4 subdivisions) at the same zooms: 37.8, 18.9, 9.45 and 3.78 px per cm.
        assert_eq!(d(37.8, 4), (1, 1));
        assert_eq!(d(18.9, 4), (2, 1));
        assert_eq!(d(9.45, 4), (4, 2));
        assert_eq!(d(3.78, 4), (8, 8));
        // Hostile sizes leave the ruler blank rather than loop or panic.
        assert_eq!(d(0.0, 8), (RULER_SPARSEST, RULER_SPARSEST));
        for px in [-1.0, f32::NAN, f32::INFINITY] {
            let (l, t) = d(px, 0);
            assert!(l >= 1 && t >= 1, "{px}");
        }
        // Every numbered position is on the ticks' grid, whatever the zoom.
        for div in [4, 8] {
            for z in 1..=200 {
                let px = z as f32 * 0.5;
                let d = ruler_density(px, div);
                assert_eq!(div * d.label_every % d.tick_every, 0, "{px} px per unit, {div} subdivisions");
            }
        }
    }

    /// #1878: a ruler numbers the text area only (the margins get ticks), its numbers are never
    /// closer than 30 px however far out the page is zoomed, and whole units without a number
    /// keep a long tick.
    #[test]
    fn ruler_marks_number_the_text_area_and_thin_out_zoomed_out() {
        let (unit, page_w, text_w) = (72.0, 612.0, 468.0);
        let at = |marks: &[(f32, RulerMark)], d: f32| marks.iter().find(|(td, _)| (*td - d).abs() < 1e-3).map(|(_, m)| *m);
        let labels = |marks: &[(f32, RulerMark)]| {
            marks
                .iter()
                .filter_map(|(d, m)| match m {
                    RulerMark::Label(n) => Some((*d, *n)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // 90 px per inch: a number at every inch of the text area, 1 to 6; the margins get the
        // whole-inch tick instead (the page edge an inch before the margin, 7 past it).
        let marks = ruler_marks(72.0, text_w, page_w, (unit, 8, 1), 1.25);
        assert_eq!(labels(&marks), (1..=6).map(|n| (72.0 + n as f32 * unit, n)).collect::<Vec<_>>());
        assert_eq!(at(&marks, 0.0), Some(RulerMark::Tick(UNIT_TICK)));
        assert_eq!(at(&marks, 72.0 + 7.0 * unit), Some(RulerMark::Tick(UNIT_TICK)));
        // 10.8 px per inch: a number every 4 inches, ticks at whole and half inches only.
        let tiny = ruler_marks(72.0, text_w, page_w, (unit, 8, 1), 0.15);
        assert_eq!(labels(&tiny), vec![(72.0 + 4.0 * unit, 4)]);
        assert!(tiny.iter().all(|(d, _)| ((d - 72.0) / (unit / 2.0)).fract().abs() < 1e-3), "{tiny:?}");
        assert!(tiny.len() >= 8, "{tiny:?}");
        // At every zoom and in every unit the numbers are at least 30 px apart, positive, and
        // only across the text area.
        for z in 1..=100 {
            let scale = z as f32 * 0.05;
            for (unit, div, step) in [(72.0, 8, 1), (wordcraft_geom::PT_PER_CM, 4, 1), (wordcraft_geom::PT_PER_CM, 4, 10), (72.0, 8, 72)] {
                let marks = ruler_marks(72.0, text_w, page_w, (unit, div, step), scale);
                let found = labels(&marks);
                let xs: Vec<f32> = found.iter().map(|(d, _)| d * scale).collect();
                assert!(xs.windows(2).all(|w| w[1] - w[0] >= RULER_LABEL_PX - 1e-3), "scale {scale}, unit {unit}: {xs:?}");
                assert!(found.iter().all(|(d, n)| *d > 72.0 && *d <= 72.0 + text_w + 1e-3 && *n > 0), "scale {scale}, unit {unit}: {found:?}");
            }
        }
        // Hostile sizes: nothing, or a bounded list, and no panic.
        assert!(ruler_marks(f32::NAN, text_w, page_w, (unit, 8, 1), 1.0).is_empty());
        assert!(ruler_marks(72.0, text_w, f32::INFINITY, (unit, 8, 1), 1.0).is_empty());
        assert!(ruler_marks(72.0, text_w, page_w, (0.0, 8, 1), 1.0).is_empty());
        assert!(ruler_marks(1e9, text_w, 1e9, (unit, 8, 1), 1.0).len() <= RULER_MAX_MARKS);
        assert!(ruler_marks(72.0, text_w, 1e9, (unit, 8, 1), 1.0).len() <= RULER_MAX_MARKS);
    }

    /// The rulers as one frame of `a` drew them: each bar (the grey strip over the page) and the
    /// numbers along it, as (text, centre x) across and (text, centre y) down.
    #[derive(Default)]
    struct DrawnRulers {
        h_bar: Option<Rect>,
        v_bar: Option<Rect>,
        h_labels: Vec<(String, f32)>,
        v_labels: Vec<(String, f32)>,
    }

    fn drawn_rulers(ctx: &egui::Context, a: &mut WordApp, t: &mut f64) -> DrawnRulers {
        *t += 1.0 / 60.0;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
        let input = egui::RawInput { time: Some(*t), screen_rect: Some(screen), ..Default::default() };
        let out = ctx.run_ui(input, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        });
        let grey = Tokens::get(ctx).ruler_margin;
        let mut d = DrawnRulers::default();
        if let Some(area) = a.canvas.canvas_rect {
            let h = Rect::from_min_max(pos2(area.min.x, area.min.y - RULER), pos2(area.max.x, area.min.y));
            let v = Rect::from_min_max(pos2(area.min.x - RULER, area.min.y), pos2(area.min.x, area.max.y));
            for c in &out.shapes {
                match &c.shape {
                    // The bars: the widest grey rect across the top strip, the tallest down the
                    // left one (a table's markers are grey too, but small).
                    egui::Shape::Rect(rs) if rs.fill == grey => {
                        let r = rs.rect;
                        if r.min.y >= h.min.y && r.max.y <= h.max.y && d.h_bar.is_none_or(|b| b.width() < r.width()) {
                            d.h_bar = Some(r);
                        }
                        if r.min.x >= v.min.x && r.max.x <= v.max.x && d.v_bar.is_none_or(|b| b.height() < r.height()) {
                            d.v_bar = Some(r);
                        }
                    }
                    egui::Shape::Text(text) => {
                        let centre = text.pos + text.galley.size() / 2.0;
                        if text.pos.y >= h.min.y && text.pos.y < h.max.y && text.pos.x >= h.min.x {
                            d.h_labels.push((text.galley.text().to_owned(), centre.x));
                        } else if text.pos.x >= v.min.x && text.pos.x < v.max.x && text.pos.y >= v.min.y && text.pos.y < v.max.y {
                            d.v_labels.push((text.galley.text().to_owned(), centre.y));
                        }
                    }
                    _ => {}
                }
            }
        }
        out.drop_without_applying_deltas();
        d
    }

    /// #1878: in Multiple Pages both rulers belong to the caret's page: the horizontal one spans
    /// that page's rect on screen and the vertical one its height, and they follow the caret to
    /// another page of the row and to the next row.
    #[test]
    fn the_rulers_span_the_caret_page_in_multiple_pages() {
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "One"})).unwrap();
        for text in ["Two", "Three", "Four"] {
            a.run("insert.pageBreak", json!({})).unwrap();
            a.run("text.insert", json!({"text": text})).unwrap();
        }
        a.run("edit.goto", json!({"page": 1})).unwrap();
        a.run("view.multiplePages", json!({})).unwrap();
        let mut t = 0.0;
        let frames = |a: &mut WordApp, t: &mut f64| {
            for _ in 0..4 {
                drawn_rulers(&ctx, a, t);
            }
            drawn_rulers(&ctx, a, t)
        };
        let d = frames(&mut a, &mut t);
        let rects = a.canvas.page_rects.clone();
        assert!(rects.len() == 4 && a.canvas.cols >= 2, "pages side by side: {} per row of {}", a.canvas.cols, rects.len());
        let same_x = |bar: Option<Rect>, pr: Rect| bar.is_some_and(|b| (b.min.x - pr.min.x).abs() < 0.5 && (b.max.x - pr.max.x).abs() < 0.5);
        let same_y = |bar: Option<Rect>, pr: Rect| bar.is_some_and(|b| (b.min.y - pr.min.y).abs() < 0.5 && (b.max.y - pr.max.y).abs() < 0.5);
        assert!(same_x(d.h_bar, rects[0]), "the horizontal ruler spans page 1: {:?} vs {:?}", d.h_bar, rects[0]);
        assert!(same_y(d.v_bar, rects[0]), "the vertical ruler spans page 1: {:?} vs {:?}", d.v_bar, rects[0]);
        assert!(!same_x(d.h_bar, rects[1]), "pages 1 and 2 sit side by side");
        // The caret moves to the second page of the row: the horizontal ruler follows it there.
        a.run("edit.goto", json!({"page": 2})).unwrap();
        let d = frames(&mut a, &mut t);
        let rects = a.canvas.page_rects.clone();
        assert!(same_x(d.h_bar, rects[1]), "the horizontal ruler follows the caret to page 2: {:?} vs {:?}", d.h_bar, rects[1]);
        assert!(same_y(d.v_bar, rects[1]));
        // And down to the next row: the vertical ruler moves to that page's height.
        let below = (0..rects.len()).find(|i| rects[*i].min.y > rects[0].min.y + 1.0).expect("a second row of pages");
        a.run("edit.goto", json!({"page": below + 1})).unwrap();
        let d = frames(&mut a, &mut t);
        let rects = a.canvas.page_rects.clone();
        assert!(same_y(d.v_bar, rects[below]), "the vertical ruler spans page {}: {:?} vs {:?}", below + 1, d.v_bar, rects[below]);
        assert!(same_x(d.h_bar, rects[below]));
        assert!(!same_y(d.v_bar, rects[0]), "page 1 is in the row above");
    }

    /// #1878: the rulers number the text area only, a number gives way to an indent marker on
    /// it, and zoomed far out the numbers thin out instead of piling up.
    #[test]
    fn ruler_numbers_skip_the_margins_and_the_markers_and_thin_out() {
        let ctx = egui::Context::default();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("view.zoom", json!({"value": 100})).unwrap();
        let mut t = 0.0;
        let frames = |a: &mut WordApp, t: &mut f64| {
            for _ in 0..4 {
                drawn_rulers(&ctx, a, t);
            }
            drawn_rulers(&ctx, a, t)
        };
        let names = |labels: &[(String, f32)]| labels.iter().map(|(s, _)| s.clone()).collect::<Vec<String>>();
        // Inches at 100% on a Letter page with inch margins: 1 to 6 across the text, 1 to 9 down
        // it; the page edges an inch outside the margins get no "1", "7" or "10".
        let d = frames(&mut a, &mut t);
        assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5", "6"]);
        let down = names(&d.v_labels);
        assert!(down.len() >= 6 && down[..6] == ["1", "2", "3", "4", "5", "6"], "down the page to the bottom of the view: {down:?}");
        let pr = a.canvas.page_rects[0];
        let scale = pr.width() / 612.0;
        assert!((d.v_labels[0].1 - (pr.min.y + 144.0 * scale)).abs() < 1.0, "the first number is an inch below the top margin");
        // A left indent of an inch puts the indent markers on the "1", which gives way.
        a.run("para.indents", json!({"left": 72.0})).unwrap();
        let d = frames(&mut a, &mut t);
        assert_eq!(names(&d.h_labels), ["2", "3", "4", "5", "6"]);
        a.run("para.indents", json!({"left": 0.0})).unwrap();
        let d = frames(&mut a, &mut t);
        assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5", "6"]);
        // At 10% an inch is 9.6 px: a number every 4 inches, the text keeps its size.
        a.run("view.zoom", json!({"value": 10})).unwrap();
        let d = frames(&mut a, &mut t);
        assert_eq!(names(&d.h_labels), ["4"]);
        assert_eq!(names(&d.v_labels), ["4", "8"]);
        let (y4, y8) = (d.v_labels[0].1, d.v_labels[1].1);
        assert!(y8 - y4 >= RULER_LABEL_PX, "the numbers stay apart: {y4} and {y8}");
    }

    /// #1878: a number also gives way to a tab stop's glyph (set, or being dragged to it) and to
    /// a table's column and row markers sitting on it.
    #[test]
    fn ruler_numbers_give_way_to_tab_stops_and_table_markers() {
        use crate::ruler_tabs::TabDrag;
        use wordcraft_doc::props::{TabAlign, TabLeader};
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let frames = |a: &mut WordApp, t: &mut f64| {
            for _ in 0..4 {
                drawn_rulers(&ctx, a, t);
            }
            drawn_rulers(&ctx, a, t)
        };
        let names = |labels: &[(String, f32)]| labels.iter().map(|(s, _)| s.clone()).collect::<Vec<String>>();
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("text.insert", json!({"text": "Hello"})).unwrap();
        a.run("view.zoom", json!({"value": 100})).unwrap();
        assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
        // A tab stop an inch in sits on the "1".
        a.run("para.tabs", json!({"tabs": [{"pos": 72.0}]})).unwrap();
        assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["2", "3", "4", "5", "6"]);
        // Dragged to two inches it counts where it lands: the "1" is back and the "2" gives way.
        let para = a.session.sel.focus.clone();
        let drag =
            |at: f32, off: bool| Some(TabDrag { from: Some(72.0), para: para.clone(), align: TabAlign::Left, leader: TabLeader::None, at, off });
        a.canvas.tab_drag = drag(144.0, false);
        assert_eq!(names(&drawn_rulers(&ctx, &mut a, &mut t).h_labels), ["1", "3", "4", "5", "6"]);
        // Off the ruler it counts nowhere.
        a.canvas.tab_drag = drag(144.0, true);
        assert_eq!(names(&drawn_rulers(&ctx, &mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
        a.run("para.tabs", json!({"clearAll": true})).unwrap();
        assert_eq!(names(&frames(&mut a, &mut t).h_labels), ["1", "2", "3", "4", "5", "6"]);
        // A three-column table with the caret in the first cell. Narrowing that column puts the
        // table's right border six inches from the margin: its marker takes the "6", while the
        // caret cell's own edge (and so its indent marker) ends well short of any whole inch.
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.table", json!({"rows": 1, "cols": 3})).unwrap();
        a.run("text.insert", json!({"text": "A"})).unwrap();
        a.run("view.zoom", json!({"value": 100})).unwrap();
        let cell = |a: &mut WordApp, c: usize| {
            a.session.layout().pages[0]
                .items
                .iter()
                .find_map(|it| match it {
                    wordcraft_layout::Placed::Cell { rect, row: 0, cell, .. } if *cell == c => Some(*rect),
                    _ => None,
                })
                .unwrap()
        };
        let body = a.session.layout().pages[0].body;
        let (first, last) = (cell(&mut a, 0), cell(&mut a, 2));
        let width = body.x + 432.0 - first.x - (last.right() - first.right());
        a.run("table.columnWidth", json!({"width": width})).unwrap();
        let d = frames(&mut a, &mut t);
        let (first, last) = (cell(&mut a, 0), cell(&mut a, 2));
        assert!((last.right() - (body.x + 432.0)).abs() < 0.5, "the table ends six inches in: {last:?}");
        let off_inch = (first.right() - body.x).rem_euclid(72.0);
        assert!((12.0..=60.0).contains(&off_inch), "the caret cell's edge is off the whole inches: {first:?}");
        assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5"]);
        let down = names(&d.v_labels);
        assert!(down.len() >= 3 && down[..3] == ["1", "2", "3"], "{down:?}");
        // The row's bottom an inch below the top margin takes the "1" down the page.
        let height = body.y + 72.0 - first.y;
        a.run("table.rowHeight", json!({"height": height, "rule": "exact"})).unwrap();
        let d = frames(&mut a, &mut t);
        assert!((cell(&mut a, 0).bottom() - (body.y + 72.0)).abs() < 0.5);
        let down = names(&d.v_labels);
        assert!(down.len() >= 3 && down[..3] == ["2", "3", "4"], "{down:?}");
        assert_eq!(names(&d.h_labels), ["1", "2", "3", "4", "5"]);
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
        // The drawn I-beam takes the canvas colour in the gap and on a cut margin (#1746).
        assert!(on_visible_page(&moved, &bands, pos2(moved[0].center().x, seam - 40.0)));
        assert!(!on_visible_page(&moved, &bands, pos2(moved[0].center().x, seam + 0.5)));
        assert!(!on_visible_page(&moved, &bands, pos2(moved[0].min.x - 5.0, moved[0].center().y)));
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
        let mut pointer_interrupted = false;
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
            pointer_interrupted |= ime_interrupted(&h) == Some(true);
        }
        assert!(pointer_interrupted, "a pointer click still interrupts composition");
    }

    /// Kittest's `MissingGlyphPolicy::Panic` is right: a missing glyph is a bug. But the interface
    /// fonts need not carry Japanese (craft-fonts is optional, and a CI Linux runner has no
    /// Japanese system font), and these IME tests compose Japanese through the interface font
    /// chain. Draw a stand-in for exactly those characters; every other missing glyph still panics.
    fn kana_test_glyphs(ctx: &egui::Context) {
        ctx.add_glyph_rasterizer(egui::GlyphRasterizer::new("wordcraft::canvas::tests::kana", |req| {
            let kana = *req.family == egui::FontFamily::Proportional
                && !req.cluster.is_empty()
                && req.cluster.chars().all(|c| matches!(c, 'に' | 'ほ' | 'ん'));
            kana.then(|| egui::RasterizedGlyph {
                bitmap: egui::GlyphBitmap {
                    image: egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                    offset_px: egui::vec2(0.0, 0.0),
                    is_color: false,
                },
                advance_px: (req.font_size_px * 0.5).max(1.0),
            })
        }));
    }

    /// The fixture above must work with no Japanese font anywhere: only a bundled Latin face, the
    /// strict missing-glyph policy, and the kana stand-in.
    #[test]
    fn the_ime_fixture_draws_kana_without_a_japanese_font() {
        let ctx = egui::Context::default();
        ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
        let mut fonts = egui::FontDefinitions::empty();
        fonts
            .font_data
            .insert("Latin".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"))));
        fonts.families.insert(egui::FontFamily::Proportional, vec!["Latin".into()]);
        ctx.set_fonts(fonts);
        kana_test_glyphs(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = ui.painter().layout_no_wrap("にほん".into(), crate::theme::regular(14.0), egui::Color32::WHITE);
            assert_eq!(g.rows.first().map_or(0, |r| r.row.glyphs.len()), 3, "one glyph per composed kana");
        });
        output.textures_delta.clear();
    }

    /// While an input method composes, the caret and the candidate window sit at its cursor inside
    /// the composition: the end of the range it reports, or the end of the text with no range. The
    /// range comes from the OS and is clamped, whatever it holds.
    #[test]
    fn the_caret_follows_the_input_method_cursor_in_the_composition() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("document.setText", json!({"text": "The"})).unwrap();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_context_setup(kana_test_glyphs)
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
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
        let ime_x = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|ime| ime.cursor_rect.min.x);
        let start = ime_x(&h).expect("the canvas places the candidate window");
        let mut compose = |range: Option<std::ops::Range<usize>>| {
            h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: range }));
            h.step();
            h.step();
            ime_x(&h).expect("the canvas places the candidate window")
        };
        let before = compose(Some(0..0));
        assert_eq!(before, start, "a cursor at the composition's start is the caret before composing");
        let end = compose(None);
        let mid = compose(Some(1..1));
        assert!(end > mid && mid > before, "the cursor moves through the composition: {before} {mid} {end}");
        assert_eq!(compose(Some(0..99)), end, "a range past the text ends at its end");
        assert_eq!(compose(Some(std::ops::Range { start: 5, end: 1 })), mid, "a reversed range uses its end");
        assert_eq!(compose(Some(usize::MAX..usize::MAX)), end);
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("にほん".into())));
        h.step();
        assert_eq!(h.state().canvas.ime_preedit_range, None, "the commit clears the range");
        assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("にほん"));
    }

    /// The outgoing IME descriptor is built from settled, visible logical text. A pre-edit does
    /// not change the model; an accepted commit alone earns `InputMethod`; ordinary typing earns
    /// `Other`. The commit mixes Korean, Japanese, ASCII and an emoji.
    #[test]
    fn ime_descriptor_reports_visible_logical_text_and_exact_cause() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("ab")), Default::default());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_context_setup(kana_test_glyphs)
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
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
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 2));
        h.step();
        let text = |h: &egui_kittest::Harness<'_, WordApp>| {
            h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).map(|p| p.text.clone())
        };
        let cause = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
        assert_eq!(text(&h), Some("ab".into()), "the committed text is available");
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
        h.step();
        assert_eq!(text(&h), Some("ab".into()), "the pre-edit never enters the committed model");
        assert_eq!(cause(&h), Some(ImeChangeCause::Other), "a pre-edit is no mutation");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("한日本A😀".into())));
        h.step();
        assert!(text(&h).unwrap_or_default().contains("한日本A😀"), "the commit is in the visible text");
        assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted commit is the IME's change");
        h.event(egui::Event::Text("x".into()));
        h.step();
        assert_eq!(cause(&h), Some(ImeChangeCause::Other), "ordinary typing is an application change");
    }

    /// Tracked deletions never go to the input method. The caret may straddle one and the
    /// selection direction survives the visible mapping.
    #[test]
    fn ime_descriptor_hides_tracked_deletions_and_preserves_reversed_selection() {
        use wordcraft_doc::props::CharProps;
        use wordcraft_doc::{Paragraph, Run, para_block};
        let mut p = Paragraph::new();
        p.text = "abDELcd".into();
        p.runs = vec![
            Run { len: 2, props: CharProps::default() },
            Run { len: 3, props: CharProps { del: Some(0), ..Default::default() } },
            Run { len: 2, props: CharProps::default() },
        ];
        let mut doc = wordcraft_doc::Document::from_text("x");
        doc.body = vec![para_block(p)];
        let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 2), focus: wordcraft_doc::Pos::body(0, 7) };
        h.step();
        let payload =
            |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().and_then(|i| i.surrounding_text.available()).cloned();
        let p = payload(&h).expect("the visible text is available");
        assert_eq!(p.text, "abcd");
        assert_eq!(p.cursor, 4);
        assert_eq!(p.anchor, 2, "the reversed selection stays reversed");
    }

    /// Byte deletion keeps the selection out of both erased sides, then the commit replaces it;
    /// both accepted mutations are `InputMethod` receipts over bounded snapshots.
    #[test]
    fn ime_byte_deletion_preserves_the_selection_and_earns_input_method() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("abcd")), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 3), focus: wordcraft_doc::Pos::body(0, 1) };
        h.step();
        let cause = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.change_cause);
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 1 }));
        h.step();
        let body = |h: &egui_kittest::Harness<'_, WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
        assert_eq!(body(&h), "bc", "the selected text is preserved");
        assert_eq!(h.state().session.sel.anchor.off, 2, "the selection anchor moved with the erased prefix");
        assert_eq!(h.state().session.sel.focus.off, 0);
        assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "an accepted deletion is the IME's change");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("X".into())));
        h.step();
        assert_eq!(body(&h), "X", "the commit replaces the preserved selection");
        assert_eq!(cause(&h), Some(ImeChangeCause::InputMethod), "delete plus commit chain into one IME change");
    }

    /// A selection wider than the protocol limit is honestly unavailable, not truncated.
    #[test]
    fn ime_descriptor_is_unavailable_for_a_too_wide_selection() {
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 0), focus: wordcraft_doc::Pos::body(0, 5000) };
        h.step();
        let ime = h.output().platform_output.ime.clone().expect("the canvas owns the IME context");
        assert!(ime.surrounding_text.available().is_none(), "a 5000-byte selection is unavailable");
    }

    /// Replacing the document starts a new IME context; caret motion inside it does not.
    #[test]
    fn ime_generation_changes_on_document_replacement_but_not_on_caret_motion() {
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
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
        let generation = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
        let first = generation(&h).expect("an IME context");
        h.state_mut().run("caret.right", json!({})).unwrap();
        h.step();
        assert_eq!(generation(&h), Some(first), "caret motion keeps the context");
        h.state_mut().session.set_document(wordcraft_doc::Document::from_text("new"));
        h.step();
        assert!(generation(&h) > Some(first), "a replaced document starts a new context");
    }

    /// An equation's structured tree has no reversible surrounding text: report `Unavailable`
    /// while keeping the stable editing-session context and candidate geometry through caret
    /// motion. A real content commit is proved by the engine's checked revision and earns
    /// `InputMethod`.
    #[test]
    fn ime_equation_is_unavailable_with_a_stable_session_context() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
        assert!(app.session.math.is_some(), "the equation is being edited");
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
        let ime = h.output().platform_output.ime.clone().expect("the equation places its candidate window");
        assert!(ime.surrounding_text.available().is_none(), "no fabricated equation surroundings");
        let generation = ime.context_generation;
        h.state_mut().run("equation.move", json!({"dir": "left"})).unwrap();
        h.step();
        let moved = h.output().platform_output.ime.clone().expect("still editing");
        assert_eq!(moved.context_generation, generation, "caret motion keeps the equation's IME context");
        assert!(moved.surrounding_text.available().is_none());
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
        h.step();
        let committed = h.output().platform_output.ime.clone().expect("still editing");
        assert_eq!(committed.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the content commit");
        assert!(committed.surrounding_text.available().is_none());
    }

    /// A transformed equation commit (build-up or Math AutoCorrect) is `Other`: the engine's
    /// per-command outcome marks it transformed even though the revision moved and content
    /// changed. The equation really built up; no InputMethod receipt is fabricated.
    #[test]
    fn ime_transformed_equation_commits_are_other() {
        use egui::ImeChangeCause;
        for (typed, latex) in [("a/b ", "\\frac{a}{b}"), ("\\alpha ", "\\alpha")] {
            let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
            app.run("insert.equation", json!({})).unwrap();
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
            h.event(egui::Event::Ime(egui::ImeEvent::Commit(typed.into())));
            h.step();
            let info = h.state_mut().run("equation.get", json!({})).unwrap();
            assert_eq!(info["latex"].as_str(), Some(latex), "the commit was transformed: {typed:?}");
            let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
            assert_eq!(ime.change_cause, ImeChangeCause::Other, "an automatic equation rewrite is not the IME's own edit: {typed:?}");
        }
    }

    /// A literal IME commit after an earlier AutoCorrect is still the IME's own change. The
    /// stale `"AutoFormat"` undo label left by that earlier step must not classify this command.
    #[test]
    fn ime_literal_commit_after_autoformat_is_input_method() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        for text in ["Word", " ", "(c)", " "] {
            app.run("text.insert", json!({"text": text})).unwrap();
        }
        assert_eq!(app.session.undo_label(), Some("AutoFormat"), "the earlier AutoCorrect left its undo label");
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
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "Word \u{a9} x");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the literal commit is not the earlier AutoFormat");
    }

    /// A real allocator exhaustion (not a hand-built stamp) keeps the engine revision absent;
    /// an equation commit and a wide body deletion still happen but can never certify a receipt.
    #[test]
    fn exhausted_ime_revisions_never_certify_equation_or_wide_body_edits() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x"})).unwrap();
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
        h.state_mut().session.exhaust_ime_state_revision();
        h.step();
        assert_eq!(h.state_mut().session.ime_state_revision(), None, "the allocator is exhausted");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
        h.step();
        let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
        assert_eq!(linear, "xy", "the real commit still happened");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "without a revision the change cannot be certified");

        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
        h.step();
        h.state_mut().session.exhaust_ime_state_revision();
        h.step();
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
        h.step();
        let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert_eq!(body.matches('a').count(), 4999, "the wide body delete still happened");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "the exhausted revision cannot prove the delete");
    }

    /// A body replaced under the same document id starts a new IME context and interrupts the
    /// live composition, even when the replacement text (and so the bounded window) is identical.
    #[test]
    fn ime_generation_changes_on_body_replacement_with_the_same_text() {
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_context_setup(kana_test_glyphs)
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
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
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit { text: "に".into(), active_range_chars: None }));
        h.step();
        assert_eq!(h.state().canvas.ime_preedit.as_str(), "に");
        let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
        h.state_mut().run("document.setText", json!({"text": "hello"})).unwrap();
        h.step();
        let ime = h.output().platform_output.ime.clone().expect("the canvas still owns the IME context");
        assert!(ime.context_generation > first, "a replaced body starts a new context");
        assert!(ime.should_interrupt_composition, "the old composition is interrupted");
        assert!(h.state().canvas.ime_preedit.is_empty(), "the old pre-edit is not kept");
        assert!(ime.surrounding_text.available().is_some(), "the new body still has surroundings");
    }

    /// Undo (and a version restore) puts a whole body back: the IME generation follows it too.
    #[test]
    fn ime_generation_changes_on_undo_and_version_restore() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("one")), Default::default());
        app.run("file.versions", json!({"save": "v1"})).unwrap();
        app.run("document.setText", json!({"text": "two"})).unwrap();
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
        let generation = |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.context_generation);
        let first = generation(&h).expect("an IME context");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.step();
        assert!(generation(&h) > Some(first), "Undo swaps the body back");
        let after_undo = generation(&h);
        h.state_mut().run("file.versions", json!({"restore": 0})).unwrap();
        h.step();
        assert!(generation(&h) > after_undo, "a version restore replaces the body");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "one");
    }

    /// With smart quotes off the engine inserts the quote as typed: that is the input method's
    /// own edit and earns an `InputMethod` receipt (it used to be forced to `Other`).
    #[test]
    fn ime_apostrophe_with_smart_quotes_off_is_an_input_method_change() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("tools.autocorrect", json!({"smartQuotes": false})).unwrap();
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
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "'", "the quote is not curled");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine inserted it unchanged");
    }

    /// With smart quotes on, the same commit is AutoFormat work and stays `Other` (the existing
    /// contract), so the fix distinguishes the two instead of always assuming a transformation.
    #[test]
    fn ime_apostrophe_with_smart_quotes_on_stays_other() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        // Smart quotes are on by default.
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
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("'".into())));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "\u{2018}", "curled");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "AutoFormat changed the text");
    }

    /// A real IME-only literal commit is the input method's own edit.
    #[test]
    fn ime_literal_commit_is_input_method() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
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
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("literal".into())));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "literal");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
    }

    /// A formatting change before an IME commit in the same frame must force `Other`, even
    /// though an available body stamp cannot see the style change.
    #[test]
    fn para_alignment_before_ime_commit_is_other() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
        h.step();
        h.state_mut().run("para.alignCenter", json!({})).unwrap();
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
        h.step();
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
        assert_eq!(
            h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
            Some(wordcraft_doc::props::Align::Center),
            "the application command really ran"
        );
    }

    /// A formatting change after an accepted IME commit in the same frame must also force
    /// `Other`: the pending application signal survives the receipt and is drained at observation.
    #[test]
    fn para_alignment_after_ime_commit_is_other() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("hello")), Default::default());
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
        h.state_mut().session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 5));
        h.step();
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
        h.event(egui::Event::Key {
            key: egui::Key::E,
            physical_key: Some(egui::Key::E),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        h.step();
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "hellox");
        assert_eq!(
            h.state().session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).and_then(|p| p.props.align),
            Some(wordcraft_doc::props::Align::Center),
            "the application command really ran"
        );
    }

    /// A selected non-empty SmartArt graphic consumes an IME commit as an application edit: its
    /// item changes, the body does not, and the frame is `Other`.
    #[test]
    fn smart_art_item_ime_commit_is_other_and_does_not_reach_the_body() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
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
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
        h.step();
        let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end().to_string();
        assert_eq!(body, "", "the commit must not fall through into the body");
        let item = wordcraft_engine::cmd::smart_art::selected_smart_art(&h.state().session).and_then(|g| g.items.first().map(|i| i.text.clone()));
        assert_eq!(item.as_deref(), Some("Ax"), "the SmartArt item really changed");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other);
    }

    /// A selected graphic with no items consumes the commit without editing anything. The body
    /// stays empty and no InputMethod receipt is fabricated.
    #[test]
    fn empty_smart_art_ime_commit_is_consumed_without_body_insertion() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.smartArt", json!({"items": ["A"]})).unwrap();
        // `insert.smartArt` refuses an empty item list, but an imported/edited model may hold
        // none: make the selected graphic empty while keeping its identity.
        let at = app.session.sel.ordered().0;
        if let Some(wordcraft_doc::InlineObject::Graphic { graphic, .. }) =
            app.session.doc.para_mut(at.story, &at.path).expect("paragraph").object_at_mut(at.off)
            && let Some(spec) = std::sync::Arc::make_mut(graphic).smart_art.as_mut()
        {
            std::sync::Arc::make_mut(spec).items.clear();
        }
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
        let depth = h.state().session.undo_depth();
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("x".into())));
        h.step();
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), "", "no body insertion");
        assert_eq!(h.state().session.undo_depth(), depth, "consuming an empty graphic changes no document");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_eq!(ime.change_cause, ImeChangeCause::Other, "no receipt and no application edit");
    }

    /// A wide selection's surrounding text is unavailable; deleting inside it still earns
    /// `InputMethod` when both engine revisions are present and strictly increase.
    #[test]
    fn ime_wide_unavailable_deletion_uses_engine_revisions() {
        use egui::ImeChangeCause;
        let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(&"a".repeat(5000))), Default::default());
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
        // A wide selection with one character before it: the counted deletion is outside the
        // selection, so the selection stays wide (unavailable) after it.
        h.state_mut().session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos::body(0, 1), focus: wordcraft_doc::Pos::body(0, 5000) };
        h.step();
        let before = h.output().platform_output.ime.clone().expect("an IME context");
        assert!(before.surrounding_text.available().is_none(), "the wide selection is unavailable");
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
        h.step();
        let body = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert_eq!(body.matches('a').count(), 4999);
        let after = h.output().platform_output.ime.clone().expect("an IME context");
        assert!(after.surrounding_text.available().is_none(), "the selection is still wide after the deletion");
        assert_eq!(after.change_cause, ImeChangeCause::InputMethod, "the engine revisions prove the change");
    }

    /// A transactional equation delete that cannot make progress is refused as a whole and
    /// interrupts the composition without a receipt.
    #[test]
    fn ime_equation_delete_at_start_is_refused_without_a_receipt() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
        app.run("equation.move", json!({"dir": "home"})).unwrap();
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
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
        h.step();
        let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
        assert_eq!(linear, "x+y", "nothing is deleted");
        let ime = h.output().platform_output.ime.clone().expect("an IME context");
        assert_ne!(ime.change_cause, ImeChangeCause::InputMethod);
        assert!(ime.should_interrupt_composition, "the refused equation delete interrupts the composition");
    }

    /// A byte-counted replacement inside an equation is refused as a whole: nothing is deleted,
    /// only the immediately paired stale `Text`/`Commit` is dropped and the composition is
    /// interrupted. A later direct commit in the same raw batch or a later frame is fresh input
    /// and applies, so the adjacent-pair bound never swallows it.
    #[test]
    fn ime_equation_byte_delete_drops_only_its_immediate_replacement() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
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
        let linear = |h: &mut egui_kittest::Harness<'_, WordApp>| {
            h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string()
        };
        // Harness::event steps each queued event in a separate frame. Insert the complete
        // protocol replacement into RawInput to exercise a single native input batch.
        h.input_mut().events.extend([
            egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
            egui::Event::Ime(egui::ImeEvent::Commit("Z".into())),
            egui::Event::Ime(egui::ImeEvent::Commit("W".into())),
        ]);
        h.step();
        assert_eq!(linear(&mut h), "x+yW", "the adjacent stale commit is dropped and the next direct commit applies");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh direct commit is the input method's change");
        assert!(ime.should_interrupt_composition, "the refused byte deletion still interrupts the stale composition");

        // An adjacent `Text` + `Commit` pair is one replacement; a third commit is fresh.
        let base = linear(&mut h);
        h.input_mut().events.extend([
            egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }),
            egui::Event::Text("Q".into()),
            egui::Event::Ime(egui::ImeEvent::Commit("Q2".into())),
            egui::Event::Ime(egui::ImeEvent::Commit("R".into())),
        ]);
        h.step();
        assert_eq!(linear(&mut h), format!("{base}R"), "the adjacent pair is dropped, not every later commit");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod);
        assert!(ime.should_interrupt_composition);

        // A refused deletion with no replacement in its batch must not swallow a later direct
        // commit, even with no pre-edit before it.
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }));
        h.step();
        let base = linear(&mut h);
        assert_eq!(linear(&mut h), "x+yWR", "the refused deletion alone changed nothing");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("Y".into())));
        h.step();
        assert_eq!(linear(&mut h), format!("{base}Y"), "a later direct commit applies without a pre-edit");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the fresh commit is the input method's change");
        assert!(!ime.should_interrupt_composition, "the fresh commit is not treated as stale");
    }

    /// A satisfied transactional equation delete commits content and earns one `InputMethod`
    /// receipt over the checked unavailable revisions.
    #[test]
    fn ime_equation_delete_is_one_input_method_change() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
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
        h.event(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }));
        h.step();
        let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
        assert_eq!(linear, "x+");
        let ime = h.output().platform_output.ime.clone().expect("still editing the equation");
        assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the engine revision proves the delete");
    }

    /// An application change in an equation context and an IME commit in the same frame are
    /// `Other` in both orders, just as for body text.
    #[test]
    fn ime_equation_context_mixes_with_application_changes() {
        use egui::ImeChangeCause;
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x"})).unwrap();
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
        h.state_mut().run("equation.justify", json!({"jc": "left"})).unwrap();
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("y".into())));
        h.step();
        let before = h.output().platform_output.ime.clone().expect("still editing");
        assert_eq!(before.change_cause, ImeChangeCause::Other, "an application change before the receipt poisons the frame");
        let linear = h.state_mut().run("equation.get", json!({})).unwrap()["linear"].as_str().unwrap_or_default().to_string();
        assert_eq!(linear, "xy");

        h.event(egui::Event::Ime(egui::ImeEvent::Commit("z".into())));
        h.step();
        let accepted = h.output().platform_output.ime.clone().expect("still editing");
        assert_eq!(accepted.change_cause, ImeChangeCause::InputMethod);
        h.state_mut().run("equation.justify", json!({"jc": "right"})).unwrap();
        h.step();
        let after = h.output().platform_output.ime.clone().expect("still editing");
        assert_eq!(after.change_cause, ImeChangeCause::Other, "an application change after the receipt poisons the frame");
    }

    /// Replacing the edited equation under the same document id gives it a fresh session token:
    /// the IME context changes and the old composition is interrupted.
    #[test]
    fn ime_equation_replacement_starts_a_new_context() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("insert.equation", json!({"linear": "x+y"})).unwrap();
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
        let first = h.output().platform_output.ime.as_ref().map(|i| i.context_generation).expect("an IME context");
        h.state_mut().run("insert.equation", json!({"linear": "z"})).unwrap();
        h.step();
        let ime = h.output().platform_output.ime.clone().expect("the new equation owns the context");
        assert!(ime.context_generation > first, "a new equation editing session is a new context");
        assert!(ime.should_interrupt_composition, "the old equation composition is interrupted");
        assert!(h.state().session.math.is_some(), "the new equation is being edited");
    }

    #[test]
    fn context_menu_crop_item_activates_crop_mode() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::para::{InlineObject, OBJ};
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let pic =
            InlineObject::Image { media: "m".into(), w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
        let end = Pos::body(0, OBJ.len_utf8());
        app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..6 {
            h.step();
        }
        let (page, rect) = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).map(|o| (o.page, o.rect)).expect("laid out");
        let at = page_to_screen(h.state_mut(), page, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0).expect("on screen");
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() });
            h.step();
        }
        h.step();
        assert!(h.state().canvas.context_menu_open, "context menu is open on image");
        h.get_by_label("Crop ").click();
        for _ in 0..3 {
            h.step();
        }
        assert!(crate::crop::active(h.state()).is_some(), "crop mode is active after clicking Crop in context menu");
    }

    #[test]
    fn crop_is_disabled_when_cursor_is_adjacent_without_selection() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        let pic = wordcraft_doc::para::InlineObject::Image {
            media: "m".into(),
            w: 200.0,
            h: 100.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        app.session.doc.insert_object(&Pos::body(0, 0), pic, &Default::default()).unwrap();
        // Caret is collapsed right at pos 0 (adjacent to the picture).
        app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
        assert!(crate::ribbon::has_picture_selected(&app.session), "adjacent caret sees picture in ribbon");
        assert!(!crate::widgets::enabled(&app, "ui.crop"), "ui.crop is disabled when not selected as object");
        assert!(crate::crop::active(&app).is_none());
    }

    fn page_at_index(rects: &[Rect], p: Pos2) -> Option<usize> {
        nearest_page(rects, p)
    }

    /// A page drawn without a dragged object must not pass for the page with it: the drag's start
    /// and end change the page's texture key once each, so the page is rendered again then and
    /// only then. Without this, the old raster — the object still on it where it was — stays while
    /// the preview follows the pointer, and the object shows twice.
    #[test]
    fn the_view_key_changes_when_a_drag_takes_an_object_off_the_page() {
        let doc = wordcraft_doc::Document::from_text("Hello WordCraft");
        let app = WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
        let object = Pos::body(0, 5);
        let hidden = view_key(&app, false, Some(&object));
        assert_ne!(hidden, view_key(&app, false, None), "the page without the dragged object is rendered again");
        assert_eq!(hidden, view_key(&app, false, Some(&object)), "the same drag keeps the key: no re-render every frame");
    }
}
