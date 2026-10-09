//! Answering Zotero's calls against a [`Session`]. One integration command is one transaction:
//! it starts with `Application_getActiveDocument`, ends with `Document_complete`, and all its
//! changes are a single undo step ("Zotero").
//!
//! Field ids handed to Zotero stay valid for the transaction: they are kept in a list parallel
//! to the document-order list of Zotero fields, updated as fields are inserted and removed.

use serde_json::{Value, json};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::{LineSpacing, TabAlign, TabLeader, TabStop};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{CharProps, InlineObject, ParaProps, PartKind, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};

use crate::fields::{self, ZField};
use crate::rich;
use crate::wire::Call;

/// Highest protocol version we speak.
pub const PROTOCOL_VERSION: i64 = 3;
/// Paragraph style for bibliography entries (the name Word's Zotero plugin uses).
pub const BIB_STYLE: &str = "Bibliography";
/// Where Zotero's placeholder links point.
const PLACEHOLDER_URL: &str = "https://www.zotero.org/?";

// Alert buttons (Zotero's constants) and the affirmative answer for each.
const BUTTONS_OK: i64 = 0;
const BUTTONS_OK_CANCEL: i64 = 1;
const BUTTONS_YES_NO: i64 = 2;
const BUTTONS_YES_NO_CANCEL: i64 = 3;

/// What the bridge needs from the app: dialogs and focus.
pub trait Host {
    /// Show Zotero's message. Returns the button pressed, as Zotero numbers them: OK → 1;
    /// OK/Cancel → 1/0; Yes/No → 1/0; Yes/No/Cancel → 2/1/0.
    fn alert(&mut self, text: &str, icon: i64, buttons: i64) -> i64;
    /// Bring the document window to the front.
    fn activate(&mut self) {}
}

/// No UI: alerts are logged and answered affirmatively (OK / Yes).
#[derive(Clone, Debug, Default)]
pub struct Headless {
    /// Alerts shown so far.
    pub alerts: Vec<String>,
}

impl Host for Headless {
    fn alert(&mut self, text: &str, _icon: i64, buttons: i64) -> i64 {
        log::info!("Zotero: {text}");
        self.alerts.push(text.to_string());
        match buttons {
            BUTTONS_YES_NO_CANCEL => 2,
            BUTTONS_OK | BUTTONS_OK_CANCEL | BUTTONS_YES_NO => 1,
            _ => 1,
        }
    }
}

/// The document side of Zotero's protocol.
#[derive(Debug, Default)]
pub struct Bridge {
    doc_id: i64,
    ids: Vec<u64>,
    next_id: u64,
    /// The undo checkpoint for this transaction has been taken.
    checkpointed: bool,
    /// Field (by id) the caret should end up after: the one just inserted.
    caret_after: Option<u64>,
    /// The selection when the transaction began, restored at the end when Zotero only moved
    /// it to show a citation (`Field_select`).
    start_sel: Option<Selection>,
    /// Zotero selected a field during this transaction.
    selected: bool,
    /// `Document_complete` arrived.
    pub completed: bool,
}

type R = Result<Value, String>;

fn arg(c: &Call, i: usize) -> Result<&Value, String> {
    c.args.get(i).ok_or_else(|| format!("{}: missing argument {}", c.method, i + 1))
}
fn arg_str(c: &Call, i: usize) -> Result<String, String> {
    match arg(c, i)? {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(String::new()),
        v => Ok(v.to_string()),
    }
}
fn arg_i64(c: &Call, i: usize) -> Result<i64, String> {
    match arg(c, i)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.clamp(-1e12, 1e12) as i64)).ok_or_else(|| format!("{}: bad number", c.method)),
        Value::String(s) => s.trim().parse().map_err(|_| format!("{}: argument {} is not a number", c.method, i + 1)),
        Value::Bool(b) => Ok(i64::from(*b)),
        _ => Err(format!("{}: argument {} is not a number", c.method, i + 1)),
    }
}
fn arg_bool(c: &Call, i: usize) -> Result<bool, String> {
    match arg(c, i)? {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => Ok(n.as_f64().is_some_and(|f| f != 0.0)),
        Value::String(s) => Ok(matches!(s.as_str(), "true" | "1")),
        _ => Ok(false),
    }
}
fn arg_id(c: &Call, i: usize) -> Result<u64, String> {
    let v = arg_i64(c, i)?;
    u64::try_from(v).map_err(|_| format!("{}: bad field id", c.method))
}

