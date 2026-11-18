//! The document canvas: the pages in the framework's page view (or the reading columns in Read
//! Mode), with the page tool, caret, selection marks, overlays, rulers and balloons shown over
//! them.

// PLAN: `Cargo.toml` (app-hub) needs `craft-pages`, `craft-ui-canvas`, `craft-review` and `craft-behavior`
// as dependencies for the paths below.

use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::kurbo::{Point, Size};
use craft::pages::layout::{Fit, LayoutOptions, PageDisplay, SCREEN_POINTS_PER_POINT};
use craft::pages::ui::reading::{ReadingMetrics, reading_size};
use craft::pages::ui::{MarkArea, MarkKind, PageBody, PageView, PageViewState, PlacedPage, ReadingNav, ReadingState, ReadingView, TextMarks};
use craft::ui::canvas::{Autoscroll, ContextSource, MiddleButton, NavigationOptions, ToolHost, ToolHostResponse, ToolPass, WheelScheme};
use craft::ui::contract::Show as _;

use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::DataColor;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::display::CommentDisplay;
use wordcraft_layout::{DocLayout, Page, Placed};

use crate::WordApp;
use crate::theme::{Tokens, regular, semibold};

mod balloons;
mod caret;
mod column_ruler;
mod context_menu;
#[cfg(test)]
pub(crate) use context_menu::issue_heading;
mod cursor;
mod equation_frame;
mod geometry;
mod gridlines;
mod input;
mod links;
mod navigation;
mod pages;
mod reading;
pub(crate) mod rulers;
mod source;
mod story;

#[cfg(test)]
use balloons::balloon_text;
pub use balloons::{deselect_balloon, select_balloon};
use context_menu::ContextAsk;
pub(crate) use cursor::{caret_color_on, caret_contrast};
pub use cursor::{caret_screen, pos_from_screen};
pub use geometry::{markup_width, sync_fit_zoom};
use input::UnitDrag;
pub(crate) use input::hold_document_selection;
pub(crate) use links::follow_link;
#[cfg(test)]
use links::{field_link, is_followable_link, is_generated_bookmark_link};
pub use navigation::page_to_screen;
use navigation::viewed_page;
pub(crate) use pages::{PageKeys, dark_page, page_doc, page_screen_scale, preview_sel, screen_render_options};
use pages::{page_render_options, shown_layout};
use reading::{reading_anchor_page, reading_page_start, web_layout_width};
pub use story::{editing_header_footer, in_text_box};
use story::dims_body;
pub(crate) use story::{part_kind, text_box_host};

/// Points on screen per document point at 100% (96 px per inch, like a word processor).
pub const PX_PER_PT: f32 = SCREEN_POINTS_PER_POINT as f32;


/// The pages the canvas showed in its last pass, for readers outside the pass (the control channel, the status bar,
/// tests).
#[derive(Clone, Debug)]
pub(crate) struct ShownPages {
    /// The canvas body on screen.
    pub rect: Rect,
    /// The pages on screen.
    pub pages: Vec<PlacedPage>,
    /// The canvas widget's id; `None` until a pass has shown it.
    pub canvas: Option<egui::Id>,
}

impl Default for ShownPages {
    fn default() -> Self {
        ShownPages { rect: Rect::NOTHING, pages: Vec::new(), canvas: None }
    }
}

