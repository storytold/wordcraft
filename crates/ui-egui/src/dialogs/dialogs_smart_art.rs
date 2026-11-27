//! Insert › SmartArt (a picker of layouts by category, with previews drawn in code) and the
//! SmartArt Text Pane (the selected graphic's items as an editable outline). Each change runs one
//! command (`insert.smartArt`, `smartArt.items`, `smartArt.addShape`…), so agents get the same
//! result without the UI. The picker opens by name through [`open`]; the pane is shown through
//! [`crate::panes`].

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use craft::behavior::kurbo::Point;
use craft::behavior::overlay::Side;
use craft::behavior::{CancelReason, EditEvent, EditEvents, GestureId, IconId, Mixed, Selection, SelectionChange, keyed};
use craft::pages::ui::PlacedPage;
use craft::theme::Srgba;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas::canvas_space;
use craft::ui::canvas_controls::{TaskBar, TaskBarDismiss};
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
// PLAN: assumed `FormCx::accept` (dialogs-w5, S-L285)
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::primitives::{ContentPainterCx, DataWidth, Draw, PaintShape};
use craft::ui::runtime::edit::next_gesture_id;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::focus::{self, FocusTarget};
// PLAN: assumed `RuleKind` re-exported from `craft::ui::style`
use craft::ui::style::{DataColor, RuleKind};
use craft::ui::text::TextField;
use craft::ui::tile_grid::{GridZoom, Tile, TileGrid};
use egui::{Color32, CornerRadius, Pos2, Rect, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::smart_art::{MAX_ITEMS, SmartArtColors, SmartArtLayout, SmartArtSpec};

use crate::WordApp;
// PLAN: assumed dg.A5 `colors::{color_field, ColorFieldEmpty, AutoFace, Palette}` as the contract row gives them (ribbon-w11)
use crate::colors::{AutoFace, ColorFieldEmpty, Palette, color_field};
use crate::dialogs::host::{Asks, DialogHooks, Settle, WordForm, open_form, open_with};

/// The picker's categories (`All` first), in the order shown.
pub const CATEGORIES: [&str; 8] = ["All", "List", "Process", "Cycle", "Hierarchy", "Relationship", "Matrix", "Pyramid"];

/// Insert › SmartArt: the category shown and the layout picked.
// PLAN: assumed `SmartArtLayout: Deserialize` (wordcraft-doc)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartArtForm {
    pub category: String,
    pub layout: SmartArtLayout,
    /// The document's theme colours for the previews; filled from the app by `prepare` each pass.
    #[serde(skip)]
    theme: Vec<Rgb>,
}

impl Default for SmartArtForm {
    fn default() -> Self {
        SmartArtForm { category: "All".into(), layout: SmartArtLayout::default(), theme: Vec::new() }
    }
}

impl WordForm for SmartArtForm {
    fn key(&self) -> &'static str {
        "insertSmartArt"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Insert Diagram").to_string())
    }

    fn prepare(&mut self, app: &WordApp) {
        self.theme = app.session.doc.settings.theme_colors.clone();
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let theme = self.theme.clone();
        // A double click or Enter on a layout is the OK press.
        if picker(ui, &theme, self) {
            cx.accept();
        }
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("insert.smartArt".to_string(), json!({"layout": self.layout.id()}))]
    }
}

impl SmartArtForm {
    pub fn new(_app: &WordApp) -> SmartArtForm {
        SmartArtForm::default()
    }

    /// The layouts the current category shows.
    pub fn shown(&self) -> Vec<SmartArtLayout> {
        SmartArtLayout::ALL.into_iter().filter(|l| self.category == "All" || l.category() == self.category).collect()
    }

    /// Shows `category`; a layout it does not list gives way to its first.
    fn pick_category(&mut self, category: &str) {
        self.category = category.to_string();
        if !self.shown().contains(&self.layout)
            && let Some(first) = self.shown().first()
        {
            self.layout = *first;
        }
    }
}