/// Twips from Zotero to points, kept sane.
fn twips(v: i64) -> f32 {
    (v.clamp(-31_680, 31_680) as f32) / 20.0
}

impl Bridge {
    pub fn new() -> Bridge {
        Bridge::default()
    }

    fn fresh_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1);
        self.next_id
    }

    /// The current fields, with ids in step with them.
    fn fields(&mut self, s: &Session) -> Vec<ZField> {
        let list = fields::list(&s.doc);
        if self.ids.len() != list.len() {
            // The document changed outside this bridge: number afresh.
            self.ids = (0..list.len()).map(|_| self.fresh_id()).collect();
        }
        list
    }

    fn field(&mut self, s: &Session, id: u64) -> Result<(usize, ZField), String> {
        let list = self.fields(s);
        let k = self.ids.iter().position(|x| *x == id).ok_or_else(|| format!("no field with id {id}"))?;
        list.into_iter().nth(k).map(|f| (k, f)).ok_or_else(|| format!("no field with id {id}"))
    }

    /// Register a field just inserted with its start marker at `start`; returns its id.
    fn adopt(&mut self, s: &Session, start: &Pos) -> Result<(u64, ZField), String> {
        let list = fields::list(&s.doc);
        let k = list.iter().position(|f| f.start == *start).ok_or("the new field was not found")?;
        let id = self.fresh_id();
        if self.ids.len() + 1 == list.len() {
            self.ids.insert(k.min(self.ids.len()), id);
        } else {
            self.ids = (0..list.len()).map(|_| self.fresh_id()).collect();
            if let Some(slot) = self.ids.get_mut(k) {
                *slot = id;
            }
        }
        let f = list.into_iter().nth(k).ok_or("the new field was not found")?;
        Ok((id, f))
    }

    fn forget(&mut self, k: usize) {
        if k < self.ids.len() {
            self.ids.remove(k);
        }
    }

    fn begin_edit(&mut self, s: &mut Session) {
        if !self.checkpointed {
            s.checkpoint("Zotero");
            self.checkpointed = true;
        }
    }

    fn end_edit(s: &mut Session) {
        s.doc.ensure_nonempty();
        s.touch();
        s.clamp_selection();
    }

    /// Answer one call. Errors become `ERR:` replies; Zotero shows them and cancels.
    pub fn handle(&mut self, s: &mut Session, host: &mut dyn Host, c: &Call) -> R {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.dispatch(s, host, c)));
        match r {
            Ok(r) => r,
            Err(_) => {
                s.clamp_selection();
                Err(format!("{}: internal error", c.method))
            }
        }
    }

    fn dispatch(&mut self, s: &mut Session, host: &mut dyn Host, c: &Call) -> R {
        match c.method.as_str() {
            "Application_getActiveDocument" => {
                let theirs = c.args.first().and_then(Value::as_i64).unwrap_or(PROTOCOL_VERSION);
                self.doc_id = self.doc_id.wrapping_add(1);
                self.checkpointed = false;
                self.caret_after = None;
                self.completed = false;
                self.start_sel = Some(s.sel.clone());
                self.selected = false;
                Ok(json!([theirs.clamp(1, PROTOCOL_VERSION), self.doc_id]))
            }
            "Document_displayAlert" => {
                let text = arg_str(c, 1)?;
                let icon = arg_i64(c, 2).unwrap_or(0);
                let buttons = arg_i64(c, 3).unwrap_or(0);
                Ok(json!(host.alert(&text, icon, buttons)))
            }
            "Document_activate" => {
                host.activate();
                Ok(Value::Null)
            }
            "Document_canInsertField" => Ok(json!(fields::can_insert_at(&s.doc, &s.sel.ordered().0))),
            "Document_getDocumentData" => Ok(json!(fields::get_prefs(&s.doc))),
            "Document_setDocumentData" => {
                let data = arg_str(c, 1)?;
                if data != fields::get_prefs(&s.doc) {
                    self.begin_edit(s);
                    fields::set_prefs(&mut s.doc, &data);
                    Self::end_edit(s);
                }
                Ok(Value::Null)
            }
            "Document_cursorInField" => {
                let at = s.sel.ordered().0;
                let list = self.fields(s);
                // The innermost field holding the caret: the last one starting before it.
                let hit = list.iter().enumerate().rfind(|(_, f)| f.contains(&at));
                Ok(match hit {
                    Some((k, f)) => json!([self.ids.get(k).copied().unwrap_or(0), f.code, f.note_index]),
                    None => Value::Null,
                })
            }
            "Document_insertField" => {
                let note = arg_i64(c, 2).unwrap_or(0);
                let (id, f) = self.insert_field(s, note, None)?;
                self.caret_after = Some(id);
                Ok(json!([id, f.code, f.note_index]))
            }
            "Document_insertText" => {
                let html = arg_str(c, 1)?;
                self.begin_edit(s);
                let at = delete_selection(s)?;
                let base = s.doc.para_at(&at).map(|p| p.props_at(at.off).clone()).unwrap_or_default();
                let frag = rich::fragment(&html, true, &base);
                let end = s.doc.insert_fragment(&at, &frag).map_err(|e| e.to_string())?;
                s.sel = Selection::caret(end);
                Self::end_edit(s);
                Ok(Value::Null)
            }
            "Document_getFields" => {
                let list = self.fields(s);
                let ids: Vec<u64> = self.ids.clone();
                let codes: Vec<&str> = list.iter().map(|f| f.code.as_str()).collect();
                let notes: Vec<u32> = list.iter().map(|f| f.note_index).collect();
                Ok(json!([ids, codes, notes]))
            }
            "Document_convertPlaceholdersToFields" => {
                let Value::Array(ph) = arg(c, 1)? else { return Err("placeholder ids must be an array".into()) };
                let note = arg_i64(c, 2).unwrap_or(0);
                self.convert_placeholders(s, ph, note)
            }
            "Document_setBibliographyStyle" => {
                self.begin_edit(s);
                set_bibliography_style(s, c)?;
                Self::end_edit(s);
                Ok(Value::Null)
            }
            // Zotero's field kinds are one kind in WordCraft; only the note type matters: a
            // citation moves between the text, footnotes and endnotes (switching styles).
            "Document_convert" => {
                let Value::Array(ids) = arg(c, 1)? else { return Err("field ids must be an array".into()) };
                let ids: Vec<Value> = ids.iter().take(100_000).cloned().collect();
                let notes = c.args.get(3).cloned().unwrap_or(Value::Null);
                for (i, idv) in ids.iter().enumerate() {
                    let id = match idv {
                        Value::Number(n) => n.as_u64(),
                        Value::String(x) => x.trim().parse().ok(),
                        _ => None,
                    }
                    .ok_or("bad field id")?;
                    let note = match &notes {
                        Value::Array(a) => a.get(i).and_then(Value::as_i64).unwrap_or(0),
                        v => v.as_i64().unwrap_or(0),
                    };
                    self.convert_field(s, id, note)?;
                }
                Ok(Value::Null)
            }
            "Field_convert" => {
                let id = arg_id(c, 1)?;
                let note = arg_i64(c, 3).unwrap_or(0);
                self.convert_field(s, id, note)?;
                Ok(Value::Null)
            }
            "Document_complete" => {
                let start_sel = self.start_sel.take();
                if let Some(id) = self.caret_after.take()
                    && let Ok((_, f)) = self.field(s, id)
                {
                    s.sel = Selection::caret(f.after());
                } else if std::mem::take(&mut self.selected)
                    && let Some(sel) = start_sel
                {
                    s.sel = sel;
                }
                s.clamp_selection();
                self.completed = true;
                Ok(Value::Null)
            }
            "Field_delete" => {
                let (k, f) = self.field(s, arg_id(c, 1)?)?;
                self.begin_edit(s);
                s.doc.delete_range(&f.start, &f.after()).map_err(|e| e.to_string())?;
                self.forget(k);
                let _ = remove_empty_note(s, &f);
                Self::end_edit(s);
                Ok(Value::Null)
            }
            "Field_select" => {
                let (_, f) = self.field(s, arg_id(c, 1)?)?;
                s.sel = Selection { anchor: f.start.clone(), focus: f.after() };
                s.clamp_selection();
                self.selected = true;
                Ok(Value::Null)
            }
            "Field_removeCode" => {
                let (k, f) = self.field(s, arg_id(c, 1)?)?;
                self.begin_edit(s);
                s.doc.delete_range(&f.end, &f.after()).map_err(|e| e.to_string())?;
                s.doc.delete_range(&f.start, &f.content_start()).map_err(|e| e.to_string())?;
                self.forget(k);
                Self::end_edit(s);
                Ok(Value::Null)
            }
            "Field_getText" => {
                let (_, f) = self.field(s, arg_id(c, 1)?)?;
                Ok(json!(s.doc.copy_range(&f.content_start(), &f.end).plain_text()))
            }
            "Field_setText" => {
                let id = arg_id(c, 1)?;
                let text = arg_str(c, 2)?;
                let rich = arg_bool(c, 3).unwrap_or(false);
                self.set_text(s, id, &text, rich)?;
                Ok(Value::Null)
            }
            "Field_setCode" => {
                let (_, f) = self.field(s, arg_id(c, 1)?)?;
                let code = arg_str(c, 2)?;
                self.begin_edit(s);
                let p = s.doc.para_mut(f.start.story, &f.start.path).map_err(|e| e.to_string())?;
                match p.object_at_mut(f.start.off) {
                    Some(InlineObject::FieldStart { instr, .. }) => *instr = fields::stored_code(&code),
                    _ => return Err("the field's start was not found".into()),
                }
                p.touch();
                Self::end_edit(s);
                Ok(Value::Null)
            }
            "Document_importDocument" | "Document_exportDocument" => {
                Err("WordCraft doesn't support Zotero's document transfer; the citations are already stored the way Word keeps them".into())
            }
            m => Err(format!("WordCraft doesn't support {m}")),
        }
    }

    /// Insert an empty Zotero field at the selection (in a new footnote or endnote for note
    /// styles), holding `content` if given. Returns its id and the field.
    fn insert_field(&mut self, s: &mut Session, note: i64, content: Option<Fragment>) -> Result<(u64, ZField), String> {
        if !fields::can_insert_at(&s.doc, &s.sel.ordered().0) {
            return Err("a citation can't go here (only in the text, a footnote or an endnote)".into());
        }
        self.fields(s);
        self.begin_edit(s);
        if (note == 1 || note == 2) && s.sel.focus.story == StoryRef::Body {
            let id = if note == 1 { "references.footnote" } else { "references.endnote" };
            s.join_next_undo();
            s.run(id, &json!({})).map_err(|e| e.to_string())?;
        }
        let at = delete_selection(s)?;
        let base = s.doc.para_at(&at).map(|p| p.props_at(at.off).clone()).unwrap_or_default();
        let base = CharProps { style: base.style.filter(|st| !st.ends_with("Reference")), ..base };
        let start = at.clone();
        let mut cur = s
            .doc
            .insert_object(&start, InlineObject::FieldStart { instr: fields::PREFIX.to_string(), locked: false }, &base)
            .map_err(|e| e.to_string())?;
        if let Some(frag) = content {
            cur = s.doc.insert_fragment(&cur, &frag).map_err(|e| e.to_string())?;
        }
        let after = s.doc.insert_object(&cur, InlineObject::FieldEnd, &base).map_err(|e| e.to_string())?;
        s.sel = Selection::caret(after);
        Self::end_edit(s);
        self.adopt(s, &start)
    }

    fn set_text(&mut self, s: &mut Session, id: u64, text: &str, rich: bool) -> Result<(), String> {
        let (_, f) = self.field(s, id)?;
        self.begin_edit(s);
        let base = s.doc.para(f.start.story, &f.start.path).map(|p| p.props_of_char(f.start.off).clone()).unwrap_or_default();
        let frag = rich::fragment(text, rich, &base);
        let multi = frag.blocks.len() > 1;
        let at = s.doc.delete_range(&f.content_start(), &f.end).map_err(|e| e.to_string())?;
        let end = s.doc.insert_fragment(&at, &frag).map_err(|e| e.to_string())?;
        if f.code.get(..4).is_some_and(|k| k.eq_ignore_ascii_case("BIBL")) {
            ensure_bib_style(s);
            let paths = s.doc.paths_between(&f.start, &end);
            for path in paths {
                if let Ok(p) = s.doc.para_mut(f.start.story, &path) {
                    p.props = ParaProps { style: Some(BIB_STYLE.into()), ..Default::default() };
                    p.touch();
                }
            }
        } else if multi {
            log::debug!("Zotero citation text spans paragraphs");
        }
        Self::end_edit(s);
        Ok(())
    }

    /// Move a citation to the text (`note` 0), a footnote (1) or an endnote (2). The field keeps
    /// its place in document order, so its id stays valid.
    fn convert_field(&mut self, s: &mut Session, id: u64, note: i64) -> Result<(), String> {
        let (_, f) = self.field(s, id)?;
        let target = match note {
            1 => Some(PartKind::Footnote),
            2 => Some(PartKind::Endnote),
            _ => None,
        };
        let current = match f.start.story {
            StoryRef::Body => None,
            StoryRef::Part(pid) => match s.doc.parts.get(&pid).map(|p| p.kind) {
                Some(k @ (PartKind::Footnote | PartKind::Endnote)) => Some(k),
                _ => return Ok(()),
            },
        };
        if current == target {
            return Ok(());
        }
        self.begin_edit(s);
        let saved = s.sel.clone();
        let frag = s.doc.copy_range(&f.start, &f.after());
        s.doc.delete_range(&f.start, &f.after()).map_err(|e| e.to_string())?;
        // Where the citation goes in the text: where it was, or where its note's mark was.
        let at = match f.start.story {
            StoryRef::Body => f.start.clone(),
            StoryRef::Part(pid) => match remove_empty_note(s, &f) {
                Some(p) => p,
                None => {
                    let r = note_ref_pos(s, pid).ok_or("the note's reference mark was not found")?;
                    Pos { off: r.off + OBJ.len_utf8(), ..r }
                }
            },
        };
        match target {
            None => {
                s.doc.insert_fragment(&at, &frag).map_err(|e| e.to_string())?;
            }
            Some(kind) => {
                s.sel = Selection::caret(at.clone());
                s.join_next_undo();
                let cmd = if kind == PartKind::Footnote { "references.footnote" } else { "references.endnote" };
                if let Err(e) = s.run(cmd, &json!({})) {
                    // Put the citation back where it was taken from.
                    let _ = s.doc.insert_fragment(&at, &frag);
                    s.sel = saved;
                    Self::end_edit(s);
                    return Err(e.to_string());
                }
                let c = s.sel.focus.clone();
                s.doc.insert_fragment(&c, &frag).map_err(|e| e.to_string())?;
            }
        }
        s.sel = saved;
        Self::end_edit(s);
        Ok(())
    }

    fn convert_placeholders(&mut self, s: &mut Session, ph: &[Value], note: i64) -> R {
        let mut ids = Vec::new();
        for v in ph {
            let key = match v {
                Value::String(x) => x.clone(),
                other => other.to_string(),
            };
            let url = format!("{PLACEHOLDER_URL}{key}");
            let Some((a, b)) = find_link(s, &url) else { return Err(format!("placeholder {key} was not found")) };
            self.begin_edit(s);
            let text = s.doc.copy_range(&a, &b);
            let mut text = text;
            for blk in &mut text.blocks {
                if let wordcraft_doc::Block::Para(p) = blk {
                    let n = p.len();
                    let _ = p.format(0, n, &|c| c.link = None);
                }
            }
            let at = s.doc.delete_range(&a, &b).map_err(|e| e.to_string())?;
            s.sel = Selection::caret(at);
            let (id, _) = self.insert_field(s, note, Some(text))?;
            ids.push(id);
        }
        let list = self.fields(s);
        let mut codes = Vec::new();
        let mut notes = Vec::new();
        for id in &ids {
            let f = self.ids.iter().position(|x| x == id).and_then(|k| list.get(k));
            codes.push(f.map(|f| f.code.clone()).unwrap_or_default());
            notes.push(f.map(|f| f.note_index).unwrap_or(0));
        }
        Ok(json!([ids, codes, notes]))
    }
}