pub struct CanvasState {
    /// The page view's state: zoom, fit, offset and current page. The session's view settings are copied in at the
    /// start of each pass and the zoom the framework changed is written back after it.
    pub(crate) view: PageViewState,
    /// Read Mode's column state.
    pub(crate) reading: ReadingState,
    /// What the last pass showed.
    pub(crate) shown: ShownPages,
    /// The caret's glide and blink.
    pub(crate) glide: crate::caret_glide::CaretGlide,
    /// The comment selection and drafts, shared by the Comments pane and the balloons.
    pub(crate) comments: craft::review::ui::CommentsPanelState,
    /// The one context-menu ask waiting for the pass (`ui.contextMenu`).
    pub(crate) context_request: Option<ContextAsk>,
    /// A press on a table's selection area or move handle is held (`table_mouse`).
    pub(crate) table_dragging: bool,
    /// The ruler drag's gesture, from its first applied step to its commit or cancel.
    // PLAN: assumed `craft::behavior::GestureId` (the framework's `behavior::value::GestureId`).
    pub(crate) ruler_gesture: Option<craft::behavior::GestureId>,
    /// The content control's open popup, until it closes.
    pub(crate) control_popup: Option<crate::content_controls::ControlPopup>,
    /// The SmartArt text typing run in progress.
    // PLAN: assumed `dialogs_smart_art::TypingRun` (dialogs-w17).
    pub(crate) smart_art_run: Option<crate::dialogs_smart_art::TypingRun>,
    /// The object gesture the page tool holds.
    // PLAN: written by canvas-w10's page tool; `ObjectGesture` is canvas-w11's.
    pub(crate) object_gesture: Option<crate::objects::ObjectGesture>,
    /// Page content fingerprints of the current layout, for the page revision.
    pub(crate) page_keys: PageKeys,
    /// Whether the document has tracked formatting changes, moves and deletions, by document
    /// revision: finding out walks every paragraph, too slow for every frame (#811).
    markup_facts: Cell<Option<((u64, u64), [bool; 3])>>,
    pub scroll_to_caret: bool,
    /// Set while a command runs for a mouse drag on a table: the pointer is already
    /// on the target, so the view stays put instead of following the caret (#1738).
    pub keep_view: bool,
    /// A heading navigation click aligns its page against the navigation pane, not just visible.
    pub(crate) nav_to_caret: bool,
    /// Reading controls (as opposed to window resizes) must reflow at once.
    pub(crate) read_force_reflow: bool,
    /// First text position on the column currently leading the reading view, captured from the last
    /// pass's layout. Survives changing page sizes, even when the caret stays elsewhere.
    pub(crate) read_first_visible: Option<Pos>,
    /// A text-selection drag is in progress.
    dragging: bool,
    /// Alt+drag: where the column (block) selection started (page, x, y) and its last end.
    column_drag: Option<((usize, f32, f32), (usize, f32, f32))>,
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
    /// The page being looked at (0-based), for the status bar (#3880): the caret's page while the
    /// caret is on screen, else the page at the top of the window. None until the canvas is drawn.
    pub viewed_page: Option<usize>,
    pub ime_preedit: String,
    /// The chars of `ime_preedit` the input method marks as its cursor or selection, as it reports
    /// them (unchecked).
    pub(crate) ime_preedit_range: Option<std::ops::Range<usize>>,
    /// Document faces added to the interface fonts so the input method's composition (#565) and a
    /// file name in the window caption can be drawn.
    pub(crate) ime_faces: Vec<Arc<wordcraft_fonts::FontFace>>,
    pub context_issue: Option<serde_json::Value>,
    pub context_synonyms: Option<serde_json::Value>,
    /// The right-click context menu is open (the mini toolbar stands down while it is).
    pub context_menu_open: bool,
    /// Screen rect of the selection's first line, for the floating mini toolbar.
    pub mini_anchor: Option<Rect>,
    /// The selection the mini toolbar was put away for by a click outside the document (the
    /// ribbon, a pane, the status bar); it comes back when the selection changes.
    pub mini_dismissed: Option<wordcraft_engine::Selection>,
    /// Crop mode (Picture Format › Crop) is on for the picture here (`crop`).
    pub(crate) crop: Option<wordcraft_doc::Pos>,
    /// An ink stroke being drawn with a pen (Draw tab).
    pub(crate) ink: Option<crate::ink::InkDrag>,
    /// The eraser is down: whether it has erased anything in this drag yet.
    pub(crate) erasing: Option<bool>,
    /// The lasso being drawn or dragged, and Ink Replay (Draw tab).
    pub(crate) ink_ui: crate::ink::InkUi,
    /// Synchronous Scrolling (#322): document points the partner window scrolled, still to apply…
    pub(crate) sync_in: f32,
    /// …and this window's own scrolling since the last frame, for the partner.
    pub(crate) sync_out: f32,
    /// Screen rects of the comment balloons last frame (for the control channel and tests).
    pub balloon_rects: Vec<(u32, Rect)>,
    /// Draw Table or Eraser is on (`table_pen`): presses draw instead of moving the caret.
    pub table_tool: Option<crate::table_pen::TableTool>,
    /// The Draw Table stroke (or eraser press) in progress, written during one gesture and cleared on release
    /// or cancel.
    pub(crate) table_stroke: Option<crate::table_pen::PenStroke>,
    /// An input method's `DeleteSurrounding` just erased text with this formatting: the text it
    /// sends next replaces it, keeps the formatting and joins the same undo step
    /// (`keys::delete_surrounding`, #1480).
    pub(crate) ime_join_text: Option<wordcraft_doc::props::CharProps>,
    /// An automatic change (AutoCorrect, AutoFormat) altered the text around the caret: the
    /// input method is asked to start afresh (#1480).
    pub(crate) ime_reset: bool,
    /// A replacement or an unsupported IME mutation means the next observation must interrupt
    /// the live composition, whatever the surrounding text looks like.
    pub(crate) ime_interrupt_now: bool,
    /// A nonempty preedit lost its body/equation target. The next canvas event batch must
    /// reject that old composition's queued replacement before dispatching any mutation.
    pub(crate) ime_interrupted_preedit: bool,
    /// The last bounded surrounding window and the `(revision, cursor, anchor)` it was built
    /// from. An idle pass reuses it instead of walking a long paragraph again.
    pub(crate) ime_window_cache: Option<(u64, wordcraft_doc::Pos, wordcraft_doc::Pos, Option<wordcraft_doc::ime_text::ImeWindow>)>,
    /// Create Link is waiting for a click on the text box to link to: the linking box's object.
    pub(crate) link_from: Option<wordcraft_doc::Pos>,
    /// The kind of tab stop a click on the ruler sets (the selector in the rulers' corner).
    pub tab_kind: wordcraft_doc::props::TabAlign,
    /// The rows and columns hovered in Insert › Table's grid this frame: the page shows that
    /// table where it would go (#3931). Set by the grid each frame, taken by [`show`].
    pub(crate) table_preview: Option<(usize, usize)>,
    /// A command (id, params) the page shows run on the selection, without running it: the
    /// formatting button the floating mini toolbar is hovered over. Set each frame, taken by [`show`].
    pub(crate) command_preview: Option<(String, serde_json::Value)>,
    /// The command preview the page last drew, so the bar repaints only when its hover changes.
    pub(crate) command_previewed: Option<(String, serde_json::Value)>,
    /// The scroll offset held while a command preview is shown, so the page does not move as
    /// the previewed text reflows.
    pub(crate) preview_scroll: Option<(f64, f64)>,
    /// The last previewed table's layout, the copy of the document it was laid out from and the
    /// copy's selection, with what it was laid out for (command, params, document id, revision,
    /// selection, and the document's own layout, which a view change replaces), so a still pointer
    /// doesn't lay the document out every frame. Set only while the page shows the preview.
    table_preview_layout: Option<(TablePreviewKey, wordcraft_doc::Document, wordcraft_engine::Selection, Arc<DocLayout>)>,
}

