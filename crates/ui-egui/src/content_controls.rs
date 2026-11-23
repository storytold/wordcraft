//! Content controls on the page: the frame and title tab of the control the caret is in (every
//! control in Design Mode), check boxes that tick on click, and the list or calendar of a
//! drop-down, combo box or date picker. Everything ends in a `developer.*` command.

use craft::behavior::kurbo::Point;
use craft::behavior::{CursorIcon, HitLevel, HitPriority, HitShape, Metrics, PartKey, PartRef, PointerButton, PressPhase, Sense};
use craft::pages::ui::PageBody;
use craft::ui::buttons::Button;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{
    Access, Bind, Common, Component, Cx, Entry, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _,
    ShowContainer as _, Trigger,
};
use craft::ui::menu::Menu;
use craft::ui::pickers::{Calendar, CalendarView, DatePrecision, PickedDate};
use craft::ui::popover::Popover;
use craft::ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Rect, Response, Ui, Vec2, pos2, vec2};
use serde_json::json;
use wordcraft_doc::Pos;
use wordcraft_doc::control::{ControlKind, ControlRange};
use wordcraft_layout::DocLayout;

use crate::WordApp;

/// Most controls framed at once (Design Mode in a long form).
const MAX_FRAMED: usize = 400;

/// A check box's part: `CHECK.at(i)`.
const CHECK: PartKey = PartKey::new("check");
/// The list or calendar button of the control the caret is in.
const BUTTON: PartKey = PartKey::new("button");

/// The open list or calendar of the window's caret control, kept in `CanvasState::control_popup` while it is open.
#[derive(Clone, Debug)]
pub(crate) struct ControlPopup {
    /// Content start of the control it belongs to.
    pub(crate) at: Pos,
    /// The document it was opened in: a document switch closes it.
    pub(crate) document: u64,
    /// Calendar page shown, for a date control.
    pub(crate) view: Option<CalendarView>,
}

/// Whether a control's list or calendar is open.
#[cfg(test)]
pub(crate) fn popup_open(app: &WordApp) -> bool {
    app.canvas.control_popup.is_some()
}

/// The control's content on screen, one rect per page it is on.
fn frames(app: &WordApp, layout: &DocLayout, body: &PageBody<'_>, r: &ControlRange) -> Vec<Rect> {
    let a = r.content_start();
    let mut out: Vec<(usize, Rect)> = Vec::new();
    for (pi, sr) in layout.selection_rects(&app.session.doc, &a, &r.end, app.session.page_hint) {
        let Some(page) = body.pages.iter().find(|p| p.page == pi) else { continue };
        let at = |x: f32, y: f32| page.to_screen(Point::new(f64::from(x), f64::from(y)));
        let s = Rect::from_two_pos(at(sr.x, sr.y), at(sr.x + sr.w, sr.y + sr.h));
        match out.iter_mut().find(|(p, _)| *p == pi) {
            Some((_, u)) => *u = u.union(s),
            None => out.push((pi, s)),
        }
    }
    if out.is_empty()
        && let Some(c) = layout.caret_on(&a, app.session.page_hint)
        && let Some(page) = body.pages.iter().find(|p| p.page == c.page)
    {
        let top = page.to_screen(Point::new(f64::from(c.x), f64::from(c.top)));
        out.push((c.page, Rect::from_min_size(top, vec2(0.0, c.height * page.scale as f32))));
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// One framed control: its content rects on screen, one per page, and its title tab.
struct ControlFrame {
    rects: Vec<Rect>,
    title: String,
    current: bool,
}

/// The sizes and colours of the content controls' frames, tabs and list button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct ContentControlsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// How far a frame stands off its content.
    margin: f32,
    /// The tab's height and the list button's width.
    tab_height: f32,
    /// The tab's text inset.
    pad: f32,
    radius: f32,
    arrow_width: f32,
    arrow_height: f32,
    line: StyleColor,
    accent: StyleColor,
    soft: StyleColor,
    strong: StyleColor,
    on_strong: StyleColor,
    ink: StyleColor,
}

/// A frame's rect grown by the margin.
fn grown(r: Rect, style: &ContentControlsStyle) -> Rect {
    r.expand(style.margin)
}

/// The list button hanging at the right of the last rect of the current control.
fn button_rect(last: Rect, style: &ContentControlsStyle) -> Rect {
    let g = grown(last, style);
    Rect::from_min_size(pos2(g.max.x, g.min.y), vec2(style.tab_height, g.height().max(style.tab_height)))
}

fn clicked(phase: &PressPhase) -> bool {
    matches!(phase, PressPhase::Release { start, click: true, .. } if start.button == PointerButton::Primary)
}

/// The frames and title tabs of the content controls, their check boxes and the list or date button of the control
/// the caret is in. The check boxes and the button are parts, so a press on one never reaches the page tool.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
struct ContentControls<'a> {
    #[common]
    common: Common,
    frames: &'a [ControlFrame],
    checks: &'a [Rect],
    /// The last content rect of the control that has a list or calendar button.
    button: Option<Rect>,
    toggled: Option<usize>,
    opened: bool,
    button_at: Option<Rect>,
}