/// Theme accent `n` (1–6) of the document.
fn accent_rgb(theme: &[Rgb], n: usize) -> Rgb {
    theme.get(3 + n.clamp(1, 6)).copied().unwrap_or(Rgb(0x15, 0x60, 0x82))
}

/// Theme accent `n` (1–6) of the document, as the colour a painter takes.
pub fn accent(theme: &[Rgb], n: usize) -> DataColor {
    DataColor::new(color32(accent_rgb(theme, n)))
}

/// A document colour as the colour a painter takes.
fn color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// A document colour as the theme library's colour.
fn srgba(c: Rgb) -> Srgba {
    Srgba::rgb(f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0)
}

/// Theme accent `n` at `share` of its opacity.
fn faded(theme: &[Rgb], n: usize, share: f32) -> DataColor {
    DataColor::new(color32(accent_rgb(theme, n)).gamma_multiply(share))
}

/// A small picture of `layout` in `r`, in the colours `colors` takes from `theme`; connector
/// lines in `line`.
pub fn preview(cx: &ContentPainterCx, r: Rect, layout: SmartArtLayout, colors: SmartArtColors, theme: &[Rgb], line: Color32) {
    cx.with_draw(|d| paint_preview(d, r, layout, colors, theme, line));
}

/// [`preview`] through a `Draw`.
fn paint_preview(d: &Draw<'_>, r: Rect, layout: SmartArtLayout, colors: SmartArtColors, theme: &[Rgb], line: Color32) {
    let r = r.shrink(r.width().min(r.height()) * 0.08);
    let s = r.width().min(r.height());
    let c = r.center();
    let col = |i: usize| accent(theme, colors.accent(i));
    // Unit-square coordinates (of the square in the middle of `r`) to screen.
    let at = |x: f32, y: f32| pos2(c.x + (x - 0.5) * s, c.y + (y - 0.5) * s);
    let block = |x0: f32, y0: f32, x1: f32, y1: f32, k: usize, round: f32| {
        let corners = CornerRadius::same(round.round().clamp(0.0, 255.0) as u8);
        d.data_fill(&PaintShape::Rect(Rect::from_min_max(at(x0, y0), at(x1, y1)), corners), col(k));
    };
    let thin = DataWidth::points(1.0);
    let segment = |a: Pos2, b: Pos2| d.stroke(&PaintShape::Path(vec![a, b]), thin, DataColor::new(line));
    match layout {
        SmartArtLayout::BasicList => {
            for (k, (x, y)) in [(0.0, 0.18), (0.35, 0.18), (0.7, 0.18), (0.175, 0.55), (0.525, 0.55)].into_iter().enumerate() {
                block(x, y, x + 0.3, y + 0.27, k, 0.0);
            }
        }
        SmartArtLayout::Process => {
            for k in 0..3 {
                let x = k as f32 * 0.37;
                block(x, 0.36, x + 0.26, 0.64, k, 3.0);
                if k < 2 {
                    let ax = x + 0.285;
                    d.data_fill(&PaintShape::Polygon(vec![at(ax, 0.45), at(ax + 0.06, 0.5), at(ax, 0.55)]), faded(theme, colors.accent(k), 0.6));
                }
            }
        }
        SmartArtLayout::Cycle => {
            d.stroke(&PaintShape::Circle(c, s * 0.34), thin, DataColor::new(line));
            for k in 0..5 {
                let a = (-90.0 + 72.0 * k as f32).to_radians();
                d.data_fill(&PaintShape::Circle(c + vec2(a.cos(), a.sin()) * s * 0.34, s * 0.12), col(k));
            }
        }
        SmartArtLayout::Hierarchy => {
            segment(at(0.5, 0.3), at(0.5, 0.45));
            segment(at(0.17, 0.45), at(0.83, 0.45));
            for x in [0.17, 0.5, 0.83] {
                segment(at(x, 0.45), at(x, 0.6));
            }
            block(0.33, 0.08, 0.67, 0.3, 0, 3.0);
            for (k, x) in [0.02, 0.35, 0.68].into_iter().enumerate() {
                block(x, 0.6, x + 0.3, 0.82, k + 1, 3.0);
            }
        }
        SmartArtLayout::Pyramid => {
            // Half the pyramid's width at height `y` (apex at 0.05, base at 0.95).
            let hw = |y: f32| 0.45 * (y - 0.05) / 0.9;
            for (k, (y0, y1)) in [(0.05, 0.37), (0.37, 0.66), (0.66, 0.95)].into_iter().enumerate() {
                let pts = if k == 0 {
                    vec![at(0.5, y0), at(0.5 + hw(y1), y1), at(0.5 - hw(y1), y1)]
                } else {
                    vec![at(0.5 - hw(y0), y0), at(0.5 + hw(y0), y0), at(0.5 + hw(y1), y1), at(0.5 - hw(y1), y1)]
                };
                let shape = PaintShape::Polygon(pts);
                d.data_fill(&shape, col(k));
                d.stroke(&shape, thin, DataColor::new(line));
            }
        }
        SmartArtLayout::Radial => {
            let spokes: Vec<Pos2> = (0..4)
                .map(|k| {
                    let a = (-90.0 + 90.0 * k as f32).to_radians();
                    c + vec2(a.cos(), a.sin()) * s * 0.36
                })
                .collect();
            for q in &spokes {
                segment(c, *q);
            }
            d.data_fill(&PaintShape::Circle(c, s * 0.17), col(0));
            for (k, q) in spokes.into_iter().enumerate() {
                d.data_fill(&PaintShape::Circle(q, s * 0.12), col(k + 1));
            }
        }
        SmartArtLayout::Matrix => {
            for k in 0..4 {
                let (x, y) = ((k % 2) as f32 * 0.5 + 0.03, (k / 2) as f32 * 0.5 + 0.03);
                block(x, y, x + 0.44, y + 0.44, k, 3.0);
            }
        }
        SmartArtLayout::Venn => {
            for k in 0..3 {
                let a = (-90.0 + 120.0 * k as f32).to_radians();
                let q = c + vec2(a.cos(), a.sin()) * s * 0.15;
                d.data_fill(&PaintShape::Circle(q, s * 0.3), faded(theme, colors.accent(k), 0.5));
                d.stroke(&PaintShape::Circle(q, s * 0.3), thin, col(k));
            }
        }
    }
}

