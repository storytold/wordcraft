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

mod balloons;
mod context_menu;
mod cursor;
mod geometry;
mod input;
mod links;
mod margins;
mod navigation;
mod pages;
mod reading;
mod rulers;
mod story;

#[cfg(test)]
use balloons::balloon_text;
use balloons::{balloons, dashed, over_balloon};
pub use balloons::{deselect_balloon, select_balloon};
use context_menu::{context_menu, context_suggestions};
use cursor::{caret_color_at, preedit_cursor_chars};
pub(crate) use cursor::{caret_color_on, caret_contrast};
pub use cursor::{caret_screen, pos_from_screen};
pub(crate) use geometry::SEAM;
#[cfg(test)]
pub(crate) use geometry::WHITE_SPACE_PAD;
pub use geometry::{Geometry, geometry, markup_width, sync_fit_zoom};
#[cfg(test)]
use geometry::{collapse_white_space, columns_for, place_pages};
use geometry::{floating_comments, on_page_gap, on_visible_page, white_space_hidden};
#[cfg(test)]
use input::{EDGE_MARGIN, EDGE_MAX_DT, EDGE_SPEED_MAX};
use input::{UnitDrag, clamped_scroll, edge_scroll, mouse, wheel_hovered};
pub(crate) use input::{hold_document_selection, press_count};
#[cfg(test)]
use links::{field_link, is_followable_link, is_generated_bookmark_link};
#[cfg(test)]
use margins::MIN_TEXT;
#[cfg(test)]
pub(crate) use margins::{MarginEdge, dragged_margin, margin_drag};
#[cfg(test)]
use navigation::top_page;
pub use navigation::{nearest_page, page_at, page_to_screen};
use navigation::{scrollbar_page_tip, viewed_page};
#[cfg(test)]
use pages::{DRAWING_GRID_MAX_LINES, DRAWING_GRID_MIN_PX};
pub(crate) use pages::{PageKeys, page_doc, page_screen_scale, preview_sel, screen_render_options};
use pages::{
    drawing_grid, drawing_grid_step, gridline_cells, page_image, page_key, page_render_options, shown_layout, texel_aligned_rect, view_key,
    visible_tiles,
};
pub(crate) use reading::turn_read_page;
#[cfg(test)]
use reading::{READ_NAV_BAR, RESIZE_SETTLE, place_read_pages};
use reading::{
    Settle, read_wheel_turns, reading_anchor_page, reading_animation_step, reading_page_start, reading_size, reading_viewport, web_layout_width,
};
use rulers::rulers;
#[cfg(test)]
use rulers::{RULER_LABEL_PX, RULER_MAX_MARKS, RULER_SPARSEST, RulerMark, UNIT_TICK, ruler_d, ruler_density, ruler_marks, ruler_x};
pub use story::{EquationFrame, editing_header_footer, in_text_box};
use story::{dims_body, equation_frame};
pub(crate) use story::{part_kind, text_box_host};

/// Points on screen per document point at 100% (96 px per inch, like a word processor).
pub const PX_PER_PT: f32 = 96.0 / 72.0;
const GAP: f32 = 18.0;
/// Width reserved for the vertical scrollbar beside the canvas.
const SCROLLBAR: f32 = 14.0;
const RULER: f32 = 22.0;
/// Maximum device-pixel height of a tile on very tall Web/Draft pages.
const TILE_HEIGHT_PX: usize = 1024;
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
    /// Explicit scroll position to apply on the next frame (reading navigation buttons/keys, a
    /// window moving into another viewport).
    pub(crate) scroll_request: Option<egui::Vec2>,
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
    /// Another document is shown in the window: everything about the page that was shown (drags,
    /// menus, the input method's composition, a scroll in progress, rendered pages…) starts
    /// afresh. What belongs to the window or the user stays: the colours last applied, the
    /// ruler's tab kind, the faces added for interface text, the page area's size. The scroll
    /// position comes back by itself, as the page's scroll area keeps one per document in egui's
    /// memory. `seen`: the document was shown before, so the view stays where it was rather than
    /// going to the caret.
    pub(crate) fn show_other_document(&mut self, seen: bool) {
        let old = std::mem::take(self);
        let ime_reset = !old.ime_preedit.is_empty() || old.ime_preedit_range.is_some();
        *self = CanvasState {
            last_highlight: old.last_highlight,
            last_font_color: old.last_font_color,
            last_shading: old.last_shading,
            ime_faces: old.ime_faces,
            tab_kind: old.tab_kind,
            scale: old.scale,
            canvas_rect: old.canvas_rect,
            read_viewport: old.read_viewport,
            web_viewport: old.web_viewport,
            scroll_to_caret: !seen,
            ime_reset,
            ..CanvasState::default()
        };
    }

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

