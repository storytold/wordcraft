use super::{DEFAULT_TAB, MAX_DICTIONARY, UiState, WordApp, desktop_theme, recent_fonts_from, theme};

/// What the operating system says about light and dark. macOS and Windows answer through winit;
/// winit reports nothing on Linux, where the desktop portal does ([`desktop_theme`]). `None` means
/// nobody answered, not a preference.
fn system_theme(ctx: &egui::Context, desktop_dark: Option<bool>) -> Option<egui::Theme> {
    ctx.system_theme().or_else(|| desktop_dark.map(|dark| if dark { egui::Theme::Dark } else { egui::Theme::Light }))
}

impl WordApp {
    /// Preferences to save between runs: the UI state plus the current user name and editing
    /// preferences.
    pub fn prefs(&self) -> UiState {
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
        ui.keyboard = self.session.keymap.clone();
        ui.ribbon = self.session.ribbon.clone();
        ui.ink_pens = self.session.view.draw.pens.clone();
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
        // The browser still cannot keep writing to a file, even with a saved desktop choice.
        self.autosave = self.ui.autosave && self.services.download.is_none();
        self.session.autosave = self.autosave;
        self.ui.backstage = false;
        // Word opens on Home every launch, so the tab `ui.json` holds is not restored. Going File
        // and back to the editor within the run still returns to the tab that was showing.
        self.ui.tab = DEFAULT_TAB.into();
        let recent_fonts = recent_fonts_from(self.ui.recent_fonts.iter().map(String::as_str));
        self.ui.recent_fonts = recent_fonts;
        // A `ui.json` from before the theme setting: its dark-mode switch picks Dark or Light.
        if let Some(dark) = self.ui.legacy_dark.take() {
            self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
        }
        // A `ui.json` from before #822: its interface size was on top of the display scale, so it
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
        // Keys for commands this version doesn't have are dropped.
        self.session.keymap = std::mem::take(&mut self.ui.keyboard);
        let reg = self.session.registry.clone();
        self.session.keymap.retain_known(&reg);
        // So are ribbon and toolbar commands.
        self.session.ribbon = std::mem::take(&mut self.ui.ribbon);
        self.session.ribbon.retain_known(&reg);
        self.session.view.draw.pens = std::mem::take(&mut self.ui.ink_pens);
    }

    /// Follow the desktop's light/dark choice while the app runs (Linux, where winit reports no
    /// system theme; see [`desktop_theme`]). The desktop app calls this once at start-up; unit
    /// tests and `ui_shot` don't, so what they draw doesn't depend on the host's desktop theme.
    pub fn watch_desktop_theme(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.desktop_theme = desktop_theme::DesktopTheme::start(move || ctx.request_repaint());
    }

    /// Whether the interface theme setting currently resolves to dark (`System` asks the OS).
    pub fn ui_is_dark(&self) -> bool {
        self.ui.theme.is_dark(self.ctx.as_ref().and_then(|ctx| system_theme(ctx, self.desktop_dark)))
    }

    /// Whether animation effects are on: the Animation effects setting, where `System` follows
    /// what the system last reported (on when it reports nothing).
    pub fn animations_on(&self) -> bool {
        self.ui.animations.enabled(self.system_animations)
    }

    /// What the system last said about its animation effects (`None`: it doesn't say).
    pub fn system_animations(&self) -> Option<bool> {
        self.system_animations
    }

    /// Ask the host for the system's animation setting.
    pub(super) fn read_system_animations(&mut self) {
        self.system_animations = self.services.animations_enabled.as_ref().and_then(|ask| ask());
    }

    /// Carry View › Full Screen (`view.fullScreen`, F11), Focus and Full Screen View (#1678) to
    /// the window, and follow the window when the system takes it in or out of full screen (the
    /// macOS title bar button, the window manager): in is Full Screen, out ends all three. Leaving
    /// full screen gives the window back its earlier size and place: the system keeps them, and
    /// [`window_geometry`] never saves a full-screen size (#980).
    pub(crate) fn sync_full_screen(&mut self, ctx: &egui::Context) {
        let v = &self.session.view;
        let want = v.full_screen || v.focus_mode || v.full_screen_view;
        let seen = ctx.input(|i| i.viewport().fullscreen);
        if seen != self.seen_full_screen {
            self.seen_full_screen = seen;
            if let Some(on) = seen
                && on != want
            {
                let v = &mut self.session.view;
                v.full_screen = on;
                if !on {
                    v.focus_mode = false;
                    v.full_screen_view = false;
                }
                self.sent_full_screen = on;
            }
        }
        let v = &self.session.view;
        let want = v.full_screen || v.focus_mode || v.full_screen_view;
        if want != self.sent_full_screen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(want));
            self.sent_full_screen = want;
        }
    }
}

#[cfg(test)]
mod autosave_preference_tests {
    use crate::{Services, UiState, WordApp};
    use serde_json::json;

    fn app(services: Services) -> WordApp {
        WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services)
    }

    #[test]
    fn autosave_defaults_off_and_remembers_the_raw_choice() {
        let legacy: UiState = serde_json::from_str("{}").unwrap();
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
        let mut a = app(Services { download: Some(Box::new(|_, _| Ok(()))), ..Default::default() });
        a.apply_prefs(UiState { autosave: true, ..Default::default() });
        assert!(!a.autosave && !a.session.autosave && !a.autosaves());
        assert!(a.autosave_path.is_none());
    }

    #[test]
    fn switching_documents_keeps_the_app_autosave_choice() {
        let mut a = app(Services::default());
        let first = a.session.document_id();
        a.run("file.new", json!({})).unwrap();
        assert!(!a.autosave && !a.session.autosave);
        a.apply_prefs(UiState { autosave: true, ..Default::default() });
        a.session.switch_document(first).unwrap();
        a.sync_document();
        assert!(a.autosave && a.session.autosave);
        a.run("file.new", json!({})).unwrap();
        a.apply_prefs(UiState { autosave: false, ..Default::default() });
        a.session.switch_document(first).unwrap();
        a.sync_document();
        assert!(!a.autosave && !a.session.autosave);
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