/// Delete the selection; returns where it collapsed.
fn delete_selection(s: &mut Session) -> Result<Pos, String> {
    let (a, b) = s.sel.ordered();
    let at = if a == b { a } else { s.doc.delete_range(&a, &b).map_err(|e| e.to_string())? };
    s.sel = Selection::caret(at.clone());
    Ok(at)
}

/// The first run of text linked to `url`, as a range in one paragraph.
fn find_link(s: &Session, url: &str) -> Option<(Pos, Pos)> {
    for story in std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))) {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            let mut found: Option<(usize, usize)> = None;
            for (range, props) in p.run_ranges() {
                let hit = props.link.as_deref().is_some_and(|l| l == url || l.strip_prefix(url).is_some_and(|rest| rest.starts_with('&')));
                match (hit, found) {
                    (true, None) => found = Some((range.start, range.end)),
                    (true, Some((a, _))) => found = Some((a, range.end)),
                    (false, Some(_)) => break,
                    (false, None) => {}
                }
            }
            if let Some((a, b)) = found {
                return Some((Pos::new(story, path.clone(), a), Pos::new(story, path, b)));
            }
        }
    }
    None
}

/// Where the body references note `id`.
fn note_ref_pos(s: &Session, id: u32) -> Option<Pos> {
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let hit = p.object_offsets().into_iter().find(|o| matches!(p.object_at(*o), Some(InlineObject::NoteRef { id: n, .. }) if *n == id));
        if let Some(off) = hit {
            return Some(Pos::new(StoryRef::Body, path, off));
        }
    }
    None
}