/// A colour variant's name in the interface language.
pub fn colors_label(c: SmartArtColors) -> String {
    match c {
        SmartArtColors::Colorful => tl!("Colorful").to_string(),
        _ => crate::i18n::fmt(tl!("Accent {n}"), &[("n", &c.accent(0).to_string())]),
    }
}

/// Swatches of a colour variant in `r`: the accents its shapes take.
pub fn swatches(cx: &ContentPainterCx, r: Rect, colors: SmartArtColors, theme: &[Rgb]) {
    let n = if colors == SmartArtColors::Colorful { 5 } else { 3 };
    let cells: Vec<DataColor> = (0..n).map(|k| accent(theme, colors.accent(k))).collect();
    paint_strip(cx, r, cells);
}

/// Swatches in `r` of the colours the items of `g` have now: their own or the style's.
pub fn custom_swatches(cx: &ContentPainterCx, r: Rect, g: &SmartArtSpec, theme: &[Rgb]) {
    let shown: Vec<DataColor> = (0..g.items.len())
        .filter_map(|i| match g.items.get(i)?.color {
            Some(c) => Some(DataColor::new(color32(c))),
            None => Some(accent(theme, g.style_accent(i)?)),
        })
        .take(5)
        .collect();
    paint_strip(cx, r, shown);
}