impl<'a> ContentControls<'a> {
    fn new(label: impl Into<Label>, frames: &'a [ControlFrame], checks: &'a [Rect], button: Option<Rect>) -> Self {
        ContentControls { common: Common::new(), frames, checks, button, toggled: None, opened: false, button_at: None }.label(label)
    }
}

/// What the controls report: the check box clicked, whether the list button was clicked, and where the button is.
struct ContentControlsOutput {
    response: Response,
    toggled: Option<usize>,
    opened: bool,
    button: Option<Rect>,
}

impl Outcome for ContentControlsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for ContentControls<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = ContentControlsStyle;
    type Output = ContentControlsOutput;

    fn style(cx: &StyleCx<'_>, _: ContentControlsStyleKey) -> ContentControlsStyle {
        let t = cx.tokens;
        ContentControlsStyle {
            metrics: Metrics::new(),
            margin: t.space.gap.tight,
            tab_height: t.size.control.sm,
            pad: t.space.pad.chip,
            radius: t.shape.radius.sm,
            arrow_width: t.size.icon.sm,
            arrow_height: t.size.handle.sm,
            line: cx.color(t.color.overlay.line),
            accent: cx.color(t.color.accent.solid),
            soft: cx.color(t.color.accent.soft),
            strong: cx.color(t.color.accent.strong),
            on_strong: cx.color(t.color.text.on_strong),
            ink: cx.color(t.color.text.normal),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let handle = HitPriority::new(HitLevel::Handle, 0);
        for (i, r) in self.checks.iter().enumerate() {
            cx.part(CHECK.at(i as u64), HitShape::Rect(grown(*r, cx.style)))
                .sense(Sense::CLICK)
                .priority(handle)
                .cursor(CursorIcon::PointingHand)
                .pointer_only("Developer > Check Box ticks the control the caret is in");
        }
        if let Some(b) = self.button.map(|last| button_rect(last, cx.style)) {
            cx.part(PartRef::from(BUTTON), HitShape::Rect(b))
                .sense(Sense::CLICK)
                .priority(handle)
                .cursor(CursorIcon::PointingHand)
                .pointer_only("Developer > Choose Item and Set Date run the same commands");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.button_at = self.button.map(|last| button_rect(last, cx.style));
        for i in 0..self.checks.len() {
            if cx.part_if_declared(CHECK.at(i as u64)).is_some_and(|r| clicked(&r.press.phase)) {
                self.toggled = Some(i);
            }
        }
        if cx.part_if_declared(PartRef::from(BUTTON)).is_some_and(|r| clicked(&r.press.phase)) {
            self.opened = true;
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let style = cx.style;
        let corner = CornerRadius::from(style.radius);
        for f in self.frames {
            let color = if f.current { style.accent } else { style.line };
            for r in &f.rects {
                draw.stroke(&PaintShape::Rect(grown(*r, style), corner), StrokeWidth::Thin, color);
            }
            let Some(first) = f.rects.first().map(|r| grown(*r, style)) else { continue };
            if f.title.is_empty() {
                continue;
            }
            let (fill, ink) = if f.current { (style.strong, style.on_strong) } else { (style.soft, style.accent) };
            let run = TextRun::new(f.title.as_str()).role(TypeRole::Caption).color(ink);
            let width = (draw.layout(&run, f32::INFINITY).galley.size().x + 2.0 * style.pad).min(cx.rect.width());
            let tab = Rect::from_min_size(pos2(first.min.x, first.min.y - style.tab_height), vec2(width, style.tab_height));
            draw.fill(&PaintShape::Rect(tab, corner), fill);
            draw.text(tab.shrink2(vec2(style.pad, 0.0)), &run);
        }
        if let Some(b) = self.button_at {
            draw.fill(&PaintShape::Rect(b, corner), style.soft);
            draw.stroke(&PaintShape::Rect(b, corner), StrokeWidth::Thin, style.accent);
            let (c, w, h) = (b.center(), style.arrow_width / 2.0, style.arrow_height / 2.0);
            draw.fill(&PaintShape::Polygon(vec![pos2(c.x - w, c.y - h), pos2(c.x + w, c.y - h), pos2(c.x, c.y + h)]), style.ink);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        for i in 0..self.checks.len() {
            access.node(CHECK.at(i as u64), Role::CHECK_BOX).name = Name::Text(tl!("Check box").to_string());
        }
        if self.button.is_some() {
            access.node(PartRef::from(BUTTON), Role::BUTTON).name = Name::Text(tl!("Choose item").to_string());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ContentControlsOutput {
        ContentControlsOutput { response: cx.response.clone(), toggled: self.toggled, opened: self.opened, button: self.button_at }
    }
}

/// Draws the frames and title tabs, ticks check boxes on a click and runs the list or calendar
/// of the current control (inside the canvas body, after the selection).
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) {
    let story = app.session.sel.focus.story;
    let current = app.session.doc.control_at(&app.session.sel.focus);
    let design = app.session.view.design_mode;
    let document = app.session.document_id();
    // A document switch, or the caret leaving the control, closes the popup.
    if app.canvas.control_popup.as_ref().is_some_and(|p| p.document != document || current.as_ref().is_none_or(|c| c.content_start() != p.at)) {
        app.canvas.control_popup = None;
    }
    let mut shown: Vec<ControlRange> =
        if design { app.session.doc.control_ranges(story).into_iter().take(MAX_FRAMED).collect() } else { current.clone().into_iter().collect() };
    // Check boxes are clickable wherever they are, framed or not.
    let all = if design { shown.clone() } else { app.session.doc.control_ranges(story) };
    let mut check_rects = Vec::new();
    let mut check_at = Vec::new();
    for r in all.iter().filter(|r| matches!(r.control.kind, ControlKind::CheckBox { .. })).take(MAX_FRAMED) {
        if let Some(f) = frames(app, layout, body, r).first() {
            check_rects.push(*f);
            check_at.push(r.content_start());
        }
    }
    shown.sort_by_key(|r| r.depth);
    let mut framed = Vec::new();
    // The list/calendar button sits at the right of the current control. With a table tool on, the pen
    // owns the pointer, so no button takes the press.
    let mut button: Option<(Rect, ControlRange)> = None;
    for r in &shown {
        let is_current = current.as_ref().is_some_and(|c| c.start == r.start);
        let rects = frames(app, layout, body, r);
        // The title tab above the first line (its tag in Design Mode when it has no title).
        let title = if r.control.title.is_empty() && design { r.control.tag.clone() } else { r.control.title.clone() };
        if is_current
            && app.canvas.table_tool.is_none()
            && matches!(r.control.kind, ControlKind::ComboBox { .. } | ControlKind::DropDown { .. } | ControlKind::Date { .. })
            && let Some(last) = rects.last()
        {
            button = Some((*last, r.clone()));
        }
        framed.push(ControlFrame { rects, title, current: is_current });
    }
    let out = ContentControls::new(tl!("Content controls").to_string(), &framed, &check_rects, button.as_ref().map(|(last, _)| *last))
        .show_at(ui, body.rect);
    if let Some(at) = out.toggled.and_then(|i| check_at.get(i)) {
        let at = serde_json::to_value(at).unwrap_or_default();
        let _ = app.run("developer.toggleCheckBox", json!({"at": at}));
    }
    let Some((_, range)) = button else {
        app.canvas.control_popup = None;
        return;
    };
    if out.opened {
        app.canvas.control_popup =
            if app.canvas.control_popup.is_some() { None } else { Some(ControlPopup { at: range.content_start(), document, view: None }) };
    }
    let (Some(popup), Some(anchor)) = (app.canvas.control_popup.clone(), out.button) else {
        app.canvas.control_popup = None;
        return;
    };
    let at = serde_json::to_value(&popup.at).unwrap_or_default();
    let mut open = true;
    match &range.control.kind {
        ControlKind::ComboBox { items, .. } | ControlKind::DropDown { items, .. } => {
            let entries: Vec<Entry<usize>> = items.iter().enumerate().map(|(i, item)| Entry::new(i, item.display.clone())).collect();
            let shown = Menu::new(tl!("Choose item").to_string())
                .open(Bind::bound(&mut open))
                .opened_by(&out.response, Trigger::Manual)
                .anchor(Anchor::Rect(anchor))
                .pick(ui, &entries);
            if let Some(index) = shown.picked {
                let _ = app.run("developer.chooseItem", json!({"at": at, "index": index}));
                open = false;
            }
        }
        ControlKind::Date { .. } => {
            let (y, m, d) = today();
            let now = PickedDate::day(y as i32, m as u8, d as u8);
            let value = date_of(app, &popup.at).map(|(y, m, d)| PickedDate::day(y as i32, m as u8, d as u8));
            let dates = Env::of(ui).dates();
            let mut view = popup.view.unwrap_or_else(|| CalendarView::open(value, Some(now), &[DatePrecision::Day], &dates));
            let mut picked = None;
            Popover::new(tl!("Choose date").to_string(), Bind::bound(&mut open))
                .opened_by(&out.response)
                .overlay(|overlay| overlay.opener_anchor(Anchor::Rect(anchor)))
                .show(ui, |ui, _| {
                    let shown = Calendar::new(tl!("Calendar").to_string(), view, value).today(now).show(ui);
                    if let Some(moved) = shown.moved {
                        view = moved;
                    }
                    picked = picked.or(shown.picked);
                    if Button::new(tl!("Today").to_string()).show(ui).clicked() {
                        picked = Some(now);
                    }
                });
            if let Some(p) = app.canvas.control_popup.as_mut() {
                p.view = Some(view);
            }
            if let Some(day) = picked {
                let _ = app.run("developer.setDate", json!({"at": at, "date": day.iso()}));
                open = false;
            }
        }
        _ => open = false,
    }
    if !open {
        app.canvas.control_popup = None;
    }
}

fn date_of(app: &WordApp, at: &Pos) -> Option<(i64, u32, u32)> {
    let r = app.session.doc.control_at(at)?;
    match &r.control.kind {
        ControlKind::Date { full_date, .. } => wordcraft_engine::cmd::controls::parse_date(full_date),
        _ => None,
    }
}

fn today() -> (i64, u32, u32) {
    // The local day, as Word's date picker shows it (#1707).
    let (y, m, d, ..) = wordcraft_engine::cmd::local_now();
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// A harness showing a drop-down control followed by text, with the caret in the control and
    /// its list button on screen.
    fn dropdown_harness() -> egui_kittest::Harness<'static, WordApp> {
        let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("text.insert", json!({"text": "tail text"})).unwrap();
        app.run("caret.docStart", json!({})).unwrap();
        app.run("developer.dropDown", json!({"items": ["A", "B"]})).unwrap();
        app.run("view.commentsPane", json!({"value": false})).unwrap();
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
        let before = h.state().session.sel.focus.clone();
        assert!(h.state().session.doc.control_at(&before).is_some(), "the caret starts in the control");
        h
    }

    /// #5389: pressing a drop-down's list button opens the list and leaves the caret in the
    /// control, even when text follows the control and the button hangs over it.
    #[test]
    fn pressing_the_list_button_keeps_the_caret_in_the_control() {
        let mut h = dropdown_harness();
        h.get_by_label("Choose item").click();
        for _ in 0..3 {
            h.step();
        }
        let after = h.state().session.sel.focus.clone();
        assert!(h.state().session.doc.control_at(&after).is_some(), "the caret stays in the control");
        assert!(popup_open(h.state()), "the list is open");
    }

    /// A table tool owns the pointer: a press, drag and release that start over the list button
    /// draw a table and leave the popup closed.
    #[test]
    fn the_draw_table_tool_starts_a_stroke_over_the_list_button() {
        let mut h = dropdown_harness();
        let at = h.get_by_label("Choose item").rect().center();
        h.state_mut().canvas.table_tool = Some(crate::table_pen::TableTool::Draw);
        h.step();
        let depth = h.state().session.undo_depth();
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(at + vec2(12.0, 8.0) * i as f32));
            h.step();
        }
        h.event(egui::Event::PointerButton {
            pos: at + vec2(48.0, 32.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        });
        for _ in 0..3 {
            h.step();
        }
        assert!(!popup_open(h.state()), "the press did not open the list");
        assert!(h.state().session.undo_depth() > depth, "the stroke ran table.draw");
    }
}
