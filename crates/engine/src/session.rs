//! The editing session: document, selection, undo/redo, layout, view state.

use std::sync::Arc;

use serde_json::Value;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, PartKind, Path, Pos, StoryRef};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, ViewMode};

use crate::{CmdError, ImeEditOutcome, Registry};

mod documents;
mod views;
pub use documents::{DocInfo, DocState};
use documents::{document_id_mark, next_document_id, untitled_name};
pub use views::WindowView;
mod history;
mod ime;
mod layout;
mod lifecycle;
mod query;
mod run;
mod snapshot;

use crate::doc_state;
use crate::ime as ime_state;
use crate::locale;
use crate::prefs;
use crate::snapshot as snapshot_types;
use crate::view;
pub(crate) use doc_state::Widened;
pub use doc_state::{CellRange, ColumnBlock, FindState, MathEdit, Selection, UNDO_LIST_MAX, next_math_session};
use doc_state::{TypedWord, Undo, doc_bib_style};
use ime_state::ImeCaretState;
pub use locale::set_system_locale;
pub(crate) use locale::system_locale;
pub use prefs::Prefs;
pub use query::blank_document;
pub use snapshot_types::EditSnapshot;
use snapshot_types::{LayoutSlots, RunSnapshot};
pub use view::{StylesPaneDock, ViewState};

