//! View › Markdown Source: a Markdown file's document as editable text.
//!
//! The document holds no Markdown while it is edited, so entering writes it out as text, and the
//! text waits beside the document ([`MdSource`]) while it is edited. Leaving reads a changed text
//! back into a new document; one Undo takes back the whole visit. A visit that changes nothing
//! leaves the document as it was, whatever Markdown can't represent.
//!
//! While the text is open it is the document: commands that edit the document are refused, those
//! that read it (Save, Export) first get the text applied, and those that change the view leave.
//!
//! The file's own text is kept beside the document too ([`FileText`]): while it still stands for
//! the document, the view shows it and Save writes it, unchanged. An edit in the formatted view
//! makes the document something only writing it out again can say.

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::{Document, Path, Pos, StoryRef};

use crate::doc_state::Selection;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The largest text the view takes, in bytes: its text box lays all of it out after each edit.
pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

const LABEL: &str = "Markdown Source";

/// The text of an open Markdown Source view.
#[derive(Clone)]
pub struct MdSource {
    /// What the view shows and edits.
    pub text: String,
    /// The caret in `text`, as a byte offset. The front end keeps it current.
    pub caret: usize,
    /// The front end still has to put its caret at `caret` and show it.
    pub place_caret: bool,
    /// The front end still has to reset its typing history from earlier visits.
    pub reset_undo: bool,
    /// Markdown can't represent all of the document: applying a changed text loses something.
    pub loses_content: bool,
    /// When the front end last applied the text while it was being typed, on its own clock.
    pub applied_at_ms: f64,
    /// The text on entry.
    entry_text: Arc<str>,
    /// The document on entry, which that text stands for: back to it, nothing is lost.
    entry_doc: Arc<Document>,
    /// The line each top-level block of `entry_doc` starts on.
    entry_lines: Vec<usize>,
    /// The text the document now holds.
    applied: Arc<str>,
    /// The visit has its undo step.
    has_step: bool,
    /// The undo mark recorded when the visit took its undo step, if one was taken.
    step: Option<u64>,
    /// The line each top-level block starts on, from the last parse.
    block_lines: Vec<usize>,
}

impl MdSource {
    /// The text holds edits the document doesn't have yet.
    pub fn is_pending(&self) -> bool {
        self.text != *self.applied
    }
}

/// A Markdown file's own text, which keeps what the document can't (front matter, comments, a
/// code block's language, the author's own spacing): the text the file was opened or last saved
/// with, and the last text the view applied. Either stands for the document while reading it
/// gives the document as it is, so an Undo back to either finds it again.
#[derive(Clone, Default)]
pub struct FileText {
    saved: Option<String>,
    latest: Option<String>,
    cached_exact: RefCell<Option<(u64, Option<String>)>>,
}

impl FileText {
    /// A file's bytes as its text, if they can be written back as they are: valid UTF-8,
    /// not empty, and no longer than the view takes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let keep = !bytes.is_empty() && bytes.len() <= MAX_SOURCE_BYTES;
        let saved = keep.then(|| std::str::from_utf8(bytes).ok()).flatten().map(str::to_string);
        Self { saved, latest: None, cached_exact: RefCell::new(None) }
    }

    /// The text just written to the file, which is the file's own text now.
    pub fn saved(text: Option<String>) -> Self {
        Self { saved: text, latest: None, cached_exact: RefCell::new(None) }
    }

    /// No text is kept.
    pub fn is_empty(&self) -> bool {
        self.saved.is_none() && self.latest.is_none()
    }
}

/// The kept text that stands for the document as it is, if one does: the latest first.
fn exact_text(s: &Session) -> Option<String> {
    if let Some((cached_rev, ref res)) = *s.md_file.cached_exact.borrow()
        && cached_rev == s.rev()
    {
        return res.clone();
    }
    let kept = [s.md_file.latest.as_deref(), s.md_file.saved.as_deref()];
    let res = kept.into_iter().flatten().find(|text| parse(&s.doc, text).same_content(&s.doc)).map(str::to_string);
    *s.md_file.cached_exact.borrow_mut() = Some((s.rev(), res.clone()));
    res
}

/// What Save writes to the Markdown file `path`, in place of the document written out again: the
/// file's own text, while it stands for the document. Not for a Print Markup copy, which adds
/// the comments.
pub(crate) fn text_to_save(s: &Session, path: &std::path::Path, markup: bool) -> Option<String> {
    if markup || !is_markdown_file(path) {
        return None;
    }
    exact_text(s)
}

