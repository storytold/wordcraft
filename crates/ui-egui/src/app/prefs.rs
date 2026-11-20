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
        ui.autosave = self.autosave;
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
        // The browser still cannot keep writing to a file, even with a saved desktop choice.
        self.autosave = self.ui.autosave && !self.browser_saves();
        self.session.autosave = self.autosave;
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

#[cfg(test)]
mod autosave_preference_tests {
    use crate::prefs::Prefs;
    use crate::{Services, WordApp};
    use serde_json::json;

    fn app(services: Services) -> WordApp {
        let files = std::sync::Arc::new(craft::host::ScriptedFiles::new());
        let host = craft::host::Host::builder(crate::APP).files(files).build();
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services).with_host(std::sync::Arc::new(host))
    }

    #[test]
    fn autosave_defaults_off_and_remembers_the_raw_choice() {
        let legacy: Prefs = serde_json::from_str("{}").unwrap();
        assert!(!legacy.autosave);
        let mut a = app(Services::default());
        assert!(!a.autosave && !a.session.autosave);
        for on in [true, false] {
            // File > Options > Save changes this raw choice, even before a file is eligible.
            a.autosave = on;
            a.session.autosave = on;
            let stored = serde_json::to_string(&a.prefs()).unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&stored).unwrap()["autoSave"], on);
            let mut back = app(Services::default());
            back.apply_prefs(serde_json::from_str(&stored).unwrap());
            assert_eq!(back.autosave, on);
            assert_eq!(back.session.autosave, on);
            assert!(!back.autosaves(), "remembering On grants no consent to rewrite a file");
            assert!(back.autosave_path.is_none());
        }
    }

    #[test]
    fn browser_restore_keeps_autosave_off() {
        let files = std::sync::Arc::new(craft::host::ScriptedFiles::held().in_browser());
        let host = craft::host::Host::builder(crate::APP).files(files).build();
        let mut a = app(Services::default()).with_host(std::sync::Arc::new(host));
        a.apply_prefs(Prefs { autosave: true, ..Default::default() });
        assert!(!a.autosave && !a.session.autosave && !a.autosaves());
        assert!(a.autosave_path.is_none());
    }

    #[test]
    fn switching_documents_keeps_the_app_autosave_choice() {
        let mut a = app(Services::default());
        a.run("text.insert", json!({"text": "First document"})).unwrap();
        let first = a.session.document_id();
        a.run("file.new", json!({})).unwrap();
        assert!(!a.autosave && !a.session.autosave);
        a.apply_prefs(Prefs { autosave: true, ..Default::default() });
        a.session.switch_document(first).unwrap();
        a.sync_document();
        assert!(a.autosave && a.session.autosave);
        a.run("file.new", json!({})).unwrap();
        a.apply_prefs(Prefs { autosave: false, ..Default::default() });
        a.session.switch_document(first).unwrap();
        a.sync_document();
        assert!(!a.autosave && !a.session.autosave);
    }

    #[test]
    fn native_window_document_carry_keeps_the_global_autosave_choice() {
        let mut a = app(Services::default());
        a.run("text.insert", json!({"text": "First document"})).unwrap();
        let first = a.session.document_id();
        a.autosave = true;
        a.session.autosave = true;
        a.run("file.new", json!({})).unwrap();
        let second = a.session.document_id();
        // Like File > Options in this window: the other document still carries its old mirror.
        a.autosave = false;
        a.session.autosave = false;
        a.carry_document_state(second, first);
        assert!(!a.session.autosave);
        a.autosave = true;
        a.session.autosave = true;
        a.carry_document_state(first, second);
        assert!(a.session.autosave);
        assert!(a.autosave_path.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn manual_save_does_not_opt_in_to_automatic_writes() {
        struct SavedFile(std::path::PathBuf);
        impl Drop for SavedFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = std::env::temp_dir().join(format!(
            "wordcraft-autosave-opt-in-{}-{}.docx",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        assert!(!path.exists());
        let _cleanup = SavedFile(path.clone());
        wordcraft_engine::io::save_path(&path, &wordcraft_doc::Document::new()).unwrap();
        let mut a = app(Services::default());
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        a.run("text.insert", json!({"text": "Explicitly saved"})).unwrap();
        a.run("file.save", json!({})).unwrap();
        let saved = std::fs::read(&path).unwrap();
        a.run("text.insert", json!({"text": "Still unsaved"})).unwrap();
        a.last_autosave = 0.0;
        a.canvas.caret_visible_since = 0.0;
        a.autosave_tick(10_000.0);
        assert_eq!(std::fs::read(&path).unwrap(), saved, "manual Save must not permit later automatic writes");
        assert!(a.session.dirty && !a.autosave && !a.session.autosave);
        a.run("file.autosave", json!({"value": true})).unwrap();
        a.canvas.caret_visible_since = 0.0;
        a.autosave_tick(20_000.0);
        assert_ne!(std::fs::read(&path).unwrap(), saved, "explicit On covers the same saved file");
        assert!(!a.session.dirty && a.autosaves());
    }
}
