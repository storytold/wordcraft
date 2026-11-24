//! The sizes Read Mode lays its columns out at, which come from the body text's measure: the app reaches the text
//! layout only inside a component's `measure`, so a zero-size probe component reads it there.

use craft::behavior::{Input, Metrics, Sense};
use craft::pages::ui::reading::ReadingMetrics;
use craft::ui::contract::{Common, Component, Cx, MeasureCx, NoValue, Outcome, Props, Role, Show as _};
use craft::ui::primitives::ReadingMeasure;
use craft::ui::style::{ComponentStyle, StyleCx, TypeRole};
use egui::{Response, Ui};

/// Measures the body text's reading widths; shows nothing.
#[derive(Component, Props)]
#[component(canvas_control)]
struct ReadingProbe {
    #[common]
    common: Common,
    measured: Option<ReadingMetrics>,
}

/// What the probe measured this pass.
struct ReadingProbeOutput {
    response: Response,
    metrics: Option<ReadingMetrics>,
}

impl Outcome for ReadingProbeOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The gaps the metrics take from the tokens.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct ReadingProbeStyle {
    #[style(metrics)]
    metrics: Metrics,
    column_gap: f32,
    nav_bar: f32,
}

impl Component for ReadingProbe {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = ReadingProbeStyle;
    type Output = ReadingProbeOutput;

    fn style(cx: &StyleCx<'_>, _: ReadingProbeStyleKey) -> ReadingProbeStyle {
        ReadingProbeStyle { metrics: Metrics::new(), column_gap: cx.tokens.space.gap.section, nav_bar: cx.tokens.size.panel_header }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> egui::Vec2 {
        let measure = ReadingMeasure::of(TypeRole::Body, |run| cx.layout(run));
        self.measured = Some(ReadingMetrics::new(cx.style.column_gap, cx.style.nav_bar, measure));
        egui::Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> ReadingProbeOutput {
        ReadingProbeOutput { response: cx.response.clone(), metrics: self.measured }
    }
}

/// The reading metrics under the current theme, or `None` when the probe did not measure this pass.
pub(super) fn read(ui: &mut Ui) -> Option<ReadingMetrics> {
    ReadingProbe { common: Common::new(), measured: None }.label("Reading metrics").show(ui).metrics
}
