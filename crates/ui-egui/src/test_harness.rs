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

/// Like [`builder`], with the install spec `spec` makes (called when the context is set up).
pub(crate) fn builder_with<S>(spec: impl FnOnce() -> InstallSpec + 'static) -> HarnessBuilder<S> {
    egui_kittest::Harness::builder().with_context_setup(move |ctx| {
        craft::ui::shell::embed::install(ctx, spec(), &crate::ui_fonts(Lang::EN)).expect("the craft runtime installs on a fresh test context");
    })
}

/// A bare egui context with the craft runtime installed (the framework's test spec), for a test that
/// runs frames by hand (`ctx.run_ui`) instead of through a kittest harness. A frame on a context
/// without the runtime has no command registry, so the canvas's publish of its view commands
/// asserts. Install before the first frame; give each frame the app's input with
/// `embed::before_input(ctx, |out| app.frame_input(out))`.
pub(crate) fn context() -> egui::Context {
    let ctx = egui::Context::default();
    craft::ui::shell::embed::install(&ctx, craft::ui::testing::install_spec(), &crate::ui_fonts(Lang::EN))
        .expect("the craft runtime installs on a fresh test context");
    ctx
}

/// Like [`context`], with the install spec `spec` makes (a memory clipboard, a fake system). The runtime installs
/// once per context, so a test that wants its own spec makes the context here, not by installing on [`context`]'s.
pub(crate) fn context_with(spec: impl FnOnce() -> InstallSpec) -> egui::Context {
    let ctx = egui::Context::default();
    craft::ui::shell::embed::install(&ctx, spec(), &crate::ui_fonts(Lang::EN)).expect("the craft runtime installs on a fresh test context");
    ctx
}

/// One frame of `h` as the app's driver runs it: the runtime is first handed the input the app makes
/// (`WordApp::frame_input`: the command catalog, each command's state, the fired commands), as
/// eframe's raw-input hook does (`embed::before_input`). `Harness::step` and `run_steps` skip the
/// hook, so a command-bound control is named by its command id and a clicked control's command never
/// runs. A test that finds such a control by its label, or needs its command to run, steps here.
pub(crate) fn frame(h: &mut egui_kittest::Harness<'_, crate::WordApp>) {
    let ctx = h.ctx.clone();
    craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
    h.step();
}

/// [`frame`] under the name some tests use.
pub(crate) fn step(h: &mut egui_kittest::Harness<'_, crate::WordApp>) {
    frame(h);
}

/// [`frame`] as a method, for tests that step a harness by `h.app_frame()` where they would `h.step()`.
pub(crate) trait AppFrames {
    /// One frame as [`frame`] runs it.
    fn app_frame(&mut self);
}

impl AppFrames for egui_kittest::Harness<'_, crate::WordApp> {
    fn app_frame(&mut self) {
        frame(self);
    }
}

/// The frame's logic as the app's driver runs it: the app's own `logic`, then the runtime's close path
/// (`embed::logic`), which turns a close request on the window (`ui.closeWindow`, the OS button) into the app's
/// close decision. `WordApp::logic` alone never reads a close request.
pub(crate) fn logic(ctx: &egui::Context, app: &mut crate::WordApp) {
    app.logic(ctx);
    craft::ui::shell::embed::logic(ctx, || app.close_decision());
}

/// [`logic`] for a kittest closure, where the harness's `step` and `run_steps` skip eframe's input hook: the
/// runtime is handed the app's input first (as `embed::before_input` does before a frame), at the head of the
/// pass, so what the app made reaches the passes after it. A harness that cannot run the hook before its pass
/// runs it here, one frame behind, instead of leaving command states, fired commands and focus requests out.
pub(crate) fn driver_logic(ctx: &egui::Context, app: &mut crate::WordApp) {
    craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
    logic(ctx, app);
}

/// One frame of `app` on a bare context from [`context`], as the app's driver runs it: the runtime is handed the
/// app's input first (`embed::before_input`), then the app's `logic` and `ui` run in the pass. Without that hook
/// a frame has no command states, so no key reaches a command and a focus request waits for ever. The pass's
/// texture changes are dropped, since no renderer takes them.
pub(crate) fn pass(ctx: &egui::Context, app: &mut crate::WordApp, input: egui::RawInput) {
    craft::ui::shell::embed::before_input(ctx, |out| app.frame_input(out));
    ctx.run_ui(input, |ui| {
        logic(ui.ctx(), app);
        app.ui(ui);
    })
    .drop_without_applying_deltas();
}

/// `n` frames of `h`, each as [`frame`] runs it.
pub(crate) fn frames(h: &mut egui_kittest::Harness<'_, crate::WordApp>, n: usize) {
    for _ in 0..n {
        frame(h);
    }
}