/// `cells` side by side across `r`, a gap between them.
fn paint_strip(cx: &ContentPainterCx, r: Rect, cells: Vec<DataColor>) {
    let w = r.width() / cells.len().max(1) as f32;
    cx.with_draw(|d| {
        for (k, color) in cells.into_iter().enumerate() {
            let x = r.left() + k as f32 * w;
            let cell = Rect::from_min_size(pos2(x, r.top()), vec2(w, r.height()));
            d.data_fill(&PaintShape::Rect(cell, CornerRadius::ZERO), color);
            // The theme's divider separates neighbours, so no spacing literal is needed.
            if k > 0 {
                d.rule(RuleKind::Divider, pos2(x, r.top()), pos2(x, r.bottom()));
            }
        }
    });
}

/// The picker's three columns: categories, the layouts of the category and the picked layout's preview. True when a
/// layout was activated (double click or Enter).
fn picker(ui: &mut Ui, theme: &[Rgb], form: &mut SmartArtForm) -> bool {
    let mut activated = false;
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| category_list(ui, form));
        Stack::new().show(ui, |ui, _| activated |= layout_tiles(ui, theme, form));
        Stack::new().show(ui, |ui, _| layout_detail(ui, theme, form));
    });
    activated
}

fn category_list(ui: &mut Ui, form: &mut SmartArtForm) {
    let positions: Vec<usize> = (0..CATEGORIES.len()).collect();
    let current = CATEGORIES.iter().position(|c| *c == form.category).unwrap_or(0);
    let out = List::<usize>::new(tl!("Categories").to_string())
        .select(SelectMode::Single)
        .natural_height(true)
        .selection(Bind::given(Selection::single(current)))
        .show_items(ui, &keyed(&positions, |i| *i), &mut |key, _| ListRow::new(tl!(CATEGORIES.get(*key).copied().unwrap_or("All")).to_string()));
    for event in out.events {
        let picked = match event {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.last().copied(),
            CollectionEvent::Selection { change: SelectionChange::Toggle(key), .. } => Some(key),
            _ => None,
        };
        if let Some(category) = picked.and_then(|k| CATEGORIES.get(k)) {
            form.pick_category(category);
        }
    }
}

fn layout_tiles(ui: &mut Ui, theme: &[Rgb], form: &mut SmartArtForm) -> bool {
    let shown = form.shown();
    let positions: Vec<usize> = (0..shown.len()).collect();
    let current = shown.iter().position(|l| *l == form.layout).unwrap_or(0);
    let out = TileGrid::<usize>::new(tl!("Layouts").to_string())
        .thumb_size(ThumbSize::Large)
        .zoom(GridZoom::Fixed(1.4))
        .select(SelectMode::Single)
        .card_columns(3)
        // Production's picker is three tile rows high and scrolls the rest; the cap also makes the grid the one owner
        // of its scroll inside the dialog's scrolling body.
        .max_rows(3)
        .selection(Bind::given(Selection::single(current)))
        .show_items(ui, &keyed(&positions, |i| *i), &mut |key, _| {
            let layout = shown.get(*key).copied().unwrap_or_default();
            Tile::new(tl!(layout.label()).to_string()).thumb(ThumbAspect::SQUARE, move |cx, rect| {
                // PLAN: `ContentPainterCx` has no muted-ink accessor; the plain ink until it gains one
                preview(cx, rect, layout, SmartArtColors::default(), theme, cx.ink());
            })
        });
    let mut activated = false;
    for event in out.response.events {
        match event {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                if let Some(layout) = keys.last().and_then(|k| shown.get(*k)) {
                    form.layout = *layout;
                }
            }
            CollectionEvent::Selection { change: SelectionChange::Toggle(key), .. } => {
                if let Some(layout) = shown.get(key) {
                    form.layout = *layout;
                }
            }
            CollectionEvent::Activate(key) => {
                if let Some(layout) = shown.get(key) {
                    form.layout = *layout;
                }
                activated = true;
            }
            _ => {}
        }
    }
    activated
}

fn layout_detail(ui: &mut Ui, theme: &[Rgb], form: &SmartArtForm) {
    let layout = form.layout;
    Picture::painter(tl!(layout.label()).to_string(), ThumbAspect::new(4, 3), move |cx, rect| {
        preview(cx, rect, layout, SmartArtColors::default(), theme, cx.ink());
    })
    .extent(PictureSize::Sample(ThumbSize::Large))
    .show(ui);
    Text::new(tl!(layout.label()).to_string()).show(ui);
    Text::new(tl!(layout.description()).to_string()).show(ui);
}