/// Command specifications for Markdown Source view toggling and text inspection.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        // Not `pure`: leaving edits the document. Entering changes nothing, so it adds no undo step.
        CommandSpec::new(TOGGLE, LABEL, "View › Views", toggle).params(r#"{"value"?: bool}"#).when(can_toggle),
        CommandSpec::new(TEXT, "Markdown Source Text", "View › Views", text)
            .params(r#"{"text"?: string (up to 2 MB), "caret"?: integer >= 0}"#)
            .when(|s| s.md_source.is_none().then_some("only in Markdown Source")),
    ]
}

const TOGGLE: &str = "view.markdownSource";
const TEXT: &str = "view.markdownSourceText";
/// The views that replace this one: they show the document, so the text goes into it first.
const LEAVES: [&str; 5] = ["view.printLayout", "view.webLayout", "view.draft", "view.outline", "view.readMode"];

/// Whether a file's name says it is Markdown.
pub fn is_markdown_file(path: &std::path::Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}

/// Whether the current session's path points to a Markdown file.
fn is_markdown_path(s: &Session) -> bool {
    s.path.as_deref().is_some_and(is_markdown_file)
}

/// Checks whether the Markdown Source view can currently be toggled in `s`.
fn can_toggle(s: &Session) -> Option<&'static str> {
    if s.md_source.is_some() {
        None
    } else if !is_markdown_path(s) {
        Some("only for Markdown (.md) files")
    } else if s.view.read_mode {
        Some("not in Read Mode")
    } else if s.doc.settings.track_changes || !s.doc.revisions.is_empty() {
        Some("not with tracked changes")
    } else if s.doc.settings.protection.is_some() {
        Some("not in a protected document")
    } else {
        None
    }
}

/// Toggles or sets the Markdown Source view state.
fn toggle(s: &mut Session, v: &Value) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(s.md_source.is_none());
    match (on, s.md_source.is_some()) {
        (true, false) => enter(s)?,
        (false, true) => leave(s),
        _ => {}
    }
    Ok(json!({"value": s.md_source.is_some()}))
}

/// Gets or sets the Markdown Source text and caret position.
fn text(s: &mut Session, v: &Value) -> CmdResult {
    let new_text = p::str(v, "text");
    if new_text.is_some_and(|t| t.len() > MAX_SOURCE_BYTES) {
        return Err(CmdError::Params(too_long()));
    }
    if let Some(cv) = v.get("caret").filter(|c| !c.is_null())
        && cv.as_u64().is_none()
    {
        return Err(CmdError::Params("`caret` must be a non-negative integer".into()));
    }
    let Some(src) = s.md_source.as_mut() else { return Err(CmdError::Disabled("only in Markdown Source".into())) };
    let mut edited = false;
    if let Some(t) = new_text {
        edited = src.text != t;
        src.text = t.to_string();
        src.caret = src.text.floor_char_boundary(src.caret.min(src.text.len()));
    }
    if let Some(caret) = v.get("caret").and_then(Value::as_u64) {
        let caret = usize::try_from(caret).unwrap_or(usize::MAX).min(src.text.len());
        src.caret = src.text.floor_char_boundary(caret);
        src.place_caret = true;
    }
    if edited {
        src.place_caret = true;
        s.dirty = true;
    }
    let mut result = json!({"caret": src.caret, "losesContent": src.loses_content});
    if new_text.is_none() {
        result["text"] = json!(&src.text);
    }
    Ok(result)
}

/// Error message returned when Markdown Source text exceeds [`MAX_SOURCE_BYTES`].
fn too_long() -> String {
    format!("Markdown Source takes up to {} MB of text", MAX_SOURCE_BYTES / (1024 * 1024))
}

/// Enters Markdown Source view, preparing source text and mapping caret position.
fn enter(s: &mut Session) -> Result<(), CmdError> {
    // The file's own text when it still says what the document holds, so nothing is lost.
    let (text, loses_content, block_lines) = match exact_text(s) {
        Some(text) => {
            let (_, lines) = parse_with_lines(&s.doc, &text);
            (text, false, lines)
        }
        None => {
            let text = wordcraft_formats::markdown::export(&s.doc);
            if text.len() > MAX_SOURCE_BYTES {
                return Err(CmdError::Failed(too_long()));
            }
            let (doc, lines) = parse_with_lines(&s.doc, &text);
            let loses_content = !doc.same_content(&s.doc);
            (text, loses_content, lines)
        }
    };
    let block = if s.sel.focus.story == StoryRef::Body { s.sel.focus.path.0.first().copied().unwrap_or(0) as usize } else { 0 };
    let line = block_lines.get(block).copied().unwrap_or(0);
    let caret = line_start_byte(&text, line);
    let text_arc: Arc<str> = Arc::from(text.as_str());
    let entry_lines = block_lines.clone();
    s.md_source = Some(MdSource {
        caret,
        place_caret: true,
        reset_undo: true,
        loses_content,
        applied_at_ms: 0.0,
        entry_doc: Arc::new(s.doc.clone()),
        entry_lines,
        entry_text: text_arc.clone(),
        applied: text_arc,
        text,
        has_step: false,
        step: None,
        block_lines,
    });
    Ok(())
}

