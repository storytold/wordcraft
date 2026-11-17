//! Field types another application acts on when it opens the document or updates its fields
//! (#3628): DDE and DDEAUTO start a program; INCLUDETEXT, INCLUDEPICTURE, LINK, IMPORT, INCLUDE,
//! DATABASE (its data source) and RD (the document a table of contents or index takes entries
//! from) fetch their target, over the network for a UNC path. WordCraft never evaluates them, but
//! the people a document goes to open it in Word.
//!
//! An agent can be steered by text in the very document it edits, so a command run for
//! automation (MCP, the control channel and the input it sends: inside [`from_automation`]) can't
//! put one in a document. [`Session::run`](crate::Session::run) holds every command to that:
//! after a command run for automation, no open document may hold an active field, or field
//! markup kept as read, that it didn't hold before (for a document the command opened: that the
//! document it started in didn't hold). A command that breaks this is refused and its changes are
//! undone ([`after_run`]). The exceptions are listed in [`brings_them_in`]: opening a file by its
//! path (the file already holds them), and Undo and Redo (the document held them).
//!
//! The commands that bring content in also check before they change anything, with a reason that
//! names the field:
//!
//! - its code ([`check`]: `insert.field`, `fields.setCode`, mail merge fields);
//! - content brought in from elsewhere ([`check_incoming`]: a paste, another file's text, a
//!   building block, a compared or combined document), unless the document already has a field
//!   with that very code, so copying the document's own fields still works (one cut without
//!   Track Changes has left the document, so pasting it back from automation doesn't);
//! - a document whose bytes it supplies ([`check_supplied`]: `file.open` and
//!   `file.newFromTemplate` with `data`). A document it opens from a file keeps its fields: the
//!   file already holds them.
//!
//! The person at the keyboard still can, and so can automation on a computer whose host sets
//! `WORDCRAFT_ALLOW_ACTIVE_FIELDS=1`: a setting of the host, never a command parameter, so a
//! prompt can't turn it on.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use serde_json::Value;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::kept::KeptXml;
use wordcraft_doc::{Block, Document, InlineObject, Paragraph};

use crate::CmdError;

/// The field types refused from automation, upper case.
pub const ACTIVE_FIELDS: &[&str] = &["DDE", "DDEAUTO", "INCLUDETEXT", "INCLUDEPICTURE", "LINK", "IMPORT", "INCLUDE", "DATABASE", "RD"];

/// The environment variable a host sets to `1` to let automation insert them anyway.
pub const ALLOW_ENV: &str = "WORDCRAFT_ALLOW_ACTIVE_FIELDS";

thread_local! {
    /// Whether this thread runs a command for automation.
    static AUTOMATION: Cell<bool> = const { Cell::new(false) };
}

/// Puts the previous state back when dropped (also when `f` panics).
struct Restore(bool);

impl Drop for Restore {
    fn drop(&mut self) {
        AUTOMATION.with(|a| a.set(self.0));
    }
}

/// Run `f` as automation: the commands it runs on this thread can't insert [`ACTIVE_FIELDS`].
pub fn from_automation<T>(f: impl FnOnce() -> T) -> T {
    let _restore = Restore(AUTOMATION.with(|a| a.replace(true)));
    f()
}

/// The first of `instr`'s field types, its own and those of the fields nested in it (`{ … }`),
/// that is in [`ACTIVE_FIELDS`]. A type ends where a word of the code does for the repo's field
/// parser (`FieldCode::parse`): at a space, a quote, a switch or the start or end of a nested
/// field, so `DDEAUTO"cmd"` and `INCLUDETEXT{ FILENAME }` are active. A quote or `=` before it
/// is skipped, erring towards finding one.
pub fn active_field(instr: &str) -> Option<&'static str> {
    std::iter::once(instr).chain(instr.split('{').skip(1)).find_map(|code| {
        let code = code.trim_start_matches(|c: char| c.is_whitespace() || c == '"' || c == '=');
        let name = code.split(|c: char| c.is_whitespace() || matches!(c, '{' | '}' | '\\' | '"')).next().unwrap_or("");
        ACTIVE_FIELDS.iter().copied().find(|f| f.eq_ignore_ascii_case(name))
    })
}

/// Whether this thread runs a command for automation and the host doesn't allow active fields.
fn enforced() -> bool {
    AUTOMATION.with(Cell::get) && !std::env::var_os(ALLOW_ENV).is_some_and(|v| v == "1")
}

/// The error for `what` (`DDEAUTO fields`) that can't `how` from automation.
fn refused(what: &str, how: &str) -> CmdError {
    CmdError::Disabled(format!("{what} can't {how} from automation (the host can allow them with {ALLOW_ENV}=1)"))
}

/// Something in a document that Word acts on, or may: an active field, or markup that WordCraft
/// keeps as it was read and writes back unchanged, holding a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Item<'a> {
    /// An active field: its code (trimmed) and its type.
    Field(&'a str, &'static str),
    /// Markup written out as it is ([`raw_markup`]) that holds field markup ([`field_markup`]),
    /// whatever field it is: telling which would take a parser for markup WordCraft didn't model.
    Markup(&'a str),
}

impl Item<'_> {
    /// What a refusal names: `DDEAUTO fields`.
    fn what(&self) -> String {
        match self {
            Item::Field(_, name) => format!("{name} fields"),
            Item::Markup(_) => "Fields in markup kept as read".to_string(),
        }
    }

    /// The item as a key that outlives the document: its kind, then its code or markup.
    fn key(&self) -> String {
        match self {
            Item::Field(code, _) => format!("field\u{0}{code}"),
            Item::Markup(xml) => format!("markup\u{0}{xml}"),
        }
    }
}