/// Update the deferred viewport sizes and return the text anchor to restore after reflow.
fn update_viewport(app: &mut WordApp, ui: &Ui, full: Rect, now: f64) -> Option<Pos> {
    let mut reflow_anchor: Option<Pos> = None;
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
    reflow_anchor
}

struct CanvasView {
    area: Rect,
    hruler: Option<Rect>,
    vruler: Option<Rect>,
    dark_page: bool,
    sepia_page: bool,
    animate: bool,
    opts: egui::InputOptions,
    geo: Geometry,
    dim_body: bool,
    grid_story: Option<StoryRef>,
    caret: Option<wordcraft_layout::hit::Caret>,
}

fn prepare_view(app: &mut WordApp, ui: &Ui, full: Rect, layout: &DocLayout) -> CanvasView {
    use wordcraft_engine::cmd::view::ReadPageColor;
    let read_mode = app.session.view.read_mode;
    let dark_page = if read_mode { app.session.view.read_page_color == ReadPageColor::Inverse } else { app.session.view.dark_mode };
    let sepia_page = read_mode && app.session.view.read_page_color == ReadPageColor::Sepia;
    let show_ruler = app.session.view.ruler
        && app.session.view.mode == wordcraft_layout::ViewMode::Print
        && !read_mode
        && crate::chrome::Screen::of(&app.session.view) == crate::chrome::Screen::Normal;
    let (hruler, vruler, area) = if show_ruler {
        let h = Rect::from_min_max(pos2(full.min.x + RULER, full.min.y), pos2(full.max.x, full.min.y + RULER));
        let v = Rect::from_min_max(pos2(full.min.x, full.min.y + RULER), pos2(full.min.x + RULER, full.max.y));
        (Some(h), Some(v), Rect::from_min_max(pos2(full.min.x + RULER, full.min.y + RULER), full.max))
    } else {
        (None, None, full)
    };
    let area = if read_mode {
        let column = app.session.view.read_width * app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        reading_viewport(area, column, app.session.view.read_zoom.clamp(0.75, 1.5) * PX_PER_PT)
    } else {
        area
    };
    app.canvas.canvas_rect = Some(area);
    let animate = app.animations_on();
    let opts = ui.ctx().options(|o| o.input_options);
    let zoom_delta = ui.input(|i| app.canvas.wheel.zoom(i, &opts, animate));
    let mut zoom_anchor = None;
    if (zoom_delta - 1.0).abs() > 1e-3 && ui.rect_contains_pointer(area) {
        if !read_mode && let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
            zoom_anchor = page_at(&app.canvas.page_rects, layout, app.canvas.scale, p).map(|at| (p, at));
        }
        let pct =
            (app.canvas.scale / PX_PER_PT * zoom_delta * 100.0).clamp(if read_mode { 75.0 } else { 10.0 }, if read_mode { 150.0 } else { 500.0 });
        let id = if read_mode { "view.readTextSize" } else { "view.zoom" };
        app.canvas.wheel.cancel();
        let _ = app.run(id, json!({"value": pct}));
    }
    let avail = if read_mode { area.size() } else { area.size() - vec2(SCROLLBAR, 0.0) };
    let geo = geometry(app, layout, avail);
    if read_mode {
        app.canvas.scroll_max = (geo.size - area.size()).max(egui::Vec2::ZERO);
    }
    if (geo.scale - app.canvas.scale).abs() > 1e-6 {
        ui.ctx().request_repaint();
    }
    app.canvas.scale = geo.scale;
    sync_fit_zoom(app, geo.scale);
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
    if geo.cols != app.canvas.cols {
        app.canvas.cols = geo.cols;
        if !read_mode && !anchored {
            app.canvas.scroll_to_caret = true;
        }
    }
    let caret = layout.caret_on(&preview_sel(app).unwrap_or(app.session.sel_ends()).focus, app.session.page_hint);
    let dim_body = dims_body(app, layout);
    let grid_story = text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    let grid_story = matches!(part_kind(app, grid_story), Some(PartKind::Header | PartKind::Footer)).then_some(grid_story);
    if let Some(c) = caret
        && !app.session.selects_whole_story()
    {
        app.session.page_hint = c.page;
    }
    CanvasView { area, hruler, vruler, dark_page, sepia_page, animate, opts, geo, dim_body, grid_story, caret }
}

