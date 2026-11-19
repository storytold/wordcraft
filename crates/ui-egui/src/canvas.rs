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

mod balloons;
mod caret;
mod column_ruler;
pub(crate) mod context_menu;
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
mod reading_metrics;
pub(crate) mod rulers;
mod source;
mod story;

pub use balloons::{deselect_balloon, select_balloon};
use context_menu::ContextAsk;
pub(crate) use cursor::{caret_color_on, caret_contrast};
pub use cursor::{caret_screen, pos_from_screen};
pub use geometry::{markup_width, sync_fit_zoom};
use input::UnitDrag;
pub(crate) use input::{hold_document_selection, release_document_selection};
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
    /// Every page of the view's layout, on screen or not, at its screen position this pass.
    pub laid: Vec<Rect>,
    /// The area the pages show in: the body inside the rulers while they show.
    pub view: Rect,
    /// The canvas widget's id; `None` until a pass has shown it.
    pub canvas: Option<egui::Id>,
}

impl Default for ShownPages {
    fn default() -> Self {
        ShownPages { rect: Rect::NOTHING, pages: Vec::new(), laid: Vec::new(), view: Rect::NOTHING, canvas: None }
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
    /// The page's request for the keyboard has not been taken yet (a window's page starts with it set: production's
    /// `want_focus` started on). Kept until a real run takes it, because a measuring run moves no focus
    /// (`focus::request` is `false` there).
    pub(crate) focus_pending: bool,
    /// The page asks for the keyboard between this pass and the next ([`CanvasState::focus_page_after_pass`]): always
    /// (no ids), or only while focus is still on one of the ids.
    focus_after_pass: Option<Vec<egui::Id>>,
    /// How many page pictures the page view kept resident after the last pass (S-L352).
    pub(crate) resident_pictures: usize,
    /// The caret's glide and blink.
    pub(crate) glide: crate::caret_glide::CaretGlide,
    /// The comment selection and drafts, shared by the Comments pane and the balloons.
    pub(crate) comments: craft::review::ui::CommentsPanelState,
    /// The one context-menu ask waiting for the pass (`ui.contextMenu`).
    pub(crate) context_request: Option<ContextAsk>,
    /// A press on a table's selection area or move handle is held (`table_mouse`).
    pub(crate) table_dragging: bool,
    /// The table border drag in progress, a ruler's too, from the press to its release or cancel: the page paints
    /// its dashed preview from it (`table_resize`).
    pub(crate) table_resize: Option<crate::table_resize::ResizeDrag>,
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
    /// A press on a part of the selected object's frame (a handle to size it, the one to turn it) is open: the
    /// frame is the framework's `TransformBox`, not the page tool's [`ObjectGesture`], so it is recorded here, each
    /// pass, from the box's output.
    pub(crate) frame_held: bool,
    /// The picture pixels of the object the frame's turn handle is turning, drawn once for the turn.
    pub(crate) turning: Option<crate::objects::Turning>,
    /// Page content fingerprints of the current layout, for the page revision.
    pub(crate) page_keys: PageKeys,
    /// The caret colour last worked out and the key of the place it was worked out for, so a blink doesn't rebuild the
    /// page's draw list every frame (`cursor::caret_color_at`).
    pub(crate) caret_bg: Option<(u64, Color32)>,
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
    /// Web Layout's width follows the window once a drag has settled, not on every frame of it.
    web_viewport: reading::Settle,
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
<<<<<<< HEAD
=======
    /// Document faces added to the interface fonts so the input method's composition (#565) and a
    /// file name in the window caption can be drawn.
    pub(crate) ime_faces: Vec<std::sync::Arc<wordcraft_fonts::FontFace>>,
    /// The document face this window's composition is drawn in, as the fonts were last installed
    /// (its [`crate::theme::composition_family`]).
    pub(crate) ime_face: Option<wordcraft_fonts::FaceRef>,
    pub want_focus: bool,
>>>>>>> up/main
    pub context_issue: Option<serde_json::Value>,
    pub context_synonyms: Option<serde_json::Value>,
    /// The right-click context menu is open (the mini toolbar stands down while it is).
    pub context_menu_open: bool,
    /// Where the open context menu is anchored: the framework keeps an anchor only for a context opening, and this menu opens by the app's one ask.
    pub(crate) context_menu_at: Option<Pos2>,
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
    /// The preview a bar list (font, size, highlight, font colour) started: the framework reports a
    /// list's preview once, when it starts (`Preview::Shows`) and once when it ends, so the bar keeps
    /// it here and re-asserts it each frame until the list's `Preview::Ended`.
    pub(crate) list_preview: Option<(String, serde_json::Value)>,
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

    /// Asks for the page's keyboard focus. A request the framework could not take (no page shown yet, or a measuring
    /// run, which moves no focus) is kept and made again by the next pass that shows the page. Whether it was taken.
    pub fn focus_page(&mut self, ctx: &egui::Context) -> bool {
        let taken = self.shown.canvas.is_some_and(|id| focus::request(ctx, FocusTarget::Id(id)));
        self.focus_pending = !taken;
        taken
    }

    /// Asks for the page's keyboard focus once this pass is over, for a control whose press moves focus at the
    /// pass's end (a ribbon tab: the framework's press focus would override a request made in the pass).
    pub(crate) fn focus_page_after_pass(&mut self) {
        self.focus_after_pass = Some(Vec::new());
    }

    /// [`focus_page_after_pass`](Self::focus_page_after_pass) for a menu that closed this pass, opened by `opener`:
    /// only while focus is still in the menu or back on its opener then. The press that closed it may have given a
    /// field the focus, which keeps it.
    pub(crate) fn focus_page_after_menu(&mut self, ctx: &egui::Context, opener: egui::Id) {
        if self.focus_after_pass.is_none() {
            let held = ctx.memory(|m| m.focused());
            self.focus_after_pass = Some(std::iter::once(opener).chain(held).collect());
        }
    }

    /// Makes the request [`focus_page_after_pass`](Self::focus_page_after_pass) kept, between passes: the framework
    /// takes it at the next pass's begin.
    pub(crate) fn take_focus_after_pass(&mut self, ctx: &egui::Context) {
        let Some(ids) = self.focus_after_pass.take() else { return };
        let focused = ctx.memory(|m| m.focused());
        if ids.is_empty() || ids.iter().any(|id| focused == Some(*id) || focus::has(ctx, *id)) {
            self.focus_page(ctx);
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

    /// The area the pages show in, once a pass has shown them: the canvas body inside the rulers while they show.
    pub fn view_rect(&self) -> Option<Rect> {
        self.shown.canvas.map(|_| self.shown.view)
    }

    /// Every page's rectangle on screen in the last pass, in page order, including pages scrolled out of view.
    pub fn laid_page_rects(&self) -> &[Rect] {
        &self.shown.laid
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

    /// Whether a press on a part of the selected object's frame (size or turn handle) is open.
    pub fn frame_gesture(&self) -> bool {
        self.frame_held
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
            focus_pending: true,
            focus_after_pass: None,
            resident_pictures: 0,
            glide: crate::caret_glide::CaretGlide::default(),
            comments: Default::default(),
            context_request: None,
            ruler_gesture: None,
            control_popup: None,
            table_dragging: false,
            table_resize: None,
            smart_art_run: None,
            object_gesture: None,
            frame_held: false,
            turning: None,
            page_keys: PageKeys::default(),
            caret_bg: None,
            markup_facts: Cell::new(None),
            scroll_to_caret: true,
            keep_view: false,
            nav_to_caret: false,
            read_force_reflow: false,
            web_viewport: Default::default(),
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
<<<<<<< HEAD
=======
            ime_faces: Vec::new(),
            ime_face: None,
            want_focus: true,
>>>>>>> up/main
            context_issue: None,
            context_synonyms: None,
            context_menu_open: false,
            context_menu_at: None,
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
            list_preview: None,
            preview_scroll: None,
            table_preview_layout: None,
        }
    }
}

/// Settles the reading size and Web Layout's width from the room, and returns the text anchor to restore after a
/// Read Mode reflow.
fn update_viewport(app: &mut WordApp, full: Rect, metrics: &ReadingMetrics, side: f32, ctx: &egui::Context) -> Option<Pos> {
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
            // The page view keeps its side gutter (the rail width) clear on both sides of a page, so the page
            // fits the window only when its width leaves that room.
            let w = web_layout_width((full.width() - 2.0 * side).max(0.0), scale);
            let changed = (app.session.view.web_width - w).abs() > 0.5;
            // At once on entry or a zoom (the viewport holds still), after a window drag has settled.
            if app.canvas.web_viewport.apply(full.size(), ctx.input(|i| i.time), changed, false) {
                app.session.view.web_width = w;
                // The pixel offset no longer points at the same text: keep the caret's line in view.
                app.canvas.scroll_to_caret = true;
            } else if changed {
                ctx.request_repaint_after(std::time::Duration::from_millis(170));
            }
        } else {
            app.canvas.web_viewport.leave();
        }
        app.canvas.read_force_reflow = false;
        app.canvas.read_first_visible = None;
    }
    reflow_anchor
}

/// The layout the pass shows, after the viewport sizes are settled.
fn prepare_layout(app: &mut WordApp, full: Rect, metrics: &ReadingMetrics, side: f32, ctx: &egui::Context) -> Arc<DocLayout> {
    let reflow_anchor = update_viewport(app, full, metrics, side, ctx);
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

<<<<<<< HEAD
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
    // A wheel or pinch zoom is the view's own, anchored at the pointer, and reaches the session rounded to a whole
    // percent (`show_pages`): the view keeps its exact zoom while it rounds to the session's, or the rounding would
    // move the place under the pointer on the next pass.
    let session_zoom = f64::from(v.zoom.clamp(0.1, 5.0));
    if (state.zoom * 100.0).round() != (session_zoom * 100.0).round() {
        state.zoom = session_zoom;
    }
    state.display = if web || v.fit == "onePage" || v.fit == "pageWidth" || v.single_column { PageDisplay::Single } else { PageDisplay::Columns };
    state.hide_white_space = v.hide_white_space && v.mode == wordcraft_layout::ViewMode::Print && !editing_part;
}

/// Every page of the pass's layout on screen: the layout's content rects moved by where a placed page shows.
fn laid_rects(out: &craft::pages::ui::PageViewOutput) -> Vec<Rect> {
    let Some(shift) = out.pages.iter().find_map(|placed| {
        let r = out.layout.rects.get(placed.page)?;
        Some(placed.rect.min - pos2(r.x0 as f32, r.y0 as f32))
    }) else {
        return Vec::new();
    };
    let at = |x: f64, y: f64| pos2(x as f32, y as f32) + shift;
    out.layout.rects.iter().map(|r| Rect::from_min_max(at(r.x0, r.y0), at(r.x1, r.y1))).collect()
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
    let mut tool = input::PageTextTool::new(app, layout.clone(), body, Env::of(ui).view().shell());
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
    app.canvas.shown = ShownPages { rect: columns, pages: Vec::new(), laid: Vec::new(), view: columns, canvas: Some(out.response.id) };
    app.canvas.viewed_page = None;
    app.canvas.read_first_visible = reading_page_start(layout, app.canvas.reading.column);
    app.canvas.scroll_to_caret = false;
    // The text menu stays available in Reading Mode (copying and looking things up): opened by
    // `ui.contextMenu`, or by a right click on the columns. The bar over it is off (`context_menu_visible`).
    let ask = app.canvas.context_request.take().or_else(|| {
        let r = &out.response;
        r.secondary_clicked().then(|| r.interact_pointer_pos()).flatten().map(ContextAsk::AtPointer)
    });
    context_menu::show(app, ui, &out.response, ask);
    if nav.action {
        let _ = app.run("view.printLayout", json!({}));
    }
}

/// Render the document canvas for the current frame: the pages (or Read Mode's columns) and everything over them.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let full = ui.available_rect_before_wrap();
    let Some(metrics) = reading_metrics::read(ui) else { return };
    let layout = prepare_layout(app, full, &metrics, Env::of(ui).tokens.size.rail.width, ui.ctx());
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
    // The caret the follow wants whose page the pass did not place: its page, the page's height and the caret.
    let mut unplaced = None;
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
            // Shown after the selection areas: across controls the later one is on top of a press, and a border
            // beats the area it lies in (the engine's `target_at` answers none where a border is).
            crate::table_mouse::show(app, ui, layout, body);
            crate::table_resize::show(app, ui, layout, body);
            crate::ink::show(app, ui, layout, body);
            crate::dialogs_smart_art::frame_button(app, ui, body.pages, layout);
            if let Some(pass) = body.pass {
                ran = true;
                tool_response = Some(run_page_tool(app, ui, layout, body, pass));
                caret::paint_caret(app, ui, pass, body, Env::of(ui).motion.mode);
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
                            unplaced = Some((c.page, h, *c));
                        }
                    }
                }
                // The mini toolbar's anchor is settled in the pass that shows it. Anchor only to visible
                // selection geometry, and to the live selection while a preview moves the text.
                let anchor_rects =
                    if app.canvas.command_previewed.is_some() { live_selection_screen_rects(app, body.pages) } else { screen_rects(&selected, body.pages) };
                app.canvas.mini_anchor = crate::mini_toolbar::visible_anchor(anchor_rects, body.view.rect);
                // The bar is its own floating layer: shown after the host, it keeps its buttons' presses (S-L410).
                crate::mini_toolbar::show(app, ui, body);
            }
        });
    let caret = caret_after.or(caret);
    app.canvas.page_keys = source.into_keys();
    // The caret's page was not placed this pass: the whole layout says where it lies on screen, so the follow moves the
    // view by the same margin rule as for a placed page, from whichever side the caret is on.
    if let Some((page, page_h, c)) = unplaced
        && let Some(rect) = laid_rects(&out).get(page).copied()
    {
        let s = rect.height() / page_h;
        let top_left = rect.min + vec2(c.x * s, c.top * s);
        let r = Rect::from_min_size(top_left, vec2((c.width * s).max(2.0), c.height * s)).expand2(vec2(40.0, 60.0));
        fix.delta = (overshoot((r.min.x, r.max.x), (body_rect.min.x, body_rect.max.x)), overshoot((r.min.y, r.max.y), (body_rect.min.y, body_rect.max.y)));
        fix.go_to = None;
    }
    // A jump to a page that was not placed at all (`go_to`) puts the caret's line at the view's top edge: the follow
    // stays on for the next pass, where the page is placed, so the caret ends with the same margin as any other follow.
    // While the body's rect is still settling (the chrome around the page is measured over the first passes) the follow
    // waits, and moves the view once against the rect the page keeps: a move made against a passing rect would leave the
    // caret where that rect put it.
    if ran && app.canvas.shown.rect != body_rect {
        fix = ScrollFix::default();
        ui.ctx().request_repaint();
    } else if ran && fix.go_to.is_none() {
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
    // A window's page has the keyboard from its first pass (production's `want_focus` started on), so typing lands
    // without a click; a modal dialog keeps the keyboard until it closes.
    let view_rect = body_view.map_or(body_rect, |v| v.rect);
    app.canvas.shown =
        ShownPages { rect: body_rect, pages: out.pages.clone(), laid: laid_rects(&out), view: view_rect, canvas: Some(out.canvas.response.id) };
    if app.canvas.focus_pending && !app.modal_open() {
        let ctx = ui.ctx().clone();
        app.canvas.focus_page(&ctx);
    }
    app.canvas.resident_pictures = out.resident_pictures();
    app.canvas.view = state;
    if out.toggle_white_space {
        let _ = app.run("view.hideWhiteSpace", json!({"value": !app.session.view.hide_white_space}));
    }

    dismiss_balloon_on_canvas_click(app, &out);
    // A press in the document brings back a mini toolbar a click outside it put away (S-L414).
    if let craft::behavior::PressPhase::Down { start, .. } = &out.canvas.press.phase
        && start.button == craft::behavior::PointerButton::Primary
    {
        app.canvas.mini_dismissed = None;
    }
    balloons::balloons(app, ui, &out, layout);
    rulers::page_rulers(app, ui, &out, layout);
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
=======
/// Render the document canvas, pages, scroll bars, rulers, and interactive controls for the current frame.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let layout = prepare_layout(app, ui, full);
    let view = prepare_view(app, ui, full, &layout);
    let scroll_target = if hold_scroll_for_preview(app) { None } else { update_caret_scroll(app, &view) };
    let motion = scroll_motion(app, ui, full, &view);
    let (scroll_area, scroll_base) = configure_scroll_area(app, ui, &view, motion);
    let CanvasView { area, hruler, vruler, paper, zooming, hinted, geo, dim_body, grid_story, caret, rescaled, .. } = view;
    let mut origin = area.min;
    let mut ui_area = ui.new_child(egui::UiBuilder::new().max_rect(area));
    // The bars are drawn with `ui_area`'s style; what the pages draw gets the inherited one back.
    let inherited = std::sync::Arc::clone(ui_area.style());
    if ui.spacing().scroll == egui::style::ScrollStyle::floating() {
        ui_area.spacing_mut().scroll = canvas_scroll_style();
    }
    canvas_scroll_thumb(ui_area.visuals_mut(), &t);
    let mut ime_geometry: Option<(Rect, Rect)> = None;
    let mut ime_interrupt = false;
    let out = scroll_area.show_viewport(&mut ui_area, |ui, viewport| {
        ui.set_style(inherited);
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
        let page_view = view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p), hinted);
        // The view without the hinting, which a stretched raster may differ in (see `stretched`).
        let shown_view = view_key(app, dim_body, hidden.as_ref().map(|(_, p)| p), true);
        let t0 = crate::now_ms();
        let PagePaintOutput { rects, visible_tile_keys, rendered } = paint_pages(PagePaint {
            app,
            ui,
            layout: &layout,
            geo: &geo,
            t: &t,
            painter: &painter,
            dim_body,
            paper,
            hidden,
            view: page_view,
            shown_view,
            zooming,
            hinted,
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
            egui::Area::new(egui::Id::unique("floating_comment"))
                .order(egui::Order::Foreground)
                .interactable(!app.modal_open())
                .fixed_pos(Pos2::ZERO)
                .sense(Sense::hover())
                .show(&ctx, |aui| {
                    aui.set_clip_rect(view_rect);
                    let ap = aui.painter().clone();
                    balloons(app, aui, &ap, &rects, &layout, geo.scale, view_rect);
                });
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
        // Alignment previews can move the selected text while the pointer is still over a mini
        // toolbar button. Keep the bar anchored to the live selection until the preview ends, or
        // it can move out from under the pointer and flicker between preview and live layouts.
        let anchor_rects = if app.canvas.command_previewed.is_some() { live_selection_screen_rects(app, &rects, geo.scale) } else { screen_rects };
        app.canvas.mini_anchor = crate::mini_toolbar::visible_anchor(anchor_rects, visible_clip);
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
                let color = caret_color(app);
                let at = CompositionAt { pr: *pr, scale, x, y1, height: c.height, turned: c.width > 0.0, color, max_tex, ppp };
                ime_dx = paint_composition(app, ui.ctx(), &painter, &layout, &at);
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
    let scroll_style = ui_area.spacing().scroll;
    finish_canvas_frame(
        CanvasFinish {
            app,
            ui,
            layout: &layout,
            geo: &geo,
            caret,
            ime_geometry,
            ime_interrupt,
            area,
            dim_body,
            hruler,
            vruler,
            scroll_base,
            rescaled,
            scroll_style,
        },
        out,
    );
}

/// Where [`paint_composition`] draws an input method's composition: from the caret at `x` on the
/// page drawn at `pr` (`scale` screen points to the point), whose bottom is at `y1` and whose
/// height is `height` points, across the page in turned text. `max_tex` and `ppp` are the
/// texture side and pixels per point the composition's glyphs must fit.
struct CompositionAt {
    pr: Rect,
    scale: f32,
    x: f32,
    y1: f32,
    height: f32,
    turned: bool,
    color: Color32,
    max_tex: f32,
    ppp: f32,
}

/// Draw the input method's composition and return how far into it the input method's cursor
/// lies, in screen points.
fn paint_composition(app: &mut WordApp, ctx: &egui::Context, painter: &egui::Painter, layout: &DocLayout, at: &CompositionAt) -> f32 {
    // The composition is interface text, but it may be in a script the interface fonts lack
    // (Bangla, Thai ...): borrow the document fonts' fallback faces, or it draws as boxes (#565).
    // CJK uses the CJK UI font, as the window caption does: the faces a document falls back
    // to for it are mostly too large to borrow (Microsoft YaHei on Windows is 19 MB), and a
    // build without craft-fonts has no CJK interface face of its own.
    if !app.want_system_cjk && app.canvas.ime_preedit.chars().any(wordcraft_fonts::is_cjk) {
        app.want_system_cjk = true;
        ctx.request_repaint();
    }
    let new = crate::theme::faces_for_preedit(&app.canvas.ime_preedit, &app.canvas.ime_faces, crate::theme::caption_char_covered, |c| {
        wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
    });
    // Word draws the composition in the font and size of the text it goes into: take the
    // face, size and baseline the layout gives text typed here (not in turned text). The
    // face is the one the run's font asks for, not a fallback face that drew the
    // character before the caret (a Chinese character in a Latin font).
    let typing = if at.turned { None } else { layout.typing_style(&app.session.sel.focus, app.session.page_hint) };
    let face = typing.as_ref().map(|(run, _)| run.requested_face);
    if !new.is_empty() || face != app.canvas.ime_face {
        app.canvas.ime_faces.extend(new);
        app.canvas.ime_face = face;
        // With every other window's faces too: the fonts are the app's.
        app.apply_ui_fonts(ctx);
        ctx.request_repaint();
    }
    // New fonts apply from the next frame; until the window's composition family is bound (or
    // without a style run) the composition is drawn as before, in the interface font.
    let family = crate::theme::composition_family(app.windows.shown());
    let bound = ctx.fonts(|f| f.definitions().families.contains_key(&family));
    // egui draws each glyph into its font atlas and panics on one wider than the atlas
    // (1,638-point text at 500% is about 11,000 points): keep the composition's glyphs
    // within a quarter of the atlas side, and at most 1,024 pixels, as each glyph stays
    // in the atlas. Larger text gets a smaller composition.
    let most = (at.max_tex / 4.0).min(1024.0) / at.ppp;
    let size = |s: f32| s.min(most).max(1.0);
    let (font, base) = match &typing {
        Some((run, base)) if bound && run.size.is_finite() && run.size > 0.0 => {
            (egui::FontId::new(size(run.size * at.scale), family), Some(at.pr.min.y + base * at.scale))
        }
        _ => (regular(size(at.height * at.scale * 0.8)), None),
    };
    let g = painter.layout_no_wrap(app.canvas.ime_preedit.clone(), font, Color32::PLACEHOLDER);
    let cursor = preedit_cursor_chars(&app.canvas.ime_preedit, app.canvas.ime_preedit_range.as_ref());
    let dx = g.pos_from_cursor(egui::text::CCursor::new(cursor)).min.x;
    // The galley's first baseline sits on the line's baseline, as typed text would.
    let baseline = g.rows.first().and_then(|r| r.glyphs.first().map(|gl| r.pos.y + gl.pos.y)).unwrap_or(g.size().y);
    let top = base.map_or(at.y1 - g.size().y, |b| b - baseline);
    let r = Rect::from_min_size(pos2(at.x, top), g.size());
    painter.galley(r.min, g, at.color);
    painter.line_segment([pos2(r.min.x, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, at.color));
    dx
}

#[cfg(test)]
mod tests;
>>>>>>> up/main
