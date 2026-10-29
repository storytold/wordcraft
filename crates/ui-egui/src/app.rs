use super::clock::now_ms;
#[cfg(not(target_arch = "wasm32"))]
use super::file_browser;
use super::services::Services;
use super::state::{AutoSaveBlock, DefaultApp, PromptSave, SaveChoice, keeps_everything};
use super::{
    AwaitingSave, ControlRequest, DEFAULT_TAB, MAX_DICTIONARY, MAX_RECENT, QUIT, SaveInFlight, Session, UiState, Value, WordApp, applied_font,
    async_saves, backstage, canvas, chrome, colors, content_controls, crop, desktop_theme, dialogs, dialogs_table_style, documents, file_dialogs,
    i18n, json, keys, keytips, options, panes, paste_picture, printing, push_recent_font, read_aloud, recent_fonts_from, ribbon, scroll,
    system_clipboard, table_pen, theme, updates_ui, widgets, window_ui, windows, zotero,
};

mod commands;
mod control;
mod files;
mod fonts;
mod frame;
mod input;
mod pictures;
mod prefs;

pub(crate) use commands::{discards_document, works_without_document};
pub(crate) use input::control_click_as_secondary;

impl WordApp {
    /// Creates the app: adopts the system locale, then initializes the document session, UI state,
    /// services, printing, canvas, dialogs, previews, control and screenshot state, font and theme
    /// tracking.
    pub fn new(session: Session, services: Services) -> Self {
        // Numeral › System follows the system locale, found here rather than in layout.
        if let Some(tag) = sys_locale::get_locale() {
            wordcraft_engine::set_system_locale(&tag);
        }
        // What the window shows starts as a new window's.
        let window_ui::WindowUi {
            canvas,
            controls,
            dialog,
            colors,
            restrict,
            status_msg,
            previews,
            change_picture_target,
            recipient_list_pending,
            dialog_pick,
            focus_ribbon,
            control_click,
            keytip_rects,
            nav_search_focus,
            comment_fonts_for,
            find_fonts_for,
            sent_title,
            sent_full_screen,
            seen_full_screen,
            escape_free,
            word_count,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            smart_art_row_focused,
            smart_art_typing,
            outline_prev_tab,
            read_aloud_at,
            awaiting_save,
            file_dialog,
            info_editing,
            shown_document,
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
            tab,
            backstage,
            backstage_page,
            print,
            nav_collapsed,
            keytip_prefix,
            keytips,
            alt_pressed,
            alt_chord_used,
            alt_key_down,
            alt_modifier_seen,
        } = window_ui::WindowUi::new(session.document_id());
        WordApp {
            session,
            ui: UiState {
                tab,
                backstage,
                backstage_page,
                print,
                nav_collapsed,
                keytip_prefix,
                keytips,
                alt_pressed,
                alt_chord_used,
                alt_key_down,
                alt_modifier_seen,
                ..UiState::default()
            },
            services,
            printing: Default::default(),
            print_feedback_seen: None,
            integrated_titlebar: false,
            mac_titlebar: chrome::MacTitleBar::default(),
            updates_rt: updates_ui::Runtime::default(),
            control_rx: None,
            shots: crate::control::Shots::default(),
            capture_epoch: 0,
            surface_geometry: None,
            capture_geometry: None,
            synthetic: Vec::new(),
            synthetic_pending: false,
            synthetic_injected: 0,
            styled: false,
            fonts_hans: false,
            fonts_korean: false,
            fonts_system_cjk: false,
            want_system_cjk: false,
            recover_fonts_for: None,
            recent_fonts_for: Vec::new(),
            fonts_frames: 0,
            applied_theme: None,
            theme_dark: false,
            theme_seen: false,
            native_scale: 1.0,
            legacy_interface_size: None,
            // Inert until the app binary calls `watch_desktop_theme`.
            desktop_theme: desktop_theme::DesktopTheme::default(),
            desktop_dark: None,
            applied_editing: None,
            system_animations: None,
            applied_animations: None,
            frame_ms: 0.0,
            quit_requested: false,
            autosave: true,
            zotero: zotero::ZoteroLink::default(),
            ctx: None,
            ime_origin: false,
            read_aloud_error: None,
            autosave_path: None,
            saves_in_flight: Vec::new(),
            font_generation: wordcraft_fonts::FontDb::global().generation(),
            recovery_seen: false,
            no_document: None,
            comment_drafts: Default::default(),
            parked: Default::default(),
            windows: windows::Windows::new(shown_document),
            print_document: None,
            canvas,
            controls,
            dialog,
            colors,
            restrict,
            status_msg,
            previews,
            change_picture_target,
            recipient_list_pending,
            dialog_pick,
            focus_ribbon,
            control_click,
            keytip_rects,
            nav_search_focus,
            comment_fonts_for,
            find_fonts_for,
            sent_title,
            sent_full_screen,
            seen_full_screen,
            escape_free,
            word_count,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            smart_art_row_focused,
            smart_art_typing,
            outline_prev_tab,
            read_aloud_at,
            awaiting_save,
            file_dialog,
            info_editing,
            shown_document,
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
        }
    }