fn update_caret_scroll(app: &mut WordApp, view: &CanvasView) -> Option<Rect> {
    let mut target = None;
    if app.canvas.nav_to_caret {
        app.canvas.read_target_page = None;
        if let Some(c) = view.caret
            && let Some(pr) = view.geo.rects.get(c.page)
        {
            let x = (pr.left() - GAP).max(0.0);
            let y = if app.session.view.read_mode { 0.0 } else { (pr.top() + c.top * view.geo.scale - 40.0).max(0.0) };
            app.canvas.scroll_request = Some(vec2(x, y));
        }
        app.canvas.nav_to_caret = false;
    } else if app.canvas.scroll_to_caret && app.session.view.read_mode && app.canvas.scroll_request.is_none() {
        if let Some(c) = view.caret
            && let Some(pr) = view.geo.rects.get(c.page)
        {
            let x = pr.left() + c.x * view.geo.scale;
            let left = app.canvas.scroll_offset.x;
            if x < left || x > left + view.area.width() {
                app.canvas.read_target_page = None;
                app.canvas.scroll_request = Some(vec2((pr.left() - GAP).max(0.0), 0.0));
            }
        }
    } else if app.canvas.scroll_to_caret
        && let Some(c) = view.caret
        && let Some(pr) = view.geo.rects.get(c.page)
    {
        let r = Rect::from_min_size(
            pos2(pr.min.x + c.x * view.geo.scale, pr.min.y + c.top * view.geo.scale),
            vec2((c.width * view.geo.scale).max(2.0), c.height * view.geo.scale),
        );
        target = Some(r.expand2(vec2(40.0, 60.0)));
    }
    app.canvas.scroll_to_caret = false;
    target
}

struct ScrollMotion {
    wheel: egui::Vec2,
    auto: egui::Vec2,
    middle: egui::Vec2,
}