/// Opens the SmartArt picker for the name `insertSmartArt`; `None` for another name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    if name != "insertSmartArt" {
        return None;
    }
    Some(open_form(app, parent, SmartArtForm::new(app)))
}

/// The `smartArt.items` params for `g` with item `i`'s text replaced by `text`.
pub fn items_with(g: &SmartArtSpec, i: usize, text: &str) -> Value {
    let items: Vec<Value> = g
        .items
        .iter()
        .enumerate()
        .map(|(j, it)| json!({"text": if j == i { text } else { it.text.as_str() }, "level": it.level, "color": it.color.map(|c| c.hex())}))
        .collect();
    json!({"items": items})
}

/// The `smartArt.items` params for `g` without item `i` (the items under it move up a level).
pub fn items_without(g: &SmartArtSpec, i: usize) -> Value {
    let under = g.descendants(i);
    let items: Vec<Value> = g
        .items
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(j, it)| {
            json!({"text": it.text, "level": if under.contains(&j) { it.level.saturating_sub(1) } else { it.level }, "color": it.color.map(|c| c.hex())})
        })
        .collect();
    json!({"items": items})
}

/// Sets item `i` of the selected graphic to `text`: the one way typing, in the pane or on the canvas, changes an item.
fn set_item_text(app: &mut WordApp, i: usize, text: &str) -> Result<(), String> {
    let g = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).ok_or_else(|| "no graphic is selected".to_string())?;
    app.run("smartArt.items", items_with(&g, i, text)).map(|_| ())
}

/// The Text Pane button on the left edge of the selected graphic's frame, as in Word, while
/// the pane is closed.
pub fn frame_button(app: &mut WordApp, ui: &mut Ui, pages: &[PlacedPage], layout: &wordcraft_layout::DocLayout) {
    // The pane goes with the graphic: it opens again only on request.
    if wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_none() {
        app.session.view.smart_art_pane = false;
        return;
    }
    if app.session.view.smart_art_pane {
        return;
    }
    let Some((hit, _)) = crate::objects::active(app, layout) else { return };
    // `pages` is placed by the page view this body runs in, so its index is the layout's page index.
    let Some(page) = pages.iter().find(|p| p.page == hit.page) else { return };
    // The bar anchors in the canvas's document points: page points go through the page to that view.
    let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
    let r = hit.rect;
    let corners = [
        page.to_doc(&space.view, Point::new(f64::from(r.x), f64::from(r.y))),
        page.to_doc(&space.view, Point::new(f64::from(r.x), f64::from(r.bottom()))),
    ];
    let area = ui.max_rect();
    let shown = TaskBar::selection(tl!("Text Pane").to_string(), &corners)
        .side(Side::Start, craft::behavior::overlay::Align::Center)
        .dismiss(TaskBarDismiss::Keep)
        .show(ui, area, |ui, _| IconButton::new(IconId::new("textPane")).label(tl!("Text Pane").to_string()).show(ui).clicked());
    if shown.inner == Some(true) {
        let _ = app.run("smartArt.textPane", json!({"value": true}));
    }
}

/// Text typed with a SmartArt graphic selected goes at the end of its current item (the first,
/// before any is picked), and the Text Pane opens, as Word does; it never replaces the graphic.
///
/// What typing into a selected SmartArt graphic did.
///
/// A selected graphic always consumes the input, even when it has no item to edit: the typing
/// must not fall through into the body. Only the absence of a SmartArt target lets the body type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmartArtTypeResult {
    /// No SmartArt graphic is selected: the caller may type into the body.
    NotHandled,
    /// A selected graphic consumed the input without editing the document.
    ConsumedNoChange,
    /// A SmartArt item really changed.
    Changed,
}

