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
//!
//! A test of the whole app window builds with `build_eframe(|_| app)`: the harness then runs the app
//! as the desktop driver does (the `eframe::App` impl below), handing the runtime the app's input
//! before every pass, the build pass included.

use craft::ui::InstallSpec;
use egui_kittest::HarnessBuilder;

use crate::i18n::Lang;

/// The install spec the app's tests run on: the framework's test spec (the running build's shell, no OS windows or
/// services) without the gallery stories' command sets. `craft::ui::testing::install_spec` loads those sets
/// (`InstallSpec::stories`), and their default chords then hold app chords in every test window: the palette story's
/// Search takes Mod+F from Find. The app installs no stories, so neither do its tests.
pub(crate) fn install_spec() -> InstallSpec {
    InstallSpec::new(craft::host::target::ShellInfo::default(), std::sync::Arc::new(craft::ui::testing::windows::NoWindows))
}

/// A harness builder whose context has the craft runtime installed with the app's test spec
/// ([`install_spec`]). Generic over the harness state, which `build_ui_state` fixes.
pub(crate) fn builder<S>() -> HarnessBuilder<S> {
    builder_with(install_spec)
}

/// Like [`builder`], with the install spec `spec` makes (called when the context is set up).
pub(crate) fn builder_with<S>(spec: impl FnOnce() -> InstallSpec + 'static) -> HarnessBuilder<S> {
    egui_kittest::Harness::builder().with_context_setup(move |ctx| {
        // A frame may draw a file name or language name in CJK on its first pass; a CI runner has no CJK font.
        cjk_stand_in(ctx);
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
    craft::ui::shell::embed::install(&ctx, install_spec(), &crate::ui_fonts(Lang::EN)).expect("the craft runtime installs on a fresh test context");
    ctx
}

/// Like [`context`], with the install spec `spec` makes (a memory clipboard, a fake system). The runtime installs
/// once per context, so a test that wants its own spec makes the context here, not by installing on [`context`]'s.
pub(crate) fn context_with(spec: impl FnOnce() -> InstallSpec) -> egui::Context {
    let ctx = egui::Context::default();
    craft::ui::shell::embed::install(&ctx, spec(), &crate::ui_fonts(Lang::EN)).expect("the craft runtime installs on a fresh test context");
    ctx
}

/// One frame of `h`, a harness built with a ui closure, as the app's driver runs it: the runtime is first handed the
/// input the app makes (`WordApp::frame_input`: the command catalog, each command's state, the fired commands), as
/// eframe's raw-input hook does (`embed::before_input`). On such a harness `Harness::step` and `run_steps` skip the
/// hook, so a command-bound control is named by its command id and a clicked control's command never runs. A test
/// that finds such a control by its label, or needs its command to run, steps here. A `build_eframe` harness hands
/// the input itself on every step.
pub(crate) fn frame(h: &mut egui_kittest::Harness<'_, crate::WordApp>) {
    let ctx = h.ctx.clone();
    craft::ui::shell::embed::before_input(&ctx, |out| h.state_mut().frame_input(out));
    h.step();
}

/// Draws a one-pixel stand-in for CJK characters on `ctx`. Kittest's missing-glyph policy panics, which is right for
/// the characters the interface's own fonts must carry; but craft-fonts is optional and a CI Linux runner has no CJK
/// system font, while the language menu and the ribbon name languages in their own script (日本語, 中文…). Only
/// Han, kana, Hangul and CJK punctuation are stood in for; any other missing glyph still panics.
pub(crate) fn cjk_stand_in(ctx: &egui::Context) {
    fn cjk(c: char) -> bool {
        matches!(
            u32::from(c),
            0x1100..=0x11FF
                | 0x2E80..=0x2FDF
                | 0x3000..=0x30FF
                | 0x3130..=0x318F
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA960..=0xA97F
                | 0xAC00..=0xD7FF
                | 0xF900..=0xFAFF
                | 0xFF00..=0xFFEF
                | 0x20000..=0x2FA1F
        )
    }
    ctx.add_glyph_rasterizer(egui::GlyphRasterizer::new("wordcraft::test_harness::cjk", |req| {
        let wanted = !req.cluster.is_empty() && req.cluster.chars().all(cjk);
        wanted.then(|| egui::RasterizedGlyph {
            bitmap: egui::GlyphBitmap {
                image: egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
                offset_px: egui::vec2(0.0, 0.0),
                is_color: false,
            },
            advance_px: req.font_size_px.max(1.0),
        })
    }));
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

/// The app as the desktop driver runs it (apps/wordcraft's `eframe::App`), for a `build_eframe` harness: kittest
/// calls the raw-input hook before every pass, so each pass has the app's input (the root env with the host, the
/// command states, the fired commands), as in production.
impl eframe::App for crate::WordApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, _raw: &mut egui::RawInput) {
        craft::ui::shell::embed::before_input(ctx, |out| self.frame_input(out));
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        logic(ctx, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::WordApp::ui(self, ui);
    }
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
