//! The frame around the equation being edited: Word's outline, the grip at its top left, the
//! shading of the argument the caret is in and the options strip inside its right edge, whose
//! click asks for the equation menu (cv.A6's one menu mechanism).

use craft::behavior::kurbo::Point;
use craft::behavior::{CursorIcon, HitLevel, HitPriority, HitShape, Metrics, PartKey, PartRef, PointerButton, PressPhase, Sense};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::contract::{
    Access, Common, Component, Cx, Entry, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _,
};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2, pos2, vec2};
use serde_json::json;

use super::{DocLayout, WordApp};

/// The grip: it takes the press and does nothing, so a press on it stays in the equation.
const GRIP: PartKey = PartKey::new("grip");
/// The options strip: its click asks for the equation menu.
const STRIP: PartKey = PartKey::new("strip");

/// The frame around the equation being edited, on screen: Word's outline, the grip at its top
/// left and the options strip inside its right edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquationFrame {
    pub outline: Rect,
    pub grip: Rect,
    pub strip: Rect,
}

/// The frame's measures in screen points, which stay the same at every zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameSizes {
    /// How far the outline stands off the equation.
    pub pad: f32,
    /// The options strip's width.
    pub strip: f32,
    /// The grip's size; it is centred on the outline's top left corner.
    pub grip: Vec2,
}

/// Lay the frame out around `bx` on `page`: the equation's width and its line's height, plus the
/// strip.
pub(in crate::canvas) fn equation_frame(page: &PlacedPage, bx: &wordcraft_layout::hit::EquationBox, sizes: &FrameSizes) -> EquationFrame {
    let at = |x: f32, y: f32| page.to_screen(Point::new(f64::from(x), f64::from(y)));
    let (top_left, bottom_right) = (at(bx.x, bx.top), at(bx.x + bx.math.width, bx.bottom));
    let (x0, x1) = (top_left.x - sizes.pad, bottom_right.x + sizes.pad);
    let (top, bottom) = (top_left.y - sizes.pad, bottom_right.y + sizes.pad);
    let outline = Rect::from_min_max(pos2(x0, top), pos2(x1 + sizes.strip, bottom));
    let strip = Rect::from_min_max(pos2(x1, top), pos2(x1 + sizes.strip, bottom));
    let grip = Rect::from_center_size(outline.left_top(), sizes.grip);
    EquationFrame { outline, grip, strip }
}

/// The frame's sizes and colours, from the tokens.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct EquationFrameStyle {
    #[style(metrics)]
    metrics: Metrics,
    sizes: FrameSizes,
    radius: f32,
    chevron: f32,
    dot: f32,
    line: StyleColor,
    accent: StyleColor,
    soft: StyleColor,
    handle_fill: StyleColor,
    handle_stroke: StyleColor,
}

/// The paint and parts of the frame: outline, grip, argument shading and the strip that asks for the
/// equation menu.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
struct EquationFrameControl<'a> {
    #[common]
    common: Common,
    page: &'a PlacedPage,
    bx: &'a wordcraft_layout::hit::EquationBox,
    /// The screen rect of the argument the caret is in.
    argument: Option<Rect>,
    clicked: Option<Pos2>,
}

impl<'a> EquationFrameControl<'a> {
    fn new(label: impl Into<Label>, page: &'a PlacedPage, bx: &'a wordcraft_layout::hit::EquationBox, argument: Option<Rect>) -> Self {
        EquationFrameControl { common: Common::new(), page, bx, argument, clicked: None }.label(label)
    }
}

/// What the frame reports: where the strip was clicked, as the menu's anchor.
struct EquationFrameOutput {
    response: Response,
    clicked: Option<Pos2>,
}