/// Refuse `instr` when it is or holds an active field and this thread runs a command for
/// automation, unless the host allows them.
pub fn check(instr: &str) -> Result<(), CmdError> {
    match active_field(instr).filter(|_| enforced()) {
        Some(name) => Err(refused(&format!("{name} fields"), "be inserted")),
        None => Ok(()),
    }
}

/// Refuse content coming into `target` from automation (`incoming`: a paste, another file's text,
/// a building block, a compared or combined document) when it holds an active field, or field
/// markup kept as read, that `target` doesn't already hold, unless the host allows them.
pub fn check_incoming<'a>(incoming: impl IntoIterator<Item = &'a Block>, target: &Document) -> Result<(), CmdError> {
    if !enforced() {
        return Ok(());
    }
    let active = items(incoming);
    if active.is_empty() {
        return Ok(());
    }
    // Not its building blocks: Word acts on none of their fields until one is inserted.
    let held: HashSet<Item<'_>> = items(content(target)).into_iter().collect();
    match active.into_iter().find(|i| !held.contains(i)) {
        Some(item) => Err(refused(&item.what(), "come into a document with pasted or imported content")),
        None => Ok(()),
    }
}

/// Refuse a document whose bytes automation supplied (`file.open` with `data`) when it holds an
/// active field, or field markup that WordCraft keeps as read and writes back unchanged ([`items`]:
/// an element it doesn't model, a form field's definition, an embedded object's markup), unless
/// the host allows them. When any markup kept beside the model holds a field, all of it is
/// dropped: kept markup is written back as it was while its paragraph is unchanged, so a field the
/// reader didn't make an object of (nested too deep, inside an element it skips) would go out
/// unchecked. The document is then written from what was read.
pub fn check_supplied(doc: &mut Document) -> Result<(), CmdError> {
    if !enforced() {
        return Ok(());
    }
    if let Some(item) = items(stories(doc)).first() {
        return Err(refused(&item.what(), "be opened from `data`"));
    }
    if doc.kept.get().is_some_and(kept_fields) {
        doc.kept.clear();
    }
    Ok(())
}

/// What each open document held that Word acts on ([`items`], by [`Item::key`]) when a command
/// run for automation started, by document id. See [`after_run`].
pub(crate) struct Before(HashMap<u64, HashSet<String>>);

/// [`Before`] for the open `docs` (id and document) when this thread runs a command for
/// automation and the host doesn't allow active fields; `None`, without looking, otherwise.
pub(crate) fn before_run<'a>(docs: impl Iterator<Item = (u64, &'a Document)>) -> Option<Before> {
    enforced().then(|| Before(docs.map(|(id, doc)| (id, items(content(doc)).iter().map(Item::key).collect())).collect()))
}

/// Refuse command `id`, run for automation with `params` from the document `home`, when it left
/// one of the open `docs` holding something Word acts on ([`items`]) that the document didn't
/// hold before the command (for a document the command opened: that `home` didn't hold), unless
/// the command brings them in by design ([`brings_them_in`]). `Session::run` then undoes it.
pub(crate) fn after_run<'a>(
    before: &Before,
    id: &str,
    params: &Value,
    home: u64,
    docs: impl Iterator<Item = (u64, &'a Document)>,
) -> Result<(), CmdError> {
    if brings_them_in(id, params) {
        return Ok(());
    }
    let none = HashSet::new();
    for (doc_id, doc) in docs {
        let held = before.0.get(&doc_id).or_else(|| before.0.get(&home)).unwrap_or(&none);
        if let Some(item) = items(content(doc)).into_iter().find(|i| !held.contains(&i.key())) {
            return Err(refused(&item.what(), &format!("be added to a document by `{id}`")));
        }
    }
    Ok(())
}

/// The commands automation may run that bring in what Word acts on, given their `params`:
/// opening a file by its path, which already holds it (`file.open`, `file.newFromTemplate`, and
/// the `original` that `review.compare` and `review.combine` open; the document compared with it
/// is still checked by [`check_incoming`]), and Undo and Redo, which put back what the document
/// held.
fn brings_them_in(id: &str, params: &Value) -> bool {
    match id {
        "file.open" | "file.newFromTemplate" => crate::p::str(params, "data").is_none(),
        "review.compare" | "review.combine" => crate::p::str(params, "original").is_some(),
        "edit.undo" | "edit.redo" | "edit.undoTo" => true,
        _ => false,
    }
}

/// Whether any markup kept as read has a field, by its field markup in any case and with any
/// namespace prefix ([`field_markup`]). Every piece is looked at: paragraphs, tables, indexes, the
/// wrappers kept around them and the definitions they refer to, sealed or not.
fn kept_fields(k: &KeptXml) -> bool {
    let wraps = k.wraps.values().chain(k.pending_wraps.values()).chain(k.twin_wraps.values()).chain(k.table_wraps.values());
    let wraps = wraps.chain(k.pending_table_wraps.iter().map(|(_, w)| w)).flat_map(|(tags, _)| tags.iter());
    let pieces = k.paras.values().chain(k.tables.values()).chain(k.pending.values()).chain(k.defs.values());
    let pieces = pieces.chain(k.pending_tables.iter().map(|(_, xml)| xml)).chain(wraps);
    let mut pieces = pieces.chain(k.indexes.values().map(|(_, _, xml)| xml)).chain(k.pending_indexes.values().map(|(_, xml)| xml));
    pieces.any(|xml| field_markup(xml))
}