/// A run of typing into one item: one undo step from the first keystroke until the caret leaves the item.
// PLAN: assumed `CanvasState::smart_art_run: Option<TypingRun>` (canvas-w9); canvas-w10 calls `end_typing_run` once per canvas pass
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TypingRun {
    /// The gesture the run's edits belong to.
    id: GestureId,
    /// The item typed into.
    item: usize,
    /// The item's text when the run began.
    before: String,
}

/// The item typing and the pane's buttons act on: the one picked, else the first.
fn current_item(app: &WordApp, g: &SmartArtSpec) -> Option<usize> {
    app.smart_art_item.filter(|i| *i < g.items.len()).or((!g.items.is_empty()).then_some(0))
}

/// Appends typing to the selected SmartArt item, as one typing run, and returns whether the input was unhandled,
/// consumed without a change, or changed an item.
// PLAN: `type_into` takes the context the run's gesture id comes from (`next_gesture_id`); canvas-w10 passes it at `keys/canvas.rs:439,540`
pub(crate) fn type_into(app: &mut WordApp, ctx: &egui::Context, typed: &str) -> SmartArtTypeResult {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else {
        return SmartArtTypeResult::NotHandled;
    };
    let Some(i) = current_item(app, &g) else {
        // Selected but empty: consume, never insert into the body, never claim an edit.
        return SmartArtTypeResult::ConsumedNoChange;
    };
    // Typing in another item ends the run of the first.
    if app.canvas.smart_art_run.as_ref().is_some_and(|run| run.item != i) {
        close_typing_run(app);
    }
    let before = g.items.get(i).map_or(String::new(), |it| it.text.clone());
    let text = before.clone() + typed;
    let mut events = EditEvents::new();
    let id = match &app.canvas.smart_art_run {
        Some(run) => run.id,
        None => {
            let id = next_gesture_id(ctx);
            events.push(EditEvent::begin(id, Mixed::Same(before.clone()), Arc::from(tl!("Typing")), None));
            app.canvas.smart_art_run = Some(TypingRun { id, item: i, before: before.clone() });
            id
        }
    };
    events.push(EditEvent::update(id, text));
    let refused = Cell::new(false);
    crate::undo_marks::apply_edits(app, events, |app, text: String| {
        if set_item_text(app, i, &text).is_err() {
            refused.set(true);
        }
    });
    if refused.get() {
        // A selected graphic that refuses the edit still consumes the event; an IME composition
        // must start afresh rather than be applied to the body.
        let mut cancel = EditEvents::new();
        cancel.push(EditEvent::cancel(id, CancelReason::Refused, Some(Mixed::Same(before))));
        crate::undo_marks::apply_edits(app, cancel, |_, _: String| {});
        app.canvas.smart_art_run = None;
        app.canvas.ime_interrupt_now = true;
        return SmartArtTypeResult::ConsumedNoChange;
    }
    app.smart_art_item = Some(i);
    let _ = app.run("smartArt.textPane", json!({"value": true}));
    SmartArtTypeResult::Changed
}

/// Ends the open typing run when the caret has left its item (another item is current or no graphic is selected) or
/// the canvas lost focus. Called once per canvas pass with that pass's canvas focus.
pub(crate) fn end_typing_run(app: &mut WordApp, canvas_focused: bool) {
    let Some(run) = &app.canvas.smart_art_run else { return };
    let item = run.item;
    let inside =
        canvas_focused && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some_and(|g| current_item(app, &g) == Some(item));
    if !inside {
        close_typing_run(app);
    }
}

/// Ends the open typing run now, committing the item's text as one undo step.
pub(crate) fn close_typing_run(app: &mut WordApp) {
    let Some(run) = app.canvas.smart_art_run.take() else { return };
    let item = run.item;
    let now = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).and_then(|g| g.items.get(item).map(|it| it.text.clone()));
    let mut events = EditEvents::new();
    match now {
        Some(text) => events.push(EditEvent::commit(run.id, Mixed::Same(run.before), text)),
        None => events.push(EditEvent::cancel(run.id, CancelReason::Removed, Some(Mixed::Same(run.before)))),
    }
    crate::undo_marks::apply_edits(app, events, move |app, text: String| {
        let _ = set_item_text(app, item, &text);
    });
}

