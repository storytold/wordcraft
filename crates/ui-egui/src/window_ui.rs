//! What one window shows and is doing, apart from the rest of the app. [`WordApp`] holds the
//! state of the window being drawn in its own fields, so drawing code reads `app.canvas`,
//! `app.dialog`, `app.ui.tab` whatever window it is; every other window keeps its state in a
//! [`WindowUi`]. [`WordApp::swap_window`] exchanges the two by moving owned fields, so it is
//! cheap enough to run for every window on every frame.
//!
//! Every field of [`WordApp`] and [`UiState`] is one of:
//! - per window: listed in [`WindowUi`] and swapped;
//! - per document: kept for the documents not shown in [`WordApp`]'s `parked` map
//!   ([`WordApp::sync_document`]) or in the engine's session;
//! - app-wide: the session, preferences, fonts, theme, the control channel, timers and workers.
//!
//! `swap_window` names every field, so a new field doesn't compile until it is classified.

use crate::{AwaitingSave, UiState, WordApp, canvas, colors, content_controls, dialogs, dialogs_review, file_dialogs, keytips, previews, print_page};

/// A window's state while another window is drawn. Fields mirror their namesakes in [`WordApp`]
/// and, for the ribbon tab, Backstage, printing and keytips, in [`UiState`].
pub(crate) struct WindowUi {
    pub(crate) canvas: canvas::CanvasState,
    pub(crate) controls: content_controls::ControlUi,
    pub(crate) dialog: Option<dialogs::Dialog>,
    pub(crate) colors: Option<colors::ColorsDialog>,
    pub(crate) restrict: dialogs_review::RestrictPane,
    pub(crate) status_msg: Option<(String, f64)>,
    pub(crate) previews: previews::Previews,
    pub(crate) change_picture_target: Option<String>,
    pub(crate) recipient_list_pending: bool,
    pub(crate) dialog_pick: Option<String>,
    pub(crate) focus_ribbon: bool,
    pub(crate) control_click: bool,
    pub(crate) keytip_rects: Vec<(egui::Rect, String)>,
    pub(crate) nav_search_focus: bool,
    pub(crate) comment_fonts_for: Option<(u64, u64)>,
    pub(crate) find_fonts_for: Option<(String, usize, u64)>,
    pub(crate) sent_title: String,
    pub(crate) sent_full_screen: bool,
    pub(crate) seen_full_screen: Option<bool>,
    pub(crate) escape_free: bool,
    pub(crate) word_count: (u64, usize),
    pub(crate) last_autosave: f64,
    pub(crate) equation_prev_tab: Option<String>,
    pub(crate) smart_art_item: Option<usize>,
    pub(crate) smart_art_row_focused: bool,
    pub(crate) smart_art_typing: Option<usize>,
    pub(crate) outline_prev_tab: Option<String>,
    pub(crate) read_aloud_at: Option<wordcraft_engine::doc::Pos>,
    pub(crate) awaiting_save: Option<AwaitingSave>,
    pub(crate) file_dialog: Option<file_dialogs::PendingDialog>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) file_browser: crate::file_browser::State,
    pub(crate) info_editing: Option<(&'static str, u64)>,
    pub(crate) shown_document: u64,
    // From UiState.
    pub(crate) tab: String,
    pub(crate) backstage: bool,
    pub(crate) backstage_page: String,
    pub(crate) print: print_page::PrintSettings,
    pub(crate) nav_collapsed: std::collections::HashSet<wordcraft_doc::Path>,
    pub(crate) keytip_prefix: String,
    pub(crate) keytips: keytips::Phase,
    pub(crate) alt_pressed: bool,
    pub(crate) alt_chord_used: bool,
    pub(crate) alt_key_down: bool,
    pub(crate) alt_modifier_seen: bool,
}

/// A word count that matches no document revision, so the next frame counts.
pub(crate) const NOT_COUNTED: (u64, usize) = (u64::MAX, 0);

impl WindowUi {
    /// A new window showing open document `document`: nothing open over the page, Home tab.
    pub(crate) fn new(document: u64) -> WindowUi {
        let UiState {
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
            ..
        } = UiState::default();
        WindowUi {
            canvas: Default::default(),
            controls: Default::default(),
            dialog: None,
            colors: None,
            restrict: Default::default(),
            status_msg: None,
            previews: Default::default(),
            change_picture_target: None,
            recipient_list_pending: false,
            dialog_pick: None,
            focus_ribbon: false,
            control_click: false,
            keytip_rects: Vec::new(),
            nav_search_focus: false,
            comment_fonts_for: None,
            find_fonts_for: None,
            sent_title: String::new(),
            sent_full_screen: false,
            seen_full_screen: None,
            escape_free: false,
            word_count: NOT_COUNTED,
            last_autosave: 0.0,
            equation_prev_tab: None,
            smart_art_item: None,
            smart_art_row_focused: false,
            smart_art_typing: None,
            outline_prev_tab: None,
            read_aloud_at: None,
            awaiting_save: None,
            file_dialog: None,
            #[cfg(not(target_arch = "wasm32"))]
            file_browser: Default::default(),
            info_editing: None,
            shown_document: document,
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
        }
    }
}