/// Whether `xml` holds field markup: Word's `instrText` element or `instr` attribute (an
/// `instr` that isn't part of a longer name, followed by `Text` or by `=`), or OpenDocument's
/// `dde-connection` (and its declarations) or `dde-source` (a section linked by DDE), in any
/// case. Not text that merely contains those letters ("instructions", "hidden", "middle").
fn field_markup(xml: &str) -> bool {
    let bytes = xml.as_bytes();
    let instr = positions(bytes, b"instr").any(|i| {
        let before = i.checked_sub(1).and_then(|b| bytes.get(b)).is_none_or(|c| !c.is_ascii_alphanumeric() && !matches!(c, b'_' | b'-' | b'.'));
        let after = bytes.get(i + b"instr".len()..).unwrap_or_default();
        before && (after.get(..4).is_some_and(|t| t.eq_ignore_ascii_case(b"text")) || after.trim_ascii_start().first() == Some(&b'='))
    });
    instr || positions(bytes, b"dde-connection").next().is_some() || positions(bytes, b"dde-source").next().is_some()
}

/// Where `needle` (lower-case ASCII) starts in `hay`, in any case.
fn positions<'a>(hay: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    hay.windows(needle.len()).enumerate().filter(move |(_, w)| w.eq_ignore_ascii_case(needle)).map(|(i, _)| i)
}

/// The blocks of `doc`'s body and of its other stories: headers, footers, notes, comments, text
/// boxes and the note separators.
pub fn content(doc: &Document) -> impl Iterator<Item = &Block> {
    let separators = [&doc.footnote_separators, &doc.endnote_separators]
        .into_iter()
        .flat_map(|s| [&s.separator, &s.continuation_separator, &s.continuation_notice])
        .flatten()
        .flat_map(|blocks| blocks.iter());
    doc.body.iter().chain(doc.parts.values().flat_map(|p| p.blocks.iter())).chain(separators).map(|b| &**b)
}

/// The blocks of `frag` and of the text boxes it brings.
pub fn fragment_content(frag: &Fragment) -> impl Iterator<Item = &Block> {
    frag.blocks.iter().chain(frag.parts.values().flat_map(|p| p.blocks.iter().map(|b| &**b)))
}

/// [`content`] and `doc`'s building blocks.
fn stories(doc: &Document) -> impl Iterator<Item = &Block> {
    content(doc).chain(doc.glossary.iter().flat_map(|e| e.blocks.iter().map(|b| &**b)))
}

/// What in `blocks` (tables included) Word acts on, or may ([`Item`]), in order.
fn items<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> Vec<Item<'a>> {
    let mut found = Vec::new();
    for b in blocks {
        paragraphs(b, &mut |p| {
            for o in &p.objects {
                object_items(o, &mut found);
            }
        });
    }
    found
}

/// [`items`] of one inline object, a group's members included.
fn object_items<'a>(o: &'a InlineObject, found: &mut Vec<Item<'a>>) {
    if let InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } = o
        && let Some(name) = active_field(instr)
    {
        found.push(Item::Field(instr.trim(), name));
    }
    if let Some(xml) = raw_markup(o).filter(|xml| field_markup(xml)) {
        found.push(Item::Markup(xml));
    }
    if let InlineObject::Group { children, .. } = o {
        // The readers bound a group's nesting, and so does serde_json's recursion limit.
        for c in children {
            object_items(&c.obj, found);
        }
    }
}

/// The markup of `o` that the writers put out as it is rather than building it from the model:
/// an element WordCraft doesn't model (`Opaque`), a legacy form field's definition (written
/// inside its `w:fldChar`), and the markup of an OLE object, a signature line, a chart, a diagram
/// or ink read from a file.
fn raw_markup(o: &InlineObject) -> Option<&str> {
    match o {
        InlineObject::Opaque { xml, .. } => Some(xml),
        InlineObject::Field { form, .. } => Some(form),
        InlineObject::Image { ole: Some(e), .. } => Some(&e.xml),
        InlineObject::Graphic { graphic, .. } => graphic.source.as_deref().map(|e| e.xml.as_str()),
        _ => None,
    }
}