/// Leaves Markdown Source view, applying changes to the document model.
fn leave(s: &mut Session) {
    // `Session::run` took a step for this command: the visit's first, or one too many.
    // If the toggle was joined to an earlier edit (`join_next_undo`), it created no checkpoint.
    let had_step = s.md_source.as_ref().is_some_and(|src| src.has_step);
    let step = s.md_source.as_ref().and_then(|src| src.step);
    let mark = s.undo_mark();
    let toggle_created_step = step.is_some_and(|step| mark > step);
    if toggle_created_step {
        s.drop_last_undo_step();
    }
    let mark_after = s.undo_mark();
    if let Some(src) = s.md_source.as_mut() {
        src.has_step = true;
        if src.step.is_none() {
            src.step = Some(mark_after);
        }
    }
    apply(s);
    if let Some(src) = s.md_source.take()
        && (had_step || src.text != *src.entry_text)
    {
        let block = block_of_caret(&src.text, src.caret, &s.doc, &src.block_lines);
        s.sel = Selection::caret(block);
    }
}

/// The text into the document, if it has edits the document lacks. Its first change in a visit
/// takes the visit's undo step. Save, AutoSave and crash recovery see the text this way.
pub fn apply(s: &mut Session) {
    let (text, entry_text, entry_doc, has_step, entry_lines) = {
        let Some(src) = s.md_source.as_ref().filter(|src| src.is_pending()) else { return };
        if src.text.len() > MAX_SOURCE_BYTES {
            return;
        }
        (src.text.clone(), src.entry_text.clone(), src.entry_doc.clone(), src.has_step, src.entry_lines.clone())
    };
    let (doc, lines) = if text == *entry_text { ((*entry_doc).clone(), entry_lines) } else { parse_with_lines(&s.doc, &text) };
    let step = if !has_step {
        s.checkpoint(LABEL);
        Some(s.undo_mark())
    } else {
        s.md_source.as_ref().and_then(|src| src.step)
    };
    s.doc = doc;
    s.note_body_replaced();
    s.math = None;
    s.touch();
    s.clamp_selection();
    s.md_file.latest = Some(text.clone());
    *s.md_file.cached_exact.borrow_mut() = Some((s.rev(), Some(text.clone())));
    if let Some(src) = s.md_source.as_mut() {
        src.applied = Arc::from(text.as_str());
        src.block_lines = lines;
        src.has_step = true;
        src.step = step;
    }
}

/// `text` as a document and block lines, with what isn't in Markdown from `current`: its page, settings,
/// properties and language, as opening the file gave it.
fn parse_with_lines(current: &Document, text: &str) -> (Document, Vec<usize>) {
    let (mut doc, lines) = wordcraft_formats::markdown::import_on_with_lines(text, &current.last_section);
    doc.ensure_nonempty();
    doc.settings = current.settings.clone();
    doc.core = current.core.clone();
    Arc::make_mut(&mut doc.styles).default_chr.lang = current.styles.default_chr.lang.clone();
    (doc, lines)
}

/// `text` as a document, with what isn't in Markdown from `current`: its page, settings,
/// properties and language, as opening the file gave it.
fn parse(current: &Document, text: &str) -> Document {
    parse_with_lines(current, text).0
}

/// The byte offset where `line` starts, or the end of a text with fewer lines.
fn line_start_byte(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut lines = 0;
    for (i, &b) in text.as_bytes().iter().enumerate() {
        if b == b'\n' {
            lines += 1;
            if lines == line {
                return i.saturating_add(1);
            }
        }
    }
    text.len()
}

/// The 0-based line that byte offset `byte` belongs to.
fn line_of_byte(text: &str, byte: usize) -> usize {
    let slice = text.as_bytes().get(..byte).unwrap_or(text.as_bytes());
    slice.iter().filter(|&&b| b == b'\n').count()
}

/// Where the caret at byte offset `caret` of `text` lands in `doc`: at the
/// start of the top-level block its line belongs to.
fn block_of_caret(text: &str, caret: usize, doc: &Document, block_lines: &[usize]) -> Pos {
    let caret_line = line_of_byte(text, caret);
    let top = if block_lines.is_empty() { 0 } else { block_lines.partition_point(|&l| l <= caret_line).saturating_sub(1) };
    let top = top.min(doc.body.len().saturating_sub(1));
    let top = u32::try_from(top).unwrap_or(0);
    let path = doc.para_paths(StoryRef::Body).into_iter().find(|path| path.0.first().is_some_and(|b| *b >= top));
    match path {
        Some(path) => Pos { story: StoryRef::Body, path, off: 0 },
        None => Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
    }
}

