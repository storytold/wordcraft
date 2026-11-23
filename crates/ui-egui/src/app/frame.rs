use super::{Status, WordApp, canvas, chrome, documents, panes, read_aloud, ribbon, updates_ui, zotero};
use craft::platform::clock::monotonic_ms;
use craft::theme::variant::Tone;
use craft::ui::bars::StatusMessage;
use craft::ui::contract::Bind;
use craft::ui::runtime::env::ShellRegions;
use craft::ui::shell::{AppShell, ScreenModeSet, Slot, SlotSize};

use crate::i18n;
use crate::language_packs;

/// How long the status bar shows a message, in milliseconds.
pub(crate) const STATUS_MS: f64 = 6000.0;
/// How long [`WordApp::status_long`] shows one: a warning that is easy to miss in six seconds.
const STATUS_LONG_MS: f64 = 20000.0;

impl WordApp {
    /// A file that wouldn't open (#1103): log it, and say on the status bar which file and why,
    /// whatever handed it over (the command line, a macOS open event, a drop). Returns the message.
    pub fn report_open_failure(&mut self, path: &str, error: &str) -> String {
        log::warn!("{path}: {error}");
        let dir = crate::chrome::interface_direction();
        let msg = i18n::fmt(crate::tl!("Couldn't open {path}: {error}"), &[("path", &craft::i18n::path(path, dir)), ("error", error)]);
        self.post_status(msg.clone(), STATUS_MS, Tone::Bad);
        msg
    }

    /// Sets a temporary status message in the bottom status bar.
    pub fn status(&mut self, s: impl Into<String>) {
        self.post_status(s.into(), STATUS_MS, Tone::Neutral);
    }

    /// A status message that stays [`STATUS_LONG_MS`] instead of the usual six seconds.
    pub fn status_long(&mut self, s: impl Into<String>) {
        self.post_status(s.into(), STATUS_LONG_MS, Tone::Neutral);
    }

    fn post_status(&mut self, text: String, ms: f64, tone: Tone) {
        self.status = Some(Status { text, until: monotonic_ms() + ms, tone });
    }

    /// The message the status bar shows now, until its time is up.
    pub(crate) fn status_message(&self) -> Option<StatusMessage> {
        let s = self.status.as_ref().filter(|s| monotonic_ms() < s.until)?;
        Some(StatusMessage::new(s.text.clone()).tone(s.tone))
    }

    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    pub fn title_stem(&self) -> String {
        let mut title = match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Document1").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => {
                    documents::untitled_title(&self.session.resolved_filenames().0).unwrap_or_else(|| tl!("Document1").into())
                }
                None => self.session.doc.core.title.clone(),
            },
        };
        // X11 converts window titles to C strings, which cannot contain NUL (#3634).
        title.retain(|c| c != '\0');
        if title.is_empty() { tl!("Document1").into() } else { title }
    }

    /// Tell the host whether the document has unsaved changes ([`Services::on_dirty`]).
    pub(super) fn report_dirty(&self) {
        if let Some(f) = &self.services.on_dirty {
            f(self.session.dirty || panes::has_comment_drafts(self));
        }
    }

    /// Draw the window: the File page, the empty-window chrome, or the document window, as one
    /// shell. The shell owns the screen mode and the dropped-down ribbon while it draws; both are
    /// handed in as values and read back after, so no body reads a stale copy. Returns the
    /// shell's regions; a window another command swapped in mid-draw returns none.
    pub(super) fn paint_window(&mut self, ui: &mut egui::Ui) -> ShellRegions {
        crate::announce::expose(self, ui.ctx());
        let window = self.windows.shown();
        let stale = |app: &WordApp| app.windows.shown() != window;
        let mode = self.screen_mode();
        let dropped = self.ribbon_dropped;
        let key = self.windows.window_key();
        let shell = AppShell::new(key)
            .mode_set(ScreenModeSet::Document)
            .screen_mode(Bind::given(mode))
            .dropped(Bind::given(dropped))
            .window_title(self.window_title())
            .page(self.ui.backstage, crate::backstage::scope(), crate::backstage::show)
            .title_bar(chrome::title_bar::show)
            .top(Slot::new("screenBar").screen_bar(), chrome::screen_bar::show)
            // The ribbon is a tab strip over a group row: its slots are as tall as it is and stack it. A slot of the
            // default size is one options bar high and lays its content in one line, which put the groups beside the
            // tabs and collapsed every one of them.
            .top(Slot::new("ribbonDrop").drops().size(SlotSize::Content), |ui, _cx, app: &mut WordApp| {
                if !stale(app) {
                    ribbon_slot(app, ui);
                }
            })
            .top(Slot::new("ribbon").size(SlotSize::Content), |ui, _cx, app: &mut WordApp| {
                if !stale(app) {
                    ribbon_slot(app, ui);
                }
            })
            // As tall as the bar: a failure's line under the message puts Install below the first line, and a slot of
            // the default size (one options bar high) clips it, so a press on it reached nothing.
            .top(Slot::new("languagePack").size(SlotSize::Content).visible(language_packs::bar_offer(self)), language_packs::pack_bar)
            .status(true, chrome::status_bar::show)
            .body(|ui, _cx, app: &mut WordApp| {
                if !app.has_document() || stale(app) {
                    return;
                }
                panes::show(app, ui, |ui, _cx, app| {
                    if app.session.md_source.is_some() {
                        crate::md_source::show(app, ui);
                    } else {
                        canvas::show(app, ui);
                    }
                });
                if app.session.read_aloud.open && !stale(app) {
                    read_aloud::show(app, ui);
                }
            });
        let out = shell.show(ui, self);
        // The output belongs to the window that drew it: another window swapped in mid-draw
        // must not take its flags.
        if stale(self) {
            return ShellRegions::new(egui::Rect::NOTHING);
        }
        crate::ribbon_blur::update(self, ui.ctx());
        // `follow_screen_mode` maps one mode onto three view flags: write it back only on a change.
        if out.screen_mode != mode {
            self.follow_screen_mode(out.screen_mode);
        }
        self.ribbon_dropped = out.dropped;
        out.regions
    }

    /// Dialogs, panes and other overlays drawn over the window. A command that brings another
    /// window's state in ends the drawing: the rest would be the other window's.
    pub(super) fn show_overlays(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, regions: &ShellRegions) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        crate::font_download::tick(self);
        zotero::show_question(self);
        // PLAN: assumed `updates_ui::run(app: &mut WordApp)` (shell-w13, sh.A20) opens the updater's dialogs on the host.
        updates_ui::run(self);
        crate::dialogs::host::prepare(self);
        self.dialog_host.run(ui);
        crate::dialogs::host::settle(self, ctx);
        if moved(self) {
            return;
        }
        self.refocus_after_chrome_click(ctx, regions);
        if let Some(url) = self.canvas.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }
}

/// The ribbon, then its keytips, in a top slot: the keytips read what the ribbon recorded this pass.
fn ribbon_slot(app: &mut WordApp, ui: &mut egui::Ui) {
    let slot = app.ribbon_glass().then(|| crate::ribbon_blur::BlurState::reserve(ui));
    let out = ribbon::show(app, ui);
    crate::keytips::show(ui, app, &out);
    if let Some(slot) = slot {
        app.canvas.blur.set_ribbon(slot, ui.min_rect());
    }
}