/// Swap each named field of the drawn window (bound by the caller's destructuring) with its
/// namesake in the [`WindowUi`]. The `WindowUi` pattern is exhaustive, so a field added there
/// and left out here doesn't compile.
macro_rules! swap_fields {
    ($w:ident: $($(#[$attr:meta])* $field:ident),* $(,)?) => {
        let WindowUi { $($(#[$attr])* $field: _),* } = &*$w;
        $($(#[$attr])* std::mem::swap($field, &mut $w.$field);)*
    };
}

impl WordApp {
    /// Exchange the drawn window's state with `w`'s: afterwards the app draws the window `w`
    /// held, and `w` holds the one that was drawn. App-wide and per-document state stays. The
    /// session's active document is not changed: the caller activates `w`'s document
    /// (`shown_document`) with it, or [`WordApp::sync_document`] treats the change as a switch
    /// within the window.
    pub(crate) fn swap_window(&mut self, w: &mut WindowUi) {
        let WordApp {
            // App-wide.
            session: _,
            services: _,
            printing: _,
            print_feedback_seen: _,
            font_generation: _,
            integrated_titlebar: _,
            mac_titlebar: _,
            control_rx: _,
            updates_rt: _,
            // The control channel's screenshots and input go to the active window, whichever it is.
            shots: _,
            capture_epoch: _,
            surface_geometry: _,
            capture_geometry: _,
            synthetic: _,
            synthetic_pending: _,
            synthetic_injected: _,
            // The current command's origin, set and cleared around it.
            ime_origin: _,
            styled: _,
            fonts_hans: _,
            fonts_korean: _,
            fonts_system_cjk: _,
            want_system_cjk: _,
            recent_fonts_for: _,
            // File › Recover lists the copies of every window's documents.
            recover_fonts_for: _,
            fonts_frames: _,
            applied_theme: _,
            native_scale: _,
            legacy_interface_size: _,
            desktop_theme: _,
            desktop_dark: _,
            applied_editing: _,
            system_animations: _,
            applied_animations: _,
            frame_ms: _,
            quit_requested: _,
            autosave: _,
            zotero: _,
            language_tool: _,
            ctx: _,
            read_aloud_error: _,
            saves_in_flight: _,
            recovery_seen: _,
            no_document: _,
            windows: _,
            print_document: _,
            // Per document: the shown document's, swapped with `parked` on a document switch.
            autosave_path: _,
            comment_drafts: _,
            parked: _,
            // Per window.
            ui,
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
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
            info_editing,
            shown_document,
        } = self;
        let UiState {
            // App-wide: preferences, recent lists, and the settings the session owns while
            // running.
            ribbon_collapsed: _,
            quick_access_toolbar: _,
            recent: _,
            recent_symbols: _,
            recent_colors: _,
            recent_fonts: _,
            theme: _,
            interface_size: _,
            legacy_interface_size: _,
            animations: _,
            legacy_dark: _,
            nav_tab: _,
            nav_pane: _,
            show_discord: _,
            author: _,
            window: _,
            language: _,
            editing: _,
            read_aloud_rate: _,
            read_aloud_skip_citations: _,
            language_tool: _,
            keytips_enabled: _,
            dark_page: _,
            check_spelling: _,
            caret_animation: _,
            keyboard: _,
            ribbon: _,
            caption_labels: _,
            last_caption_label: _,
            ink_pens: _,
            initials: _,
            options_pane: _,
            recent_count: _,
            mini_toolbar: _,
            alignment_guides: _,
            start_screen: _,
            print_backgrounds: _,
            update_fields_before_print: _,
            dictionary: _,
            // Where each task pane sits, and their order.
            styles_pane_dock: _,
            styles_pane_float: _,
            styles_pane_width: _,
            pane_places: _,
            pane_order: _,
            // Per window.
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
        } = ui;
        swap_fields!(w:
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
            #[cfg(not(target_arch = "wasm32"))]
            file_browser,
            info_editing,
            shown_document,
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
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::WindowUi;
    use crate::{Services, WordApp, dialogs::Dialog};

    /// Swapping in a new window shows its fresh state; swapping back restores the first window's
    /// dialog, status message, composition, scroll request and tab. The session and the
    /// preferences stay put both ways.
    #[test]
    fn swap_window_round_trip() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": "Kept"})).unwrap();
        let doc = a.session.document_id();
        let rev = a.session.rev();
        a.ui.recent_count = 7;
        a.ui.tab = "Insert".into();
        a.dialog =
            Some(Dialog::SaveChanges { name: "Report".into(), then: "file.close".into(), params: json!({}), document: doc, message: String::new() });
        a.status_msg = Some(("Saved".into(), 1.0));
        a.canvas.ime_preedit = "ni".into();
        a.canvas.ime_preedit_range = Some(0..2);
        a.canvas.scroll_to_caret = false;
        a.canvas.nav_to_caret = true;

        let mut w = WindowUi::new(doc);
        a.swap_window(&mut w);
        assert!(a.dialog.is_none() && a.status_msg.is_none());
        assert!(a.canvas.ime_preedit.is_empty() && a.canvas.ime_preedit_range.is_none());
        assert!(a.canvas.scroll_to_caret && !a.canvas.nav_to_caret, "a new window goes to the caret");
        assert_eq!(a.ui.tab, crate::DEFAULT_TAB);
        assert_eq!(w.tab, "Insert");
        assert_eq!((a.session.document_id(), a.session.rev(), a.ui.recent_count), (doc, rev, 7), "app-wide state stays");

        a.swap_window(&mut w);
        assert_eq!(a.dialog.as_ref().map(Dialog::name), Some("saveChanges"));
        assert_eq!(a.status_msg.as_ref().map(|(m, _)| m.as_str()), Some("Saved"));
        assert_eq!((a.canvas.ime_preedit.as_str(), a.canvas.ime_preedit_range.clone()), ("ni", Some(0..2)));
        assert!(!a.canvas.scroll_to_caret && a.canvas.nav_to_caret);
        assert_eq!(a.ui.tab, "Insert");
        assert!(w.dialog.is_none() && w.tab == crate::DEFAULT_TAB);
        assert_eq!((a.session.document_id(), a.session.rev(), a.ui.recent_count), (doc, rev, 7));
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Kept");
    }
}
