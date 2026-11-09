//! Field types another application acts on when it opens the document or updates its fields
//! (#3628): DDE and DDEAUTO start a program; INCLUDETEXT, INCLUDEPICTURE, LINK, IMPORT, INCLUDE,
//! DATABASE (its data source) and RD (the document a table of contents or index takes entries
//! from) fetch their target, over the network for a UNC path. WordCraft never evaluates them, but
//! the people a document goes to open it in Word.
//!
//! An agent can be steered by text in the very document it edits, so a command run for
//! automation (MCP, the control channel: inside [`from_automation`]) can't put one in a document:
//!
//! - not by its code ([`check`]: `insert.field`, `fields.setCode`, mail merge fields);
//! - not with content it brings in from elsewhere ([`check_incoming`]: a paste, another file's
//!   text, a building block, a compared or combined document), unless the document already has
//!   a field with that very code, so copying the document's own fields still works (one cut
//!   without Track Changes has left the document, so pasting it back from automation doesn't);
//! - not in a document whose bytes it supplies ([`check_supplied`]: `file.open` and
//!   `file.newFromTemplate` with `data`). A document it opens from a file keeps its fields: the
//!   file already holds them.
//!
//! The person at the keyboard still can, and so can automation on a computer whose host sets
//! `WORDCRAFT_ALLOW_ACTIVE_FIELDS=1`: a setting of the host, never a command parameter, so a
//! prompt can't turn it on.

use std::cell::Cell;
use std::collections::HashSet;

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

/// The error for an active field `name` that can't `how` from automation.
fn refused(name: &str, how: &str) -> CmdError {
    CmdError::Disabled(format!("{name} fields can't {how} from automation (the host can allow them with {ALLOW_ENV}=1)"))
}

/// Refuse `instr` when it is or holds an active field and this thread runs a command for
/// automation, unless the host allows them.
pub fn check(instr: &str) -> Result<(), CmdError> {
    match active_field(instr).filter(|_| enforced()) {
        Some(name) => Err(refused(name, "be inserted")),
        None => Ok(()),
    }
}