/// What a Text Pane row reported in one pass.
struct RowReport {
    index: usize,
    events: EditEvents<String>,
    /// Tab (`Some(true)`) or Shift+Tab (`Some(false)`) pressed in the field.
    tab: Option<bool>,
    submitted: bool,
    focused: bool,
    color: Option<ColorPick>,
}

/// A pick on an item's colour field.
enum ColorPick {
    Set(String),
    Style,
    More,
}

/// The id of item `i`'s text row, stable across passes so a focus request can name a row before it exists.
fn row_id(i: usize) -> egui::Id {
    egui::Id::unique(("smart_art_pane_row", i))
}

/// The name of item `i`'s colour field: its shape's colour.
fn color_name(g: &SmartArtSpec, i: usize) -> String {
    g.items
        .get(i)
        .filter(|it| !it.text.is_empty())
        .map_or_else(|| format!("{} ({})", tl!("Shape color"), i + 1), |it| format!("{}: {}", tl!("Shape color"), it.text))
}

/// The Text Pane: a row per item, indented by level. Typing changes the graphic as you go (one
/// undo step per item typed in); Enter adds an item after the current one; Tab demotes the
/// focused item and Shift+Tab promotes it, as Word's Text Pane does.
pub fn text_pane(app: &mut WordApp, ui: &mut Ui) {
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let theme = app.session.doc.settings.theme_colors.clone();
    // PLAN: assumed Palette::of (ribbon-w11, S-L284)
    let palette = Palette::of(app);
    let reports = RefCell::new(Vec::<RowReport>::new());
    Text::new(tl!("One line per item. Enter adds an item.").to_string()).show(ui);
    Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| {
        // The body may run twice in a pass; the last run's reports are the pass's.
        reports.borrow_mut().clear();
        for (i, it) in g.items.iter().enumerate() {
            Row::new().indent(it.level).show(ui, |ui, _| {
                let field = TextField::new(format!("{} {}", tl!("Text Pane"), i + 1), Bind::given(it.text.clone()))
                    .id(row_id(i))
                    .tab_reports()
                    .submits()
                    .show(ui);
                let color = g.style_accent(i).and_then(|n| {
                    let own = it.color.map(|c| c.hex()).unwrap_or_default();
                    let mut value = own.clone();
                    let empty = ColorFieldEmpty::Automatic {
                        label: Label::from(tl!("Style Color").to_string()),
                        value: "",
                        face: AutoFace::One(srgba(accent_rgb(&theme, n))),
                    };
                    let picked = color_field(ui, &Label::from(color_name(&g, i)), &mut value, &empty, &palette);
                    if picked.more_colors {
                        Some(ColorPick::More)
                    } else if picked.changed && value.is_empty() {
                        Some(ColorPick::Style)
                    } else if picked.changed && value != own {
                        Some(ColorPick::Set(value))
                    } else {
                        None
                    }
                });
                reports.borrow_mut().push(RowReport {
                    index: i,
                    tab: field.tab,
                    submitted: field.submitted,
                    // A text part's egui widget is not its component's id, so the response's own focus flag stays
                    // false: ask the framework's focus which stop holds the keyboard.
                    focused: field.response.has_focus()
                        || field.response.gained_focus()
                        || focus::focused_rec(ui.ctx()).is_some_and(|rec| rec.stop.id == row_id(i)),
                    events: field.events,
                    color,
                });
            });
        }
    });

    let mut add_after = None;
    for report in reports.into_inner() {
        let i = report.index;
        if report.focused {
            app.smart_art_item = Some(i);
        }
        crate::undo_marks::apply_edits(app, report.events, move |app, text: String| {
            let _ = set_item_text(app, i, &text);
        });
        match report.tab {
            Some(true) => {
                let _ = app.run("smartArt.demote", json!({"index": i}));
            }
            Some(false) => {
                let _ = app.run("smartArt.promote", json!({"index": i}));
            }
            None => {}
        }
        if report.submitted && g.items.len() < MAX_ITEMS {
            add_after = Some(i);
        }
        match report.color {
            Some(ColorPick::Set(hex)) => {
                let _ = app.run("smartArt.itemColor", json!({"index": i, "color": hex}));
            }
            Some(ColorPick::Style) => {
                let _ = app.run("smartArt.itemColor", json!({"index": i}));
            }
            Some(ColorPick::More) => more_colors(app, i, g.items.get(i).and_then(|it| it.color)),
            None => {}
        }
    }
    if let Some(i) = add_after
        && app.run("smartArt.addShape", json!({"after": i})).is_ok()
    {
        // The new item comes after this one and the items under it.
        let next = g.descendants(i).end;
        app.smart_art_item = Some(next);
        focus::request(ui.ctx(), FocusTarget::Id(row_id(next)));
    }

    let current = app.smart_art_item.filter(|i| *i < g.items.len());
    let (mut add, mut promote, mut demote, mut remove) = (false, false, false, false);
    Row::new().show(ui, |ui, _| {
        add = Button::new(tl!("Add").to_string()).enabled(g.items.len() < MAX_ITEMS).show(ui).clicked();
        promote = Button::new(tl!("Promote").to_string()).show(ui).clicked();
        demote = Button::new(tl!("Demote").to_string()).show(ui).clicked();
        remove = Button::new(tl!("Remove").to_string()).enabled(current.is_some() && g.items.len() > 1).show(ui).clicked();
    });
    let index = current.map_or(json!({}), |i| json!({"index": i}));
    if add {
        let _ = app.run("smartArt.addShape", current.map_or(json!({}), |i| json!({"after": i})));
    }
    if promote {
        let _ = app.run("smartArt.promote", index.clone());
    }
    if demote {
        let _ = app.run("smartArt.demote", index);
    }
    if remove
        && let Some(i) = current
        && app.run("smartArt.items", items_without(&g, i)).is_ok()
    {
        app.smart_art_item = i.checked_sub(1);
    }
}

