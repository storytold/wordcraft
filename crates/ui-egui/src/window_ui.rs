//! What one window shows and is doing, apart from the rest of the app. [`WordApp`] holds the
//! state of the window being drawn in its own fields, so drawing code reads `app.canvas`,
//! `app.dialog_host`, `app.ui.tab` whatever window it is; every other window keeps its state in a
//! [`WindowUi`]. [`WordApp::swap_window`] exchanges the two by moving owned fields, so it is
//! cheap enough to run for every window on every frame.
//!
//! Every field of [`WordApp`] and [`Prefs`] is one of:
//! - per window: listed in [`WindowUi`] and swapped;
//! - per document: kept for the documents not shown in [`WordApp`]'s `parked` map
//!   ([`WordApp::sync_document`]) or in the engine's session;
//! - app-wide: the session, preferences, the host, the control channel, timers and workers.
//!
//! `swap_window` names every field, so a new field doesn't compile until it is classified.

use crate::prefs::Prefs;
use crate::{Status, WordApp, backstage, canvas, chrome, colors, dialogs, file_dialogs, format_shape, previews, print_page};

/// A window's state while another window is drawn. Fields mirror their namesakes in [`WordApp`]
/// and, for the ribbon tab and Backstage, in [`Prefs`].
pub(crate) struct WindowUi {
    pub(crate) canvas: canvas::CanvasState,
    pub(crate) restrict: crate::restrict_pane::RestrictPane,
    pub(crate) spelling_review: Option<crate::proofing_pane::SpellingForm>,
    pub(crate) status: Option<Status>,
    /// Announcement belonging to this document window.
    pub(crate) announcement: Option<crate::announce::Announcement>,
    pub(crate) previews: previews::Previews,
    pub(crate) change_picture_target: Option<String>,
    pub(crate) recipient_list_pending: bool,
    pub(crate) ribbon_dropped: bool,
    pub(crate) nav_search_focus: bool,
    pub(crate) escape_free: bool,
    pub(crate) status_counts: (u64, crate::StatusCounts),
    /// Per window, as each window shows its own layout: one shared list would be read again
    /// on every frame of every window.
    pub(crate) missing_fonts: chrome::MissingFontsMemo,
    /// This window's refused or failed export, shown on its Export and Print pages (Backstage hides the status bar).
    pub(crate) export_error: String,
    pub(crate) last_autosave: f64,
    pub(crate) equation_prev_tab: Option<String>,
    pub(crate) smart_art_item: Option<usize>,
    pub(crate) outline_prev_tab: Option<String>,
    pub(crate) read_aloud_at: Option<wordcraft_engine::doc::Pos>,
    pub(crate) file_dialog: Option<file_dialogs::PendingDialog>,
    pub(crate) info_editing: Option<(&'static str, u64)>,
    pub(crate) shown_document: u64,
    pub(crate) backstage_state: backstage::BackstageState,
    pub(crate) chrome: chrome::ChromeOpen,
    pub(crate) dialog_host: craft::ui::dialog::DialogHost,
    pub(crate) dialog_hooks: Vec<(craft::ui::dialog::DialogId, dialogs::host::DialogHooks)>,
    pub(crate) dialog_answers: Vec<(craft::ui::dialog::DialogId, String, serde_json::Value)>,
    pub(crate) edits: craft::ui::edit::GestureHost<WordApp>,
    pub(crate) keytips_active: bool,
    pub(crate) options_state: craft::ui::patterns::preferences::PreferencesState,
    pub(crate) print: print_page::PrintPage,
    pub(crate) shape_variation: Option<format_shape::Variation>,
    pub(crate) equation_symbol_set: usize,
    pub(crate) md_source_visit: u64,
    pub(crate) more_colors: std::collections::HashMap<String, colors::Target>,
    // From Prefs.
    pub(crate) tab: String,
    pub(crate) backstage: bool,
    pub(crate) backstage_page: String,
    pub(crate) nav_collapsed: std::collections::HashSet<wordcraft_doc::Path>,
}

/// Status-bar counts that match no document revision, so the next frame counts.
pub(crate) const NOT_COUNTED: (u64, crate::StatusCounts) = (u64::MAX, crate::StatusCounts { words: 0, chars: 0, chars_no_spaces: 0 });

impl WindowUi {
    /// A new window showing open document `document`: nothing open over the page, Home tab.
    pub(crate) fn new(document: u64) -> WindowUi {
        let Prefs { tab, backstage, backstage_page, nav_collapsed, .. } = Prefs::default();
        WindowUi {
            canvas: Default::default(),
            restrict: Default::default(),
            spelling_review: None,
            status: None,
            announcement: None,
            previews: Default::default(),
            change_picture_target: None,
            recipient_list_pending: false,
            ribbon_dropped: false,
            nav_search_focus: false,
            escape_free: false,
            status_counts: NOT_COUNTED,
            missing_fonts: Default::default(),
            export_error: String::new(),
            last_autosave: 0.0,
            equation_prev_tab: None,
            smart_art_item: None,
            outline_prev_tab: None,
            read_aloud_at: None,
            file_dialog: None,
            info_editing: None,
            shown_document: document,
            backstage_state: Default::default(),
            chrome: Default::default(),
            dialog_host: craft::ui::dialog::DialogHost::new(),
            dialog_hooks: Default::default(),
            dialog_answers: Default::default(),
            edits: Default::default(),
            keytips_active: false,
            options_state: Default::default(),
            print: Default::default(),
            shape_variation: None,
            equation_symbol_set: Default::default(),
            md_source_visit: 0,
            more_colors: Default::default(),
            tab,
            backstage,
            backstage_page,
            nav_collapsed,
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
            // A close operation walks every window's documents, so it outlives a window swap.
            closing: _,
            native_prompt: _,
            session: _,
            services: _,
            host: _,
            settings: _,
            settings_notices: _,
            command_catalog: _,
            command_catalog_mac: _,
            command_catalog_lang: _,
            key_overrides: _,
            command_states: _,
            fonts_frames: _,
            printing: _,
            print_feedback_seen: _,
            print_gen: _,
            font_generation: _,
            updates_rt: _,
            font_notice_seen: _,
            font_notices: _,
            packs_rt: _,
            // The control channel's screenshots and input go to the active window, whichever it is.
            control: _,
            capture_token: _,
            fallback_chooser: _,
            // The current command's origin, set and cleared around it.
            ime_origin: _,
            user_origin: _,
            native_scale: _,
            auto_scale_limit: _,
            legacy_interface_size: _,
            applied_editing: _,
            frame_ms: _,
            quit_requested: _,
            autosave: _,
            zotero: _,
            ctx: _,
            read_aloud_error: _,
            saves_in_flight: _,
            recovery_seen: _,
            no_document: _,
            windows: _,
            print_document: _,
            clipboard_published: _,
            // Per document: the shown document's, swapped with `parked` on a document switch.
            autosave_path: _,
            parked: _,
            // Per window.
            ui,
            canvas,
            restrict,
            spelling_review,
            status,
            announcement,
            previews,
            change_picture_target,
            recipient_list_pending,
            ribbon_dropped,
            nav_search_focus,
            escape_free,
            status_counts,
            missing_fonts,
            export_error,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            outline_prev_tab,
            read_aloud_at,
            file_dialog,
            info_editing,
            shown_document,
            backstage_state,
            chrome,
            dialog_host,
            dialog_hooks,
            dialog_answers,
            edits,
            keytips_active,
            options_state,
            print,
            shape_variation,
            equation_symbol_set,
            md_source_visit,
            more_colors,
        } = self;
        let Prefs {
            // App-wide: preferences, recent lists, and the settings the session owns while
            // running.
            ribbon_collapsed: _,
            quick_access_toolbar: _,
            recent: _,
            recent_symbols: _,
            date_time_format: _,
            date_time_language: _,
            date_time_update: _,
            recent_colors: _,
            font_picker: _,
            theme: _,
            interface_size: _,
            legacy_interface_size: _,
            animations: _,
            legacy_dark: _,
            nav_tab: _,
            display_server: _,
            status_count: _,
            scroll_speed: _,
            touchpad_scroll_speed: _,
            nav_pane: _,
            show_discord: _,
            author: _,
            language: _,
            editing: _,
            read_aloud_rate: _,
            read_aloud_skip_citations: _,
            keytips_enabled: _,
            dark_page: _,
            check_spelling: _,
            caret_animation: _,
            keyboard: _,
            print: _,
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
            font_download_notice: _,
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
            nav_collapsed,
        } = ui;
        swap_fields!(w:
            canvas,
            restrict,
            spelling_review,
            status,
            announcement,
            previews,
            change_picture_target,
            recipient_list_pending,
            ribbon_dropped,
            nav_search_focus,
            escape_free,
            status_counts,
            missing_fonts,
            export_error,
            last_autosave,
            equation_prev_tab,
            smart_art_item,
            outline_prev_tab,
            read_aloud_at,
            file_dialog,
            info_editing,
            shown_document,
            backstage_state,
            chrome,
            dialog_host,
            dialog_hooks,
            dialog_answers,
            edits,
            keytips_active,
            options_state,
            print,
            shape_variation,
            equation_symbol_set,
            md_source_visit,
            more_colors,
            tab,
            backstage,
            backstage_page,
            nav_collapsed,
        );
        // The comment panel's state (the selected card and the comment and reply drafts) is the shown document's, not the
        // window's: it stays through the swap, and `carry_document_state` parks it with its document and brings back the
        // arriving one's, as production did with its app-wide drafts. Swapped with the canvas, the switch would park the
        // arriving window's state under the document that left.
        std::mem::swap(&mut canvas.comments, &mut w.canvas.comments);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::WindowUi;
    use crate::{Services, Status, WordApp};

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
        let _ = crate::dialogs::open(&mut a, "tabs", &json!({}));
        let is_tabs = |a: &WordApp| a.dialog_host.open_dialogs().iter().any(|d| d.key.as_deref() == Some("tabs"));
        assert!(is_tabs(&a), "the dialog is open before the swap");
        a.status = Some(Status {
            text: "Saved".into(),
            until: craft::platform::clock::monotonic_ms() + 60_000.0,
            tone: craft::theme::variant::Tone::Neutral,
        });
        a.canvas.ime_preedit = "ni".into();
        a.canvas.ime_preedit_range = Some(0..2);
        a.canvas.scroll_to_caret = false;
        a.canvas.nav_to_caret = true;

        let mut w = WindowUi::new(doc);
        a.swap_window(&mut w);
        assert!(a.dialog_host.open_dialogs().is_empty() && a.status.is_none());
        assert!(a.canvas.ime_preedit.is_empty() && a.canvas.ime_preedit_range.is_none());
        assert!(a.canvas.scroll_to_caret && !a.canvas.nav_to_caret, "a new window goes to the caret");
        assert_eq!(a.ui.tab, crate::DEFAULT_TAB);
        assert_eq!(w.tab, "Insert");
        assert_eq!((a.session.document_id(), a.session.rev(), a.ui.recent_count), (doc, rev, 7), "app-wide state stays");

        a.swap_window(&mut w);
        assert!(is_tabs(&a), "the dialog came back");
        assert_eq!(a.status.as_ref().map(|s| s.text.as_str()), Some("Saved"));
        assert_eq!((a.canvas.ime_preedit.as_str(), a.canvas.ime_preedit_range.clone()), ("ni", Some(0..2)));
        assert!(!a.canvas.scroll_to_caret && a.canvas.nav_to_caret);
        assert_eq!(a.ui.tab, "Insert");
        assert!(w.dialog_host.open_dialogs().is_empty() && w.tab == crate::DEFAULT_TAB);
        assert_eq!((a.session.document_id(), a.session.rev(), a.ui.recent_count), (doc, rev, 7));
        assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Kept");
    }
}