    pub fn with_control(mut self, rx: std::sync::mpsc::Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Start-up with no document to open (#766): show File › Home (Blank document, the templates
    /// and Recent), as Word starts, when the Start screen setting is on (the default). No document
    /// is open until the user picks Blank document or a template or opens a file (#1129): Esc or
    /// the back arrow shows an empty window with the ribbon greyed out, as Word does. Hosts call
    /// this after [`WordApp::apply_prefs`] when they have nothing to open; [`WordApp::new`] never
    /// does, so tests, `ui_shot` and windows opened for a document start in the editor. Returns
    /// whether the Start screen shows.
    pub fn start_screen(&mut self) -> bool {
        if self.ui.start_screen {
            self.ui.backstage = true;
            self.ui.backstage_page = "home".into();
            self.no_document = Some(self.session.document_id());
            self.session.release_untitled_name();
        }
        self.ui.start_screen
    }

    /// Whether a document is open: false from a Start screen launch until the user creates or
    /// opens one (#1129).
    pub fn has_document(&self) -> bool {
        self.no_document != Some(self.session.document_id())
    }

    /// Terminal feedback also reaches the normal window when printing starts through control.
    pub fn poll_printing(&mut self, ctx: &egui::Context) {
        self.printing.poll(ctx);
        self.print_feedback();
    }

    /// Per-frame logic before layout: control requests, screenshots, shortcuts, file drops.
    pub fn logic(&mut self, ctx: &egui::Context) {
        updates_ui::tick(self, ctx);
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        // New documents are written in the interface's language when WordCraft can proof it
        // (a Polish interface: Polish spelling, grammar and hyphenation from the first word).
        // An English interface writes the region's English (English (UK) on a British system,
        // #1045), a Spanish one the region's Spanish (Spanish (Uruguay) on a Uruguayan system,
        // Spain's elsewhere, #3360).
        let editing = match lang.editing_lang() {
            Some(t) => Some(t.to_string()),
            None if lang.code() == "en" => self.session.region_english(),
            None if lang.code() == "es" => Some(self.session.region_spanish()),
            None => None,
        };
        if self.applied_editing.as_ref().is_none_or(|(l, e)| *l != lang || *e != editing) {
            self.session.set_editing_language(editing.as_deref());
            self.applied_editing = Some((lang, editing));
        }
        options::sync(self);
        self.settle_async_saves();
        // Chinese text wants the Chinese face before the Japanese one (one glyph style per line).
        // An installed CJK font is read only once CJK text is shown (#241): a CJK interface, the
        // language names in Options, or a file name the caption is drawing. A file opened later
        // in this frame (dialog, drop, control channel) is picked up again at the end.
        self.ensure_caption_fonts();
        self.ensure_comment_fonts();
        self.ensure_recover_fonts();
        self.ensure_recent_fonts();
        self.ensure_find_fonts();
        self.install_ui_fonts(ctx, lang);
        if let Some(answer) = self.desktop_theme.take_change() {
            self.desktop_dark = Some(answer);
        }
        // Both palettes are installed and egui picks one: with `System` it follows an OS appearance
        // change live (winit reports it and repaints, #311). Where winit is silent (Linux) the
        // desktop portal's answer fills in instead. The interface follows the Interface theme
        // setting alone; Dark page only changes how pages are drawn (#312, #1722).
        let effective = match self.ui.theme {
            theme::Appearance::System if ctx.system_theme().is_none() => match self.desktop_dark {
                Some(true) => theme::Appearance::Dark,
                Some(false) => theme::Appearance::Light,
                None => theme::Appearance::System,
            },
            other => other,
        };
        if self.applied_theme != Some(effective) {
            theme::apply(ctx, effective);
            self.applied_theme = Some(effective);
        }
        // `System` follows the OS inside egui, so the palette actually showing is read back.
        self.note_theme(theme::Tokens::get(ctx).dark);
        // Interface size (#475, #822): egui's zoom factor multiplies the display scale the system
        // reports (native pixels per point). Auto keeps that scale; a chosen size divides it back
        // out, so "100%" is one screen pixel per point even where the system's scale is a guess.
        // Recomputed every frame, so a move to a monitor with another scale keeps a chosen size.
        // Keyboard zoom is off (above), so nothing else changes the factor; `set_zoom_factor`
        // repaints only on change.
        self.native_scale = theme::usable_scale(ctx.native_pixels_per_point().unwrap_or(1.0));
        if let Some(native) = ctx.native_pixels_per_point()
            && let Some(old) = self.legacy_interface_size.take()
        {
            self.ui.interface_size = theme::migrate_legacy_interface_size(old, native);
        }
        let zoom = self.ui.interface_size.zoom_factor(self.native_scale);
        if ctx.zoom_factor() != zoom {
            ctx.set_zoom_factor(zoom);
        }
        // Animation effects: the host is asked every frame what the system says (it answers from
        // memory), so a change made in the system's settings is followed without a restart.
        self.read_system_animations();
        let animate = self.animations_on();
        if self.applied_animations != Some(animate) {
            theme::apply_animations(ctx, animate);
            self.applied_animations = Some(animate);
        }
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        // Capture geometry/window/document identity is current before control admission and
        // completion.
        self.refresh_capture_geometry(ctx);
        let native = self.windows.native();
        if native {
            self.printing.poll(ctx);
        } else {
            self.poll_printing(ctx);
        }
        self.drain_control(ctx);
        // The active window's own work; with native windows each window does it again as it
        // draws, for itself.
        self.window_work(ctx);
        self.drain_inbox();
        self.drain_pasted();
        self.watch_fonts(ctx);
        // A script, an agent or a later result may have made another document the active one.
        self.sync_document();
        let document = self.session.document_id();
        self.session.recovery_tick();
        if self.session.document_id() != document {
            self.ui.backstage = false;
            self.sync_document();
        }
        if !self.recovery_seen && self.session.recovery_ready() {
            self.recovery_seen = true;
            if !self.session.recovery_documents().is_empty() && !self.session.dirty {
                self.ui.backstage = true;
                self.ui.backstage_page = "recover".into();
            }
        }
        if let Some(wait) = self.session.recovery_wait() {
            ctx.request_repaint_after(wait);
        }
        if !native {
            self.window_input(ctx);
        }
        self.send_control_input(ctx);
        // Screenshot deadlines, geometry and completions belong to `logic`; issuing the one
        // callback command belongs to `ui` (`flush_shot_and_synthetic`). A queued/settling capture
        // needs a real UI pass, and so does an injected synthetic step, even while the window is
        // hidden.
        crate::control::drain_screenshots(self, ctx);
        if self.shots.in_flight() || self.synthetic_pending || !self.synthetic.is_empty() {
            let viewport = self.active_viewport();
            if ctx.input_for(viewport, |i| i.viewport().visible()) != Some(true) {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::RequestPaintWhileHidden);
            }
            ctx.request_repaint_of(viewport);
            ctx.request_repaint();
        }
        // The open above may have changed the file name the caption draws.
        self.ensure_caption_fonts();
        self.ensure_comment_fonts();
        self.ensure_recover_fonts();
        self.ensure_recent_fonts();
        self.ensure_find_fonts();
        self.install_ui_fonts(ctx, lang);
        self.report_dirty();
    }

    /// A window's own work, with its state in place: print feedback for its document, Zotero,
    /// Read Aloud, its file dialog, AutoSave of its document. Runs in [`WordApp::logic`] for the
    /// active window and, with native windows, as each window draws.
    pub(crate) fn window_work(&mut self, ctx: &egui::Context) {
        if self.windows.native() {
            let doc = self.session.document_id();
            if self.print_document.is_none_or(|d| d == doc || !self.session.is_open(d)) {
                self.print_feedback();
            }
        }
        zotero::poll(self, ctx);
        read_aloud::poll(self, ctx);
        self.clear_stale_change_picture();
        let _ = self.poll_file_dialog();
        self.autosave_tick(now_ms());
        if self.autosaves() && self.session.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        }
    }