type TablePreviewKey = ((String, serde_json::Value, u64, u64, wordcraft_engine::Selection), Arc<DocLayout>);

impl CanvasState {
    /// Another document is shown in the window: everything about the page that was shown (drags,
    /// menus, the input method's composition, a scroll in progress…) starts afresh. What belongs
    /// to the window or the user stays: the colours last applied, the ruler's tab kind and the
    /// faces added for interface text. `seen`: the document was shown before, so the view stays
    /// where it was rather than going to the caret.
    pub(crate) fn show_other_document(&mut self, seen: bool) {
        let old = std::mem::take(self);
        let ime_reset = !old.ime_preedit.is_empty() || old.ime_preedit_range.is_some();
        *self = CanvasState {
            last_highlight: old.last_highlight,
            last_font_color: old.last_font_color,
            last_shading: old.last_shading,
            ime_faces: old.ime_faces,
            tab_kind: old.tab_kind,
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

    /// Asks for the page's keyboard focus; nothing until a pass has shown the canvas.
    pub fn focus_page(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.shown.canvas {
            focus::request(ctx, FocusTarget::Id(id));
        }
    }

    /// The canvas body on screen, once a pass has shown it.
    pub fn canvas_rect(&self) -> Option<Rect> {
        self.shown.canvas.map(|_| self.shown.rect)
    }

    /// The zoom the page view shows (1.0 = 100%).
    pub fn zoom(&self) -> f64 {
        self.view.zoom
    }

    /// The pages on screen in the last pass: page index and screen rectangle.
    pub fn page_rects(&self) -> impl Iterator<Item = (usize, Rect)> + '_ {
        self.shown.pages.iter().map(|p| (p.page, p.rect))
    }

    /// The canvas widget's id, once a pass has shown it.
    pub fn canvas_id(&self) -> Option<egui::Id> {
        self.shown.canvas
    }

    /// Whether `p` is on the canvas body.
    pub fn contains(&self, p: Pos2) -> bool {
        self.shown.canvas.is_some() && self.shown.rect.contains(p)
    }

    /// Whether the canvas has the keyboard.
    pub fn focused(&self, ctx: &egui::Context) -> bool {
        self.shown.canvas.is_some_and(|id| focus::has(ctx, id))
    }

    /// The object gesture the page tool holds, if one runs.
    pub fn object_gesture(&self) -> Option<&crate::objects::ObjectGesture> {
        self.object_gesture.as_ref()
    }

    /// The comment whose balloon is selected, as the document's comment id.
    pub fn selected_balloon(&self) -> Option<u32> {
        self.comments.selected.and_then(|id| u32::try_from(id.0).ok())
    }
}

impl Default for CanvasState {
    /// Creates empty canvas state: page keys, scroll flags, and inactive drag and column-selection
    /// state.
    fn default() -> Self {
        CanvasState {
            view: PageViewState::default(),
            reading: ReadingState::default(),
            shown: ShownPages::default(),
            glide: crate::caret_glide::CaretGlide::default(),
            comments: Default::default(),
            context_request: None,
            ruler_gesture: None,
            control_popup: None,
            table_dragging: false,
            smart_art_run: None,
            object_gesture: None,
            page_keys: PageKeys::default(),
            markup_facts: Cell::new(None),
            scroll_to_caret: true,
            keep_view: false,
            nav_to_caret: false,
            read_force_reflow: false,
            read_first_visible: None,
            dragging: false,
            column_drag: None,
            hf_press_switched: false,
            unit_drag: None,
            last_highlight: "yellow".into(),
            last_font_color: "C00000".into(),
            last_shading: "FFF2CC".into(),
            open_url: None,
            viewed_page: None,
            ime_preedit: String::new(),
            ime_preedit_range: None,
            ime_faces: Vec::new(),
            context_issue: None,
            context_synonyms: None,
            context_menu_open: false,
            mini_anchor: None,
            mini_dismissed: None,
            crop: None,
            ink: None,
            erasing: None,
            ink_ui: Default::default(),
            sync_in: 0.0,
            sync_out: 0.0,
            balloon_rects: Vec::new(),
            table_tool: None,
            table_stroke: None,
            ime_join_text: None,
            ime_reset: false,
            ime_interrupt_now: false,
            ime_interrupted_preedit: false,
            ime_window_cache: None,
            link_from: None,
            tab_kind: Default::default(),
            table_preview: None,
            command_preview: None,
            command_previewed: None,
            preview_scroll: None,
            table_preview_layout: None,
        }
    }
}

/// Settles the reading size and Web Layout's width from the room, and returns the text anchor to restore after a
/// Read Mode reflow.
fn update_viewport(app: &mut WordApp, full: Rect, metrics: &ReadingMetrics) -> Option<Pos> {
    let mut reflow_anchor: Option<Pos> = None;
    if app.session.view.read_mode {
        let scale = ReadingView::screen_scale(app.session.view.read_zoom.clamp(0.75, 1.5));
        let (w, h) = reading_size(full.size(), scale, metrics, app.session.view.read_column_width);
        let changed = (app.session.view.read_width - w).abs() > 0.5 || (app.session.view.read_height - h).abs() > 0.5;
        let forced = std::mem::take(&mut app.canvas.read_force_reflow);
        if changed || forced {
            reflow_anchor = app.canvas.read_first_visible.clone();
            app.session.view.read_width = w;
            app.session.view.read_height = h;
            // Without an anchor (Read Mode was just entered) the view starts at the caret.
            app.canvas.scroll_to_caret |= reflow_anchor.is_none();
        }
    } else {
        if app.session.view.mode == wordcraft_layout::ViewMode::Web {
            // Web Layout wraps to the work area (#1876).
            let scale = app.session.view.zoom.clamp(0.1, 5.0) * PX_PER_PT;
            let w = web_layout_width(full.width(), scale);
            if (app.session.view.web_width - w).abs() > 0.5 {
                app.session.view.web_width = w;
                // The pixel offset no longer points at the same text: keep the caret's line in view.
                app.canvas.scroll_to_caret = true;
            }
        }
        app.canvas.read_force_reflow = false;
        app.canvas.read_first_visible = None;
    }
    reflow_anchor
}

/// The layout the pass shows, after the viewport sizes are settled.
fn prepare_layout(app: &mut WordApp, full: Rect, metrics: &ReadingMetrics) -> Arc<DocLayout> {
    let reflow_anchor = update_viewport(app, full, metrics);
    let layout = shown_layout(app);
    if let Some(anchor) = reflow_anchor
        && let Some(page) = reading_anchor_page(&layout, &anchor)
    {
        app.canvas.reading.column = page;
    }
    layout
}

/// A click on the page leaves the selected comment balloon.
fn dismiss_balloon_on_canvas_click(app: &mut WordApp, out: &craft::pages::ui::PageViewOutput) {
    if app.canvas.selected_balloon().is_some() && out.canvas.click_at().is_some() {
        deselect_balloon(app);
    }
}

/// While the page shows a command preview the view stays where it was when the preview began:
/// no scrolling to the caret, and the offset is held against the previewed text's reflow.
fn hold_scroll_for_preview(app: &mut WordApp, offset: &mut (f64, f64)) -> bool {
    if app.canvas.command_previewed.is_none() {
        app.canvas.preview_scroll = None;
        return false;
    }
    *offset = *app.canvas.preview_scroll.get_or_insert(*offset);
    true
}

/// The selection's rects by page, in page points, from the layout `layout` is of (the preview's while one shows).
fn selection_page_rects(app: &mut WordApp, layout: &DocLayout) -> Vec<(usize, wordcraft_geom::Rect)> {
    if let Some(sel) = preview_sel(app) {
        let (a, b) = sel.ordered();
        if a == b {
            Vec::new()
        } else {
            wordcraft_engine::cmd::table_mouse::selection_rects(layout, page_doc(app), &a, &b, app.session.page_hint)
        }
    } else if crate::objects::selected(app).is_some() {
        Vec::new()
    } else if let Some(range) = app.session.cell_range() {
        wordcraft_engine::cmd::table_mouse::cell_rects(layout, &app.session.doc, range)
    } else if !app.session.sel.is_collapsed() {
        match app.session.column_segments() {
            Some(segs) => segs.iter().flat_map(|(a, b)| layout.selection_rects(&app.session.doc, a, b, app.session.page_hint)).collect(),
            None => {
                let (a, b) = app.session.sel.ordered();
                wordcraft_engine::cmd::table_mouse::selection_rects(layout, &app.session.doc, &a, &b, app.session.page_hint)
            }
        }
    } else {
        Vec::new()
    }
}

/// The live selection's rects on screen, from the document's own layout rather than a command
/// preview's, which can move the lines (Align Right) from under the mini toolbar's pointer.
fn live_selection_screen_rects(app: &mut WordApp, pages: &[PlacedPage]) -> Vec<Rect> {
    let layout = app.session.layout();
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
    screen_rects(&selected, pages)
}

/// Page-point rects on the pages that are on screen, as screen rects.
fn screen_rects(rects: &[(usize, wordcraft_geom::Rect)], pages: &[PlacedPage]) -> Vec<Rect> {
    rects
        .iter()
        .filter_map(|(page, r)| {
            let placed = pages.iter().find(|p| p.page == *page)?;
            let a = placed.to_screen(Point::new(f64::from(r.x), f64::from(r.y)));
            let b = placed.to_screen(Point::new(f64::from(r.x + r.w), f64::from(r.y + r.h)));
            Some(Rect::from_two_pos(a, b))
        })
        .collect()
}

/// A page-point rect as a mark area.
fn mark_area(page: usize, r: &wordcraft_geom::Rect, kind: MarkKind) -> MarkArea {
    let rect = craft::behavior::kurbo::Rect::new(f64::from(r.x), f64::from(r.y), f64::from(r.x + r.w), f64::from(r.y + r.h));
    MarkArea::new(page, rect, kind)
}

/// The selection, Find › Reading Highlight and Read Aloud marks, in page points.
fn mark_areas(app: &mut WordApp, layout: &DocLayout, selected: &[(usize, wordcraft_geom::Rect)]) -> Vec<MarkArea> {
    let mut areas: Vec<MarkArea> = selected.iter().map(|(page, r)| mark_area(*page, r, MarkKind::Selection)).collect();
    // Find's places are the document's, not the preview's.
    if preview_sel(app).is_none() {
        let hint = app.session.page_hint;
        let found: Vec<(Pos, Pos)> = app.session.find_highlights().iter().take(5000).cloned().collect();
        for (a, b) in &found {
            for (page, r) in layout.selection_rects(&app.session.doc, a, b, hint) {
                areas.push(mark_area(page, &r, MarkKind::Found));
            }
        }
    }
    if let Some((a, b)) = crate::read_aloud::highlight(app) {
        for (page, r) in layout.selection_rects(&app.session.doc, &a, &b, app.session.page_hint) {
            areas.push(mark_area(page, &r, MarkKind::Spoken));
        }
    }
    areas
}

/// The navigation options of WordCraft's canvas: the document wheel scheme, and middle-click autoscroll where the
/// platform has the gesture (not macOS, iOS or the web, whose middle button has its own meaning).
fn navigation_options() -> NavigationOptions {
    let mut options = NavigationOptions::default();
    options.middle_button =
        if cfg!(any(target_os = "macos", target_os = "ios", target_arch = "wasm32")) { MiddleButton::Pan } else { MiddleButton::Autoscroll };
    options.wheel = WheelScheme::Document;
    options
}

/// The session's view settings as the page view's state for this pass.
fn sync_view_state(app: &WordApp, state: &mut PageViewState) {
    let v = &app.session.view;
    let web = v.mode != wordcraft_layout::ViewMode::Print;
    let editing_part = matches!(app.session.sel.focus.story, StoryRef::Part(_));
    state.continuous = true;
    state.cover = false;
    state.page_gaps = !web;
    state.fit = if web {
        Fit::None
    } else {
        match v.fit.as_str() {
            "pageWidth" => Fit::Width,
            "onePage" | "multiplePages" => Fit::Page,
            _ => Fit::None,
        }
    };
    state.zoom = f64::from(v.zoom.clamp(0.1, 5.0));
    state.display = if web || v.fit == "onePage" || v.fit == "pageWidth" || v.single_column { PageDisplay::Single } else { PageDisplay::Columns };
    state.hide_white_space = v.hide_white_space && v.mode == wordcraft_layout::ViewMode::Print && !editing_part;
}

/// What the page body decided about the scroll position, applied after the pass.
#[derive(Clone, Copy, Debug, Default)]
struct ScrollFix {
    /// Move the offset by this much (screen points).
    delta: (f64, f64),
    /// Bring this page, with this fraction of its height, to the top.
    go_to: Option<(usize, f64)>,
}

/// How far a caret rect `target` lies outside `view` on one axis: negative above/left, positive below/right.
fn overshoot(target: (f32, f32), view: (f32, f32)) -> f64 {
    if target.0 < view.0 {
        f64::from(target.0 - view.0)
    } else if target.1 > view.1 {
        f64::from(target.1 - view.1)
    } else {
        0.0
    }
}

/// Runs the one page tool of this pass: the ink or table pen while one is on, else the text tool.
fn run_page_tool(app: &mut WordApp, ui: &Ui, layout: &Arc<DocLayout>, body: &PageBody<'_>, pass: &ToolPass) -> ToolHostResponse {
    // Page keys reach the page in every tool mode, once per pass, before the host runs.
    // PLAN: assumed `keys::page_input(app: &mut WordApp, input: &[EditorInput], pass: &ToolPass, ctx: &egui::Context)` (canvas-w10).
    crate::keys::page_input(app, &pass.editor_input, pass, ui.ctx());
    let host = ToolHost::new().enabled(!app.modal_open());
    if let Some(mut tool) = crate::ink::active_tool(app, body) {
        return host.run(ui, pass, &mut tool);
    }
    if let Some(mut tool) = crate::table_pen::active_tool(app, body) {
        return host.run(ui, pass, &mut tool);
    }
    let mut tool = input::PageTextTool::new(app, layout.clone(), body, pass, ui.ctx(), Env::of(ui).view().shell());
    host.run(ui, pass, &mut tool)
}

/// The paper under each page, as the page raster draws it without a page color.
fn paper_color(app: &WordApp) -> Color32 {
    let p = page_render_options(app, false).blank_paper();
    Color32::from_rgb(p.0, p.1, p.2)
}

/// Read Mode: the reading columns, with the page-turn row under them.
fn show_reading(app: &mut WordApp, ui: &mut Ui, layout: &Arc<DocLayout>, full: Rect, metrics: &ReadingMetrics) {
    let zoom = app.session.view.read_zoom.clamp(0.75, 1.5);
    let paper = DataColor::new(paper_color(app));
    let columns = Rect::from_min_max(full.min, pos2(full.max.x, (full.max.y - metrics.nav_bar).max(full.min.y)));
    let footer = Rect::from_min_max(pos2(full.min.x, columns.max.y), full.max);
    let mut state = std::mem::take(&mut app.canvas.reading);
    let source = source::WordPages::new(app, layout, false, None);
    let out = ReadingView::new(tl!("Document"), &source, &mut state, paper).zoom(zoom).show_at(ui, columns);
    let nav = ui
        .scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            ReadingNav::new(&mut state, layout.pages.len()).position(true).action(tl!("Edit Document")).show(ui)
        })
        .inner;
    app.canvas.page_keys = source.into_keys();
    app.canvas.reading = state;
    app.canvas.shown = ShownPages { rect: columns, pages: Vec::new(), canvas: Some(out.response.id) };
    app.canvas.viewed_page = None;
    app.canvas.read_first_visible = reading_page_start(layout, app.canvas.reading.column);
    app.canvas.scroll_to_caret = false;
    if nav.action {
        let _ = app.run("view.printLayout", json!({}));
    }
}

