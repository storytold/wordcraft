//! The caret and the input method's composition, painted after the tool host ran, so this
//! pass's press and keys have moved the caret before it is looked at. The marks are an app
//! canvas control, so they paint through the pipeline's `Draw` with theme stroke widths.

use craft::behavior::kurbo::Point;
use craft::behavior::motion::MotionMode;
use craft::behavior::{Input, Metrics};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::canvas::ToolPass;
use craft::ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, Pos2, Rect, Response, Ui, pos2, vec2};
use wordcraft_layout::DocLayout;

use super::cursor::{caret_color_at, preedit_cursor_chars};
use super::story::dims_body;
use super::{WordApp, preview_sel};
use crate::caret_glide::{Bar, Look, LookRequest, blink_fading, look};

/// Where the caret stands on screen in this pass's placement, and what the painters need of it.
struct Placement {
    /// The caret's bar, pixel-snapped.
    bar: Bar,
    /// The page the caret is on.
    page: usize,
    /// The page point whose paper colour the caret contrasts with.
    probe: (f32, f32),
    /// The caret's height on screen.
    height: f32,
    /// The caret's left edge before snapping: where a composition starts.
    left: f32,
    /// The caret's bottom edge.
    bottom: f32,
    /// The caret is inside the equation being edited.
    in_equation: bool,
    /// The selection is collapsed, so a caret is drawn.
    collapsed: bool,
}

/// The one rule for where the caret is, for the candidate window and the painter alike. `None`
/// without a caret on a placed page.
fn place(app: &WordApp, doc: &DocLayout, pages: &[PlacedPage]) -> Option<Placement> {
    if let Some(m) = app.session.math.clone()
        && let Some(bx) = doc.equation_box(&m.at, app.session.page_hint)
        && let Some(pr) = pages.iter().find(|p| p.page == bx.page)
        && let Some(slot) = bx.math.slot(&m.pos.path, m.pos.off)
    {
        let s = pr.scale as f32;
        let x = (pr.rect.min.x + (bx.x + slot.x) * s).round() + 0.5;
        let y0 = pr.rect.min.y + (bx.base - slot.y - slot.a) * s;
        let y1 = pr.rect.min.y + (bx.base - slot.y + slot.d) * s;
        return Some(Placement {
            bar: [pos2(x, y0), pos2(x, y1)],
            page: bx.page,
            probe: (bx.x + slot.x, bx.base - slot.y + (slot.d - slot.a) / 2.0),
            height: y1 - y0,
            left: x,
            bottom: y1,
            in_equation: true,
            collapsed: true,
        });
    }
    let sel = preview_sel(app).unwrap_or(&app.session.sel);
    let c = doc.caret_on(&sel.focus, app.session.page_hint)?;
    let pr = pages.iter().find(|p| p.page == c.page)?;
    let scale = pr.scale as f32;
    let x = pr.rect.min.x + c.x * scale;
    let y0 = pr.rect.min.y + c.top * scale;
    let y1 = y0 + c.height * scale;
    let bar = if c.width > 0.0 {
        // Turned text (a table cell's text direction): the caret lies across the page.
        [pos2(x, y0.round() + 0.5), pos2(x + c.width * scale, y0.round() + 0.5)]
    } else {
        [pos2(x.round() + 0.5, y0), pos2(x.round() + 0.5, y1)]
    };
    Some(Placement {
        bar,
        page: c.page,
        probe: (c.x + c.width / 2.0, c.top + c.height / 2.0),
        height: y1 - y0,
        left: x,
        bottom: y1,
        in_equation: false,
        collapsed: sel.is_collapsed(),
    })
}

/// The candidate window's rect for a caret bar: at least a point across each way.
fn report_rect(bar: Bar) -> Rect {
    let r = Rect::from_two_pos(bar[0], bar[1]);
    Rect::from_min_max(r.min, pos2(r.max.x.max(r.min.x + 1.0), r.max.y.max(r.min.y + 1.0)))
}

fn step(app: &mut WordApp, request: LookRequest) -> Look {
    let mut glide = std::mem::take(&mut app.canvas.glide);
    let l = look(&mut glide, app, request);
    app.canvas.glide = glide;
    l
}