/// Every paragraph of `b`, in tables at any depth (a document read from a file has its depth
/// bounded by the reader).
fn paragraphs<'a>(b: &'a Block, f: &mut dyn FnMut(&'a Paragraph)) {
    match b {
        Block::Para(p) => f(p),
        Block::Table(t) => {
            for cell in t.rows.iter().flat_map(|r| r.cells.iter()) {
                for cb in &cell.blocks {
                    paragraphs(cb, f);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    /// A session with a new, empty document.
    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

    /// The codes of the fields in the body of `s`'s document, in order.
    fn codes(s: &Session) -> Vec<String> {
        crate::cmd::fields::story_fields(s, wordcraft_doc::StoryRef::Body).into_iter().map(|f| f.instr).collect()
    }

    #[test]
    fn active_fields_are_found_in_the_code_and_nested_fields() {
        assert_eq!(active_field(r#"DDEAUTO excel "C:\x.xls""#), Some("DDEAUTO"));
        assert_eq!(active_field(r#"includepicture "\\host\share\a.png" \d"#), Some("INCLUDEPICTURE"));
        assert_eq!(active_field(r#"IF { MERGEFIELD A } = "1" "{ INCLUDETEXT \\h\s\t.docx }" "no""#), Some("INCLUDETEXT"));
        assert_eq!(active_field("{LINK Excel.Sheet.8 x.xls}"), Some("LINK"));
        // A quote ends the type, as a word of a field code ends there (`FieldCode::parse`).
        assert_eq!(active_field(r#"DDEAUTO"cmd" "/c calc""#), Some("DDEAUTO"));
        assert_eq!(active_field(r#"INCLUDETEXT"\\h\s\t.docx""#), Some("INCLUDETEXT"));
        assert_eq!(active_field(r#"IF 1 = 1 "{INCLUDEPICTURE"\\h\s\x.png"}" """#), Some("INCLUDEPICTURE"));
        assert_eq!(active_field(r#""DDE" excel x"#), Some("DDE"));
        // So does the start of a nested field.
        assert_eq!(active_field("INCLUDETEXT{ FILENAME }"), Some("INCLUDETEXT"));
        assert_eq!(active_field(r#"includepicture{ QUOTE "\\h\s\x.png" } \d"#), Some("INCLUDEPICTURE"));
        // A data source and a referenced document are fetched when the fields update.
        assert_eq!(active_field(r#"DATABASE \d "\\h\s\x.mdb" \s "SELECT * FROM t""#), Some("DATABASE"));
        assert_eq!(active_field(r#"RD "\\h\s\x.docx" \f"#), Some("RD"));
        for passive in
            ["PAGE", r#"DATE \@ "d MMMM""#, "MERGEFIELD Linkage", r"REF Include_note \h", "DOCPROPERTY DDE", "REF RDate", r#"QUOTE "INCLUDE""#]
        {
            assert_eq!(active_field(passive), None, "{passive}");
        }
    }

    #[test]
    fn automation_cannot_insert_an_active_field() {
        // #3628: an agent steered by the document's text.
        let mut s = session();
        for instr in ACTIVE_FIELDS.iter().map(|f| format!(r#"{f} "\\attacker\share\x""#)) {
            let r = from_automation(|| s.run("insert.field", &json!({"instr": instr})));
            assert!(matches!(r, Err(CmdError::Disabled(_))), "{instr}: {r:?}");
        }
        let quoted = r#"INCLUDEPICTURE"\\attacker\s\x.png""#;
        assert!(matches!(from_automation(|| s.run("insert.field", &json!({"instr": quoted}))), Err(CmdError::Disabled(_))));
        let nested = r#"IF 1 = 1 "{ DDEAUTO cmd "/c calc" }" """#;
        assert!(matches!(from_automation(|| s.run("insert.field", &json!({"instr": nested}))), Err(CmdError::Disabled(_))));
        assert!(codes(&s).is_empty());
        // Passive fields still go in.
        from_automation(|| s.run("insert.field", &json!({"instr": "PAGE"}))).unwrap();
        assert_eq!(codes(&s), ["PAGE"]);
    }

    #[test]
    fn automation_cannot_rewrite_a_field_into_an_active_one() {
        let mut s = session();
        s.run("insert.field", &json!({"instr": "PAGE"})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        let r = from_automation(|| s.run("fields.setCode", &json!({"code": r#"INCLUDETEXT "\\host\s\t.docx""#})));
        assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
        assert_eq!(codes(&s), ["PAGE"]);
        // A merge rule's text can't smuggle one in either.
        let r = from_automation(|| s.run("mailings.rules", &json!({"rule": "IF", "field": "A", "value": "1", "then": "{ DDE cmd x }"})));
        assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    }

    #[test]
    fn the_person_at_the_keyboard_can_insert_them() {
        let mut s = session();
        s.run("insert.field", &json!({"instr": "INCLUDEPICTURE \"pic.png\""})).unwrap();
        // Outside the automation scope again after it.
        assert!(from_automation(|| s.run("insert.field", &json!({"instr": "LINK x"}))).is_err());
        s.run("insert.field", &json!({"instr": "LINK x"})).unwrap();
        assert_eq!(codes(&s), ["INCLUDEPICTURE \"pic.png\"", "LINK x"]);
    }

    const DDE: &str = r#"DDEAUTO cmd "/c calc""#;

    /// A session whose document has a DDEAUTO field the person at the keyboard inserted, and a
    /// copy of it on the clipboard.
    fn with_active_field_copied() -> Session {
        let mut s = session();
        s.run("text.insert", &json!({"text": "Report "})).unwrap();
        s.run("insert.field", &json!({"instr": DDE})).unwrap();
        s.run("select.all", &json!({})).unwrap();
        s.run("edit.copy", &json!({})).unwrap();
        s
    }

    /// Whether `r` is automation's refusal of the DDEAUTO field.
    fn refused(r: &Result<serde_json::Value, CmdError>) -> bool {
        matches!(r, Err(CmdError::Disabled(m)) if m.contains("DDEAUTO"))
    }

    /// A file holding `doc`, in a folder of its own for the test `name`.
    fn saved(name: &str, doc: &wordcraft_doc::Document) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wc-active-fields-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("other.docx");
        std::fs::write(&path, crate::io::save_bytes("other.docx", doc).unwrap()).unwrap();
        path
    }

    #[test]
    fn automation_cannot_paste_an_active_field_into_a_document_without_it() {
        // #3628 review: copied from one document, pasted into the user's other one.
        let mut s = with_active_field_copied();
        s.run("file.new", &json!({})).unwrap();
        s.run("text.insert", &json!({"text": "Mine"})).unwrap();
        let r = from_automation(|| s.run("edit.paste", &json!({})));
        assert!(refused(&r), "{r:?}");
        assert!(codes(&s).is_empty());
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Mine");
        // Content without one still pastes.
        s.run("select.all", &json!({})).unwrap();
        s.run("edit.copy", &json!({})).unwrap();
        from_automation(|| s.run("edit.paste", &json!({}))).unwrap();
    }

    #[test]
    fn automation_can_paste_a_field_the_document_already_has() {
        // Copying the document's own field elsewhere in it brings nothing new.
        let mut s = with_active_field_copied();
        s.sel = crate::Selection::caret(s.doc.end_of(wordcraft_doc::StoryRef::Body));
        from_automation(|| s.run("edit.paste", &json!({}))).unwrap();
        assert_eq!(codes(&s), [DDE, DDE]);
    }

    #[test]
    fn a_field_cut_by_automation_is_pasted_back_only_under_track_changes() {
        // Cut without Track Changes, the field has left the document: pasting it is bringing it in.
        let mut s = with_active_field_copied();
        from_automation(|| s.run("edit.cut", &json!({}))).unwrap();
        assert!(codes(&s).is_empty());
        let r = from_automation(|| s.run("edit.paste", &json!({})));
        assert!(refused(&r), "{r:?}");
        assert!(codes(&s).is_empty());
        // A tracked cut leaves it in the document, marked deleted, so the paste (a move) goes in.
        let mut s = with_active_field_copied();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        from_automation(|| s.run("edit.cut", &json!({}))).unwrap();
        s.sel = crate::Selection::caret(s.doc.end_of(wordcraft_doc::StoryRef::Body));
        from_automation(|| s.run("edit.paste", &json!({}))).unwrap();
        assert_eq!(codes(&s), [DDE, DDE]);
    }

    #[test]
    fn automation_cannot_bring_an_active_field_in_from_another_file() {
        let path = saved("text-from-file", &with_active_field_copied().doc);
        let path = path.to_string_lossy().to_string();
        let mut s = session();
        let r = from_automation(|| s.run("insert.textFromFile", &json!({"path": path})));
        assert!(refused(&r), "{r:?}");
        assert!(codes(&s).is_empty());
        // Nor by comparing or combining with it: its text would come in as insertions.
        for cmd in ["review.compare", "review.combine"] {
            let r = from_automation(|| s.run(cmd, &json!({"path": path})));
            assert!(refused(&r), "{cmd}: {r:?}");
            assert!(codes(&s).is_empty(), "{cmd}");
        }
        // The person at the keyboard still can.
        s.run("insert.textFromFile", &json!({"path": path})).unwrap();
        assert_eq!(codes(&s), [DDE]);
        let _ = std::fs::remove_dir_all(std::path::Path::new(&path).parent().unwrap());
    }

    #[test]
    fn automation_cannot_insert_a_building_block_holding_an_active_field() {
        // The document's own Quick Part: Word acts on none of its fields until one is inserted.
        let mut s = with_active_field_copied();
        s.run("insert.quickParts", &json!({"save": "Header", "in": "document"})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        assert!(codes(&s).is_empty());
        let r = from_automation(|| s.run("insert.quickParts", &json!({"insert": "Header", "in": "document"})));
        assert!(refused(&r), "{r:?}");
        assert!(codes(&s).is_empty());
    }

    #[test]
    fn automation_cannot_open_bytes_holding_an_active_field() {
        // #3628 review: a document written by the agent, opened from `data`, then saved for the
        // user to send.
        let bytes = crate::io::save_bytes("a.xml", &with_active_field_copied().doc).unwrap();
        let data = crate::cmd::insert::base64_encode(&bytes);
        let mut s = session();
        let before = s.document_id();
        let r = from_automation(|| s.run("file.open", &json!({"path": "a.xml", "data": data})));
        assert!(refused(&r), "{r:?}");
        assert_eq!(s.document_id(), before);
        let r = from_automation(|| s.run("file.newFromTemplate", &json!({"path": "a.dotx", "data": data})));
        assert!(refused(&r), "{r:?}");
        assert_eq!(s.document_id(), before);
        // A file it opens by its path keeps its fields: the file already holds them.
        let path = saved("open", &with_active_field_copied().doc);
        from_automation(|| s.run("file.open", &json!({"path": path.to_string_lossy()}))).unwrap();
        assert_eq!(codes(&s), [DDE]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_field_kept_only_as_read_from_supplied_bytes_is_not_written_back() {
        // Markup kept as read is written back as it was while its paragraph is unchanged, so a
        // field in it that the reader made no field object of would go out unchecked. Since the
        // readers moved to craft-libs no reader in this repository fills `Document::kept`, so
        // this is checked on the document as `check_supplied` gets it from a reader.
        let with_kept = || {
            let mut doc = Document::from_text("Hello");
            let xml = r#"<text:p>Hello<text:dde-connection text:connection-name="Calc">x</text:dde-connection></text:p>"#;
            doc.kept = wordcraft_doc::kept::Kept::new(KeptXml { format: "odt".into(), paras: [(1, xml.into())].into(), ..Default::default() });
            doc
        };
        // The person at the keyboard keeps it…
        let mut doc = with_kept();
        check_supplied(&mut doc).unwrap();
        assert!(doc.kept.get().is_some());
        // …but bytes automation supplied lose it, and are written from what was read.
        let mut doc = with_kept();
        from_automation(|| check_supplied(&mut doc)).unwrap();
        assert!(doc.kept.get().is_none());
        assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello");
        // Kept markup without field markup stays.
        let mut doc = Document::from_text("Hello");
        doc.kept = wordcraft_doc::kept::Kept::new(KeptXml { paras: [(1, "<text:p>Hello</text:p>".into())].into(), ..Default::default() });
        from_automation(|| check_supplied(&mut doc)).unwrap();
        assert!(doc.kept.get().is_some());
    }

    #[test]
    fn field_markup_anywhere_in_the_kept_markup_counts() {
        // Not only a paragraph's or a table's markup is written back as read: so are the wrappers
        // kept around them and the definitions they refer to.
        let xml = |x: &str| -> std::sync::Arc<str> { x.into() };
        let field = r#"<w:fldSimple w:instr="INCLUDETEXT x"/>"#;
        let kept = [
            KeptXml { paras: [(1, xml(field))].into(), ..Default::default() },
            KeptXml { wraps: [(1, (vec![xml(field)], 1))].into(), ..Default::default() },
            KeptXml { twin_wraps: [((1, 2), (vec![xml(field)], 1))].into(), ..Default::default() },
            KeptXml { table_wraps: [(1, (vec![xml(field)], 1))].into(), ..Default::default() },
            KeptXml { pending_tables: vec![(0, xml(field))], ..Default::default() },
            KeptXml { pending_indexes: [(1, (2, xml(field)))].into(), ..Default::default() },
            KeptXml { defs: [("auto:P1".to_string(), xml("<text:dde-connection-decl/>"))].into(), ..Default::default() },
        ];
        for (i, k) in kept.iter().enumerate() {
            assert!(kept_fields(k), "{i}");
        }
        assert!(!kept_fields(&KeptXml { paras: [(1, xml("<w:p><w:r><w:t>Hi</w:t></w:r></w:p>"))].into(), ..Default::default() }));
        // Field markup with any prefix, or none, and spaces round `=`…
        for markup in [r#"<fldSimple instr = "LINK x"/>"#, "<W:INSTRTEXT>DDE x</W:INSTRTEXT>", r#"<text:section office:dde-source="x"/>"#] {
            assert!(field_markup(markup), "{markup}");
        }
        // …but not text or names that merely hold its letters.
        let plain = r#"<text:p x:xinstr="1" style:vertical-align="middle" text:display="hidden">Added instructions<w:instrument/></text:p>"#;
        assert!(!field_markup(plain));
    }

    /// The engine's test binary run again in a child process with only
    /// [`host_opt_in_in_a_child_process`] and [`ALLOW_ENV`] set to `value`: the variable belongs to
    /// the process, so setting it here would race the other tests. Panics unless the child ran
    /// that one test and it passed.
    #[cfg(not(target_arch = "wasm32"))]
    fn run_child_with_opt_in(value: &str) {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["active_fields::tests::host_opt_in_in_a_child_process", "--exact", "--ignored", "--test-threads=1"])
            .env(ALLOW_ENV, value)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{ALLOW_ENV}={value}: {stdout}{}", String::from_utf8_lossy(&out.stderr));
        assert!(stdout.contains("1 passed"), "{ALLOW_ENV}={value}: the child ran no test: {stdout}");
    }

    /// The host's opt-in: with `WORDCRAFT_ALLOW_ACTIVE_FIELDS=1` automation can insert an active
    /// field, paste one in and open bytes holding one; any other value still refuses all three.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_host_can_allow_them_for_automation() {
        for value in ["1", "0", "true"] {
            run_child_with_opt_in(value);
        }
    }

    /// Run only by [`the_host_can_allow_them_for_automation`], in a child process whose
    /// environment sets [`ALLOW_ENV`]; it checks nothing without that variable.
    #[test]
    #[ignore = "run by the_host_can_allow_them_for_automation with WORDCRAFT_ALLOW_ACTIVE_FIELDS set"]
    fn host_opt_in_in_a_child_process() {
        let Some(value) = std::env::var_os(ALLOW_ENV) else { return };
        let allowed = value == "1";
        let expect = |r: &Result<serde_json::Value, CmdError>, what: &str| {
            assert_eq!(r.is_ok(), allowed, "{what} with {ALLOW_ENV}={value:?}: {r:?}");
            if !allowed {
                assert!(refused(r), "{what}: {r:?}");
            }
        };

        let mut s = session();
        expect(&from_automation(|| s.run("insert.field", &json!({"instr": DDE}))), "insert.field");
        assert_eq!(codes(&s).len(), usize::from(allowed));

        let mut s = with_active_field_copied();
        s.run("file.new", &json!({})).unwrap();
        expect(&from_automation(|| s.run("edit.paste", &json!({}))), "edit.paste");
        assert_eq!(codes(&s).len(), usize::from(allowed));

        let bytes = crate::io::save_bytes("a.xml", &with_active_field_copied().doc).unwrap();
        let data = crate::cmd::insert::base64_encode(&bytes);
        let mut s = session();
        expect(&from_automation(|| s.run("file.open", &json!({"path": "a.xml", "data": data}))), "file.open with data");
        assert_eq!(codes(&s).len(), usize::from(allowed));
    }

    /// A document holding `obj` after the word "Raw", in its body.
    fn holding(obj: InlineObject) -> Document {
        let mut doc = Document::from_text("Raw");
        let end = doc.end_of(wordcraft_doc::StoryRef::Body);
        doc.insert_object(&end, obj, &Default::default()).unwrap();
        doc
    }

    /// A legacy form field whose definition (`form`, written inside its `w:fldChar` as it is)
    /// closes that `w:fldChar` and adds a field of its own: `DDEAUTO`.
    fn form_field_smuggling_dde() -> InlineObject {
        let form = r#"<w:ffData/></w:fldChar></w:r><w:r><w:instrText xml:space="preserve"> DDEAUTO cmd "/c calc" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:fldChar w:fldCharType="begin">"#;
        InlineObject::Field { instr: "FORMTEXT".into(), result: String::new(), locked: false, dirty: false, code: false, form: form.into() }
    }

    /// An element WordCraft doesn't model, kept as read: a `w:fldSimple` running `DDEAUTO`.
    fn opaque_dde() -> InlineObject {
        InlineObject::Opaque { format: "docx".into(), xml: r#"<w:fldSimple w:instr="DDEAUTO cmd /c calc"/>"#.into(), text: String::new() }
    }

    /// The fields in `doc` saved as DOCX that Word acts on, read from the package as Word reads
    /// them (each `instrText` and each `instr` attribute in every XML part), as `part kind: code`.
    /// The glossary's fields are told apart from the rest: Word acts on none of them until one is
    /// inserted.
    fn active_in_docx(doc: &Document) -> std::collections::BTreeSet<String> {
        let bytes = crate::io::save_bytes("out.docx", doc).unwrap();
        let mut zip = craft_zip::Archive::open(&bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 256 << 20 }).unwrap();
        let names: Vec<String> = zip.names().filter(|n| n.ends_with(".xml")).map(|n| n.into_owned()).collect();
        let mut found = std::collections::BTreeSet::new();
        for name in names {
            let xml = String::from_utf8_lossy(&zip.read(&name, 64 << 20).unwrap().unwrap()).replace("&quot;", "\"").replace("&amp;", "&");
            let kind = if name.starts_with("word/glossary") { "glossary" } else { "document" };
            // `<w:instrText …>code</w:instrText>`: the text after each tag that names it.
            let texts = xml.split("instrText").skip(1).filter_map(|p| p.split_once('>')?.1.split('<').next());
            // `w:instr="code"` (`w:fldSimple`).
            let attrs = xml.split("instr=\"").skip(1).filter_map(|p| p.split('"').next());
            for code in texts.chain(attrs) {
                if active_field(code).is_some() {
                    found.insert(format!("{kind}: {}", code.trim()));
                }
            }
        }
        found
    }

    #[test]
    fn markup_kept_as_read_that_holds_a_field_is_checked() {
        // #3628 review: an element WordCraft doesn't model and a form field's definition are
        // written out as they are, so a field inside one reaches the DOCX without being a field
        // object. Opened by the person at the keyboard, both do reach it…
        for (what, obj) in [("an unmodelled element", opaque_dde()), ("a form field's definition", form_field_smuggling_dde())] {
            let data = crate::cmd::insert::base64_encode(&crate::io::save_bytes("x.json", &holding(obj)).unwrap());
            let mut s = session();
            s.run("file.open", &json!({"path": "x.json", "data": data})).unwrap();
            let out = active_in_docx(&s.doc);
            assert!(out.iter().any(|f| f.starts_with("document: DDEAUTO")), "{what}: {out:?}");
            // …but automation can't open them from `data`, nor bring them in from a file.
            let mut s = session();
            let before = s.document_id();
            let r = from_automation(|| s.run("file.open", &json!({"path": "x.json", "data": data})));
            assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("markup kept as read")), "{what}: {r:?}");
            assert_eq!(s.document_id(), before, "{what}");
            let dir = std::env::temp_dir().join(format!("wc-active-fields-markup-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("x.json");
            std::fs::write(&path, crate::cmd::insert::base64_decode(&data).unwrap()).unwrap();
            let r = from_automation(|| s.run("insert.textFromFile", &json!({"path": path.to_string_lossy()})));
            assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("markup kept as read")), "{what}: {r:?}");
            assert!(active_in_docx(&s.doc).is_empty(), "{what}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn every_piece_of_markup_written_as_it_is_is_looked_at() {
        let embedded = |xml: &str| std::sync::Arc::new(wordcraft_doc::graphic::Embedded { xml: xml.into(), refs: vec![] });
        let field = r#"<w:r><w:instrText>INCLUDEPICTURE "\\h\s\x.png"</w:instrText></w:r>"#;
        let ole = InlineObject::Image {
            media: String::new(),
            w: 1.0,
            h: 1.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: Some(embedded(field)),
        };
        let graphic = wordcraft_doc::graphic::Graphic { source: Some(embedded(field)), ..Default::default() };
        let chart = InlineObject::Graphic { w: 1.0, h: 1.0, alt: String::new(), float: Default::default(), graphic: std::sync::Arc::new(graphic) };
        let member = |obj| wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj };
        let group =
            |obj| InlineObject::Group { w: 1.0, h: 1.0, float: Default::default(), ch_w: 1.0, ch_h: 1.0, children: vec![member(obj)], canvas: None };
        for (what, obj) in [
            ("an unmodelled element", opaque_dde()),
            ("a form field's definition", form_field_smuggling_dde()),
            ("an OLE object's markup", ole),
            ("a chart's markup", chart),
            ("a group's member", group(group(opaque_dde()))),
        ] {
            let doc = holding(obj);
            assert!(matches!(items(content(&doc)).as_slice(), [Item::Markup(_)]), "{what}");
        }
        // A form field's definition without a field in it is no item.
        let form = InlineObject::Field {
            instr: "FORMCHECKBOX".into(),
            result: String::new(),
            locked: false,
            dirty: false,
            code: false,
            form: "<w:ffData><w:checkBox/></w:ffData>".into(),
        };
        assert!(items(content(&holding(form))).is_empty());
    }

    #[test]
    fn a_command_that_adds_one_from_automation_is_refused_and_undone() {
        // #3628 review: the checks at the commands aren't the guarantee. `Session::run` refuses any
        // command run for automation that leaves an open document holding a field Word acts on that
        // it didn't, however the command put it there.
        let sneak = crate::CommandSpec::new("test.sneak", "Sneak", "", |s, _| {
            let end = s.doc.end_of(wordcraft_doc::StoryRef::Body);
            s.doc.insert_object(
                &end,
                InlineObject::Field { instr: DDE.into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
                &Default::default(),
            )?;
            Ok(json!({}))
        });
        let opens = crate::CommandSpec::new("test.opens", "Opens", "", |s, _| {
            s.add_document(holding(opaque_dde()));
            Ok(json!({}))
        });
        let mut s = Session::new(Document::from_text("Mine"));
        s.registry = std::sync::Arc::new(crate::Registry::new(vec![sneak, opens]));
        let home = s.document_id();
        let r = from_automation(|| s.run("test.sneak", &json!({})));
        assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("DDEAUTO fields") && m.contains("`test.sneak`")), "{r:?}");
        assert!(codes(&s).is_empty(), "the field is taken out again");
        assert!(!s.can_undo(), "and leaves no undo step");
        // A document the command opens holding one is closed again.
        let r = from_automation(|| s.run("test.opens", &json!({})));
        assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("markup kept as read")), "{r:?}");
        assert_eq!((s.document_id(), s.documents().len()), (home, 1));
        // The person at the keyboard can do both.
        s.run("test.sneak", &json!({})).unwrap();
        assert_eq!(codes(&s), [DDE]);
        s.run("test.opens", &json!({})).unwrap();
        assert_eq!(s.documents().len(), 2);
    }

    /// Every command, run for automation (as MCP runs it: confined to its folders) with params
    /// built from its parameter description, against a session that holds `DDEAUTO` fields
    /// everywhere content can come from: the clipboard, the Clipboard pane, the Spike, a Quick
    /// Part, the document's own building blocks, and files in each format (one holding the field
    /// in markup kept as read). The document saved as DOCX afterwards holds no field Word acts on
    /// that it didn't before. A new command or import path that brings one in fails here.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn no_command_from_automation_adds_a_field_word_acts_on() {
        // The sources: the clipboard, the Clipboard pane, a Quick Part, a building block of the
        // document's own, and the Spike, filled by the person at the keyboard.
        let mut src = with_active_field_copied();
        src.run("insert.quickParts", &json!({"save": "Evil"})).unwrap();
        src.run("insert.quickParts", &json!({"save": "Evil", "in": "document"})).unwrap();
        let glossary = src.doc.glossary.clone();
        let (clipboard, clip_history) = (src.clipboard.clone(), src.clip_history.clone());
        src.run("edit.spike", &json!({})).unwrap();
        let blocks = src.building_blocks.clone();
        let mine = || {
            let mut doc = Document::from_text("Mine");
            doc.glossary = glossary.clone();
            let mut s = Session::new(doc);
            s.clipboard = clipboard.clone();
            s.clip_history = clip_history.clone();
            s.building_blocks = blocks.clone();
            s
        };
        assert!(mine().building_blocks.len() >= 2 && mine().clipboard.is_some());
        let before = active_in_docx(&mine().doc);
        assert_eq!(before.iter().map(String::as_str).collect::<Vec<_>>(), [format!("glossary: {DDE}")], "only its building block");

        // The files, in a folder automation may read, and one it may write.
        let base = std::env::temp_dir().join(format!("wc-active-fields-every-command-{}", std::process::id()));
        let (input, output) = (base.join("in"), base.join("out"));
        for dir in [&input, &output] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let evil = with_active_field_copied().doc;
        let mut files = vec![];
        let sources = [("evil.docx", &evil), ("evil.fodt", &evil), ("evil.rtf", &evil), ("evil.html", &evil), ("evil.json", &evil)];
        let raw = [("opaque.json", holding(opaque_dde())), ("form.json", holding(form_field_smuggling_dde()))];
        for (name, doc) in sources.into_iter().chain(raw.iter().map(|(n, d)| (*n, d))) {
            let bytes = crate::io::save_bytes(name, doc).unwrap();
            std::fs::write(input.join(name), &bytes).unwrap();
            files.push((input.join(name).to_string_lossy().into_owned(), crate::cmd::insert::base64_encode(&bytes)));
        }
        let roots = crate::file_access::AutomationRoots::new(Some(&input), Some(&output)).unwrap();

        // Params: every name the command's description gives, and the usual ones, each set to
        // the field's code; or to a whole document holding it; or, for a command that reads a
        // file, to each file's path, with and without its bytes as `data`.
        const PATHS: &[&str] = &["path", "original", "revised", "file", "template", "source", "src"];
        const USUAL: &[&str] = &["text", "instr", "code", "value", "name", "insert", "then", "else", "html", "rtf", "markdown", "document"];
        let as_json = serde_json::to_value(&evil).unwrap();
        // Only these may bring a field in: they open a file by its path, which holds it already.
        let may = |id: &str, v: &serde_json::Value| match id {
            "file.open" | "file.newFromTemplate" => v.get("data").is_none(),
            "review.compare" | "review.combine" => v.get("original").is_some(),
            _ => false,
        };
        let mut added = vec![];
        let reg = crate::cmd::registry();
        for spec in reg.all() {
            let names: Vec<&str> = spec.params.split('"').skip(1).step_by(2).chain(USUAL.iter().copied()).collect();
            let all = |value: &dyn Fn(&str) -> Option<serde_json::Value>| {
                serde_json::Value::Object(names.iter().filter_map(|n| Some((n.to_string(), value(n)?))).collect())
            };
            let mut params = vec![json!({}), all(&|_| Some(json!(DDE))), all(&|_| Some(as_json.clone()))];
            if names.iter().any(|n| PATHS.contains(n) || *n == "data") || spec.id.starts_with("file.") {
                for (path, data) in &files {
                    for with_data in [false, true] {
                        params.push(all(&|n| match n {
                            n if PATHS.contains(&n) => Some(json!(path)),
                            "data" => with_data.then(|| json!(data)),
                            _ => Some(json!(DDE)),
                        }));
                        if with_data {
                            params.last_mut().unwrap()["data"] = json!(data);
                        }
                    }
                }
            }
            for v in &params {
                let mut s = mine();
                let r = crate::file_access::confine(roots.as_ref(), || from_automation(|| s.run(spec.id, v)));
                let after = active_in_docx(&s.doc);
                let new: Vec<&String> = after.difference(&before).collect();
                if !new.is_empty() && !may(spec.id, v) {
                    added.push(format!("{} {v}: {new:?} ({r:?})", spec.id));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&base);
        assert!(added.is_empty(), "{} commands added fields Word acts on:\n{}", added.len(), added.join("\n"));
    }
}