/// After deleting a citation that was alone in a note, remove the note too (as Word's plugin
/// does for note styles). Returns where its reference mark was.
fn remove_empty_note(s: &mut Session, f: &ZField) -> Option<Pos> {
    let StoryRef::Part(id) = f.start.story else { return None };
    let blocks = s.doc.story(StoryRef::Part(id))?;
    let mut rest = String::new();
    for b in blocks {
        if let Some(p) = b.as_para() {
            rest.push_str(&p.text);
        }
    }
    if rest.chars().any(|c| c != OBJ && !c.is_whitespace()) {
        return None;
    }
    let at = note_ref_pos(s, id)?;
    let after = Pos { off: at.off + OBJ.len_utf8(), ..at.clone() };
    s.doc.delete_range(&at, &after).ok()?;
    s.doc.parts.remove(&id);
    s.sel = Selection::caret(at.clone());
    Some(at)
}

fn ensure_bib_style(s: &mut Session) {
    if s.doc.styles.get(BIB_STYLE).is_none() {
        s.doc.styles.upsert(Style {
            id: BIB_STYLE.into(),
            name: BIB_STYLE.into(),
            kind: StyleKind::Paragraph,
            based_on: Some("Normal".into()),
            next: Some(BIB_STYLE.into()),
            ..Default::default()
        });
    }
}

