use super::doc_state::MAX_UI_REQUESTS;
use super::prefs::Prefs;
use super::view::ViewState;
use super::{Arc, DocState, Document, ImeEditOutcome, Registry, Session, Value};

impl Session {
    /// Queue a request for the front end. A headless session never drains the queue, so only
    /// the newest [`MAX_UI_REQUESTS`] are kept.
    pub fn push_ui_request(&mut self, req: Value) {
        if self.ui_requests.len() >= MAX_UI_REQUESTS {
            self.ui_requests.remove(0);
        }
        self.ui_requests.push(req);
    }

    /// Save the custom dictionary through [`Session::user_dict_path`], if the host wired one up,
    /// with `removed` (words just taken out) left out of the merge with the file. A failed save
    /// keeps the words in memory and never fails the command (#40), as does a file outside the
    /// automation roots in force: it is read, then replaced beside itself (#3737).
    pub(crate) fn persist_user_dictionary(&self, removed: &[String]) {
        let Some(path) = self.user_dict_path.as_deref() else { return };
        let name = path.to_string_lossy();
        if let Err(e) = crate::file_access::check_read(&name)
            .and_then(|()| crate::file_access::check_write(&name))
            .and_then(|()| wordcraft_proof::save_user_dictionary(path, removed).map_err(|e| e.to_string()))
        {
            log::warn!("custom dictionary: save failed: {e}");
        }
    }

    /// Use `dir` as the themes folder: read the custom colour and font sets and themes kept there,
    /// and save new ones to it (#1869). See [`crate::cmd::themes`].
    pub fn load_custom_themes(&mut self, dir: &std::path::Path) {
        self.custom_themes = crate::cmd::themes::CustomThemes::load(dir);
        self.themes_dir = Some(dir.to_path_buf());
    }

    /// Start an editing session at the document beginning with empty history and
    /// default view, preferences and a yellow highlighter independent of document undo.
    pub fn new(doc: Document) -> Self {
        Self::with_registry(doc, Arc::new(crate::cmd::registry()))
    }

    /// Initialise session defaults while sharing the immutable command registry with previews.
    pub(super) fn with_registry(doc: Document, registry: Arc<Registry>) -> Self {
        let DocState {
            doc,
            sel,
            view,
            pending,
            path,
            file_name,
            dirty,
            pending_move,
            find,
            goal_x,
            page_hint,
            new_header_parts,
            hf_return,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            join_anchor,
            autocap,
            rev,
            document_id,
            untitled,
            made_rev,
            ime_body_epoch,
            cache,
            layout,
            typing,
            originals,
            picture_looks,
            versions,
            autosave,
            bib_style,
            merge,
            math,
            password,
            #[cfg(not(target_arch = "wasm32"))]
            recovery_slot,
            also_selected,
            column,
            column_mode,
            widened,
            edit_marks,
            go_back_step,
        } = DocState::new(doc, 1, ViewState::default());
        Session {
            doc,
            sel,
            view,
            pending,
            last_highlight: wordcraft_doc::props::Highlight::Yellow,
            path,
            file_name,
            user_dict_path: None,
            themes_dir: None,
            custom_themes: Default::default(),
            dirty,
            clipboard: None,
            pending_move,
            clipboard_text: String::new(),
            clipboard_html: String::new(),
            clip_history: Default::default(),
            find,
            goal_x,
            page_hint,
            registry,
            author: "WordCraft User".into(),
            initials: String::new(),
            painter: None,
            status: String::new(),
            new_header_parts,
            hf_return,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            equation_join_next,
            ime_text_join,
            join_anchor,
            autocap,
            rev,
            document_id,
            untitled,
            made_rev,
            protect_facts: std::sync::Mutex::new(None),
            ime_body_epoch,
            ime_state_revision: None,
            ime_state_seen: None,
            ime_edit_outcome: ImeEditOutcome::default(),
            #[cfg(any(test, feature = "test-support"))]
            ime_state_exhausted: false,
            pending_ime_application_change: false,
            ime_origin: false,
            cache,
            layout,
            typing,
            originals,
            picture_looks,
            last_command: None,
            recording: None,
            macros: Default::default(),
            versions,
            building_blocks: Default::default(),
            autosave,
            bib_style,
            merge,
            ui_requests: Vec::new(),
            math,
            math_latex: false,
            math_normal_text: false,
            also_selected,
            prefs: Prefs::default(),
            system_region: Default::default(),
            keymap: crate::KeyMap::default(),
            ribbon: crate::RibbonLayout::default(),
            editing_lang: None,
            read_aloud: Default::default(),
            password,
            #[cfg(not(target_arch = "wasm32"))]
            recovery: None,
            #[cfg(not(target_arch = "wasm32"))]
            recovery_slot,
            column,
            column_mode,
            widened,
            edit_marks,
            go_back_step,
            others: Vec::new(),
            order: vec![document_id],
            untitled_count: 1,
        }
    }
}