/// Paints the caret, its glide and fade, and the composition, over the pages of `body`; reports
/// the editor to the input method with this pass's final caret.
pub(crate) fn paint_caret(app: &mut WordApp, ui: &mut Ui, pass: &ToolPass, body: &PageBody<'_>, motion: MotionMode) {
    let ctx = ui.ctx().clone();
    let focused = app.canvas.focused(&ctx);
    crate::dialogs_smart_art::end_typing_run(app, focused);
    let doc = app.session.layout();
    let Some(pl) = place(app, &doc, body.pages) else { return };
    crate::keys::report_editor(app, pass, &ctx, report_rect(pl.bar));

    let zoom_key = body.view.zoom.to_bits();
    // The glide runs in content space: where the document's origin lands on screen.
    let content_origin = body.view.to_screen(Point::ZERO);
    let now = craft::ui::runtime::pointer::frame(&ctx).time;
    let since_ms = app.canvas.glide.since_restart_ms(craft::platform::clock::monotonic_ms());
    let on = ((since_ms / 530.0) as u64).is_multiple_of(2);
    let dim_body = dims_body(app, &doc);
    let frame_nr = ctx.cumulative_frame_nr();
    let keys = !pass.editor_input.is_empty();
    // PLAN: assumed `ToolPass::press.phase` readable here (`PressPhase::Idle` when no button is involved).
    let pointer = !matches!(pass.press.phase, craft::behavior::press::PressPhase::Idle);
    // The view this pass shows: the page view reports its view on every pass, changed or not, so the glide compares
    // where the document lies on screen and at what zoom with the last pass's.
    let view = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (zoom_key, content_origin.x.to_bits(), content_origin.y.to_bits()).hash(&mut h);
        h.finish()
    };
    let request = |target: Bar, shown: bool| LookRequest {
        content_origin,
        target,
        shown,
        since_ms,
        on,
        now,
        frame_nr,
        viewport: pass.rect,
        keys,
        pointer,
        motion,
        view,
    };
    let ink = |app: &mut WordApp| caret_color_at(app, &doc, pl.page, pl.probe.0, pl.probe.1, zoom_key, dim_body);

    let shown = pl.collapsed && focused;
    let mut bars: Vec<(Bar, Color32)> = Vec::new();
    let mut composition = None;
    if focused && !pl.in_equation && !app.canvas.ime_preedit.is_empty() {
        // The caret is steady inside the composition, so the glide stands down while it shows.
        app.canvas.glide.reset();
        let text = app.canvas.ime_preedit.clone();
        let chars = text.chars().count().max(1);
        let at = preedit_cursor_chars(&text, app.canvas.ime_preedit_range.as_ref());
        composition = Some(Composition {
            left: pl.left,
            bottom: pl.bottom,
            height: pl.height,
            room: (pass.rect.right() - pl.left).max(0.0),
            text,
            // PLAN: `TextLayout` has no cursor lookup; the caret sits at its share of the composition's width (canvas-w1).
            cursor: at as f32 / chars as f32,
            color: ink(app),
        });
    } else {
        let look = step(app, request(pl.bar, shown));
        if shown && look.opacity > 0.0 {
            bars.push((look.bar, ink(app).gamma_multiply(look.opacity)));
        }
        if look.moving {
            ctx.request_repaint();
        } else if shown && blink_fading(since_ms / 1000.0, app.canvas.glide.secs()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if focused {
            ctx.request_repaint_after(std::time::Duration::from_millis(530 - (since_ms as u64 % 530)));
        }
    }
    if !bars.is_empty() || composition.is_some() {
        CaretMarks::new(tl!("Text cursor").to_string(), bars, composition).show(ui);
    }
}

/// An input method's composition drawn from the caret.
struct Composition {
    left: f32,
    bottom: f32,
    height: f32,
    /// The width from `left` to the canvas's right edge.
    room: f32,
    text: String,
    /// Where the caret stands in the composition, as a share of its width.
    cursor: f32,
    color: Color32,
}

/// The style of the caret marks: the composition's ink.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct CaretMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The composition's text.
    ink: StyleColor,
}

/// The caret bars and the composition underline, which contrast with the paper they lie on.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub(crate) struct CaretMarks {
    #[common]
    common: Common,
    bars: Vec<(Bar, Color32)>,
    composition: Option<Composition>,
}

impl CaretMarks {
    /// The marks named `label`: caret bars in screen points with their colours, and a composition.
    fn new(label: impl Into<Label>, bars: Vec<(Bar, Color32)>, composition: Option<Composition>) -> Self {
        CaretMarks { common: Common::new(), bars, composition }.label(label)
    }
}

impl Component for CaretMarks {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = CaretMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: CaretMarksStyleKey) -> CaretMarksStyle {
        // PLAN: a text run takes a theme colour only, so the composition's glyphs use the overlay ink while its underline and caret use the paper contrast (canvas-w1: a data-colour text run). The type role fixes its size.
        CaretMarksStyle { metrics: Metrics::new(), ink: cx.color(cx.tokens.color.overlay.stroke) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for (bar, color) in &self.bars {
            draw.stroke(&PaintShape::Path(bar.to_vec()), StrokeWidth::Medium, DataColor::new(*color));
        }
        let Some(c) = &self.composition else { return };
        let rect = Rect::from_min_size(pos2(c.left, c.bottom - c.height), vec2(c.room, c.height));
        let layout = draw.text(rect, &TextRun::new(c.text.as_str()).color(cx.style.ink));
        let color = DataColor::new(c.color);
        let width = layout.size().x;
        let underline: Vec<Pos2> = vec![pos2(c.left, c.bottom), pos2(c.left + width, c.bottom)];
        draw.stroke(&PaintShape::Path(underline), StrokeWidth::Thin, color);
        let x = (c.left + width * c.cursor).round() + 0.5;
        draw.stroke(&PaintShape::Path(vec![pos2(x, c.bottom - c.height), pos2(x, c.bottom)]), StrokeWidth::Medium, color);
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
