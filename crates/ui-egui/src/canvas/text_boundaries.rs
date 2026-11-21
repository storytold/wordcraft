//! Screen-only guides for the laid-out text area. Export and print rendering never see them.

use craft::behavior::{Input, Metrics};
use craft::pages::ui::PageBody;
use craft::ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Rect, Response, Ui};

use super::{DocLayout, WordApp};

#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct TextBoundariesStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
}

#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
struct TextBoundaries<'a> {
    #[common]
    common: Common,
    rects: &'a [Rect],
}

impl Component for TextBoundaries<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TextBoundariesStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TextBoundariesStyleKey) -> TextBoundariesStyle {
        TextBoundariesStyle { metrics: Metrics::new(), line: cx.color(cx.tokens.color.overlay.stroke) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        for rect in self.rects {
            ui.draw().stroke(&PaintShape::Rect(*rect, 0.0.into()), StrokeWidth::Thin, cx.style.line);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

pub(super) fn show(app: &WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody<'_>) {
    if !app.session.view.text_boundaries || app.session.view.mode != wordcraft_layout::ViewMode::Print || app.session.view.read_mode {
        return;
    }
    let rects: Vec<_> = body
        .pages
        .iter()
        .filter_map(|placed| {
            let page = layout.pages.get(placed.page)?;
            let bounds = page.body;
            let scale = placed.scale as f32;
            let rect = Rect::from_min_size(placed.rect.min + egui::vec2(bounds.x, bounds.y) * scale, egui::vec2(bounds.w, bounds.h) * scale);
            (rect.is_finite() && rect.is_positive()).then_some(rect)
        })
        .collect();
    TextBoundaries { common: Common::new(), rects: &rects }.label(tl!("Text Boundaries").to_string()).show_at(ui, body.rect);
}