/// Opens More Colors for item `i`'s shape colour; the pick runs `smartArt.itemColor` for that item.
// PLAN: assumed `WordApp::more_colors: HashMap<String, colors::Target>` and `colors::more_colors(current, key, tab) -> ColorPickerDialog` (ribbon-w11, rb.A4)
fn more_colors(app: &mut WordApp, i: usize, current: Option<Rgb>) {
    let key = format!("smartart-item-{i}");
    app.more_colors.insert(key.clone(), crate::colors::Target::at("smartArt.itemColor", json!({"index": i, "color": null}), "/color"));
    let dialog = crate::colors::more_colors(current, key, Default::default());
    open_with(app, None, dialog, DialogHooks { prepare: None, settle: settle_more_colors });
}

/// Routes a finished More Colors dialog to its target; true once the dialog is gone.
fn settle_more_colors(app: &mut WordApp, ctx: &egui::Context, id: DialogId) -> Settle {
    match app.dialog_host.take_by_id::<craft::color_ui::ui::ColorPickerDialog>(id) {
        Some(DialogResult::Ok(result)) => {
            crate::colors::picked(app, ctx, result);
            Settle::Done
        }
        Some(_) => Settle::Done,
        None if app.dialog_host.get_by_id::<craft::color_ui::ui::ColorPickerDialog>(id).is_none() => Settle::Done,
        None => Settle::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_an_item_lifts_the_items_under_it() {
        let g = SmartArtSpec::sample(SmartArtLayout::Hierarchy);
        let v = items_without(&g, 1);
        let levels: Vec<u64> = v["items"].as_array().unwrap().iter().map(|i| i["level"].as_u64().unwrap()).collect();
        assert_eq!(levels, [0, 1, 1, 2, 2]);
        assert_eq!(items_with(&g, 0, "Boss")["items"][0]["text"], "Boss");
    }
}
