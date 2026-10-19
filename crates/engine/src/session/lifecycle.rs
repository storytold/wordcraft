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

    /// Whether an optional module is turned on ([`Prefs::modules`], `law` for LawCraft): a
    /// module is off until the user switches it on, and a host installs its commands only then.
    pub fn module_enabled(&self, id: &str) -> bool {
        self.prefs.module_enabled(id)
    }

    /// Install commands an optional module supplies (`law.profiles.apply` from LawCraft, say).
    /// The host calls this once at start-up, when the module is turned on, and every way in
    /// reaches them from then on: the ribbon, the command palette, shortcuts, the CLI, the JSON
    /// control channel and the MCP server all dispatch by id. A command whose id the core
    /// already has is skipped, so a module can never shadow core behaviour. Returns how many
    /// commands were added.
    ///
    /// Nothing here knows what any module does: the engine holds the specs and the switch, the
    /// module brings the behaviour.
    pub fn install_commands(&mut self, specs: Vec<CommandSpec>) -> usize {
        let mut all = self.registry.all().to_vec();
        let mut added = 0;
        for spec in specs {
            if self.registry.get(spec.id).is_some() {
                log::warn!("command `{}` is already registered; the module's is ignored", spec.id);
                continue;
            }
            all.push(spec);
            added += 1;
        }
        self.registry = Arc::new(crate::Registry::new(all));
        added
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
            lint_findings,
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
            lint_findings,
            export_check_commands: Vec::new(),
            module_en: wordcraft_proof::ModuleEn::default(),
        }
    }

    /// Document-level lint findings for Review › Proofing (`review.issues`): every source's, in
    /// source order, at most [`crate::lint::MAX_FINDINGS`].
    pub fn lint_findings(&self) -> Vec<serde_json::Value> {
        self.lint_findings.values().flatten().take(crate::lint::MAX_FINDINGS).cloned().collect()
    }

    /// The findings one source stored last.
    pub fn lint_findings_of(&self, source: &str) -> &[serde_json::Value] {
        self.lint_findings.get(source).map_or(&[], Vec::as_slice)
    }

    /// Replace one source's document-level lint findings (a module's check command calls this
    /// with its own `source`, so separate checks don't overwrite each other).
    pub fn set_lint_findings(&mut self, source: &str, findings: Vec<crate::lint::Finding>) {
        let json: Vec<serde_json::Value> =
            findings.into_iter().take(crate::lint::MAX_FINDINGS).map(|f| crate::lint::finding_json(&f, crate::cmd::pos_json)).collect();
        if json.is_empty() {
            self.lint_findings.remove(source);
        } else {
            self.lint_findings.insert(source.to_string(), json);
        }
    }

    /// Forget every source whose name starts with `prefix` (a module turned off).
    pub fn clear_lint_findings(&mut self, prefix: &str) {
        self.lint_findings.retain(|k, _| !k.starts_with(prefix));
    }

    /// Keep only the export checks whose command id passes `keep` (a module turned off drops its own).
    pub fn retain_export_check_commands(&mut self, keep: impl Fn(&str) -> bool) {
        self.export_check_commands.retain(|(id, _)| keep(id));
    }

    /// Remove module commands by id (a module turned off). Core commands are never removed: a
    /// module can't have installed one ([`Session::install_commands`] skips them). Returns how
    /// many were removed.
    pub fn uninstall_commands(&mut self, ids: &[&str]) -> usize {
        let core = crate::cmd::registry();
        let all: Vec<CommandSpec> = self.registry.all().iter().filter(|c| !ids.contains(&c.id) || core.get(c.id).is_some()).cloned().collect();
        let removed = self.registry.all().len() - all.len();
        if removed > 0 {
            self.registry = Arc::new(crate::Registry::new(all));
        }
        removed
    }

    /// Commands run before a save or export; each command's result may include a `findings` array.
    pub fn set_export_check_commands(&mut self, commands: Vec<(String, serde_json::Value)>) {
        self.export_check_commands = commands;
    }

    /// Run [`export_check_commands`] with the export's own options and target `format` (a lower-case
    /// extension such as `pdf`, `docx`, `png`) and return their findings. Each check stores its own
    /// findings; a check that fails outright is stored as a finding of its own source.
    /// Each command's parameters carry the actual `markup` option and the output `format`, so a
    /// check that re-exports the document (such as `textSearchablePdf`) verifies what is being
    /// written and only the checks that apply to that format run.
    pub fn run_export_checks(&mut self, opts: &crate::io::ExportOptions, format: &str) -> Vec<serde_json::Value> {
        let cmds = self.export_check_commands.clone();
        let mut all = Vec::new();
        for (id, params) in cmds {
            let mut params = params;
            if let Some(obj) = params.as_object_mut() {
                obj.insert("markup".into(), serde_json::json!(opts.markup));
                obj.insert("format".into(), serde_json::json!(format));
            }
            // A check that may change the document (a word count kept current) and finds nothing
            // to change leaves no undo step and doesn't mark the document changed.
            let mutates = self.registry.get(&id).is_some_and(|c| c.mutates);
            let kept = mutates.then(|| {
                (
                    self.doc.clone(),
                    self.history.clone(),
                    self.redo.clone(),
                    self.dirty,
                    self.rev,
                    self.typing_open,
                    self.undo_evicted,
                    self.edit_marks.clone(),
                )
            });
            // An export runs these on its own: they aren't steps of a macro being recorded, and
            // Repeat still repeats what the person last did.
            let recording = self.recording.as_ref().map(|(_, steps)| steps.len());
            let repeat = self.last_command.clone();
            let result = self.run(&id, &params);
            if let (Some(n), Some((_, steps))) = (recording, self.recording.as_mut()) {
                steps.truncate(n);
            }
            self.last_command = repeat;
            match result {
                Ok(v) => {
                    if let Some((doc, history, redo, dirty, rev, typing_open, undo_evicted, edit_marks)) = kept
                        && self.doc == doc
                    {
                        self.history = history;
                        self.redo = redo;
                        self.dirty = dirty;
                        self.rev = rev;
                        self.typing_open = typing_open;
                        self.undo_evicted = undo_evicted;
                        self.edit_marks = edit_marks;
                    }
                    // A check that failed on an earlier export and runs now drops that failure.
                    self.set_lint_findings(&format!("export:{id}"), Vec::new());
                    if let Some(arr) = v.get("findings").and_then(serde_json::Value::as_array) {
                        for f in arr {
                            all.push(f.clone());
                            if all.len() >= crate::lint::MAX_FINDINGS {
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    // The same shape as any other lint finding, so Review › Proofing and
                    // `review.issues` can show a failed check beside the rest.
                    let finding = crate::lint::Finding::error(id.clone(), e.to_string());
                    all.push(crate::lint::finding_json(&finding, crate::cmd::pos_json));
                    self.set_lint_findings(&format!("export:{id}"), vec![finding]);
                    if all.len() >= crate::lint::MAX_FINDINGS {
                        break;
                    }
                }
            }
            if all.len() >= crate::lint::MAX_FINDINGS {
                break;
            }
        }
        all
    }
}