fn scroll_motion(app: &mut WordApp, ui: &Ui, full: Rect, view: &CanvasView) -> ScrollMotion {
    // Read Mode's centred columns leave margins at the sides; the wheel turns pages over them too.
    let wheel_area = if app.session.view.read_mode { full } else { view.area };
    let hovered = wheel_hovered(ui.rect_contains_pointer(wheel_area), ui.ctx().dragged_id().is_some(), app.canvas.drag_selecting());
    let notch = crate::scroll::notch_px(app.canvas.scale / PX_PER_PT);
    let wheel_delta = if app.session.view.read_mode {
        let turns = ui.input(|i| read_wheel_turns(&i.events, hovered, &mut app.canvas.read_wheel_accum));
        for _ in 0..turns.unsigned_abs() {
            turn_read_page(app, turns.signum() as f32);
        }
        if let Some(page) = app.canvas.read_target_page {
            let step = app.session.view.read_width * view.geo.scale + GAP;
            let x = (page as f32 * step).min(app.canvas.scroll_max.x);
            let dt = ui.input(|i| i.stable_dt);
            let next = if view.animate { reading_animation_step(app.canvas.scroll_offset.x, x, dt) } else { x };
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
        ui.input(|i| app.canvas.wheel.frame(i, &view.opts, hovered, notch, view.area.height(), view.animate))
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
        app.canvas.auto_scroll.update(ui, view.area)
    };
    ScrollMotion { wheel: wheel_delta, auto: auto_delta, middle: middle_delta }
}

fn configure_scroll_area(app: &mut WordApp, ui: &Ui, view: &CanvasView, motion: ScrollMotion) -> egui::ScrollArea {
    let delta = motion.wheel + motion.auto + motion.middle;
    // The canvas scrolls itself on wheel/touchpad input (crate::scroll): touchpads 1:1 with
    // momentum, wheel notches eased in (or all at once with animation effects off). Applied
    // before drawing, so it shows this frame. In reading mode, wheel and touchpad events turn
    // whole columns instead.
    let mut scroll_area = egui::ScrollArea::both()
        // One scroll position per open document, so switching back finds the page as it was left.
        .id_salt(("canvas_scroll", app.session.document_id()))
        .auto_shrink([false, false])
        // Bringing the caret into view glides there, or jumps with animation effects off.
        .animated(view.animate)
        .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: false, ..Default::default() });
    if app.session.view.read_mode {
        scroll_area = scroll_area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    }
    if delta != egui::Vec2::ZERO || app.canvas.scroll_request.is_some() || app.session.view.read_mode {
        let before = app.canvas.scroll_offset;
        let step = app.session.view.read_width * view.geo.scale + GAP;
        let snapped = if step.is_finite() && step > 0.0 { vec2((before.x / step).round() * step, 0.0) } else { egui::Vec2::ZERO };
        let after = match app.canvas.scroll_request.take() {
            Some(request) => request.clamp(egui::Vec2::ZERO, app.canvas.scroll_max),
            None if app.session.view.read_mode => snapped.clamp(egui::Vec2::ZERO, app.canvas.scroll_max),
            None => clamped_scroll(before, delta, app.canvas.scroll_max),
        };
        if after == before && motion.wheel != egui::Vec2::ZERO {
            app.canvas.wheel.hit_edge();
        }
        if motion.auto != egui::Vec2::ZERO && after != before {
            // The scroll keeps going without new input.
            ui.ctx().request_repaint();
        }
        scroll_area = scroll_area.scroll_offset(after);
    }
    if app.canvas.wheel.is_animating() {
        ui.ctx().request_repaint();
    }
    scroll_area
}

fn prepare_layout(app: &mut WordApp, ui: &Ui, full: Rect) -> std::sync::Arc<DocLayout> {
    let now = ui.input(|i| i.time);
    let reflow_anchor = update_viewport(app, ui, full, now);
    if app.canvas.focused && !matches!(app.dialog, Some(crate::dialogs::Dialog::SaveChanges { .. } | crate::dialogs::Dialog::FormatLosses { .. })) {
        crate::keys::canvas_events(app, ui.ctx());
    } else {
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
    layout
}

struct CanvasFinish<'a> {
    app: &'a mut WordApp,
    ui: &'a mut Ui,
    layout: &'a std::sync::Arc<DocLayout>,
    geo: &'a Geometry,
    caret: Option<wordcraft_layout::hit::Caret>,
    ime_geometry: Option<(Rect, Rect)>,
    ime_interrupt: bool,
    area: Rect,
    dim_body: bool,
    hruler: Option<Rect>,
    vruler: Option<Rect>,
}

fn update_canvas_scroll_state(
    app: &mut WordApp,
    ui: &mut Ui,
    layout: &std::sync::Arc<DocLayout>,
    geo: &Geometry,
    caret: Option<wordcraft_layout::hit::Caret>,
    out: &egui::scroll_area::ScrollAreaOutput<(egui::Response, Vec<Rect>)>,
) {
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
            app.canvas.read_first_visible = reading_page_start(layout, page.min(layout.pages.len().saturating_sub(1)));
        }
    }
    // The viewport barely moves between frames; the auto-scroll decides this frame's scroll before
    // the scroll area is built, so it reads last frame's viewport.
    app.canvas.viewport = Some(out.inner_rect);
}

fn update_canvas_focus(app: &mut WordApp, ui: &mut Ui, resp: &egui::Response, ime_geometry: Option<(Rect, Rect)>, ime_interrupt: bool) {
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
}

fn dismiss_balloon_on_canvas_click(app: &mut WordApp, ui: &Ui, resp: &egui::Response) {
    if app.canvas.balloon.is_some()
        && ui.input(|i| i.pointer.any_pressed())
        && resp.contains_pointer()
        && !ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| over_balloon(app, p))
    {
        deselect_balloon(app);
    }
}