/// An editing session: the app-wide state (clipboard, preferences, macros, the command registry…)
/// and the open documents. The active document's state is in the session's own fields, which
/// every command works on; the other open documents wait in [`DocState`] values and are swapped
/// in when they become active (see [`Session::documents`]).
pub struct Session {
    pub doc: Document,
    pub sel: Selection,
    pub view: ViewState,
    /// Formatting picked with a collapsed caret, applied to the next typed text.
    pub pending: Option<CharProps>,
    /// Last highlighter choice for the shortcut; palette state, outside document undo.
    pub(crate) last_highlight: wordcraft_doc::props::Highlight,
    pub path: Option<std::path::PathBuf>,
    /// Name given to a document not saved yet (File › Info › Rename): what Save suggests. A file
    /// name, not document metadata, so it is not part of the document or its undo history.
    pub file_name: Option<String>,
    pub dirty: bool,
    /// Where the custom dictionary is kept on disk, if the host wired one up. Only the desktop
    /// app does: headless sessions (MCP, `wordcraft-cli run`), tests and the web build leave
    /// `None`, so an agent's words stay in memory and never change the person's dictionary.
    /// Add to Dictionary and File › Options › Proofing save through it (#40).
    pub user_dict_path: Option<std::path::PathBuf>,
    pub clipboard: Option<Fragment>,
    /// A tracked cut the next paste can turn into a move (see `cmd::moves`).
    pub pending_move: Option<crate::cmd::moves::PendingMove>,
    /// Plain text mirror of the clipboard (for the system clipboard).
    pub clipboard_text: String,
    /// HTML for the system clipboard beside [`Self::clipboard_text`]: the copy's formatting for
    /// programs that read HTML, equations as MathML (#816); empty when nothing was copied.
    pub clipboard_html: String,
    /// Items collected by Copy and Cut this session, for the Clipboard pane.
    pub clip_history: crate::cmd::edit::ClipHistory,
    pub find: FindState,
    /// Page x the caret tries to keep on Up/Down.
    pub goal_x: Option<f32>,
    /// Page the caret was last on (headers/footers repeat on many pages).
    pub page_hint: usize,
    pub registry: Arc<Registry>,
    /// Author name for comments and tracked changes.
    pub author: String,
    /// Initials for comments (File › Options › General); empty = the name's initials.
    pub initials: String,
    /// Format painter: copied formatting waiting to be applied (and whether it stays on).
    pub painter: Option<(CharProps, wordcraft_doc::props::ParaProps, bool)>,
    /// Last message for the status bar / agents.
    pub status: String,
    /// Headers/footers `insert.createHeader`/`insert.createFooter` made for a page that had none
    /// (#960): `insert.closeHeader` takes those still empty away again, as Word does, and saving
    /// leaves them out. Kept after that, so one an undo brings back goes again on the next close.
    pub new_header_parts: Vec<u32>,
    /// The body selection a command left for a header or footer: `insert.closeHeader` puts it back.
    pub hf_return: Option<Selection>,
    /// Undo and redo steps. Shared so `run` can snapshot both stacks cheaply (a pointer per
    /// step) and put them back exactly when a command fails.
    history: Vec<Arc<Undo>>,
    /// Undo steps dropped so far because of the undo limit.
    undo_evicted: u64,
    redo: Vec<Arc<Undo>>,
    /// Typing is coalesced into one undo step until something else happens.
    typing_open: bool,
    /// The next mutating command joins the previous undo step (later frames of a drag).
    join_next: bool,
    /// The next equation typing command joins a successful equation `deleteSurrounding`'s undo
    /// step. Unlike [`Self::join_next`], any other command consumes and discards it, so an
    /// unrelated Bold or document replacement can never be merged into the deletion.
    equation_join_next: bool,
    /// The next `text.insert` at this exact selection joins a successful body
    /// `DeleteSurrounding`'s undo step. Any other command consumes and discards it, so an
    /// intervening edit owns its own checkpoint instead of being merged into the deletion.
    ime_text_join: Option<Selection>,
    /// Whether the undo group has a step of its own for the next joined command to merge
    /// into. A command that changed nothing leaves the group without one, so the next
    /// command in the group starts the step the group needs.
    join_anchor: bool,
    /// The newest automatic capitalisation, for Word's "Automatically add words to list" (#1857).
    pub(crate) autocap: Option<crate::cmd::tools::AutoCap>,
    rev: u64,
    /// Which document this is, unique in the process: changes when [`Session::set_document`]
    /// replaces it (an open into an untouched blank document, recover), never on an edit or when
    /// another document becomes active and this one comes back.
    document_id: u64,
    /// The number in an untitled document's name (`Document2`); 0 for one opened from a file.
    untitled: u32,
    /// `rev` when the document was made, for [`Session::is_untouched_blank`].
    made_rev: u64,
    /// What Restrict Editing's outright refusals depend on, for greying out commands
    /// ([`Session::why_disabled`]): computed once per document, revision and author instead of
    /// for every button on every frame.
    protect_facts: std::sync::Mutex<Option<((u64, u64, String), crate::cmd::protect::Facts)>>,
    /// How many times the document's body was replaced under the same [`Self::document_id`]
    /// (`document.setText`, version restore, compare, Undo/Redo). It is the body's IME
    /// replacement epoch: a composition must not survive the swap, even when the new body has
    /// the same text or the same bounded surrounding window.
    ime_body_epoch: u64,
    /// Engine-owned, checked, process-monotonic revision for IME receipts whose target cannot
    /// provide surrounding text. `None` means the allocator is exhausted (fail closed).
    ime_state_revision: Option<u64>,
    /// The caret/equation-session token the revision was last allocated for. It covers selection
    /// and caret changes without walking the document.
    ime_state_seen: Option<ImeCaretState>,
    /// The current command's explicit IME effect classification.
    ime_edit_outcome: ImeEditOutcome,
    /// Simulation of a globally exhausted IME revision allocator for front-end producer tests.
    /// Only [`Self::exhaust_ime_state_revision`] sets it; production builds do not compile it,
    /// and it only makes receipt classification fail closed.
    #[cfg(any(test, feature = "test-support"))]
    ime_state_exhausted: bool,
    /// A successful non-IME command changed document state since the last IME observation. It is
    /// drained before publishing a receipt and at observation time; it is never cleared by a later
    /// command. It covers mixed formatting/IME frames when available stamps ignore the revision.
    pending_ime_application_change: bool,
    /// Whether the current command was entered through [`Session::run_ime`]. Nested commands
    /// inherit the origin.
    ime_origin: bool,
    cache: LayoutCache,
    layout: LayoutSlots,
    /// The word being typed, left without a spelling squiggle ([`TypedWord`]).
    typing: Option<TypedWord>,
    /// Picture edits: edited media key → original media key (Reset Picture).
    pub originals: std::collections::HashMap<String, String>,
    /// Picture effects, styles and borders: drawn media key → what was drawn on which bitmap,
    /// so the next one replaces them instead of drawing over them (#2052).
    pub picture_looks: std::collections::HashMap<String, crate::cmd::objects::PictureLook>,
    /// Last mutating command (Repeat).
    pub last_command: Option<(String, Value)>,
    /// Macro being recorded: (name, steps).
    pub recording: Option<(String, Vec<(String, Value)>)>,
    pub macros: std::collections::BTreeMap<String, Vec<(String, Value)>>,
    /// Saved versions: (label, date, document).
    pub versions: Vec<(String, String, Document)>,
    /// Quick Parts / AutoText entries.
    pub building_blocks: std::collections::BTreeMap<String, Fragment>,
    pub autosave: bool,
    /// Citation style: APA, MLA, Chicago, IEEE.
    pub bib_style: String,
    /// Mail merge data source and preview.
    pub merge: crate::cmd::mailings::MergeState,
    /// Requests from commands to the UI (open a dialog, scroll…), drained by the front end.
    pub ui_requests: Vec<Value>,
    /// Read Aloud player (Review › Speech).
    pub read_aloud: crate::speech::ReadAloud,
    /// Preferences the front end saves between runs.
    pub prefs: Prefs,
    /// The system region (#838): the paper a new blank document starts on, the default
    /// measurement unit and the decimal separator. The front end sets it from the platform (the
    /// browser's locale on the web); headless sessions keep `en-US`. A region the user chose
    /// ([`Prefs::region`]) wins: read it with [`Session::region`].
    pub system_region: wordcraft_geom::Region,
    /// Custom keyboard shortcuts (`tools.customizeKeyboard`); the front end saves them too.
    pub keymap: crate::KeyMap,
    /// Custom ribbon and Quick Access Toolbar (`tools.customizeRibbon`); the front end saves it too.
    pub ribbon: crate::RibbonLayout,
    /// The language new blank documents (and opened plain text) are written in, a BCP 47 tag;
    /// `None` is WordCraft's default, US English. The front end sets it from the interface
    /// language with [`Session::set_editing_language`] (a Polish interface writes Polish).
    pub editing_lang: Option<String>,
    /// Editing inside an equation: which one and the caret in it.
    pub math: Option<MathEdit>,
    /// Equations are typed in LaTeX rather than the linear format.
    pub math_latex: bool,
    /// Text typed into equations is normal (non-math) text.
    pub math_normal_text: bool,
    /// The password Word documents are saved with (File › Info › Protect Document › Encrypt with
    /// Password, `file.encrypt`); `None` saves them unencrypted. Kept from a password-protected
    /// file that was opened, cleared when the document is replaced.
    pub password: Option<crate::io::Password>,
    /// The recovery worker every open document's copy is kept through (desktop hosts only).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) recovery: Option<crate::recovery::Recovery>,
    /// The active document's recovery copy.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) recovery_slot: crate::recovery::Slot,
    /// More pictures and shapes selected along with the one the selection holds (Shift+click),
    /// for Group. Cleared by any edit or selection change other than adding to it.
    pub also_selected: Vec<Pos>,
    /// Column (block) selection, if one was made (see [`Session::column_segments`]).
    pub column: Option<ColumnBlock>,
    /// Column selection mode (Ctrl+Shift+F8): caret movement extends the block.
    pub column_mode: bool,
    /// A selection widened to whole table cells or rows, and where it really runs (see
    /// [`Session::sel_ends`]).
    pub(crate) widened: Option<Widened>,
    /// Where the last edits happened, oldest first (at most [`EDIT_MARKS`]), for Go Back
    /// (Shift+F5). Positions aren't mapped through later edits; they're clamped when used.
    edit_marks: Vec<Pos>,
    /// How many Go Back presses since the last edit (which place comes next).
    go_back_step: usize,
    /// The open documents other than the active one, in no particular order.
    others: Vec<DocState>,
    /// Every open document's id, the active one's too, in the order they were opened.
    order: Vec<u64>,
    /// Untitled documents made so far, for the next one's name (`Document3`).
    untitled_count: u32,
}

impl Session {
    /// File name of the persisted custom dictionary inside the settings folder.
    pub const USER_DICT_FILE: &str = "custom-dictionary.txt";

    /// Save the custom dictionary through [`Session::user_dict_path`], if the host wired one up,
    /// with `removed` (words just taken out) left out of the merge with the file. A failed save
    /// keeps the words in memory and never fails the command (#40).
    pub(crate) fn persist_user_dictionary(&self, removed: &[String]) {
        if let Some(path) = self.user_dict_path.as_deref()
            && let Err(e) = wordcraft_proof::save_user_dictionary(path, removed)
        {
            log::warn!("custom dictionary: save failed: {e}");
        }
    }