    /// A window's own input before it draws: ⌘Q. Runs in [`WordApp::logic`] with one window,
    /// else as each window draws (each native window has its own input). The SmartArt Text
    /// Pane's Tab keys and dropped files are read in the first UI pass
    /// ([`WordApp::handle_ui_input_events`]).
    pub(crate) fn window_input(&mut self, ctx: &egui::Context) {
        if self.windows.native() && ctx.input_mut(|i| i.consume_shortcut(&QUIT)) {
            self.quit();
        }
        // The drawn window's caption, comments and Find results borrow faces it may not have.
        if self.windows.native() {
            self.ensure_caption_fonts();
            self.ensure_comment_fonts();
            self.ensure_find_fonts();
        }
    }

    /// Lay out the windows: on the desktop each open document's in its own native window
    /// ([`windows`]), on the web the one window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.windows.native() {
            self.windows_ui(ui);
        } else {
            self.window_ui(ui);
        }
    }

    /// Lay out the drawn window. A command that brings another window's state in (New, Open,
    /// Switch Windows) ends this window's drawing for the frame: the rest would be the other
    /// window's.
    pub(crate) fn window_ui(&mut self, ui: &mut egui::Ui) {
        let window = self.windows.shown();
        let moved = |app: &WordApp| app.windows.shown() != window;
        let t0 = now_ms();
        let ctx = ui.ctx().clone();
        i18n::set_current(i18n::Lang::from_pref(&self.ui.language));
        // Animation effects off: egui's scroll areas get a wheel notch whole, not eased in.
        if !self.animations_on() {
            let opts = ctx.options(|o| o.input_options);
            ctx.input_mut(|i| i.smooth_scroll_delta = scroll::unsmoothed_scroll(i, &opts));
        }
        self.handle_ui_input_events(&ctx);
        // Fonts installed by `logic` take effect on the next frame.
        if self.fonts_frames < 2 {
            self.fonts_frames += 1;
            ctx.request_repaint();
            return;
        }
        let t = theme::Tokens::get(&ctx);
        self.escape_free = self.dialog.is_none() && !self.ui.backstage && self.ui.keytips == keytips::Phase::Off && !egui::Popup::is_any_open(&ctx);
        // Keytips are read before layout so the frame paints the new state; badges paint after
        // the ribbon so they sit on top of it.
        crate::keytips::logic(self, &ctx);
        // A failed Recent open, or a failed Save As, is news only until File closes (#2938).
        if !self.ui.backstage {
            backstage::forget_open_error(&ctx);
            backstage::forget_save_as_error(&ctx);
        }
        self.paint_window(ui, &ctx, t);
        if !moved(self) {
            self.show_overlays(ui, &ctx);
        }
        if !moved(self) {
            self.sync_title_and_frame(&ctx, t0);
        }
        // The synthetic step this frame took is done even when drawing ended early.
        self.flush_shot_and_synthetic(&ctx);
    }
}
