use craft::settings::{LoadReport, SaveStatus, SharedSettings};
use craft::ui::notices::{SaveResult, SettingsLoad};
use craft::ui::shell::ScreenMode;

use super::{DEFAULT_TAB, MAX_DICTIONARY, WordApp, recent_fonts_from, theme};
use crate::prefs::{Prefs, Shortcuts};

impl WordApp {
    /// Preferences to save between runs: the UI state plus the current user name and editing
    /// preferences.
    pub fn prefs(&self) -> Prefs {
        let mut ui = self.ui.clone();
        ui.author = Some(self.session.author.clone());
        ui.initials = self.session.initials.clone();
        ui.dictionary = wordcraft_engine::proof::user_dictionary();
        ui.editing = self.session.prefs.clone();
        ui.read_aloud_rate = self.session.read_aloud.rate();
        ui.read_aloud_skip_citations = self.session.read_aloud.skip_citations;
        ui.dark_page = self.session.view.dark_mode;
        ui.nav_pane = self.session.view.nav_pane;
        ui.styles_pane_dock = self.session.view.styles_pane_dock;
        ui.check_spelling = self.session.view.proofing;
        ui.keyboard = Shortcuts(self.session.keymap().clone());
        ui.ribbon = self.session.ribbon.clone();
        ui.ink_pens = self.session.view.draw.pens.clone();
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: Prefs) {
        self.ui = ui;
        self.ui.backstage = false;
        // Word opens on Home every launch, so the tab the settings hold is not restored. Going File
        // and back to the editor within the run still returns to the tab that was showing.
        self.ui.tab = DEFAULT_TAB.into();
        let recent_fonts = recent_fonts_from(self.ui.font_picker.recent.iter().map(String::as_str));
        self.ui.font_picker.recent = recent_fonts;
        // Settings from before the theme setting: their dark-mode switch picks Dark or Light.
        if let Some(dark) = self.ui.legacy_dark.take() {
            self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
        }
        // Settings from before #822: the interface size was on top of the display scale, so it
        // becomes absolute on the first frame that knows that scale (`logic`). A chosen `uiScale`
        // (the current key) wins; "auto" can't be told from a missing key, so the old size does.
        self.legacy_interface_size =
            self.ui.legacy_interface_size.take().filter(|_| self.ui.interface_size.is_auto()).and_then(|old| theme::legacy_interface_percent(&old));
        // The session owns the name from here on; `prefs` copies it back when saving.
        // A saved empty name stays empty (#4368); one of only spaces keeps the default.
        if let Some(author) = self.ui.author.take()
            && (author.is_empty() || !author.trim().is_empty())
        {
            self.session.author = author;
        }
        self.session.initials = std::mem::take(&mut self.ui.initials).chars().take(wordcraft_engine::cmd::options::MAX_INITIALS_CHARS).collect();
        for w in std::mem::take(&mut self.ui.dictionary).iter().take(MAX_DICTIONARY) {
            let w = w.trim();
            if !w.is_empty() && w.chars().count() <= wordcraft_engine::cmd::options::MAX_WORD_CHARS {
                wordcraft_engine::proof::add_word(w);
            }
        }
        self.session.prefs = std::mem::take(&mut self.ui.editing);
        // Check spelling as you type follows the saved preference (#1792).
        self.session.view.proofing = self.session.prefs.check_spelling;
        self.session.view.read_page_color = self.session.prefs.read_page_color;
        // The saved dictionaries folder (#1063) is searched from the start.
        if let Some(f) = self.session.prefs.dictionaries_folder.as_deref() {
            wordcraft_engine::cmd::options::use_dictionaries_folder(Some(f));
        }
        self.session.read_aloud.set_rate(self.ui.read_aloud_rate);
        self.session.read_aloud.skip_citations = self.ui.read_aloud_skip_citations;
        self.session.view.dark_mode = self.ui.dark_page;
        self.session.view.nav_pane = self.ui.nav_pane;
        self.session.view.styles_pane_dock = self.ui.styles_pane_dock;
        self.session.view.proofing = self.ui.check_spelling;
        self.session.prefs.check_spelling = self.ui.check_spelling;
        *self.session.keymap_mut() = std::mem::take(&mut self.ui.keyboard).0;
        // Ribbon and toolbar commands this version doesn't have are dropped.
        let reg = self.session.registry.clone();
        self.session.ribbon = std::mem::take(&mut self.ui.ribbon);
        self.session.ribbon.retain_known(&reg);
        self.session.view.draw.pens = std::mem::take(&mut self.ui.ink_pens);
    }