impl Outcome for EquationFrameOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for EquationFrameControl<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = EquationFrameStyle;
    type Output = EquationFrameOutput;

    fn style(cx: &StyleCx<'_>, _: EquationFrameControlStyleKey) -> EquationFrameStyle {
        let t = cx.tokens;
        EquationFrameStyle {
            metrics: Metrics::new(),
            sizes: FrameSizes { pad: t.space.gap.tight, strip: t.size.handle.md, grip: vec2(t.size.handle.sm, t.size.control.sm) },
            radius: t.shape.radius.sm,
            chevron: t.size.handle.sm,
            dot: t.stroke.medium,
            line: cx.color(t.color.overlay.line),
            accent: cx.color(t.color.accent.solid),
            soft: cx.color(t.color.accent.soft),
            handle_fill: cx.color(t.color.overlay.handle_fill),
            handle_stroke: cx.color(t.color.overlay.handle_stroke),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let frame = equation_frame(self.page, self.bx, &cx.style.sizes);
        let handle = HitPriority::new(HitLevel::Handle, 0);
        cx.part(PartRef::from(GRIP), HitShape::Rect(frame.grip))
            .sense(Sense::CLICK)
            .priority(handle)
            .cursor(CursorIcon::Default)
            .pointer_only("the grip only keeps a press inside the equation");
        cx.part(PartRef::from(STRIP), HitShape::Rect(frame.strip))
            .sense(Sense::CLICK)
            .priority(handle)
            .cursor(CursorIcon::PointingHand)
            .pointer_only("the context menu key opens the same equation menu");
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let released = cx
            .part_if_declared(PartRef::from(STRIP))
            .is_some_and(|r| matches!(&r.press.phase, PressPhase::Release { start, click: true, .. } if start.button == PointerButton::Primary));
        if released {
            self.clicked = Some(equation_frame(self.page, self.bx, &cx.style.sizes).strip.left_bottom());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let style = cx.style;
        let EquationFrame { outline, grip, strip } = equation_frame(self.page, self.bx, &style.sizes);
        let corner = CornerRadius::from(style.radius);
        if let Some(arg) = self.argument {
            draw.fill(&PaintShape::Rect(arg, CornerRadius::ZERO), style.soft);
        }
        draw.fill(&PaintShape::Rect(strip, CornerRadius::ZERO), style.soft);
        let (c, w) = (strip.center(), style.chevron / 2.0);
        draw.fill(&PaintShape::Polygon(vec![pos2(c.x - w, c.y - w / 2.0), pos2(c.x + w, c.y - w / 2.0), pos2(c.x, c.y + w / 2.0)]), style.accent);
        draw.stroke(&PaintShape::Rect(outline, corner), StrokeWidth::Thin, style.line);
        draw.fill(&PaintShape::Rect(grip, corner), style.handle_fill);
        draw.stroke(&PaintShape::Rect(grip, corner), StrokeWidth::Thin, style.handle_stroke);
        for k in 0..3 {
            let at = pos2(grip.center().x, grip.min.y + grip.height() * (k as f32 + 1.0) / 4.0);
            draw.fill(&PaintShape::Circle(at, style.dot), style.accent);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.node(PartRef::from(STRIP), Role::BUTTON).name = Name::Text(tl!("Equation options").to_string());
    }

    fn finish(self, cx: Cx<'_, Self>) -> EquationFrameOutput {
        EquationFrameOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}

/// Draws the frame of the equation being edited. Returns the strip's bottom left corner when its click
/// asks for the equation menu in this pass; the caller opens the menu.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) -> Option<Pos2> {
    let m = app.session.math.as_ref()?;
    let bx = layout.equation_box(&m.at, app.session.page_hint)?;
    let page = body.pages.iter().find(|p| p.page == bx.page)?;
    let argument = if m.pos.path.is_empty() {
        None
    } else {
        bx.math.arg_extent(&m.pos.path).map(|(x0, x1, up, down)| {
            let at = |x: f32, y: f32| page.to_screen(Point::new(f64::from(x), f64::from(y)));
            Rect::from_two_pos(at(bx.x + x0, bx.base - up), at(bx.x + x1, bx.base + down))
        })
    };
    EquationFrameControl::new(tl!("Equation frame").to_string(), page, &bx, argument).show_at(ui, body.rect).clicked
}

/// What a row of the equation menu does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EquationAct {
    /// A structure's action at a nesting level.
    Structure { action: String, level: u64 },
    /// Display (`true`) or inline (`false`).
    Display(bool),
    /// Professional or linear form.
    Convert(&'static str),
    /// Equation Number.
    Number,
    /// Close Equation.
    Close,
}

/// The rows of the equation menu: the structures' actions, display and conversion, with a separator between
/// nesting levels.
// PLAN: the ruling gives `app: &WordApp`, but `Session::run` takes `&mut self`, so this takes `&mut WordApp`.
pub(crate) fn equation_menu_rows(app: &mut WordApp) -> Vec<Entry<EquationAct>> {
    let acts = app.session.run("equation.structureActions", &json!({})).ok().and_then(|v| v.get("actions").cloned()).unwrap_or_default();
    let mut rows = Vec::new();
    let mut last_level = None;
    for a in acts.as_array().into_iter().flatten() {
        let (Some(action), Some(label), Some(level)) =
            (a.get("action").and_then(|v| v.as_str()), a.get("label").and_then(|v| v.as_str()), a.get("level").and_then(|v| v.as_u64()))
        else {
            continue;
        };
        if last_level.is_some_and(|l| l != level) {
            rows.push(Entry::separator());
        }
        last_level = Some(level);
        rows.push(Entry::new(EquationAct::Structure { action: action.to_string(), level }, tl!(label).to_string()));
    }
    if last_level.is_some() {
        rows.push(Entry::separator());
    }
    // Read, not run: a command would consume an input method's pending replacement join.
    let display = wordcraft_engine::cmd::equation::at_caret(&app.session).is_some_and(|(display, _)| display);
    let items = [
        (if display { "Change to Inline" } else { "Change to Display" }, EquationAct::Display(!display)),
        ("Professional", EquationAct::Convert("professional")),
        ("Linear", EquationAct::Convert("linear")),
        ("Equation Number", EquationAct::Number),
        ("Close Equation", EquationAct::Close),
    ];
    rows.extend(items.into_iter().map(|(label, act)| Entry::new(act, tl!(label).to_string())));
    rows
}

/// Runs the command a row of [`equation_menu_rows`] stands for.
pub(crate) fn run_equation_act(app: &mut WordApp, act: &EquationAct) {
    let _ = match act {
        EquationAct::Structure { action, level } => app.run("equation.structure", json!({"action": action, "level": level})),
        EquationAct::Display(on) => app.run("equation.display", json!({"value": on})),
        EquationAct::Convert(to) => app.run("equation.convert", json!({"to": to})),
        EquationAct::Number => app.run("equation.number", json!({})),
        EquationAct::Close => app.run("equation.exit", json!({})),
    };
}
