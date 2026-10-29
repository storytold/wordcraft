use super::{DEFAULT_TAB, MAX_DICTIONARY, UiState, WordApp, desktop_theme, recent_fonts_from, theme};
use crate::{LtKeyError, LtKeyPlace, secrets};

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
        ui.author = self.session.author.clone();
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
        // A key the credential store holds is left out (#764).
        ui.language_tool_key_in_store = self.language_tool_key_in_store();
        if ui.language_tool_key_in_store {
            ui.language_tool.api_key.clear();
        }
        ui
    }

    /// Restore preferences saved by [`WordApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
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
        let author = std::mem::take(&mut self.ui.author);
        if !author.trim().is_empty() {
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
        // Keys for commands this version doesn't have are dropped.
        self.session.keymap = std::mem::take(&mut self.ui.keyboard);
        let reg = self.session.registry.clone();
        self.session.keymap.retain_known(&reg);
        // So are ribbon and toolbar commands.
        self.session.ribbon = std::mem::take(&mut self.ui.ribbon);
        self.session.ribbon.retain_known(&reg);
        self.session.view.draw.pens = std::mem::take(&mut self.ui.ink_pens);
        self.load_secrets();
    }

    /// Read the LanguageTool API key back from the credential store when `ui.json` says it is
    /// there (#764). A key still in `ui.json` (from before #764, or kept there when the store
    /// refused it) wins; [`WordApp::sync_secrets`] moves it.
    fn load_secrets(&mut self) {
        let in_store = std::mem::take(&mut self.ui.language_tool_key_in_store);
        self.lt_key = LtKeyPlace::default();
        let Some(store) = &self.services.secrets else { return };
        if !in_store || !self.ui.language_tool.api_key.is_empty() {
            return;
        }
        match store.get(secrets::LANGUAGE_TOOL_KEY) {
            Ok(Some(key)) if key.len() <= secrets::MAX_SECRET_LEN => {
                self.ui.language_tool.api_key = key.clone();
                self.lt_key.stored = Some(key);
            }
            // Removed outside WordCraft, or too long to be a key: say so rather than let it vanish.
            Ok(_) => {
                log::warn!("credential store: the LanguageTool API key is no longer there");
                self.lt_key.error = Some(LtKeyError::Lost);
            }
            Err(e) => {
                log::warn!("credential store: cannot read the LanguageTool API key: {e}");
                self.lt_key.unread = true;
                self.lt_key.error = Some(LtKeyError::Store(e));
            }
        }
    }

    /// Bring the credential store up to date with the LanguageTool API key (#764): save a new or
    /// changed key there, or forget a cleared one. Run before saving `ui.json`, which then leaves
    /// the key out ([`WordApp::prefs`]). `true` when the store changed, so `ui.json` should be
    /// written again. Without a store, or when it fails, the key stays in `ui.json` and
    /// File › Options says why.
    pub fn sync_secrets(&mut self) -> bool {
        let Some(store) = &self.services.secrets else { return false };
        let key = &self.ui.language_tool.api_key;
        if key.is_empty() {
            // The key couldn't be read: the user didn't clear it, so leave it in the store.
            if self.lt_key.unread || self.lt_key.stored.take().is_none() {
                return false;
            }
            self.lt_key.error = match store.delete(secrets::LANGUAGE_TOOL_KEY) {
                Ok(()) => None,
                Err(e) => {
                    log::warn!("credential store: cannot remove the LanguageTool API key: {e}");
                    Some(LtKeyError::NotRemoved(e))
                }
            };
            return true;
        }
        if self.lt_key.stored.as_deref() == Some(key.as_str()) {
            return false;
        }
        // A longer key would be read back as lost on the next start, so it stays in ui.json.
        if key.len() > secrets::MAX_SECRET_LEN {
            self.lt_key.error = Some(LtKeyError::Store("the key is too long for the credential store".into()));
            return false;
        }
        match store.set(secrets::LANGUAGE_TOOL_KEY, key) {
            Ok(()) => {
                self.lt_key = LtKeyPlace { stored: Some(key.clone()), unread: false, error: None };
                true
            }
            Err(e) => {
                log::warn!("credential store: cannot save the LanguageTool API key: {e}");
                self.lt_key.error = Some(LtKeyError::Store(e));
                false
            }
        }
    }

    /// Whether the LanguageTool API key is kept in the credential store rather than `ui.json`.
    pub fn language_tool_key_in_store(&self) -> bool {
        let key = &self.ui.language_tool.api_key;
        if key.is_empty() { self.lt_key.unread } else { self.lt_key.stored.as_deref() == Some(key.as_str()) }
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