    pub fn new(doc: Document) -> Self {
        let start = doc.start_of(StoryRef::Body);
        let bib_style = doc_bib_style(&doc);
        Session {
            doc,
            sel: Selection::caret(start),
            view: ViewState::default(),
            pending: None,
            path: None,
            file_name: None,
            user_dict_path: None,
            dirty: false,
            clipboard: None,
            pending_move: None,
            clipboard_text: String::new(),
            clipboard_html: String::new(),
            clip_history: Default::default(),
            find: FindState::default(),
            goal_x: None,
            page_hint: 0,
            registry: Arc::new(crate::cmd::registry()),
            author: "WordCraft User".into(),
            initials: String::new(),
            painter: None,
            status: String::new(),
            new_header_parts: Vec::new(),
            hf_return: None,
            history: Vec::new(),
            undo_evicted: 0,
            redo: Vec::new(),
            typing_open: false,
            join_next: false,
            autocap: None,
            rev: 1,
            document_id: 1,
            cache: LayoutCache::new(),
            layout: None,
            typing: None,
            originals: Default::default(),
            picture_looks: Default::default(),
            last_command: None,
            recording: None,
            macros: Default::default(),
            versions: Vec::new(),
            building_blocks: Default::default(),
            autosave: true,
            bib_style,
            merge: Default::default(),
            ui_requests: Vec::new(),
            math: None,
            math_latex: false,
            math_normal_text: false,
            also_selected: Vec::new(),
            prefs: Prefs::default(),
            system_region: Default::default(),
            keymap: crate::KeyMap::default(),
            ribbon: crate::RibbonLayout::default(),
            editing_lang: None,
            read_aloud: Default::default(),
            password: None,
            #[cfg(not(target_arch = "wasm32"))]
            recovery: None,
            column: None,
            column_mode: false,
            widened: None,
            edit_marks: Vec::new(),
            go_back_step: 0,
        }
    }

    /// Remember the caret as the place of an edit, for Go Back. Edits in the same paragraph as
    /// the last one (typing on) update that place rather than adding one.
    fn note_edit(&mut self) {
        let at = self.sel.focus.clone();
        match self.edit_marks.last_mut() {
            Some(last) if last.story == at.story && last.path == at.path => *last = at,
            _ => {
                self.edit_marks.push(at);
                if self.edit_marks.len() > EDIT_MARKS {
                    self.edit_marks.remove(0);
                }
            }
        }
        self.go_back_step = 0;
    }

    /// Go Back (Shift+F5): the next of the last edit places, most recent first, cycling; a place
    /// the caret is already at is skipped. `None` before any edit.
    pub fn go_back(&mut self) -> Option<Pos> {
        let n = self.edit_marks.len();
        for _ in 0..n {
            let i = n - 1 - self.go_back_step % n;
            self.go_back_step = self.go_back_step.wrapping_add(1);
            let Some(p) = self.edit_marks.get(i) else { continue };
            let p = self.doc.clamp(p);
            if p != self.sel.focus || n == 1 {
                return Some(p);
            }
        }
        self.edit_marks.last().map(|p| self.doc.clamp(p))
    }

    /// The line pieces of the current column selection, if there is one (the selection hasn't
    /// moved since the block was made).
    pub fn column_segments(&self) -> Option<&[(Pos, Pos)]> {
        // Cells that are one run of the text are an ordinary stream selection.
        self.column.as_ref().filter(|c| c.sel == self.sel && !c.cells.as_ref().is_some_and(|r| r.stream)).map(|c| c.segments.as_slice())
    }

    /// The whole table cells selected, if the selection is that (see [`CellRange`]).
    pub fn cell_range(&self) -> Option<&CellRange> {
        self.column.as_ref().filter(|c| c.sel == self.sel).and_then(|c| c.cells.as_ref())
    }

    /// The selection as it was made: where a drag, Shift+click or Shift+arrows started and where
    /// the caret went, even when Word's table rule widened it to whole cells or rows (`sel` holds
    /// those).
    pub fn sel_ends(&self) -> &Selection {
        self.widened.as_ref().filter(|w| w.sel == self.sel).map_or(&self.sel, |w| &w.ends)
    }

    pub fn selects_whole_story(&self) -> bool {
        let st = self.sel.focus.story;
        let (a, b) = self.sel.ordered();
        a == self.doc.start_of(st) && b == self.doc.end_of(st)
    }