/// `Document_setBibliographyStyle(doc, firstLineIndent, bodyIndent, lineSpacing, entrySpacing,
/// tabStops, count)`, all in twips; line spacing 240 is single.
fn set_bibliography_style(s: &mut Session, c: &Call) -> Result<(), String> {
    let first = arg_i64(c, 1)?;
    let body = arg_i64(c, 2)?;
    let line = arg_i64(c, 3)?;
    let entry = arg_i64(c, 4)?;
    let tabs: Vec<TabStop> = match c.args.get(5) {
        Some(Value::Array(t)) => t
            .iter()
            .take(64)
            .filter_map(Value::as_i64)
            .map(|v| TabStop { pos: twips(v).max(0.0), align: TabAlign::default(), leader: TabLeader::default() })
            .collect(),
        _ => Vec::new(),
    };
    ensure_bib_style(s);
    let st = s.doc.styles.get_mut(BIB_STYLE).ok_or("the bibliography style is missing")?;
    st.para.indent_left = Some(twips(body));
    st.para.indent_first = Some(twips(first));
    st.para.line_spacing = Some(if line > 0 { LineSpacing::Multiple((line as f32 / 240.0).clamp(0.5, 5.0)) } else { LineSpacing::Multiple(1.0) });
    st.para.space_before = Some(0.0);
    st.para.space_after = Some(twips(entry).max(0.0));
    st.para.tabs = if tabs.is_empty() { None } else { Some(tabs) };
    Ok(())
}