/// Why the open text refuses command `id`, if it does.
pub(crate) fn refusal(s: &Session, id: &str, mutates: bool) -> Option<&'static str> {
    (s.md_source.is_some()
        && id != TOGGLE
        && id != TEXT
        && !LEAVES.contains(&id)
        && (mutates || matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo" | "edit.repeat")))
    .then_some("leave Markdown Source first")
}

/// What a command does while the text is open, before it runs (see the module's header).
pub(crate) fn before_command(s: &mut Session, id: &str, mutates: bool) -> Result<(), CmdError> {
    if s.md_source.is_none() || id == TOGGLE || id == TEXT {
        return Ok(());
    }
    if LEAVES.contains(&id) {
        s.run(TOGGLE, &json!({"value": false}))?;
    } else if let Some(why) = refusal(s, id, mutates) {
        return Err(CmdError::Disabled(format!("{id}: {why}")));
    } else if !id.starts_with("caret.") && !id.starts_with("select.") && !id.starts_with("view.") {
        apply(s);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wordcraft_layout::ViewMode;

    const SAMPLE: &str = "# Title\n\nFirst paragraph.\n\n## Second\n\n- one\n- two\n\nLast **bold** paragraph.\n";

    /// Creates a test session with a Markdown document and a dummy path.
    fn session(md: &str) -> Session {
        let mut s = Session::new(crate::io::open_bytes("a.md", md.as_bytes()).unwrap());
        s.path = Some(PathBuf::from("/tmp/wordcraft-md-source-test.md"));
        s.dirty = false;
        s
    }

    /// Runs the Markdown Source text command with `text`.
    fn set_text(s: &mut Session, text: &str) {
        s.run(TEXT, &json!({"text": text})).unwrap();
    }

    /// Extracts the plain body text of `s`.
    fn body_text(s: &Session) -> String {
        s.doc.plain_text(StoryRef::Body)
    }

    /// Verifies entering and leaving without changes does not mutate document or add undo step.
    #[test]
    fn a_visit_that_changes_nothing_is_no_edit() {
        let mut s = session(SAMPLE);
        let before = s.doc.clone();
        let depth = s.undo_depth();
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(s.md_source.is_some());
        assert!(!s.dirty, "entering is not an edit");
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(s.md_source.is_none());
        assert!(s.doc.same_content(&before));
        assert!(!s.dirty, "a visit without edits leaves no unsaved changes");
        assert_eq!(s.undo_depth(), depth, "and no undo step");
    }

    /// Verifies edited source text becomes document upon leaving and one undo reverts the visit.
    #[test]
    fn an_edited_text_becomes_the_document_and_one_undo_takes_it_back() {
        let mut s = session(SAMPLE);
        let before = s.doc.clone();
        s.run(TOGGLE, &json!({})).unwrap();
        let text = s.run(TEXT, &json!({})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("# Title"), "{text}");
        set_text(&mut s, &text.replace("First paragraph.", "Changed paragraph."));
        assert!(s.dirty, "the edit is an unsaved change");
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(body_text(&s).contains("Changed paragraph."));
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.same_content(&before), "one undo restores the document");
        assert!(s.can_redo());
    }

    static TEST_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Generates a unique temporary directory for test file isolation.
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("wordcraft-md-{tag}-{}-{n}-{nanos}", std::process::id()))
    }

    /// Verifies saving within source view writes text to disk while keeping view open and preserving undo.
    #[test]
    fn save_writes_the_text_and_the_visit_stays_one_undo_step() {
        let dir = unique_temp_dir("source-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.md");
        let mut s = session(SAMPLE);
        s.path = Some(path.clone());
        let before = s.doc.clone();
        let depth = s.undo_depth();
        s.run(TOGGLE, &json!({})).unwrap();
        set_text(&mut s, "# Saved one\n");
        s.run("file.save", &json!({})).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("Saved one"));
        assert!(!s.dirty);
        assert!(s.md_source.is_some(), "saving keeps the view open");
        set_text(&mut s, "# Saved two\n");
        s.run("file.save", &json!({})).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("Saved two"));
        set_text(&mut s, "# Left three\n");
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(body_text(&s).contains("Left three"));
        assert_eq!(s.undo_depth(), depth + 1, "one step for the whole visit");
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.same_content(&before));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Verifies returning to the original entry text restores formatting that Markdown cannot represent and maps the caret using entry block lines.
    #[test]
    fn going_back_to_the_entry_text_restores_what_markdown_cannot_hold() {
        let mut s = session(SAMPLE);
        std::sync::Arc::make_mut(&mut s.doc.styles).get_mut("Normal").unwrap().chr.bold = Some(true);
        let before = s.doc.clone();
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(s.md_source.as_ref().unwrap().loses_content, "Markdown doesn't keep a changed style");
        let text = s.md_source.as_ref().unwrap().text.clone();
        set_text(&mut s, "# Other\n");
        apply(&mut s);
        assert!(!s.doc.same_content(&before));
        set_text(&mut s, &text);
        let last_off = text.find("Last ").unwrap_or(0);
        s.run(TEXT, &json!({"caret": last_off})).unwrap();
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(s.doc.same_content(&before));
        assert_eq!(s.sel.focus.path.0.first().copied(), Some(5));
    }

    /// Verifies that leaving Markdown Source with `join_next_undo` after an applied edit keeps the visit's undo checkpoint.
    #[test]
    fn leaving_joined_after_an_applied_edit_keeps_the_visits_undo_step() {
        let mut s = session(SAMPLE);
        let before = body_text(&s);
        s.run(TOGGLE, &json!({})).unwrap();
        set_text(&mut s, "# Intermediate\n");
        apply(&mut s);
        assert_eq!(s.undo_depth(), 1);
        set_text(&mut s, "# Final\n");
        s.join_next_undo();
        s.run(TOGGLE, &json!({"value": false})).unwrap();
        assert!(s.md_source.is_none());
        assert!(body_text(&s).contains("Final"));
        assert_eq!(s.undo_depth(), 1);
        assert!(s.can_undo());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(body_text(&s), before);
    }

    /// Verifies that opening an unedited Markdown file reports no content loss on entry.
    #[test]
    fn markdown_the_document_came_from_is_not_reported_as_lost() {
        let s = &mut session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(!s.md_source.as_ref().unwrap().loses_content);
    }

    /// Verifies standard document editing commands are disabled while Markdown Source view is open.
    #[test]
    fn edits_are_refused_while_the_text_is_open_and_nothing_is_lost() {
        let mut s = session(SAMPLE);
        let before = s.doc.clone();
        s.run(TOGGLE, &json!({})).unwrap();
        set_text(&mut s, "# Kept\n");
        for id in ["format.bold", "edit.undo", "edit.redo"] {
            assert!(matches!(s.run(id, &json!({})), Err(CmdError::Disabled(_))), "{id}");
        }
        assert!(matches!(s.run("text.insert", &json!({"text": "x"})), Err(CmdError::Disabled(_))));
        assert!(s.doc.same_content(&before));
        assert_eq!(s.md_source.as_ref().unwrap().text, "# Kept\n");
    }

    /// Verifies switching view mode leaves Markdown Source view and applies edits.
    #[test]
    fn changing_the_view_leaves_with_the_edits() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        set_text(&mut s, "# Applied\n");
        s.run("view.webLayout", &json!({})).unwrap();
        assert!(s.md_source.is_none());
        assert!(body_text(&s).contains("Applied"));
        assert_eq!(s.view.mode, ViewMode::Web);
    }

    /// Verifies only unprotected Markdown files without tracked changes can enter Markdown Source.
    #[test]
    fn only_markdown_files_without_tracking_or_protection_enter() {
        let mut s = Session::new(Document::from_text("Text"));
        assert!(s.run(TOGGLE, &json!({})).is_err(), "no file");
        s.path = Some(PathBuf::from("/tmp/a.docx"));
        assert!(s.run(TOGGLE, &json!({})).is_err(), "not Markdown");
        s.path = Some(PathBuf::from("/tmp/A.MARKDOWN"));
        s.doc.settings.track_changes = true;
        assert!(s.run(TOGGLE, &json!({})).is_err(), "tracking");
        s.doc.settings.track_changes = false;
        s.doc.settings.protection = Some("readOnly".into());
        assert!(s.run(TOGGLE, &json!({})).is_err(), "protected");
        s.doc.settings.protection = None;
        s.view.read_mode = true;
        assert!(s.run(TOGGLE, &json!({})).is_err(), "Read Mode");
        s.view.read_mode = false;
        s.run(TOGGLE, &json!({})).unwrap();
    }

    /// Verifies source text exceeding limit is refused with CmdError::Failed.
    #[test]
    fn a_text_too_long_is_refused() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        let long = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert!(matches!(s.run(TEXT, &json!({"text": long})), Err(CmdError::Params(_))));
        s.run(TOGGLE, &json!({})).unwrap();
        s.set_document(Document::from_text(&"x".repeat(MAX_SOURCE_BYTES + 1)));
        s.path = Some(PathBuf::from("/tmp/a.md"));
        assert!(matches!(s.run(TOGGLE, &json!({})), Err(CmdError::Failed(_))));
        assert!(s.md_source.is_none());
    }

    /// Verifies caret stays synchronized with its paragraph when entering and leaving source view.
    #[test]
    fn the_caret_stays_with_its_paragraph_both_ways() {
        let mut s = session(SAMPLE);
        let target = s
            .doc
            .para_paths(StoryRef::Body)
            .into_iter()
            .find(|path| s.doc.para(StoryRef::Body, path).is_some_and(|para| para.plain_text().contains("Last")))
            .unwrap();
        s.sel = Selection::caret(Pos { story: StoryRef::Body, path: target.clone(), off: 2 });
        s.run(TOGGLE, &json!({})).unwrap();
        let src = s.md_source.as_ref().unwrap();
        let line: String = src.text[src.caret..].chars().take_while(|c| *c != '\n').collect();
        assert!(line.contains("Last"), "caret on {line:?} in {:?}", src.text);
        let changed = src.text.replace("First paragraph.", "First changed paragraph.");
        let caret = changed.find("Last").unwrap();
        s.run(TEXT, &json!({"text": changed, "caret": caret})).unwrap();
        s.run(TOGGLE, &json!({})).unwrap();
        let para = s.doc.para(StoryRef::Body, &s.sel.focus.path).unwrap();
        assert!(para.plain_text().contains("Last"), "{:?}", para.plain_text());
    }

    /// Verifies that the caret in the trailing paragraph appended after a final table maps to the end of the text.
    #[test]
    fn caret_in_appended_paragraph_after_final_table_stays_at_end() {
        let md = "| Col1 | Col2 |\n|---|---|\n| Val1 | Val2 |\n";
        let mut s = session(md);
        let trailing_para = s
            .doc
            .para_paths(StoryRef::Body)
            .into_iter()
            .find(|path| path.0.first() == Some(&1))
            .expect("document ending in a table must have an appended paragraph");
        s.sel = Selection::caret(Pos { story: StoryRef::Body, path: trailing_para, off: 0 });

        s.run(TOGGLE, &json!({})).unwrap();
        let src = s.md_source.as_ref().unwrap();
        assert_eq!(src.caret, src.text.len(), "caret must land after the table at the end of the text");

        s.run(TOGGLE, &json!({})).unwrap();
        assert_eq!(s.sel.focus.path.0.first(), Some(&1), "caret must stay in the appended paragraph after table");
    }

    /// Verifies hostile carets and texts do not panic.
    #[test]
    fn hostile_carets_and_texts_do_not_panic() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        for (text, caret) in [("", u64::MAX), ("é\n\n\n", 2), ("```\nopen fence", 9), ("|\n|-", 1), ("> > > >\n- - - -", 3)] {
            s.run(TEXT, &json!({"text": text, "caret": caret})).unwrap();
            s.run(TOGGLE, &json!({})).unwrap();
            s.clamp_selection();
            assert!(s.doc.para(s.sel.focus.story, &s.sel.focus.path).is_some(), "{text:?}");
            s.run(TOGGLE, &json!({})).unwrap();
        }
    }

    /// Verifies a parked document preserves its Markdown Source text.
    #[test]
    fn a_parked_document_keeps_its_text() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        set_text(&mut s, "# Mine\n");
        let first = s.document_id();
        let _second = s.add_opened_document(Document::from_text("Other"));
        assert!(s.md_source.is_none(), "the other document has no text open");
        #[cfg(not(target_arch = "wasm32"))]
        {
            let first_rec = s.recovery_docs().find(|r| r.id == first).unwrap();
            assert!(first_rec.doc.plain_text(StoryRef::Body).contains("Mine"), "pending text was applied before parking");
        }
        s.activate(first).unwrap();
        assert_eq!(s.md_source.as_ref().unwrap().text, "# Mine\n");
    }

    /// What the document can't hold, all of which a save without formatted edits keeps.
    const OWN: &str = "---\ntitle: Notes\n---\n\nTitle\n=====\n\n<!-- a comment -->\n\n* star bullet\n* another\n\n- [ ] a task\n- [x] done\n\n```bash\necho hi\n```\n\nSome *emphasis* and a [link](https://example.com \"its title\").\nA wrapped\nline.\n\n![a picture](pic.png)\n\nLast paragraph.\n";

    /// A scratch folder holding `name` with `text`, opened in a new session.
    fn open_file(dir: &str, name: &str, text: &[u8]) -> (Session, PathBuf) {
        let dir = unique_temp_dir(&format!("own-{dir}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let mut s = Session::new(Document::from_text(""));
        s.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
        (s, path)
    }

    /// Saves the session document via `file.save`.
    fn save(s: &mut Session) -> Value {
        s.run("file.save", &json!({})).unwrap()
    }

    /// A scratch folder holding `name`, opened from the bytes `file.open` is handed (`data`, as a
    /// platform hands a file over). The path holds other bytes, so a save that left them alone
    /// would show.
    fn open_handed_over(dir: &str, name: &str, text: &[u8]) -> (Session, PathBuf) {
        let dir = unique_temp_dir(&format!("handed-{dir}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, b"not the opened text").unwrap();
        let mut s = Session::new(Document::from_text(""));
        let data = crate::cmd::insert::base64_encode(text);
        s.run("file.open", &json!({"path": path.to_string_lossy(), "data": data})).unwrap();
        (s, path)
    }

    /// Verifies a Markdown file opened from handed-over bytes keeps its own text and saves it back
    /// byte-for-byte, as one opened from its path does.
    #[test]
    fn a_file_opened_from_handed_over_bytes_keeps_its_own_text() {
        for (dir, text) in [("plain", OWN.to_string()), ("crlf", OWN.replace('\n', "\r\n"))] {
            let (mut s, path) = open_handed_over(dir, "own.md", text.as_bytes());
            assert!(!s.md_file.is_empty(), "{dir}: the opened text must be kept");
            assert_eq!(save(&mut s)["rewritten"], false, "{dir}");
            assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes(), "{dir}");
            done(&path);
        }
    }

    /// Removes the temporary test folder created for `path`.
    fn done(path: &std::path::Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Verifies unedited Markdown files are written back byte-for-byte upon save.
    #[test]
    fn a_file_saved_without_edits_is_written_back_byte_for_byte() {
        for (dir, text) in [("plain", OWN.to_string()), ("crlf", OWN.replace('\n', "\r\n")), ("bom", format!("\u{FEFF}{OWN}"))] {
            let (mut s, path) = open_file(dir, "own.md", text.as_bytes());
            assert_eq!(save(&mut s)["rewritten"], false, "{dir}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{dir}");
            done(&path);
        }
    }

    /// Verifies BOM-prefixed Markdown files retain original bytes when saved without edits.
    #[test]
    fn a_bom_prefixed_file_saved_without_edits_retains_its_original_bytes() {
        let bom = [b"\xEF\xBB\xBF".as_slice(), OWN.as_bytes()].concat();
        let (mut s, path) = open_file("bom-exact", "own.md", &bom);
        assert!(!s.md_file.is_empty(), "BOM file text must be kept");
        assert_eq!(save(&mut s)["rewritten"], false);
        let read = std::fs::read(&path).unwrap();
        assert_eq!(read, bom, "Saved bytes must match original BOM bytes byte-for-byte");
        done(&path);
    }

    /// Verifies Markdown Source view displays the file's own unedited text.
    #[test]
    fn the_view_shows_the_files_own_text() {
        let (mut s, path) = open_file("view", "own.md", OWN.as_bytes());
        s.run(TOGGLE, &json!({})).unwrap();
        let src = s.md_source.as_ref().unwrap();
        assert_eq!(src.text, OWN);
        assert!(!src.loses_content);
        done(&path);
    }

    /// Verifies edits made directly in Markdown Source view are saved as typed.
    #[test]
    fn edits_in_the_view_are_saved_as_typed() {
        let (mut s, path) = open_file("typed", "own.md", OWN.as_bytes());
        s.run(TOGGLE, &json!({})).unwrap();
        let typed = OWN.replace("Last paragraph.", "<!-- another -->\n\n* Last *changed* paragraph.");
        set_text(&mut s, &typed);
        s.run(TOGGLE, &json!({})).unwrap();
        assert_eq!(save(&mut s)["rewritten"], false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), typed);
        // Saved while the view is open: the same.
        s.run(TOGGLE, &json!({})).unwrap();
        let again = typed.replace("another", "a third");
        set_text(&mut s, &again);
        assert_eq!(save(&mut s)["rewritten"], false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), again);
        done(&path);
    }

    /// Verifies formatted edits force re-exporting the document on save with a warning.
    #[test]
    fn an_edit_in_the_formatted_view_writes_the_document_out_again_once_with_a_warning() {
        let (mut s, path) = open_file("formatted", "own.md", OWN.as_bytes());
        s.run("text.insert", &json!({"text": "New "})).unwrap();
        assert_eq!(save(&mut s)["rewritten"], true);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("New "), "{written}");
        assert!(!written.contains("<!-- a comment -->"), "written out again: {written}");
        s.run("text.insert", &json!({"text": "Again "})).unwrap();
        assert_eq!(save(&mut s)["rewritten"], false, "warned once: the file holds the document's own text now");
        done(&path);
    }

    /// Verifies undoing back to original kept text restores original formatting.
    #[test]
    fn undoing_back_to_a_kept_text_finds_it_again() {
        // A formatted edit undone.
        let (mut s, path) = open_file("undo", "own.md", OWN.as_bytes());
        s.run("text.insert", &json!({"text": "New "})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(save(&mut s)["rewritten"], false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), OWN);

        // A visit to the view undone: back to the opened text, then redone: the typed one.
        s.run(TOGGLE, &json!({})).unwrap();
        let typed = OWN.replace("Last paragraph.", "Typed paragraph.");
        set_text(&mut s, &typed);
        s.run(TOGGLE, &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        save(&mut s);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), OWN);
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(save(&mut s)["rewritten"], true, "the typed text was given up for the saved one");
        assert!(std::fs::read_to_string(&path).unwrap().contains("Typed paragraph."));
        done(&path);
    }

    /// Verifies files that cannot go back as they were are written out without rewriting warnings.
    #[test]
    fn files_that_cannot_go_back_as_they_were_are_written_out_without_a_warning() {
        let latin1 = b"Caf\xE9 au lait.\n".to_vec();
        for (dir, bytes) in [("latin1", latin1), ("empty", Vec::new())] {
            let (mut s, path) = open_file(dir, "own.md", &bytes);
            assert!(s.md_file.is_empty(), "{dir}");
            s.run("text.insert", &json!({"text": "New "})).unwrap();
            assert_eq!(save(&mut s)["rewritten"], false, "{dir}");
            done(&path);
        }
        // A Word document saved as Markdown is a copy written out as always.
        let mut s = Session::new(Document::from_text("Hello"));
        let dir = unique_temp_dir("own-docx");
        std::fs::create_dir_all(&dir).unwrap();
        s.path = Some(dir.join("a.docx"));
        let r = s.run("file.saveAs", &json!({"path": dir.join("a.md").to_string_lossy()})).unwrap();
        assert_eq!(r["rewritten"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Verifies markdown source text command sets place_caret flag on explicit carets and edits.
    #[test]
    fn markdown_source_text_command_sets_place_caret() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        assert!(s.md_source.as_ref().unwrap().place_caret);
        s.md_source.as_mut().unwrap().place_caret = false;
        s.run(TEXT, &json!({"caret": 3})).unwrap();
        assert!(s.md_source.as_ref().unwrap().place_caret);
        s.md_source.as_mut().unwrap().place_caret = false;
        s.run(TEXT, &json!({"text": "# Changed\n"})).unwrap();
        assert!(s.md_source.as_ref().unwrap().place_caret);
    }

    /// Verifies caret finds its matching line in file's own text.
    #[test]
    fn the_caret_finds_its_line_in_the_files_own_text() {
        let (mut s, path) = open_file("caret", "own.md", OWN.as_bytes());
        let target = s
            .doc
            .para_paths(StoryRef::Body)
            .into_iter()
            .find(|path| s.doc.para(StoryRef::Body, path).is_some_and(|para| para.plain_text().contains("Last")))
            .unwrap();
        s.sel = Selection::caret(Pos { story: StoryRef::Body, path: target, off: 0 });
        s.run(TOGGLE, &json!({})).unwrap();
        let src = s.md_source.as_ref().unwrap();
        let line: String = src.text[src.caret..].chars().take_while(|c| *c != '\n').collect();
        assert_eq!(line, "Last paragraph.");
        done(&path);
    }

    /// Verifies opening another document over session closes Markdown Source view.
    #[test]
    fn opening_over_the_document_closes_the_view() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        s.set_document(Document::from_text("New"));
        assert!(s.md_source.is_none());
    }

    /// Verifies invalid and negative carets are refused with CmdError::Params.
    #[test]
    fn invalid_and_negative_carets_are_refused() {
        let mut s = session(SAMPLE);
        s.run(TOGGLE, &json!({})).unwrap();
        for bad in [json!({"caret": -1}), json!({"caret": -100}), json!({"caret": 2.5}), json!({"caret": "three"})] {
            assert!(matches!(s.run(TEXT, &bad), Err(CmdError::Params(_))), "{bad:?}");
        }
    }

    /// Verifies markdown source text command is marked as mutating.
    #[test]
    fn markdown_source_text_is_not_marked_pure() {
        let s = session(SAMPLE);
        let spec = s.registry.get(TEXT).unwrap();
        assert!(spec.mutates, "view.markdownSourceText mutates session and dirty state");
    }
}