struct PagePaint<'a> {
    app: &'a mut WordApp,
    ui: &'a Ui,
    layout: &'a std::sync::Arc<DocLayout>,
    geo: &'a Geometry,
    t: &'a Tokens,
    painter: &'a egui::Painter,
    dim_body: bool,
    dark_page: bool,
    sepia_page: bool,
    hidden: Option<(usize, Pos)>,
    view: u64,
    content_origin: Pos2,
    ppp: f32,
    max_tex: f32,
    grid_story: Option<StoryRef>,
}

struct PagePaintOutput {
    rects: Vec<Rect>,
    visible_tile_keys: HashSet<(usize, usize)>,
    rendered: usize,
}

fn draw_page_gridlines(app: &WordApp, painter: &egui::Painter, t: &Tokens, page: &Page, visual_sr: Rect, page_scale: f32) {
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
}

fn paint_pages(pass: PagePaint<'_>) -> PagePaintOutput {
    let PagePaint { app, ui, layout, geo, t, painter, dim_body, dark_page, sepia_page, hidden, view, content_origin, ppp, max_tex, grid_story } =
        pass;
    let mut visible_tile_keys = HashSet::new();
    let mut rendered = 0;
    let mut rects = Vec::with_capacity(geo.rects.len());
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
            if visible || cached { page_key(app, layout, i, view, scale_px) ^ if tiled { (tile_height_px as u64) << 32 } else { 0 } } else { 0 };
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
        let painter =
            if band_top <= 0.0 && band_bot >= sr.height() { painter.clone() } else { painter.with_clip_rect(painter.clip_rect().intersect(band)) };
        // Shadow and paper.
        painter.rect_filled(visual_sr.translate(vec2(0.0, 2.0)).expand(1.5), 1.0, t.page_shadow);
        let paper = if dark_page {
            Color32::from_gray(wordcraft_render::DARK_PAPER)
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
                        wordcraft_geom::Rect::new(0.0, start_px as f32 / scale_px, page.w, (height_px as f32 - 0.01).max(0.01) / scale_px),
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
        draw_page_gridlines(app, &painter, t, page, visual_sr, page_scale);
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
    PagePaintOutput { rects, visible_tile_keys, rendered }
}

fn paint_read_aloud_highlight(
    app: &mut WordApp,
    painter: &egui::Painter,
    layout: &std::sync::Arc<DocLayout>,
    rects: &[Rect],
    geo: &Geometry,
    t: &Tokens,
) {
    if let Some((a, b)) = crate::read_aloud::highlight(app) {
        for (pi, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
            if let Some(pr) = rects.get(pi) {
                let sr = Rect::from_min_size(pos2(pr.min.x + r.x * geo.scale, pr.min.y + r.y * geo.scale), vec2(r.w * geo.scale, r.h * geo.scale));
                painter.rect_filled(sr, 0.0, t.accent.gamma_multiply(0.16));
                painter.hline(sr.x_range(), sr.max.y - 0.5, egui::Stroke::new(1.5, t.accent));
            }
        }
    }
}

fn canvas_response(ui: &mut Ui, content: Rect, scroll_target: Option<Rect>) -> egui::Response {
    // A fixed id: an automatic one follows how many widgets came before, which Read Mode's
    // chrome changes, and a new id dropped the keyboard focus, so the arrows, Page Up/Down
    // and Escape did nothing in Read Mode until the reader clicked a page (#1772).
    // It also keeps the focus when Focus hides the rulers (#1678).
    ui.advance_cursor_after_rect(content);
    let response = ui.interact(content, egui::Id::unique("wordcraft_canvas"), Sense::click_and_drag());
    if let Some(target) = scroll_target {
        ui.scroll_to_rect(Rect::from_min_size(ui.min_rect().min + target.min.to_vec2(), target.size()), None);
    }
    response
}

fn handle_primary_pointer(
    app: &mut WordApp,
    ui: &Ui,
    resp: &egui::Response,
    rects: &[Rect],
    layout: &DocLayout,
    geo: &Geometry,
    middle_gesture: bool,
) {
    // Double-click the space between two pages (or the line between them) to hide or show it.
    if resp.double_clicked()
        && app.session.view.mode == wordcraft_layout::ViewMode::Print
        && !app.session.view.read_mode
        && let Some(p) = resp.interact_pointer_pos()
        && on_page_gap(rects, &geo.bands, p)
    {
        let _ = app.run("view.hideWhiteSpace", json!({"value": !app.session.view.hide_white_space}));
    } else
    // Draw Table / Eraser own the mouse while on (no caret moves); else the usual editing.
    if !middle_gesture && !crate::table_pen::pointer(app, ui, resp, rects, layout, geo.scale) {
        mouse(app, ui, resp, rects, layout, geo.scale);
    }
}

fn paint_white_space_seams(painter: &egui::Painter, rects: &[Rect], geo: &Geometry, t: &Tokens) {
    for (i, pair) in rects.windows(2).enumerate() {
        let Some(&(_, bot)) = geo.bands.get(i) else { continue };
        if white_space_hidden(&geo.rects, &geo.bands) {
            let y = pair[0].min.y + bot;
            painter.rect_filled(Rect::from_min_max(pos2(pair[0].min.x, y), pos2(pair[0].max.x, y + SEAM)), 0.0, t.border_strong);
        }
    }
}

fn handle_secondary_selection(app: &mut WordApp, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, geo: &Geometry, secondary_clicked: bool) {
    // Selected table cells: a right-click off them drops them, even between the selection's ends
    // (beside a selected column), so the menu acts on the cell clicked.
    if secondary_clicked
        && let Some(range) = app.session.cell_range()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(rects, layout, geo.scale, p)
    {
        let at = wordcraft_geom::Point::new(x, y);
        let on_cells =
            wordcraft_engine::cmd::table_mouse::cell_rects(layout, &app.session.doc, range).iter().any(|(pi, r)| *pi == page && r.contains(at));
        if !on_cells && let Some(pos) = layout.hit(page, x, y, app.session.sel.focus.story) {
            app.session.sel = wordcraft_engine::Selection::caret(pos);
        }
    }
    // Right-click: move the caret there (unless inside the selection), then the context menu.
    // Right-click in an equation puts the caret there and opens the equation menu.
    if secondary_clicked
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((page, x, y)) = page_at(rects, layout, geo.scale, p)
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
        && let Some((page, x, y)) = page_at(rects, layout, geo.scale, p)
    {
        if crate::objects::select_at(app, layout, rects, geo.scale, p) {
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
}

fn finish_canvas_frame(finish: CanvasFinish<'_>, out: egui::scroll_area::ScrollAreaOutput<(egui::Response, Vec<Rect>)>) {
    let CanvasFinish { app, ui, layout, geo, caret, ime_geometry, ime_interrupt, area, dim_body, hruler, vruler } = finish;
    update_canvas_scroll_state(app, ui, layout, geo, caret, &out);
    let (resp, rects) = out.inner;
    // A click on the page leaves the selected comment balloon.
    dismiss_balloon_on_canvas_click(app, ui, &resp);
    update_canvas_focus(app, ui, &resp, ime_geometry, ime_interrupt);
    // Screen readers: the document under the canvas, with the caret (#488).
    crate::a11y::expose(app, ui.ctx(), resp.id, layout, &rects, area);
    // The buttons that start or stop middle-click scrolling don't also click the page.
    let middle_gesture = app.canvas.auto_scroll.blocks_input();
    handle_primary_pointer(app, ui, &resp, &rects, layout, geo, middle_gesture);
    // A click on a check box ticks it; on a list's button opens the list.
    if !middle_gesture {
        crate::content_controls::clicks(app, &resp);
    }
    let secondary_clicked = resp.secondary_clicked() && !middle_gesture;
    handle_secondary_selection(app, &resp, &rects, layout, geo, secondary_clicked);
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
        let over_object = latest.and_then(|p| crate::objects::cursor(app, layout, &rects, geo.scale, p));
        // The selection bar: Word shows a right-pointing arrow there; egui has none, so the
        // plain arrow stands in (#856).
        let in_bar = app.session.sel.focus.story == StoryRef::Body
            && (app.canvas.unit_drag.as_ref().is_some_and(|d| d.unit == "line")
                || latest.and_then(|p| page_at(&rects, layout, geo.scale, p)).is_some_and(|(pg, x, y)| layout.selection_bar_at(pg, x, y).is_some()));
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
    crate::table_resize::cursor(app, ui, &resp, layout, &rects, geo.scale);
    crate::table_mouse::cursor(app, ui, &resp, layout, &rects, geo.scale);
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
    if let (Some(h), Some(v)) = (hruler, vruler) {
        rulers(app, ui, h, v, &rects, layout, geo.scale);
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

/// Lays out and paints the document pages for print, Read and Web views, settles viewports, draw
/// rulers and overlays, anchors zoom and scrolling, and routes pointer, selection, IME and caret
/// input.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let layout = prepare_layout(app, ui, full);
    let view = prepare_view(app, ui, full, &layout);
    let scroll_target = update_caret_scroll(app, &view);
    let motion = scroll_motion(app, ui, full, &view);
    let scroll_area = configure_scroll_area(app, ui, &view, motion);
    let CanvasView { area, hruler, vruler, dark_page, sepia_page, geo, dim_body, grid_story, caret, .. } = view;
    let mut origin = area.min;
    let mut ui_area = ui.new_child(egui::UiBuilder::new().max_rect(area));
    let mut ime_geometry: Option<(Rect, Rect)> = None;
    let mut ime_interrupt = false;
    let out = scroll_area.show_viewport(&mut ui_area, |ui, viewport| {
        origin = ui.min_rect().min - viewport.min.to_vec2();
        let content = Rect::from_min_size(ui.min_rect().min, geo.size);
        let resp = canvas_response(ui, content, scroll_target);
        // Read Mode's neighbouring columns end right at the viewport's edges: keep their
        // shadows from showing there as hairlines.
        let clip = if app.session.view.read_mode { ui.clip_rect().shrink2(vec2(3.0, 0.0)) } else { ui.clip_rect() };
        let painter = ui.painter_at(clip);
        let ppp = ui.ctx().pixels_per_point();
        let max_tex = ui.ctx().input(|i| i.max_texture_side).min(wordcraft_render::MAX_SIDE as usize).max(1) as f32;
        let content_origin = ui.min_rect().min;
        let hidden = app.canvas.dragged_off_page();
        let page_view = view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p));
        let t0 = crate::now_ms();
        let PagePaintOutput { rects, visible_tile_keys, rendered } = paint_pages(PagePaint {
            app,
            ui,
            layout: &layout,
            geo: &geo,
            t: &t,
            painter: &painter,
            dim_body,
            dark_page,
            sepia_page,
            hidden,
            view: page_view,
            content_origin,
            ppp,
            max_tex,
            grid_story,
        });
        paint_white_space_seams(&painter, &rects, &geo, &t);
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
        paint_read_aloud_highlight(app, &painter, &layout, &rects, &geo, &t);
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
                let look = crate::caret_glide::look(
                    app,
                    ui,
                    crate::caret_glide::LookRequest {
                        content_origin,
                        geometry: &geo,
                        target: [pos2(x, y0), pos2(x, y1)],
                        shown: focused,
                        since_ms: since,
                        on,
                    },
                );
                if look.opacity > 0.0 && focused {
                    let caret_color = caret_color_at(app, &layout, pi, ex + slot.x, base - slot.y + (slot.d - slot.a) / 2.0, page_view, dim_body);
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
            let caret_color =
                |app: &mut WordApp| caret_color_at(app, &layout, c.page, c.x + c.width / 2.0, c.top + c.height / 2.0, page_view, dim_body);
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
            let look = crate::caret_glide::look(
                app,
                ui,
                crate::caret_glide::LookRequest { content_origin, geometry: &geo, target: bar, shown, since_ms: since, on },
            );
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
    finish_canvas_frame(
        CanvasFinish { app, ui, layout: &layout, geo: &geo, caret, ime_geometry, ime_interrupt, area, dim_body, hruler, vruler },
        out,
    );
}

#[cfg(test)]
mod tests;
