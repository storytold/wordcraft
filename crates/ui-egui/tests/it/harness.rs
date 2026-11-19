//! The one place the integration tests build a headless egui harness.
//!
//! A `WordApp` frame needs the craft runtime installed on the egui context before the first frame
//! (kittest's builder runs one frame while it builds): on a bare context every command resolves to
//! app scope and the canvas's publish of its view commands asserts. The crate's own tests use
//! `test_harness::builder` for the same reason; integration tests cannot reach a `cfg(test)` module,
//! so this is their copy.

use egui_kittest::HarnessBuilder;

/// A harness builder whose context has the craft runtime installed with the framework's test spec.
pub fn builder<S>() -> HarnessBuilder<S> {
    egui_kittest::Harness::builder().with_context_setup(|ctx| {
        craft::ui::shell::embed::install(ctx, craft::ui::testing::install_spec(), &wordcraft_ui_egui::ui_fonts(wordcraft_ui_egui::i18n::Lang::EN))
            .expect("the craft runtime installs on a fresh test context");
    })
}