/// Render the document canvas for the current frame: the pages (or Read Mode's columns) and everything over them.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let full = ui.available_rect_before_wrap();
    // PLAN: assumed `Env::view(&self).reading_metrics() -> ReadingMetrics`: `ReadingMetrics::of` needs the tokens and a
    // text layout, which the app cannot reach from a `Ui` (framework gap).
    let metrics = Env::of(ui).reading_metrics();
    let layout = prepare_layout(app, full, &metrics);
    if app.session.view.read_mode {
        show_reading(app, ui, &layout, full, &metrics);
    } else {
        show_pages(app, ui, &layout, full);
    }
}

/// The page view and everything shown over it.
fn show_pages(app: &mut WordApp, ui: &mut Ui, layout: &Arc<DocLayout>, full: Rect) {
    let mut state = std::mem::take(&mut app.canvas.view);
    sync_view_state(app, &mut state);
    let before = (state.zoom, state.fit);
    if app.canvas.nav_to_caret {
        app.canvas.nav_to_caret = false;
        let c = layout.caret_on(&preview_sel(app).unwrap_or(app.session.sel_ends()).focus, app.session.page_hint);
        if let Some(c) = c
            && let Some(page) = layout.pages.get(c.page)
        {
            state.go_to = Some((c.page, f64::from(c.top / page.h.max(1.0))));
            state.current = c.page;
        }
    }
    // Scrolling the partner window did (Synchronous Scrolling, #322), in screen points at this zoom.
    let scale = state.zoom * f64::from(PX_PER_PT);
    let synced = f64::from(std::mem::take(&mut app.canvas.sync_in)) * scale;
    state.offset.1 += synced;
    let held = hold_scroll_for_preview(app, &mut state.offset);
    let started_at = state.offset.1;

    let caret = layout.caret_on(&preview_sel(app).unwrap_or(app.session.sel_ends()).focus, app.session.page_hint);
    if let Some(c) = &caret
        && !app.session.selects_whole_story()
    {
        app.session.page_hint = c.page;
    }
    let dim_body = dims_body(app, layout);
    let hidden = crate::objects::dragged_off_page(app.canvas.object_gesture.as_ref());
    let selected = selection_page_rects(app, layout);
    let areas = mark_areas(app, layout, &selected);
    let bodies: Vec<(f64, f64)> = layout.pages.iter().map(|p| (f64::from(p.body.y), f64::from(p.body.bottom()))).collect();
    let show_rulers = app.session.view.ruler
        && app.session.view.mode == wordcraft_layout::ViewMode::Print
        && crate::chrome::Screen::of(&app.session.view) == crate::chrome::Screen::Normal;
    let options = LayoutOptions {
        markup: f64::from(markup_width(app)),
        min_columns: if app.session.view.multi_page { 2 } else { 1 },
        ..LayoutOptions::WORD
    };
    let paper = DataColor::new(paper_color(app));
    let source = source::WordPages::new(app, layout, dim_body, hidden);
    // The describer owns its snapshot, as the source does, so the page view can hold it while the body edits the app.
    let described = preview_sel(app).unwrap_or(app.session.sel_ends()).clone();
    let describer = crate::a11y::DocumentDescriber::new(source.doc(), layout.clone(), (described.anchor.clone(), described.focus.clone()));
    let regions = crate::table_pen::regions(app, layout);

    let mut tool_response: Option<ToolHostResponse> = None;
    let mut strip_click: Option<Pos2> = None;
    let mut caret_after = None;
    let mut body_rect = full;
    let mut body_view = None;
    let mut ran = false;
    let mut fix = ScrollFix::default();
    let out = PageView::new(tl!("Document"), &source, &mut state, paper)
        .options(options)
        .navigation(navigation_options())
        .editor(craft::ui::primitives::EditorPurpose::Text)
        .rulers(show_rulers)
        .snap(crate::objects::snap(app, layout))
        .tool_regions(regions)
        .access(&describer)
        .show(ui, |ui, body| {
            body_rect = body.rect;
            body_view = Some(body.view);
            // Canvas controls take their presses first, then the tool host runs, then the caret shows over them.
            TextMarks::over(body.pages).areas(&areas).show_at(ui, body.rect);
            gridlines::show(app, ui, layout, body);
            crate::content_controls::show(app, ui, layout, body);
            strip_click = equation_frame::show(app, ui, layout, body);
            crate::crop::show(app, ui, layout, body);
            crate::objects::show(app, ui, layout, body);
            crate::table_resize::show(app, ui, layout, body);
            crate::table_mouse::show(app, ui, layout, body);
            crate::ink::show(app, ui, layout, body);
            crate::dialogs_smart_art::frame_button(app, ui, body.pages, layout);
            if let Some(pass) = body.pass {
                ran = true;
                tool_response = Some(run_page_tool(app, ui, layout, body, pass));
                caret::paint_caret(app, ui, pass, body, Env::of(ui).motion);
                // This pass's keys and press have moved the caret: follow it, not the one from before the pass.
                let fresh = app.session.layout();
                caret_after = fresh.caret_on(&preview_sel(app).unwrap_or(app.session.sel_ends()).focus, app.session.page_hint);
                if app.canvas.scroll_to_caret
                    && !held
                    && let Some(c) = &caret_after
                {
                    match body.pages.iter().find(|p| p.page == c.page) {
                        Some(placed) => {
                            let s = placed.scale as f32;
                            let top_left = placed.to_screen(Point::new(f64::from(c.x), f64::from(c.top)));
                            let r = Rect::from_min_size(top_left, vec2((c.width * s).max(2.0), c.height * s)).expand2(vec2(40.0, 60.0));
                            fix.delta = (overshoot((r.min.x, r.max.x), (body.rect.min.x, body.rect.max.x)), overshoot((r.min.y, r.max.y), (body.rect.min.y, body.rect.max.y)));
                        }
                        None => {
                            let h = fresh.pages.get(c.page).map_or(1.0, |p| p.h.max(1.0));
                            fix.go_to = Some((c.page, f64::from(c.top / h)));
                        }
                    }
                }
                // The mini toolbar's anchor is settled in the pass that shows it. Anchor only to visible
                // selection geometry, and to the live selection while a preview moves the text.
                let anchor_rects =
                    if app.canvas.command_previewed.is_some() { live_selection_screen_rects(app, body.pages) } else { screen_rects(&selected, body.pages) };
                app.canvas.mini_anchor = crate::mini_toolbar::visible_anchor(anchor_rects, ui.clip_rect());
            }
        });
    let caret = caret_after.or(caret);
    app.canvas.page_keys = source.into_keys();
    if ran {
        app.canvas.scroll_to_caret = false;
    }

    // The scroll position the body asked for takes effect on the state the pass stored.
    let moved_by_user = state.offset.1 - started_at;
    if fix.delta != (0.0, 0.0) {
        state.offset.0 += fix.delta.0;
        state.offset.1 += fix.delta.1;
        ui.ctx().request_repaint();
    }
    if let Some(go) = fix.go_to {
        state.go_to = Some(go);
        ui.ctx().request_repaint();
    }
    // Dragging a selection past the view's edge scrolls on.
    if let Some(step) = tool_response.as_ref().and_then(|r| r.scroll)
        && let Some(view) = body_view
    {
        let mut v = view;
        // The bounds are in document units: the layout's screen points over the view's points per unit.
        let per_unit = v.zoom.max(f64::EPSILON);
        let size = Size::new(out.layout.size.width / per_unit, out.layout.size.height / per_unit);
        if Autoscroll::scroll(&mut v, step, size).is_some() {
            out.adopt(&mut state, &v);
            ui.ctx().request_repaint();
        }
    }
    // The partner window follows this window's own scrolling, not a zoom's.
    let zoomed = (state.zoom - before.0).abs() > 1e-9 || state.fit != before.1;
    if !zoomed && scale > 0.0 {
        app.canvas.sync_out += (moved_by_user / scale) as f32;
    }
    // A zoom the framework made (wheel, pinch) goes to the session; a fit shows its own zoom.
    if zoomed && state.fit == Fit::None {
        let pct = (state.zoom * 100.0).round().clamp(10.0, 500.0);
        let _ = app.run("view.zoom", json!({"value": pct}));
    } else {
        sync_fit_zoom(app, state.zoom);
    }
    let caret_span = caret.as_ref().and_then(|c| {
        let r = out.layout.rects.get(c.page)?;
        let h = f64::from(layout.pages.get(c.page)?.h.max(1.0));
        let s = r.height() / h;
        let top = r.y0 + f64::from(c.top) * s;
        Some((c.page, top, top + f64::from(c.height) * s))
    });
    app.canvas.viewed_page = Some(viewed_page(caret_span, &state, &out.layout, f64::from(body_rect.height())));
    app.canvas.shown = ShownPages { rect: body_rect, pages: out.pages.clone(), canvas: Some(out.canvas.response.id) };
    app.canvas.view = state;
    if out.toggle_white_space {
        let _ = app.run("view.hideWhiteSpace", json!({"value": !app.session.view.hide_white_space}));
    }

    dismiss_balloon_on_canvas_click(app, &out);
    balloons::balloons(app, ui, &out, layout);
    rulers::page_rulers(app, ui, &out, layout);
    crate::mini_toolbar::show(app, ui, &out);
    // The one context-menu ask of the pass: Shift+F10 or `ui.contextMenu` (below the caret), a click on the equation
    // frame's options strip, then the tool host's own request by how it was asked for.
    let ask = app.canvas.context_request.take().or_else(|| strip_click.map(ContextAsk::AtPointer));
    let ask = ask.or_else(|| {
        let request = tool_response.as_ref()?.context?;
        Some(match request.source {
            ContextSource::Key => ContextAsk::AtCaret,
            _ => ContextAsk::AtPointer(request.at),
        })
    });
    context_menu::show(app, ui, &out.canvas.response, ask);
}