/// Refuse content coming into `target` from automation (`incoming`: a paste, another file's text,
/// a building block, a compared or combined document) when it holds an active field whose code
/// `target` doesn't already hold, unless the host allows them.
pub fn check_incoming<'a>(incoming: impl IntoIterator<Item = &'a Block>, target: &Document) -> Result<(), CmdError> {
    if !enforced() {
        return Ok(());
    }
    let active: Vec<(&str, &'static str)> = field_codes(incoming).into_iter().filter_map(|c| Some((c.trim(), active_field(c)?))).collect();
    if active.is_empty() {
        return Ok(());
    }
    // Not its building blocks: Word acts on none of their fields until one is inserted.
    let held: HashSet<&str> = field_codes(content(target)).into_iter().map(str::trim).collect();
    match active.into_iter().find(|(code, _)| !held.contains(code)) {
        Some((_, name)) => Err(refused(name, "come into a document with pasted or imported content")),
        None => Ok(()),
    }
}

/// Refuse a document whose bytes automation supplied (`file.open` with `data`) when it holds an
/// active field, unless the host allows them. When any markup kept as read holds a field, all of
/// it is dropped: kept markup is written back as it was while its paragraph is unchanged, so a
/// field the reader didn't make an object of (nested too deep, inside an element it skips) would
/// go out unchecked. The document is then written from what was read.
pub fn check_supplied(doc: &mut Document) -> Result<(), CmdError> {
    if !enforced() {
        return Ok(());
    }
    if let Some(name) = field_codes(stories(doc)).into_iter().find_map(active_field) {
        return Err(refused(name, "be opened from `data`"));
    }
    if doc.kept.get().is_some_and(kept_fields) {
        doc.kept.clear();
    }
    Ok(())
}

/// Whether any markup kept as read has a field, by its field markup: `instr` (Word's
/// `w:instrText` and `w:fldSimple w:instr`) or `dde` (OpenDocument's `text:dde-connection`), in
/// any case. Every piece is looked at: paragraphs, tables, indexes, the wrappers kept around them
/// and the definitions they refer to, sealed or not.
fn kept_fields(k: &KeptXml) -> bool {
    let fields = |xml: &str| {
        let xml = xml.to_ascii_lowercase();
        xml.contains("instr") || xml.contains("dde")
    };
    let wraps = k.wraps.values().chain(k.pending_wraps.values()).chain(k.twin_wraps.values()).chain(k.table_wraps.values());
    let wraps = wraps.chain(k.pending_table_wraps.iter().map(|(_, w)| w)).flat_map(|(tags, _)| tags.iter());
    let pieces = k.paras.values().chain(k.tables.values()).chain(k.pending.values()).chain(k.defs.values());
    let pieces = pieces.chain(k.pending_tables.iter().map(|(_, xml)| xml)).chain(wraps);
    let mut pieces = pieces.chain(k.indexes.values().map(|(_, _, xml)| xml)).chain(k.pending_indexes.values().map(|(_, xml)| xml));
    pieces.any(|xml| fields(xml))
}

/// The blocks of `doc`'s body and of its other stories: headers, footers, notes, comments and
/// text boxes.
pub fn content(doc: &Document) -> impl Iterator<Item = &Block> {
    doc.body.iter().chain(doc.parts.values().flat_map(|p| p.blocks.iter())).map(|b| &**b)
}

/// The blocks of `frag` and of the text boxes it brings.
pub fn fragment_content(frag: &Fragment) -> impl Iterator<Item = &Block> {
    frag.blocks.iter().chain(frag.parts.values().flat_map(|p| p.blocks.iter().map(|b| &**b)))
}

/// [`content`] and `doc`'s building blocks.
fn stories(doc: &Document) -> impl Iterator<Item = &Block> {
    content(doc).chain(doc.glossary.iter().flat_map(|e| e.blocks.iter().map(|b| &**b)))
}

/// The codes of the fields in `blocks`, tables included.
fn field_codes<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> Vec<&'a str> {
    let mut codes = Vec::new();
    for b in blocks {
        paragraphs(b, &mut |p| {
            for o in &p.objects {
                if let InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } = o {
                    codes.push(instr.as_str());
                }
            }
        });
    }
    codes
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

    fn session() -> Session {
        Session::new(wordcraft_doc::Document::new())
    }

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
        // The OpenDocument reader keeps a paragraph's markup, an element it has no place for
        // included, and the writer puts it back while the paragraph is unchanged: a field inside
        // that element is no field object, so it would go out unchecked.
        let mut s = session();
        s.run("text.insert", &json!({"text": "Hello"})).unwrap();
        let xml = String::from_utf8(crate::io::save_bytes("a.fodt", &s.doc).unwrap()).unwrap();
        let at = xml.find("Hello").unwrap_or_else(|| panic!("{xml}")) + "Hello".len();
        let hidden = r#"<x:wrapper xmlns:x="urn:example"><text:dde-connection text:connection-name="Calc">x</text:dde-connection></x:wrapper>"#;
        let data = crate::cmd::insert::base64_encode(format!("{}{hidden}{}", &xml[..at], &xml[at..]).as_bytes());
        let saved_again = |automation: bool| {
            let mut s = session();
            let open = |s: &mut Session| s.run("file.open", &json!({"path": "a.fodt", "data": data}));
            if automation { from_automation(|| open(&mut s)) } else { open(&mut s) }.unwrap();
            let text = s.doc.plain_text(wordcraft_doc::StoryRef::Body);
            assert!(text.starts_with("Hello"), "{text:?}");
            String::from_utf8_lossy(&crate::io::save_bytes("a.fodt", &s.doc).unwrap()).to_string()
        };
        // Opened by the person at the keyboard, the markup is written back as read…
        let out = saved_again(false);
        assert!(out.contains("dde-connection"), "{out}");
        // …but not when automation supplied the bytes.
        let out = saved_again(true);
        assert!(out.contains("Hello") && !out.contains("dde-connection"), "{out}");
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
    }
}