    /// Whether the caret is in a header or footer.
    pub fn in_header_footer(&self) -> bool {
        let StoryRef::Part(id) = self.sel.focus.story else { return false };
        self.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer))
    }

    /// Document revision (bumped by every change).
    pub fn rev(&self) -> u64 {
        self.rev
    }
    /// Which document is open (see `document_id`): lets a caller tell an edit from a replacement.
    pub fn document_id(&self) -> u64 {
        self.document_id
    }
    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
        self.dirty = true;
    }

    /// Reading Highlight ranges for the current document (searched again after edits); empty when it's off.
    pub fn find_highlights(&mut self) -> &[(Pos, Pos)] {
        if !self.find.highlight {
            return &[];
        }
        if self.find.highlights.as_ref().is_none_or(|(r, _)| *r != self.rev) {
            let found = crate::cmd::edit::search(self, StoryRef::Body).unwrap_or_default();
            self.find.highlights = Some((self.rev, found));
        }
        self.find.highlights.as_ref().map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    /// Whether the markup area beside the page can show balloons: markup shown in Print Layout
    /// (not Read Mode or pages side by side), comments not listed in the Comments pane instead,
    /// and Track Changes Options not set to show every revision inline.
    pub fn balloons_shown(&self) -> bool {
        let v = &self.view;
        v.show_markup
            && !v.comments_pane
            && !v.read_mode
            && !v.multi_page
            && v.mode == ViewMode::Print
            && self.prefs.markup.balloons != wordcraft_layout::display::BalloonMode::Inline
    }

    /// Show revisions in balloons, with the balloons on screen: tracked deletions in the body leave
    /// the text for balloons in the markup area. Where no balloons can be shown they stay inline,
    /// as in Word; so do deletions in headers, footers, notes and text boxes, which get no balloons.
    pub fn revisions_in_balloons(&self) -> bool {
        let m = &self.prefs.markup;
        self.balloons_shown() && m.balloons == wordcraft_layout::display::BalloonMode::Revisions && m.insertions_deletions
    }

    /// Initials for new comments: the ones set in File › Options, else the user name's.
    pub fn user_initials(&self) -> String {
        let set = self.initials.trim();
        if set.is_empty() { self.author.split_whitespace().filter_map(|w| w.chars().next()).collect() } else { set.to_string() }
    }

    /// The document's file name and full path for fields (`FILENAME`) and layout context.
    /// Returns `(filename, filepath)`. Unsaved documents with no explicit file name default
    /// consistently to `("Document1", "Document1")`. Document metadata title is never used
    /// as a filename fallback (#3449).
    pub fn resolved_filenames(&self) -> (String, String) {
        if let Some(path) = &self.path {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
            let full = path.to_string_lossy().to_string();
            (name, full)
        } else if let Some(name) = &self.file_name {
            (name.clone(), name.clone())
        } else {
            ("Document1".to_string(), "Document1".to_string())
        }
    }

    /// The current layout (recomputed when the document or view changed).
    pub fn layout(&mut self) -> Arc<DocLayout> {
        let mode = if self.view.read_mode { ViewMode::Read } else { self.view.mode };
        let width = match mode {
            ViewMode::Read => self.view.read_width,
            ViewMode::Web => self.view.web_width,
            _ => DRAFT_WIDTH,
        };
        let height = if self.view.read_mode { self.view.read_height } else { 0.0 };
        let spacing = if self.view.read_mode { self.view.read_line_spacing.map(f32::to_bits) } else { None };
        // Deletions in balloons depend on the view (the Comments pane, Read Mode, side by side), so
        // a view change that moves them in or out of the text lays it out again.
        let hide_deleted = !self.view.show_markup;
        let deleted = (hide_deleted, self.prefs.markup.hidden_deletions(), self.revisions_in_balloons());
        let outline = (mode == ViewMode::Outline).then(|| self.view.outline.view());
        let typing = self.typing_word();
        if let Some((r, w, h, m, sp, l, pf, o, hd, n, t)) = &self.layout
            && *r == self.rev
            && (*w == width || mode == ViewMode::Print)
            && *h == height
            && *m == mode
            && *sp == spacing
            && *pf == self.view.proofing
            && *n == self.view.numeral
            && *hd == deleted
            && *o == outline
            && *t == typing
        {
            return l.clone();
        }
        let (fn_name, fn_path) = self.resolved_filenames();
        let (filename, filepath): (Option<Arc<str>>, Option<Arc<str>>) = (Some(Arc::from(fn_name.as_str())), Some(Arc::from(fn_path.as_str())));
        let opts = LayoutOptions {
            view: mode,
            web_width: width,
            read_height: height,
            read_line_spacing: spacing.map(f32::from_bits),
            show_hidden: self.view.marks || self.prefs.marks.hidden,
            hide_deleted,
            hide_marked: deleted.1,
            deletions_in_balloons: deleted.2,
            proofing: self.view.proofing,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),
            comment_balloons: false,
            field_codes: self.view.field_codes,
            outline: outline.clone(),
            proof: wordcraft_layout::ProofOptions { grammar: self.prefs.mark_grammar, spelling: self.prefs.spelling() },
            typing_word: typing,
            filename: filename.clone(),
            filepath: filepath.clone(),
        };
        let l = Arc::new(wordcraft_layout::layout(&self.doc, &mut self.cache, &opts));
        self.layout = Some((self.rev, width, height, mode, spacing, l.clone(), self.view.proofing, outline, deleted, self.view.numeral, typing));
        l
    }

    /// The recorded word while the caret and the word are as typing left them; else drops it.
    fn typing_word(&mut self) -> Option<wordcraft_layout::TypingWord> {
        let t = self.typing.as_ref()?;
        let p = if self.sel.is_collapsed() && self.sel.focus == t.at { self.doc.para_at(&t.at) } else { None };
        match p {
            Some(p) if typed_range(&p.text, t.at.off) == Some((t.start, t.end)) => {
                Some(wordcraft_layout::TypingWord { rev: p.rev, start: t.start, end: t.end })
            }
            _ => {
                self.typing = None;
                None
            }
        }
    }

    /// After a successful command: a `text.*` edit at the caret records the word being typed;
    /// any other caret move ends it.
    fn note_typing(&mut self, id: &str, before: Option<&Document>, sel_before: &Selection) {
        let at = &self.sel.focus;
        let edited = || before.is_some_and(|d| d.para_at(at).map(|p| &p.text) != self.doc.para_at(at).map(|p| &p.text));
        if id.starts_with("text.") && edited() {
            self.typing = self.word_being_typed();
        } else if self.sel != *sel_before {
            self.typing = None;
        }
    }

    /// The word the collapsed caret ends or sits in, as [`TypedWord`].
    fn word_being_typed(&self) -> Option<TypedWord> {
        if !self.sel.is_collapsed() {
            return None;
        }
        let at = self.sel.focus.clone();
        let (start, end) = typed_range(&self.doc.para_at(&at)?.text, at.off)?;
        Some(TypedWord { at, start, end })
    }
    /// A layout for output (PDF, images, print): no proofing marks, print view.
    pub fn export_layout(&self) -> Arc<DocLayout> {
        self.export_layout_with(false)
    }

    /// [`Session::export_layout`]; with `markup` (Print Markup) each page also has the markup
    /// area with the comments in balloons.
    pub fn export_layout_with(&self, markup: bool) -> Arc<DocLayout> {
        let (fn_name, fn_path) = self.resolved_filenames();
        let opts = LayoutOptions {
            view: ViewMode::Print,
            comment_balloons: markup && self.prefs.markup.comments,
            filename: Some(Arc::from(fn_name.as_str())),
            filepath: Some(Arc::from(fn_path.as_str())),
            ..Default::default()
        };
        Arc::new(wordcraft_layout::layout(&self.doc, &mut LayoutCache::new(), &opts))
    }

    /// Invalidate the cached layout (fonts changed etc.).
    pub fn relayout(&mut self) {
        self.layout = None;
        self.cache.clear();
    }

    /// Set the language new documents are written in ([`Session::editing_lang`]; `None` or a
    /// malformed tag: US English). A document nobody has touched yet — untitled, unchanged, no
    /// undo steps, no text — switches too, so the blank document a session starts with follows
    /// the saved proofing default, or the interface language when none is saved. Nothing else
    /// changes and no undo step is added.
    pub fn set_editing_language(&mut self, tag: Option<&str>) {
        let tag = self
            .prefs
            .default_language
            .as_deref()
            .and_then(wordcraft_proof::lang::normalize_tag)
            .or_else(|| tag.and_then(wordcraft_proof::lang::normalize_tag));
        if self.editing_lang == tag {
            return;
        }
        self.editing_lang = tag;
        let untouched = self.path.is_none() && !self.dirty && self.history.is_empty() && self.redo.is_empty();
        if untouched && self.doc.plain_text(StoryRef::Body).trim().is_empty() && self.doc.paragraph_count() <= 1 {
            self.doc.styles.default_chr.lang = Some(self.editing_lang.clone().unwrap_or_else(|| "en-US".into()));
            self.rev = self.rev.wrapping_add(1);
            self.relayout();
        }
    }

    /// Snapshot for undo before a change.
    pub fn checkpoint(&mut self, label: &str) {
        if label == "Typing" && self.typing_open {
            return;
        }
        self.typing_open = label == "Typing";
        self.history.push(Arc::new(Undo {
            label: label.to_string(),
            doc: self.doc.clone(),
            sel: self.sel.clone(),
            pending: self.pending.clone(),
            column: self.column.clone(),
        }));
        if self.history.len() > MAX_UNDO {
            self.history.remove(0);
            self.undo_evicted += 1;
        }
        self.redo.clear();
    }
    /// Record `doc`/`sel` as their own undo step after the fact and close the typing group, so
    /// Undo right after an automatic change (AutoFormat, AutoCorrect) reverts only that change.
    pub fn push_undo(&mut self, label: &str, doc: Document, sel: Selection) {
        self.history.push(Arc::new(Undo { label: label.to_string(), doc, sel, pending: self.pending.clone(), column: None }));
        if self.history.len() > MAX_UNDO {
            self.history.remove(0);
            self.undo_evicted += 1;
        }
        self.typing_open = false;
    }
    /// Make the next mutating command part of the previous undo step instead of a new one, so a
    /// drag that runs a command every frame is a single Undo. Call it on every frame of the drag
    /// except the first; it is consumed by the next `run`.
    pub fn join_next_undo(&mut self) {
        self.join_next = true;
    }
    /// Stamp the document's save metadata (modified time, last modified by, revision; created
    /// when it has none) the way Word does on every save. `file.save` calls it, and so does any
    /// front end that writes the bytes itself (the web build's download, #262). Like Word, the
    /// document also keeps the zoom it is shown at (#995).
    pub fn stamp_save(&mut self) {
        self.doc.settings.zoom = crate::cmd::view::saved_zoom(&self.view);
        let core = &mut self.doc.core;
        core.modified = crate::cmd::now_iso();
        if core.created.is_empty() {
            core.created = core.modified.clone();
        }
        core.last_modified_by = self.author.clone();
        core.revision = core.revision.saturating_add(1);
    }
    /// Whether typing is still collecting into one undo step (no other command since).
    pub fn typing_open(&self) -> bool {
        self.typing_open
    }
    /// Reopen the typing group when the last undo step is typing, so the keys typed next join
    /// it: after an input method's rewrite was joined to it (`join_next_undo` closes the group).
    pub fn continue_typing(&mut self) {
        if self.undo_label() == Some("Typing") {
            self.typing_open = true;
        }
    }
    /// Close an open typing group (caret moved, other command).
    pub fn close_typing(&mut self) {
        self.typing_open = false;
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.history.last().map(|u| u.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|u| u.label.as_str())
    }
    /// Undo steps pushed so far, evicted ones included: only grows while steps are added.
    pub(crate) fn undo_steps_so_far(&self) -> u64 {
        self.history.len() as u64 + self.undo_evicted
    }
    /// The number of undo steps.
    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }
    /// Where the undo history stands, for [`Session::merge_undo_since`].
    pub fn undo_mark(&self) -> u64 {
        (self.history.len() as u64).saturating_add(self.undo_evicted)
    }
    /// Makes the steps recorded since `mark` one undo step, the first: a drag that applied its
    /// value as it went (a slider in a pane) undoes in one go.
    pub fn merge_undo_since(&mut self, mark: u64) {
        let keep = mark.saturating_sub(self.undo_evicted).saturating_add(1);
        self.history.truncate(usize::try_from(keep).unwrap_or(usize::MAX));
    }
    pub fn undo_labels(&self) -> Vec<String> {
        self.history.iter().rev().map(|u| u.label.clone()).collect()
    }
    /// The Undo button's list (#701): the undo steps' labels (English, as commands name them),
    /// newest first, at most [`UNDO_LIST_MAX`].
    pub fn undo_history(&self) -> Vec<&str> {
        self.history.iter().rev().take(UNDO_LIST_MAX).map(|u| u.label.as_str()).collect()
    }
    /// Undo the newest `count` steps in one go, as picking an entry in the Undo list does (#701).
    /// More than the undo depth undoes everything; a step that would take the document's
    /// protection away stops it (only Stop Protection lifts it). Returns the steps undone.
    pub fn undo_steps(&mut self, count: usize) -> usize {
        let mut done = 0;
        while done < count && !self.step_lifts_protection(false) && self.undo() {
            done += 1;
        }
        done
    }
    pub fn undo(&mut self) -> bool {
        self.typing_open = false;
        // Undoing the automatic capitalisation's own step makes its word an exception; any
        // other Undo forgets the capital.
        let steps = self.history.len() as u64 + self.undo_evicted;
        if let Some(c) = self.autocap.take().filter(|c| c.depth == steps) {
            c.add_exceptions(&mut self.prefs.autocorrect);
        }
        let Some(u) = self.history.pop().map(Arc::unwrap_or_clone) else { return false };
        let cur = Undo {
            label: u.label.clone(),
            doc: std::mem::replace(&mut self.doc, u.doc),
            sel: std::mem::replace(&mut self.sel, u.sel),
            pending: std::mem::replace(&mut self.pending, u.pending),
            column: std::mem::replace(&mut self.column, u.column),
        };
        self.redo.push(Arc::new(cur));
        self.touch();
        true
    }
    /// Would Undo (or Redo) take the document's protection away? Only Stop Protection lifts it.
    pub fn step_lifts_protection(&self, redo: bool) -> bool {
        let next = if redo { self.redo.last() } else { self.history.last() };
        next.is_some_and(|u| crate::cmd::protect::protection_lifted(&self.doc, &u.doc))
    }
    pub fn redo(&mut self) -> bool {
        self.typing_open = false;
        let Some(u) = self.redo.pop().map(Arc::unwrap_or_clone) else { return false };
        let cur = Undo {
            label: u.label.clone(),
            doc: std::mem::replace(&mut self.doc, u.doc),
            sel: std::mem::replace(&mut self.sel, u.sel),
            pending: std::mem::replace(&mut self.pending, u.pending),
            column: std::mem::replace(&mut self.column, u.column),
        };
        self.history.push(Arc::new(cur));
        self.touch();
        true
    }
    /// Drop history (after open / new).
    pub fn reset_history(&mut self) {
        self.autocap = None;
        self.history.clear();
        self.redo.clear();
        self.typing_open = false;
    }

    /// The version snapshots belong to the document they were taken from: keeping the old ones
    /// would let a restore put the previous document's content into this one (#1327 follow-up).
    pub fn clear_versions(&mut self) {
        self.versions.clear();
    }

    /// The region in effect (#838): the one the user chose ([`Prefs::region`]), else the
    /// system's.
    pub fn region(&self) -> wordcraft_geom::Region {
        match self.prefs.region.as_deref() {
            Some(tag) => wordcraft_geom::Region::from_tag(tag),
            None => self.system_region.clone(),
        }
    }

    /// The English of the region in effect (`en-GB` for a British region), or `None` when the
    /// region's language isn't English, it names no territory, or it is the United States (US
    /// English is the default anyway). An English interface writes new documents in it, as Word
    /// does on a British or Canadian system (#1045).
    pub fn region_english(&self) -> Option<String> {
        let r = self.region();
        (r.language == "en" && !r.territory.is_empty() && r.territory != "US")
            .then(|| format!("en-{}", r.territory))
            .and_then(|t| wordcraft_proof::lang::normalize_tag(&t))
    }

    /// The unit rulers and dialogs show lengths in: the user's choice, else the region's.
    pub fn units(&self) -> wordcraft_geom::Unit {
        self.prefs.units.unwrap_or_else(|| self.region().units())
    }

    /// `pt` in [`Session::units`], written the region's way (`2,54 cm` in Poland).
    pub fn format_length(&self, pt: f32) -> String {
        self.region().format(self.units(), pt)
    }

    /// A new blank document (the Normal template) on the region's paper: Letter in the US and
    /// other Letter regions, A4 elsewhere (#838). Margins stay 2.54 cm (1") all round.
    pub fn blank_document(&self) -> Document {
        blank_document(&self.region())
    }

    /// Replace the document (open/new).
    pub fn set_document(&mut self, doc: Document) {
        self.autocap = None;
        self.document_id = self.document_id.wrapping_add(1);
        self.bib_style = doc_bib_style(&doc);
        self.doc = doc;
        self.doc.ensure_nonempty();
        // Our own copy belongs to the document it was copied from: the new document holds
        // neither its text nor its pictures, so pasting the copy here would bring neither
        // (#2909). The system clipboard still holds it, as HTML, so a paste there finds it.
        self.clipboard = None;
        // The cut waiting to become a move belongs to the document it was cut in: `file.open`
        // is a pure command, so the dispatcher's reset of that wait skips the document it
        // opens, and the new document would inherit the old one's cut.
        self.pending_move = None;
        self.password = None;
        self.file_name = None;
        self.sel = Selection::caret(self.doc.start_of(StoryRef::Body));
        self.pending = None;
        self.typing = None;
        self.also_selected.clear();
        self.column = None;
        self.column_mode = false;
        self.widened = None;
        self.edit_marks.clear();
        self.new_header_parts.clear();
        self.hf_return = None;
        self.go_back_step = 0;
        self.view.outline.collapsed.clear();
        self.reset_history();
        self.clear_versions();
        self.touch();
        self.dirty = false;
        self.relayout();
    }

    /// A snapshot to put back with [`Session::restore`], for a caller that runs a command and
    /// may want to take it back without a trace (cheap: the document's blocks are shared).
    ///
    /// Covered: `doc`, `sel`, the undo stack (its length, and its oldest steps when it is near
    /// the limit), `redo`, the open typing group, a pending `join_next_undo` and `dirty`. `rev`,
    /// `cache` and `layout` are derived (`restore` bumps `rev`). The other fields are not
    /// covered: a caller that lets the command change them (the view, the find state, the
    /// clipboard, macros, the file path…) saves and puts them back itself.
    ///
    /// `restore` is exact when the commands in between only add undo steps (every mutating
    /// command adds at most one; up to [`SNAPSHOT_HEAD`] may push old steps out of a full
    /// stack). Taking steps off the stack (`edit.undo`) or clearing it (`file.new`,
    /// `file.open`) in between is not covered.
    ///
    /// The destructuring lists every field, so a new field does not build until someone decides
    /// whether a snapshot must cover it.
    pub fn edit_snapshot(&self) -> EditSnapshot {
        let Session {
            doc,
            sel,
            view: _,
            pending: _,
            path: _,
            file_name: _,
            // The custom dictionary file, like the file path: host wiring, not an edit.
            user_dict_path: _,
            dirty,
            clipboard: _,
            // Like the clipboard: what the next paste can make a move of.
            pending_move: _,
            clipboard_text: _,
            clipboard_html: _,
            clip_history: _,
            find: _,
            goal_x: _,
            page_hint: _,
            registry: _,
            author: _,
            initials: _,
            painter: _,
            status: _,
            // Headers just created to edit: closing checks they still exist and are empty.
            new_header_parts: _,
            hf_return: _,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            autocap: _,
            rev: _,
            cache: _,
            layout: _,
            // The word being typed, like `column`: valid only while `sel` is where typing left it.
            typing: _,
            originals: _,
            picture_looks: _,
            last_command: _,
            recording: _,
            macros: _,
            versions: _,
            building_blocks: _,
            autosave: _,
            bib_style: _,
            merge: _,
            ui_requests: _,
            document_id: _,
            read_aloud: _,
            prefs: _,
            system_region: _,
            keymap: _,
            ribbon: _,
            // A preference for new documents, like `prefs`: not part of this document.
            editing_lang: _,
            // Equation editing mode, like `view`: not part of the document or its history.
            math: _,
            math_latex: _,
            math_normal_text: _,
            // The save password, like the file path: not an edit to the document.
            password: _,
            #[cfg(not(target_arch = "wasm32"))]
                recovery: _,
            also_selected: _,
            // Column selection and a widened selection's ends, like `sel`'s shape: valid only while
            // `sel` matches them.
            column: _,
            column_mode: _,
            widened: _,
            // Go Back's places, like the caret history: not part of the document.
            edit_marks: _,
            go_back_step: _,
        } = self;
        let head = if history.len() + SNAPSHOT_HEAD > MAX_UNDO { history.iter().take(SNAPSHOT_HEAD).cloned().collect() } else { Vec::new() };
        EditSnapshot {
            doc: doc.clone(),
            sel: sel.clone(),
            history_len: history.len(),
            evicted: *undo_evicted,
            head,
            redo: redo.clone(),
            typing_open: *typing_open,
            join_next: *join_next,
            dirty: *dirty,
        }
    }

    /// Put a snapshot back: the commands run since leave no trace in the document, the
    /// selection or undo/redo (undo steps the limit pushed out meanwhile come back). The layout
    /// is recomputed.
    pub fn restore(&mut self, snap: EditSnapshot) {
        let EditSnapshot { doc, sel, history_len, evicted, head, redo, typing_open, join_next, dirty } = snap;
        let gone = usize::try_from(self.undo_evicted.saturating_sub(evicted)).unwrap_or(usize::MAX);
        if gone > 0 {
            let back = gone.min(head.len());
            self.history.splice(0..0, head.into_iter().take(back));
        }
        self.undo_evicted = evicted;
        self.autocap = None;
        self.doc = doc;
        self.sel = sel;
        self.also_selected.clear();
        self.history.truncate(history_len);
        self.redo = redo;
        self.typing_open = typing_open;
        self.join_next = join_next;
        self.touch();
        self.dirty = dirty;
    }

    /// Make the selection valid for the current document.
    pub fn clamp_selection(&mut self) {
        self.sel.anchor = self.doc.clamp(&self.sel.anchor);
        self.sel.focus = self.doc.clamp(&self.sel.focus);
        // The equation being edited must still be there (undo, edits elsewhere).
        if let Some(m) = &self.math {
            let eq = self.doc.para_at(&m.at).and_then(|p| match p.object_at(m.at.off) {
                Some(wordcraft_doc::InlineObject::Equation { math, .. }) => Some(wordcraft_doc::math_edit::clamp(&math.nodes, &m.pos)),
                _ => None,
            });
            match eq {
                Some(pos) => {
                    if let Some(m) = self.math.as_mut() {
                        m.pos = pos;
                    }
                }
                None => self.math = None,
            }
        }
    }

    /// Why command `id` can't run now, for greying it out: its own `enabled` check, then what
    /// Restrict Editing refuses outright. `None` when it can run (or doesn't exist).
    pub fn why_disabled(&self, id: &str) -> Option<&'static str> {
        let spec = self.registry.get(id)?;
        (spec.enabled)(self).or_else(|| if spec.mutates { crate::cmd::protect::greyed_out(self, id) } else { None })
    }

    /// Run a command by id with JSON params (the single entry point for the UI, CLI, MCP and
    /// control channel). Mutating commands are undoable; a failed command leaves the document as
    /// it was; a panic inside a command becomes an error.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, CmdError> {
        let join = std::mem::take(&mut self.join_next);
        let reg = self.registry.clone();
        let Some(spec) = reg.get(id) else { return Err(CmdError::Unknown(id.to_string())) };
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(format!("{id}: {why}")));
        }
        // Restrict Editing: refused outright, or allowed as long as it only edits the exceptions.
        let guard = if spec.mutates { crate::cmd::protect::check_before(self, id, params)? } else { false };
        // Quick Parts aren't part of the document, so a refused edit puts them back itself (the Spike).
        let blocks_before = guard.then(|| self.building_blocks.clone());
        if spec.mutates && self.doc.settings.protection.as_deref() == Some("trackedChanges") {
            self.doc.settings.track_changes = true;
        }
        // Macro recording and Repeat.
        let record = !matches!(id, "tools.recordMacro" | "tools.macros" | "edit.undo" | "edit.redo" | "edit.repeat")
            && (spec.mutates || id.starts_with("caret.") || id.starts_with("select."));
        // A password never lands in a recorded macro or Repeat.
        let kept = || {
            let mut p = params.clone();
            if let Some(m) = p.as_object_mut() {
                m.remove("password");
            }
            p
        };
        if record && let Some((_, steps)) = self.recording.as_mut() {
            steps.push((id.to_string(), kept()));
        }
        if spec.mutates && !matches!(id, "edit.undo" | "edit.redo" | "edit.repeat") {
            self.last_command = Some((id.to_string(), kept()));
        }
        // Typing, caret movement and selection outside the equation commands leave the equation.
        if self.math.is_some()
            && (id.starts_with("text.") || id.starts_with("caret.") || id.starts_with("select.") || id.starts_with("edit.paste"))
            && let Some(m) = self.math.take()
        {
            let off = m.at.off + wordcraft_doc::para::OBJ.len_utf8();
            self.sel = Selection::caret(Pos { off, ..m.at });
        }
        // The undo stacks are snapshotted whole, not as a length: commands run nested commands,
        // whose checkpoints, undos and redos (and evictions at the limit) must be put back too.
        let before_doc = if spec.mutates {
            Some((
                self.doc.clone(),
                self.sel.clone(),
                self.history.clone(),
                self.redo.clone(),
                self.typing_open,
                self.undo_evicted,
                self.column.clone(),
            ))
        } else {
            None
        };
        // These mutating commands can replace the open document and discard its saved versions.
        // Keep the versions until the command and the protected-region post-check have both
        // succeeded; rejected commands must leave the old document and its snapshots together.
        let replaces_document = matches!(id, "review.compare" | "review.combine" | "mailings.finish" | "mailings.envelopes" | "mailings.labels");
        let versions_before = replaces_document.then(|| self.versions.clone());
        let identity_before = replaces_document.then(|| {
            (
                self.path.clone(),
                self.file_name.clone(),
                self.document_id,
                self.password.clone(),
                self.dirty,
                self.bib_style.clone(),
                self.merge.preview,
            )
        });
        let typing = spec.id == "text.insert" || spec.id == "equation.type";
        if spec.mutates && !typing {
            self.typing_open = false;
        }
        if spec.mutates {
            let label = if typing { "Typing" } else { spec.label };
            if !join || self.history.is_empty() {
                self.checkpoint(label);
            }
        } else if !spec.id.starts_with("caret.") && !spec.id.starts_with("view.") {
            // Non-mutating commands other than caret movement keep the typing group.
        } else if spec.id.starts_with("caret.") {
            self.typing_open = false;
        }
        // Only the paste right after a tracked cut makes it a move.
        if (spec.mutates || matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo")) && !matches!(id, "edit.cut" | "edit.paste" | "edit.pasteSpecial")
        {
            self.pending_move = None;
        }
        let sel_before = self.sel.clone();
        // A rejected command must not persist a new proofing default, including a rejection
        // by the protected-region check after its handler has succeeded. Undo stays document-only.
        let default_language_before = self.prefs.default_language.clone();
        let run = spec.run;
        let mutates = spec.mutates;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::cmd::column::dispatch(self, id, mutates, run, params)));
        let mut result = match result {
            Ok(r) => r,
            Err(p) => {
                let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                Err(CmdError::Failed(format!("{id} panicked: {msg}")))
            }
        };
        // Restrict Editing: outside the exceptions nothing may change.
        if result.is_ok()
            && guard
            && let Some((before, ..)) = &before_doc
            && !crate::cmd::protect::outside_unchanged(before, &self.doc, &self.author)
        {
            result = Err(CmdError::Disabled(format!("{id}: only the editable regions of this protected document can change")));
        }
        match &result {
            Ok(_) => {
                if spec.mutates {
                    // Formatting changed while tracking becomes a formatting revision.
                    if let Some((before, ..)) = &before_doc {
                        crate::cmd::fmt_revisions::record(self, id, before);
                    }
                    self.touch();
                    self.doc.ensure_nonempty();
                    self.doc.prune_text_boxes();
                    // Glued connectors follow the shapes they join.
                    if crate::cmd::connectors::reroutes_after(id) && crate::cmd::connectors::reroute(self) {
                        self.touch();
                    }
                }
                self.clamp_selection();
                if sel_before.focus.story == StoryRef::Body && self.in_header_footer() {
                    self.hf_return = Some(sel_before.clone());
                }
                // A selection command that moves the insertion point ends the typing group, like
                // caret movement: typing, Select, typing are two undo steps (#419).
                if !spec.mutates && self.sel != sel_before {
                    self.typing_open = false;
                }
                if spec.mutates && !matches!(id, "edit.undo" | "edit.redo") {
                    self.note_edit();
                }
                // Outline view: collapsed headings follow edits; the caret stays where it shows.
                crate::cmd::outline::after_command(self, &sel_before);
                // Extra selected objects last until something else is edited or selected; formatting
                // them keeps them selected, as in Word.
                if !matches!(id, "select.addObject" | "arrange.ungroup" | "shape.fill" | "shape.effects") && (spec.mutates || self.sel != sel_before)
                {
                    self.also_selected.clear();
                }
                self.note_typing(id, before_doc.as_ref().map(|b| &b.0), &sel_before);
            }
            Err(e) => {
                self.autocap = None;
                self.prefs.default_language = default_language_before;
                if let Some(versions) = versions_before {
                    self.versions = versions;
                }
                if let Some((path, file_name, document_id, password, dirty, bib_style, merge_preview)) = identity_before {
                    self.path = path;
                    self.file_name = file_name;
                    self.document_id = document_id;
                    self.password = password;
                    self.dirty = dirty;
                    self.bib_style = bib_style;
                    self.merge.preview = merge_preview;
                }
                if let Some((d, s, h, r, t, ev, c)) = before_doc {
                    self.doc = d;
                    self.sel = s;
                    self.history = h;
                    self.redo = r;
                    self.typing_open = t;
                    // The steps the command pushed out are back, so they no longer count as gone.
                    self.undo_evicted = ev;
                    self.column = c;
                }
                if let Some(b) = blocks_before {
                    self.building_blocks = b;
                }
                self.status = e.to_string();
            }
        }
        result
    }

    /// The paragraph path the caret is in.
    pub fn caret_path(&self) -> &Path {
        &self.sel.focus.path
    }
    pub fn story(&self) -> StoryRef {
        self.sel.focus.story
    }

    /// Formatting typing would use at the caret: the caret's, except run direction. New text
    /// takes its direction from its own letters, as on screen, so Latin typed or inserted into
    /// reopened Hebrew isn't saved as right-to-left; the DOCX writer marks right-to-left pieces
    /// itself (#1543). The bidi language stays (it only applies to complex-script text).
    /// Over a selection, as in Word, typing takes the first selected character's formatting,
    /// whichever way the selection was made (#2879).
    pub fn typing_props(&self) -> CharProps {
        let (a, _) = self.sel.ordered();
        let first = match self.doc.para_at(&a) {
            Some(p) if self.pending.is_none() && !self.sel.is_collapsed() && a.off < p.len() => Some(p.props_of_char(a.off)),
            _ => None,
        };
        let mut c = match first {
            Some(props) => {
                // The selection is replaced, so the new text isn't part of a link or a note number.
                let mut out = if props.link.is_some() { props.unlinked() } else { props.clone() };
                if matches!(out.style.as_deref(), Some("FootnoteReference" | "EndnoteReference")) {
                    out.style = None;
                }
                out
            }
            None => self.caret_props(),
        };
        c.rtl = None;
        c
    }

    /// The caret's formatting, as Reveal Formatting shows it.
    pub fn caret_props(&self) -> CharProps {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let f = &self.sel.focus;
        let Some(p) = self.doc.para_at(f) else { return CharProps::default() };
        let props = p.props_at(f.off);
        // As in Word, text typed at either end of a hyperlink isn't part of it; only typing
        // inside the link extends it.
        let inside_link = props.link.is_some() && f.off > 0 && f.off < p.len() && p.props_of_char(f.off).link == props.link;
        // Text typed next to a symbol is in the text's font, not the symbol's (`w:sym`).
        let mut out = (if props.link.is_some() && !inside_link { props.unlinked() } else { props.clone() }).without_symbol();
        // A footnote or endnote number is raised by its character style; text typed after it is body text.
        if matches!(out.style.as_deref(), Some("FootnoteReference" | "EndnoteReference")) {
            out.style = None;
        }
        out
    }

    /// Words in the document, as the status bar shows them (see [`Prefs::count_notes`]).
    pub fn word_count(&self) -> usize {
        if self.prefs.count_notes { self.doc.word_count_including_notes() } else { self.doc.word_count() }
    }

    /// Selected plain text.
    pub fn selected_text(&self) -> String {
        // A column block or whole cells that aren't one run of the text: each piece's own text,
        // a line each, so the status bar and Word Count count those cells only.
        if let Some(segs) = self.column_segments() {
            return segs.iter().map(|(a, b)| self.doc.copy_range(a.min(b), a.max(b)).plain_text()).collect::<Vec<_>>().join("\n");
        }
        let (a, b) = self.sel.ordered();
        self.doc.copy_range(&a, &b).plain_text()
    }
}

/// A new blank document on `region`'s paper (see [`Session::blank_document`]).
pub fn blank_document(region: &wordcraft_geom::Region) -> Document {
    let mut d = Document::new();
    let (_, w, h) = region.paper();
    d.last_section.page_w = w;
    d.last_section.page_h = h;
    d
}