    /// Keeps the app's preferences in `settings` and applies what it holds. Changes are saved
    /// through the store by [`WordApp::save_prefs_step`].
    #[must_use]
    pub fn with_settings(mut self, settings: SharedSettings<Prefs>) -> Self {
        let loaded = settings.lock().get().clone();
        self.apply_prefs(loaded);
        self.settings = Some(settings);
        self
    }

    /// Tells the notices what the store reported when it loaded: values reset, a file moved
    /// aside, a store that will not save. The host calls it once with the report `open` gave.
    pub fn settings_loaded(&mut self, ctx: &egui::Context, report: &LoadReport) {
        let mut load = SettingsLoad::new().reset(report.section_errors.iter().map(|e| e.section.clone()).collect::<Vec<_>>());
        if let Some(path) = &report.broken_moved_to {
            load = load.moved_aside(path.display().to_string());
        }
        if let Some(why) = &report.read_only {
            load = load.read_only(why.to_string());
        }
        self.settings_notices.loaded(ctx, &load);
    }

    /// One frame of saving: hands the store the current preferences (it schedules a save only when
    /// they changed), saves when its delay has passed, and tells the notices how a save ended.
    /// A frame's second pass finds the store unchanged and does nothing more.
    pub(crate) fn save_prefs_step(&mut self, ctx: &egui::Context) {
        let Some(settings) = self.settings.clone() else { return };
        let current = self.prefs();
        let result = {
            let mut store = settings.lock();
            store.set(current);
            store.tick(craft::ui::runtime::motion::now())
        };
        if let Some(at) = result.next_deadline {
            ctx.request_repaint_after(at.saturating_duration_since(craft::ui::runtime::motion::now()));
        }
        match result.status {
            SaveStatus::Saved => self.settings_notices.ticked(ctx, &SaveResult::Saved),
            SaveStatus::Failed(e) => self.settings_notices.ticked(ctx, &SaveResult::Failed(e.to_string())),
            SaveStatus::Idle | SaveStatus::Pending => {}
        }
    }

    /// Saves the preferences now, not at the next tick (#2716): a document taken off the recent
    /// list should survive the app being killed. `None`: no store keeps them; `Some(Err)`: the
    /// save failed, and the status line says so.
    pub(crate) fn flush_prefs(&mut self) -> Option<Result<(), String>> {
        let settings = self.settings.clone()?;
        let current = self.prefs();
        let mut store = settings.lock();
        store.set(current);
        match store.flush() {
            Ok(()) => Some(Ok(())),
            Err(e) => {
                drop(store);
                let e = e.to_string();
                self.status(crate::i18n::fmt(tl!("Could not save the preferences: {error}"), &[("error", e.as_str())]));
                Some(Err(e))
            }
        }
    }

    /// The screen mode the document window is in: the one mapping of the view's Full Screen,
    /// Focus and Full Screen View flags to the shell's [`ScreenMode`].
    pub(crate) fn screen_mode(&self) -> ScreenMode {
        crate::chrome::Screen::mode(&self.session.view)
    }

    /// Follows the window into `mode` (the shell sends the window actions itself): sets only the
    /// view flag of that mode and clears the others, so leaving full screen ends all three (#980,
    /// #1678). A mode outside the Document set changes nothing.
    pub(crate) fn follow_screen_mode(&mut self, mode: ScreenMode) {
        let v = &mut self.session.view;
        let (full, focus, view) = match mode {
            ScreenMode::Standard => (false, false, false),
            ScreenMode::FullScreenWithMenuBar => (true, false, false),
            ScreenMode::Focus => (false, true, false),
            ScreenMode::Immersive => (false, false, true),
            _ => return,
        };
        (v.full_screen, v.focus_mode, v.full_screen_view) = (full, focus, view);
    }
}
