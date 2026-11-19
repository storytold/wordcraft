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

/// One frame of `h` as the app's driver runs it: the runtime is first handed the input the app
/// makes (`WordApp::frame_input`: the command catalog, each command's state and the commands the
/// last frame fired, which the app runs), then the frame runs. `Harness::step` alone leaves a
/// fired command unrun and a command-bound control unnamed.
pub fn frame(h: &mut egui_kittest::Harness<'_, wordcraft_ui_egui::WordApp>) {
    let ctx = h.ctx.clone();
    craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
    h.step();
}
