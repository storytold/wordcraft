//! The one place the app's tests build a headless egui harness.
//!
//! Every test that shows a `WordApp` frame needs the framework runtime installed on the egui
//! context *before the first frame*: on a bare context the command board has no registry, so
//! every command, instance-scope ones included, resolves to app scope and the canvas's own
//! publish of its view commands asserts. kittest's builder runs one frame while it builds, so
//! installing after `build_ui_state` is too late. These builders install through the builder's
//! context setup, which runs first.
//!
//! Use [`builder`] instead of `egui_kittest::Harness::builder()` (the framework lint `test/kit`
//! flags direct use); chain `with_size`, `with_step_dt` and the rest on the result as before.
//! [`builder_with`] takes a custom install spec, e.g. a memory clipboard or a fake system.

use craft::ui::InstallSpec;
use egui_kittest::HarnessBuilder;

use crate::i18n::Lang;

/// A harness builder whose context has the craft runtime installed with the framework's test
/// spec. Generic over the harness state, which `build_ui_state` fixes.
pub(crate) fn builder<S>() -> HarnessBuilder<S> {
    builder_with(craft::ui::testing::install_spec)
}

/// `n` frames of `h`, each given the input the app hands the runtime (`WordApp::frame_input`):
/// the commands the controls fired in one frame run before the next. kittest's own `run_steps`
/// skips that hand-over, so a clicked button or menu row would never run its command.
pub(crate) fn frames(h: &mut egui_kittest::Harness<'_, crate::WordApp>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
        h.step();
    }
}

/// Like [`builder`], with the install spec `spec` makes (called when the context is set up).
pub(crate) fn builder_with<S>(spec: impl FnOnce() -> InstallSpec + 'static) -> HarnessBuilder<S> {
    egui_kittest::Harness::builder().with_context_setup(move |ctx| {
        craft::ui::shell::embed::install(ctx, spec(), &crate::ui_fonts(Lang::EN)).expect("the craft runtime installs on a fresh test context");
    })
}

/// A bare egui context with the craft runtime installed (the framework's test spec), for a test that
/// runs frames by hand (`ctx.run_ui`) instead of through a kittest harness. A frame on a context
/// without the runtime has no command registry, so the canvas's publish of its view commands
/// asserts. Install before the first frame.
pub(crate) fn context() -> egui::Context {
    let ctx = egui::Context::default();
    craft::ui::shell::embed::install(&ctx, craft::ui::testing::install_spec(), &crate::ui_fonts(Lang::EN))
        .expect("the craft runtime installs on a fresh test context");
    ctx
}
