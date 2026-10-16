//! The document ↔ Yjs mapping (WordCraft schema 1) and the per-user undo.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Map as JsonMap, Value as Json, json};
use wordcraft_doc::{Block, Blocks, Comment, Document, IdSpace, Paragraph, Part, PartKind, Path, Pos, StoryRef, TextBody};
use yrs::branch::BranchID;
use yrs::types::PathSegment;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Array, ArrayRef, Assoc, DeepObservable, Doc, IndexedSequence, Map, MapPrelim, MapRef, Observable, OffsetKind, Options, Origin, Out, ReadTxn,
    StateVector, StickyIndex, Text, Transact, TransactionMut, UndoManager, Update, WriteTxn, Xml, XmlElementRef, XmlFragment, XmlFragmentPrelim,
    XmlFragmentRef, XmlOut, XmlTextRef,
};

use crate::json::{from_json, to_any, to_json};
use crate::schema::{
    MAX_DEPTH, STORY_ROOTS, block_slots, byte_of_y, children_tagged, kind_name, kind_of, read_block, read_blocks, story_root, text_of, update_blocks,
    write_blocks, y_of_byte,
};
use crate::{APP, CollabError, SCHEMA};

/// Transaction origins.
const LOCAL: &str = "wordcraft/local";
const REMOTE: &str = "wordcraft/remote";
const SEED: &str = "seed";
/// Chat edits: neither the model's edits nor tracked by the undo manager.
const CHAT: &str = "wordcraft/chat";
/// Updates kept while they wait for earlier ones.
const MAX_HELD: usize = 1_000;

/// Roots besides `body` and `meta`, each with its own observer.
const MAP_ROOTS: [&str; 10] = ["headers", "footers", "notes", "textBoxes", "comments", "styles", "numbering", "settings", "media", "files"];
/// The document-wide values in `settings`.
const SETTINGS_KEYS: [&str; 9] =
    ["settings", "lastSection", "core", "sources", "customProps", "revisions", "footnoteSeparators", "endnoteSeparators", "ribbon"];
/// The key of the style sheet's defaults in `styles`.
const STYLE_DEFAULTS: &str = "@defaults";

/// The document after remote changes (or an undo).
#[derive(Debug)]
pub struct Refreshed {
    pub doc: Document,
    /// Paragraphs and tables that changed (the top-level block of the body, or a story's first
    /// block), for the interface to highlight briefly.
    pub changed: Vec<(StoryRef, Path)>,
    /// More than paragraphs changed (stories, styles, the whole document).
    pub structural: bool,
    /// The chat changed (it's in `doc.chat`).
    pub chat: bool,
}

/// What remote transactions (and undos) changed since the last [`Replica::refresh`].
#[derive(Default, Debug)]
struct Changes {
    meta: bool,
    /// Blocks were inserted into or removed from the body.
    body_list: bool,
    /// Top-level body blocks whose content changed.
    body_blocks: HashSet<BranchID>,
    roots: HashSet<&'static str>,
    /// The chat (`chat`, `chatMeta`).
    chat: bool,
}

impl Changes {
    fn any(&self) -> bool {
        self.meta || self.body_list || !self.body_blocks.is_empty() || !self.roots.is_empty() || self.chat
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn origin_is(txn: &TransactionMut, name: &str) -> bool {
    txn.origin().is_some_and(|o| *o == Origin::from(name))
}

/// Remote updates and undos change the model; our own commits, chat edits and the seed already
/// did.
fn recorded(txn: &TransactionMut) -> bool {
    !origin_is(txn, LOCAL) && !origin_is(txn, SEED) && !origin_is(txn, CHAT)
}

fn out_str(o: Option<Out>) -> Option<Arc<str>> {
    match o? {
        Out::Any(Any::String(s)) => Some(s),
        _ => None,
    }
}

fn out_json(o: Option<Out>) -> Option<Json> {
    match o? {
        Out::Any(Any::Null | Any::Undefined) => None,
        Out::Any(a) => Some(to_json(&a)),
        _ => None,
    }
}

fn branch_id<B: AsRef<yrs::branch::Branch>>(b: &B) -> BranchID {
    b.as_ref().id()
}

fn sub_map<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<MapRef> {
    match m.get(txn, key)? {
        Out::YMap(x) => Some(x),
        _ => None,
    }
}

fn sub_fragment<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<XmlFragmentRef> {
    match m.get(txn, key)? {
        Out::YXmlFragment(x) => Some(x),
        _ => None,
    }
}

/// Refuses an update that writes another app's or schema's `meta` (it's dropped, not applied).
fn foreign_meta(update: &[u8]) -> Result<(), CollabError> {
    if !update.windows(4).any(|w| w == b"meta") {
        return Ok(());
    }
    let Ok(u) = Update::decode_v1(update) else { return Ok(()) };
    let scratch = Doc::new();
    let applied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut txn = scratch.transact_mut();
        txn.apply_update(u).is_ok()
    }));
    if !matches!(applied, Ok(true)) {
        return Ok(());
    }
    let txn = scratch.transact();
    let Some(meta) = txn.get_map("meta") else { return Ok(()) };
    let app = out_str(meta.get(&txn, "app"));
    let schema = match meta.get(&txn, "schema") {
        Some(Out::Any(Any::Number(n))) => n.as_f64().map(|f| f as u64),
        _ => None,
    };
    match (app, schema) {
        (Some(a), s) if &*a != APP || s.is_some_and(|s| s != u64::from(SCHEMA)) => {
            Err(CollabError::Mismatch { app: a.to_string(), schema: s.unwrap_or(0) })
        }
        (None, Some(s)) if s != u64::from(SCHEMA) => Err(CollabError::Mismatch { app: APP.into(), schema: s }),
        _ => Ok(()),
    }
}

/// A document's shared content, comparable across peers: paragraphs with their runs merged,
/// lists in id order, as JSON (convergence tests, diagnostics).
pub fn canonical(doc: &Document) -> Json {
    fn norm_blocks(bl: &Blocks) -> Vec<Json> {
        bl.iter().map(|b| norm_block(b)).collect()
    }
    fn norm_block(b: &Block) -> Json {
        match b {
            Block::Para(p) => para_json(p),
            Block::Table(t) => json!({
                "tblPr": t.props, "grid": t.grid, "controls": t.controls,
                "rows": t.rows.iter().map(|r| json!({"trPr": r.props, "controls": r.controls,
                    "cells": r.cells.iter().map(|c| json!({"tcPr": c.props, "controls": c.controls, "blocks": norm_blocks(&c.blocks)})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            }),
        }
    }
    fn para_json(p: &Paragraph) -> Json {
        let mut runs: Vec<(String, Json)> = Vec::new();
        let mut start = 0;
        for r in &p.runs {
            let text = p.text.get(start..start + r.len).unwrap_or_default().to_string();
            start += r.len;
            let props = serde_json::to_value(&r.props).unwrap_or(Json::Null);
            match runs.last_mut() {
                Some(last) if last.1 == props => last.0.push_str(&text),
                _ => runs.push((text, props)),
            }
        }
        json!({"text": p.text, "runs": runs, "props": p.props, "mark": p.mark, "objects": p.objects, "section": p.section})
    }
    let mut numbering = doc.numbering.clone();
    numbering.abstracts.sort_by_key(|a| a.id);
    numbering.nums.sort_by_key(|n| n.id);
    let parts: BTreeMap<String, Json> = doc
        .parts
        .iter()
        .map(|(id, p)| (id.to_string(), json!({"kind": kind_name(p.kind), "body": p.body, "blocks": norm_blocks(&p.blocks)})))
        .collect();
    json!({
        "body": norm_blocks(&doc.body),
        "parts": parts,
        "comments": doc.comments,
        "styles": doc.styles,
        "numbering": numbering,
        "settings": settings_values(doc),
        "media": doc.media.iter().map(|(k, v)| (k.clone(), v.len())).collect::<BTreeMap<_, _>>(),
        "files": doc.passthrough.iter().map(|(k, v)| (k.clone(), v.len())).collect::<BTreeMap<_, _>>(),
    })
}

/// The `settings` root's values of a document.
fn settings_values(d: &Document) -> BTreeMap<&'static str, Json> {
    let v = |x: Result<Json, serde_json::Error>| x.unwrap_or(Json::Null);
    BTreeMap::from([
        ("settings", v(serde_json::to_value(&d.settings))),
        ("lastSection", v(serde_json::to_value(&d.last_section))),
        ("core", v(serde_json::to_value(&d.core))),
        ("sources", v(serde_json::to_value(&d.sources))),
        ("customProps", v(serde_json::to_value(&d.custom_props))),
        ("revisions", v(serde_json::to_value(&d.revisions))),
        ("footnoteSeparators", v(serde_json::to_value(&d.footnote_separators))),
        ("endnoteSeparators", v(serde_json::to_value(&d.endnote_separators))),
        ("ribbon", v(serde_json::to_value(&d.ribbon))),
    ])
}

fn set_settings_value(d: &mut Document, key: &str, v: Json) {
    match key {
        "settings" => {
            if let Some(x) = from_json(v) {
                d.settings = x;
            }
        }
        "lastSection" => {
            if let Some(x) = from_json(v) {
                d.last_section = x;
            }
        }
        "core" => {
            if let Some(x) = from_json(v) {
                d.core = x;
            }
        }
        "sources" => {
            if let Some(x) = from_json(v) {
                d.sources = x;
            }
        }
        "customProps" => {
            if let Some(x) = from_json(v) {
                d.custom_props = x;
            }
        }
        "revisions" => {
            if let Some(x) = from_json(v) {
                d.revisions = x;
            }
        }
        "footnoteSeparators" => {
            if let Some(x) = from_json(v) {
                d.footnote_separators = x;
            }
        }
        "endnoteSeparators" => {
            if let Some(x) = from_json(v) {
                d.endnote_separators = x;
            }
        }
        "ribbon" => d.ribbon = from_json(v).unwrap_or(None),
        _ => {}
    }
}

/// The comment fields kept as their own keys (schema 1, "comment map").
fn comment_fields(c: &Comment) -> JsonMap<String, Json> {
    let mut o = JsonMap::new();
    o.insert("story".into(), json!(c.part));
    o.insert("author".into(), json!(c.author));
    o.insert("initials".into(), json!(c.initials));
    o.insert("date".into(), json!(c.date));
    o.insert("resolved".into(), json!(c.resolved));
    if let Some(p) = c.parent {
        o.insert("parent".into(), json!(p));
    }
    if let Some(d) = &c.durable_id {
        o.insert("durableId".into(), json!(d));
    }
    if let Some(d) = &c.date_utc {
        o.insert("dateUtc".into(), json!(d));
    }
    o
}

/// A shared document: the Yjs document, kept in step with the engine's model.
pub struct Replica {
    doc: Doc,
    client_id: u64,
    undo: UndoManager,
    /// The undo manager's clock: commits with the same tick are one undo step (typing).
    tick: Arc<AtomicU64>,
    changes: Arc<Mutex<Changes>>,
    outbox: Arc<Mutex<Vec<Vec<u8>>>>,
    /// The document as last synced; `None` until the room's document is there (joining).
    base: Option<Document>,
    /// The body's top-level elements → `base`'s blocks (unchanged ones are reused).
    body_ids: HashMap<BranchID, Arc<Block>>,
    /// Updates waiting for earlier ones ([`Replica::apply_update`]).
    held: Vec<Vec<u8>>,
    /// The lowest id of this person's block not made yet (ids an undo gave up aren't made again).
    floor: u32,
}

impl Replica {
    /// An empty replica with Yjs client id `client_id` (below 2^53).
    pub fn new(client_id: u64) -> Replica {
        let client_id = (client_id & ((1u64 << 53) - 1)).max(1);
        // Garbage collection off: with it, yrs 0.28 can free items the undo manager and later
        // encodings still point at. Text indexes count UTF-16 code units, as Yjs everywhere.
        let mut options = Options::with_client_id(yrs::block::ClientID::new(client_id));
        options.skip_gc = true;
        options.offset_kind = OffsetKind::Utf16;
        let doc = Doc::with_options(options);
        let changes: Arc<Mutex<Changes>> = Default::default();
        let outbox: Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
        let ob = outbox.clone();
        if let Err(e) = doc.observe_update_v1("wordcraft/out", move |txn, e| {
            if !origin_is(txn, REMOTE) {
                lock(&ob).push(e.update.clone());
            }
        }) {
            log::warn!("collab: can't observe updates: {e}");
        }
        let meta = doc.get_or_insert_map("meta");
        let ch = changes.clone();
        meta.observe("wordcraft/meta", move |txn, _| {
            if recorded(txn) {
                lock(&ch).meta = true;
            }
        });
        let body = doc.get_or_insert_xml_fragment("body");
        let ch = changes.clone();
        body.observe_deep("wordcraft/body", move |txn, events| {
            if !recorded(txn) {
                return;
            }
            let mut ch = lock(&ch);
            for e in events.iter() {
                match e.path().front() {
                    Some(PathSegment::Index(i)) => {
                        if let Some(f) = txn.get_xml_fragment("body")
                            && let Some(child) = f.get(txn, *i)
                        {
                            ch.body_blocks.insert(child.id());
                        } else {
                            ch.body_list = true;
                        }
                    }
                    Some(PathSegment::Key(_)) => ch.body_list = true,
                    None => ch.body_list = true,
                }
            }
        });
        let tick = Arc::new(AtomicU64::new(1));
        let t = tick.clone();
        let options = yrs::undo::Options {
            capture_timeout_millis: 1,
            tracked_origins: [Origin::from(LOCAL)].into_iter().collect(),
            capture_transaction: None,
            timestamp: Arc::new(move || t.load(Ordering::SeqCst)),
            init_undo_stack: vec![],
            init_redo_stack: vec![],
        };
        let mut undo = UndoManager::with_options(options);
        undo.expand_scope(&doc, &body);
        for name in MAP_ROOTS {
            let root = doc.get_or_insert_map(name);
            let ch = changes.clone();
            root.observe_deep(format!("wordcraft/{name}"), move |txn, _| {
                if recorded(txn) {
                    lock(&ch).roots.insert(name);
                }
            });
            undo.expand_scope(&doc, &root);
        }
        let order = doc.get_or_insert_array("styleOrder");
        let ch = changes.clone();
        order.observe("wordcraft/styleOrder", move |txn, _| {
            if recorded(txn) {
                lock(&ch).roots.insert("styles");
            }
        });
        undo.expand_scope(&doc, &order);
        // The chat: observed, never in the undo manager's scope (Undo never removes a message).
        let chat = doc.get_or_insert_array(crate::chat::ROOT);
        let ch = changes.clone();
        chat.observe_deep("wordcraft/chat", move |txn, _| {
            if recorded(txn) {
                lock(&ch).chat = true;
            }
        });
        let chat_meta = doc.get_or_insert_map(crate::chat::META);
        let ch = changes.clone();
        chat_meta.observe("wordcraft/chatMeta", move |txn, _| {
            if recorded(txn) {
                lock(&ch).chat = true;
            }
        });
        Replica { doc, client_id, undo, tick, changes, outbox, base: None, body_ids: HashMap::new(), held: Vec::new(), floor: 0 }
    }

    pub fn client_id(&self) -> u64 {
        self.client_id
    }

    /// Where this replica's person makes new story, comment and list ids.
    pub fn id_space(&self) -> IdSpace {
        let start = crate::id_space(self.client_id);
        IdSpace { start, floor: self.floor.max(start) }
    }

    /// Notes the ids of `d` in this person's block, so none is made twice.
    fn note_ids(&mut self, d: &Document) {
        let space = self.id_space();
        let ids = d
            .parts
            .keys()
            .chain(d.comments.keys())
            .copied()
            .chain(d.numbering.abstracts.iter().map(|a| a.id))
            .chain(d.numbering.nums.iter().map(|n| n.id))
            .chain([d.id_space.floor.saturating_sub(1), d.numbering.id_space.floor.saturating_sub(1)]);
        for id in ids {
            if space.contains(id) {
                self.floor = self.floor.max(id.saturating_add(1));
            }
        }
    }

    /// The room's document is here (always after [`Replica::seed`]; after joining, once a peer's
    /// state arrived).
    pub fn is_ready(&self) -> bool {
        self.base.is_some()
    }

    /// The document as last synced.
    pub fn base(&self) -> Option<&Document> {
        self.base.as_ref()
    }

    /// Checks `meta`: `Ok(true)` ours, `Ok(false)` not there yet, an error for another app or
    /// schema.
    pub fn check_meta(&self) -> Result<bool, CollabError> {
        let txn = self.doc.transact();
        let Some(meta) = txn.get_map("meta") else { return Ok(false) };
        let app = out_str(meta.get(&txn, "app"));
        let schema = match meta.get(&txn, "schema") {
            Some(Out::Any(Any::Number(n))) => n.as_f64().map(|f| f as u64),
            _ => None,
        };
        match (app, schema) {
            (None, None) => Ok(false),
            (Some(a), Some(s)) if &*a == APP && s == u64::from(SCHEMA) => Ok(true),
            (a, s) => Err(CollabError::Mismatch { app: a.map(|a| a.to_string()).unwrap_or_else(|| "another app".into()), schema: s.unwrap_or(0) }),
        }
    }

    /// The room's title (`meta.title`).
    pub fn title(&self) -> Option<String> {
        let txn = self.doc.transact();
        txn.get_map("meta").and_then(|m| out_str(m.get(&txn, "title"))).map(|s| s.to_string())
    }

    fn take_outbox(&self) -> Result<Option<Vec<u8>>, CollabError> {
        let ups: Vec<Vec<u8>> = std::mem::take(&mut *lock(&self.outbox));
        match ups.len() {
            0 => Ok(None),
            1 => Ok(ups.into_iter().next()),
            _ => yrs::merge_updates_v1(ups.iter().map(|u| u.as_slice()).collect::<Vec<_>>())
                .map(Some)
                .map_err(|e| CollabError::Other(format!("merging updates: {e}"))),
        }
    }

    /// Maps the body's top-level elements to `base`'s blocks (they correspond one to one).
    fn index_body(&mut self) {
        self.body_ids.clear();
        let Some(base) = &self.base else { return };
        let txn = self.doc.transact();
        let Some(body) = txn.get_xml_fragment("body") else { return };
        let slots = block_slots(&txn, &body);
        if slots.len() != base.body.len() {
            return;
        }
        for ((_, el), b) in slots.iter().zip(&base.body) {
            self.body_ids.insert(branch_id(el), b.clone());
        }
    }

    // ------------------------------------------------------------ sharing

    /// Shares `doc`: builds the shared document from it (one transaction, origin `seed`).
    /// Returns the update (the whole document).
    pub fn seed(&mut self, doc: &Document, title: &str) -> Result<Vec<u8>, CollabError> {
        let ydoc = self.doc.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut txn = ydoc.transact_mut_with(SEED);
            let meta = txn.get_or_insert_map("meta");
            meta.insert(&mut txn, "app", APP);
            meta.insert(&mut txn, "schema", i64::from(SCHEMA));
            meta.insert(&mut txn, "title", title);
            write_doc(&mut txn, None, doc);
            crate::chat::seed(&mut txn, &doc.chat);
        }));
        if r.is_err() {
            return Err(CollabError::Other("the document couldn't be shared".into()));
        }
        let mut d = doc.clone();
        self.note_ids(&d);
        d.set_id_space(self.id_space());
        self.base = Some(d);
        self.index_body();
        *lock(&self.changes) = Changes::default();
        Ok(self.take_outbox()?.unwrap_or_default())
    }

    // ------------------------------------------------------------ local edits

    /// Turns the change from the last synced document to `cur` into one transaction with the
    /// local origin. `join`: the change continues the last undo step (typing). Returns the update
    /// to send, `None` when nothing shared changed.
    pub fn commit(&mut self, cur: &Document, join: bool) -> Result<Option<Vec<u8>>, CollabError> {
        let Some(base) = self.base.take() else { return Err(CollabError::NotReady) };
        if !join {
            self.tick.fetch_add(1, Ordering::SeqCst);
        }
        let ydoc = self.doc.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut txn = ydoc.transact_mut_with(LOCAL);
            write_doc(&mut txn, Some(&base), cur);
        }));
        let mut next = cur.clone();
        self.note_ids(&next);
        next.set_id_space(self.id_space());
        self.base = Some(next);
        self.index_body();
        if r.is_err() {
            return Err(CollabError::Other("a change couldn't be shared".into()));
        }
        self.take_outbox()
    }

    // ------------------------------------------------------------ remote changes

    /// Applies a remote update (sync step 2 or an incremental update) to the shared document.
    /// The model follows at the next [`Replica::refresh`]. An update that skips some of a peer's
    /// changes waits until the gap is filled ([`Replica::has_gaps`]).
    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), CollabError> {
        if update.len() > crate::MAX_UPDATE {
            return Err(CollabError::TooLarge(update.len()));
        }
        let u = std::panic::catch_unwind(|| Update::decode_v1(update))
            .map_err(|_| CollabError::Corrupt("not an update".into()))?
            .map_err(|e| CollabError::Corrupt(e.to_string()))?;
        foreign_meta(update)?;
        if self.gap(&u) {
            if self.held.len() >= MAX_HELD {
                self.held.remove(0);
            }
            self.held.push(update.to_vec());
            return Ok(());
        }
        self.integrate(u)?;
        loop {
            let ready = self.held.iter().position(|h| Update::decode_v1(h).is_ok_and(|u| !self.gap(&u)));
            let Some(i) = ready else { break };
            let h = self.held.remove(i);
            if let Ok(u) = Update::decode_v1(&h)
                && let Err(e) = self.integrate(u)
            {
                log::warn!("collab: a held update failed: {e}");
            }
        }
        Ok(())
    }

    /// Whether `u` starts past what we have of one of its clients.
    fn gap(&self, u: &Update) -> bool {
        let have = self.doc.transact().state_vector();
        u.state_vector_lower().iter().any(|(c, clock)| *clock > have.get(c))
    }

    fn integrate(&mut self, u: Update) -> Result<(), CollabError> {
        let doc = self.doc.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut txn = doc.transact_mut_with(REMOTE);
            txn.apply_update(u).map_err(|e| CollabError::Corrupt(e.to_string()))
        }));
        match r {
            Ok(r) => r,
            Err(_) => Err(CollabError::Corrupt("the update couldn't be applied".into())),
        }
    }

    /// Some updates wait for others that haven't arrived: ask the peers for them (sync step 1).
    pub fn has_gaps(&self) -> bool {
        !self.held.is_empty()
    }

    /// Whether remote changes (or an undo) are waiting for [`Replica::refresh`].
    pub fn has_changes(&self) -> bool {
        lock(&self.changes).any() || self.base.is_none()
    }

    /// Brings remote changes into the model. `cur` is the engine's document, already committed
    /// (so it equals the last synced one). While joining, this builds the first document once
    /// the room's arrived (keeping `cur`'s styles and settings where the room has none). `None`:
    /// nothing to do.
    pub fn refresh(&mut self, cur: &Document) -> Result<Option<Refreshed>, CollabError> {
        if self.base.is_none() {
            if !self.check_meta()? {
                return Ok(None);
            }
            *lock(&self.changes) = Changes::default();
            let doc = {
                let txn = self.doc.transact();
                let mut d = cur.clone();
                d.body = Vec::new();
                read_roots(&txn, &mut d, None, &MAP_ROOTS);
                d.body = read_body(&txn, None, &HashMap::new(), &HashSet::new()).0;
                // Joining brings the room's chat in.
                d.chat = Arc::new(crate::chat::read(&txn));
                d.ensure_nonempty();
                d
            };
            let mut doc = doc;
            self.note_ids(&doc);
            doc.set_id_space(self.id_space());
            self.base = Some(doc.clone());
            self.index_body();
            return Ok(Some(Refreshed { doc, changed: vec![], structural: true, chat: true }));
        }
        let ch = std::mem::take(&mut *lock(&self.changes));
        if !ch.any() {
            return Ok(None);
        }
        if ch.meta {
            self.check_meta()?;
        }
        let Some(base) = self.base.clone() else { return Ok(None) };
        let mut doc = base.clone();
        let mut changed = Vec::new();
        let structural = !ch.roots.is_empty();
        {
            let txn = self.doc.transact();
            if ch.chat {
                doc.chat = Arc::new(crate::chat::read(&txn));
            }
            if ch.body_list || !ch.body_blocks.is_empty() {
                let (body, fresh) = read_body(&txn, Some(&base.body), &self.body_ids, &ch.body_blocks);
                doc.body = body;
                changed.extend(fresh.into_iter().map(|i| (StoryRef::Body, Path::top(i))));
            }
            if structural {
                let mut roots: Vec<&str> = ch.roots.iter().copied().collect();
                // Stories and comments share ids: read them together, so a story id freed in one
                // and taken in another (updates arriving out of order) never hides either.
                if roots.iter().any(|r| STORY_ROOTS.contains(r) || *r == "comments") {
                    for r in STORY_ROOTS.into_iter().chain(["comments"]) {
                        if !roots.contains(&r) {
                            roots.push(r);
                        }
                    }
                }
                read_roots(&txn, &mut doc, Some(&base), &roots);
                for (id, p) in &doc.parts {
                    if base.parts.get(id) != Some(p) {
                        changed.push((StoryRef::Part(*id), Path::top(0)));
                    }
                }
            }
        }
        doc.ensure_nonempty();
        self.note_ids(&doc);
        doc.set_id_space(self.id_space());
        self.base = Some(doc.clone());
        self.index_body();
        Ok(Some(Refreshed { doc, changed, structural, chat: ch.chat }))
    }

    // ------------------------------------------------------------ undo

    /// Undoes this person's last change (only theirs). Returns the update to send; the model
    /// follows at [`Replica::refresh`]. `None`: nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Vec<u8>>, CollabError> {
        self.pop(true)
    }

    /// Redoes the last undone change.
    pub fn redo(&mut self) -> Result<Option<Vec<u8>>, CollabError> {
        self.pop(false)
    }

    fn pop(&mut self, undo: bool) -> Result<Option<Vec<u8>>, CollabError> {
        if self.base.is_none() {
            return Err(CollabError::NotReady);
        }
        let can = if undo { self.undo.can_undo() } else { self.undo.can_redo() };
        if !can {
            return Ok(None);
        }
        // The next commit starts a new step.
        self.tick.fetch_add(1, Ordering::SeqCst);
        let um = &mut self.undo;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| if undo { um.undo_blocking() } else { um.redo_blocking() }));
        if r.is_err() {
            return Err(CollabError::Other("undo failed".into()));
        }
        self.take_outbox()
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    // ------------------------------------------------------------ sync

    /// Sync step 1: our state vector.
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Whether a peer's state vector shows changes this document lacks.
    pub fn lacks(&self, sv: &[u8]) -> bool {
        let Ok(theirs) = StateVector::decode_v1(sv) else { return false };
        let ours = self.doc.transact().state_vector();
        theirs.iter().any(|(c, clock)| *clock > ours.get(c))
    }

    /// Sync step 2: what a peer with state vector `sv` lacks.
    pub fn diff_for(&self, sv: &[u8]) -> Result<Vec<u8>, CollabError> {
        let sv = StateVector::decode_v1(sv).map_err(|e| CollabError::Corrupt(e.to_string()))?;
        Ok(self.doc.transact().encode_state_as_update_v1(&sv))
    }

    // ------------------------------------------------------------ chat

    /// The room's chat as it is now.
    pub fn chat_now(&self) -> wordcraft_doc::chat::Chat {
        crate::chat::read(&self.doc.transact())
    }

    /// Applies a chat edit to the shared document (one transaction, outside undo). Returns the
    /// update to send (`None`: nothing changed), the messages a clear removed, and the chat
    /// after it, which the last synced document now holds too. Refused edits change nothing.
    pub fn chat(&mut self, op: &crate::chat::ChatOp) -> Result<(Option<Vec<u8>>, usize, wordcraft_doc::chat::Chat), CollabError> {
        let ydoc = self.doc.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut txn = ydoc.transact_mut_with(CHAT);
            crate::chat::apply(&mut txn, op)
        }));
        let removed = match r {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                // A refused edit wrote nothing; drop an empty transaction's update, if any.
                let _ = self.take_outbox();
                return Err(CollabError::Other(e));
            }
            Err(_) => return Err(CollabError::Other("the chat couldn't be changed".into())),
        };
        let chat = self.chat_now();
        if let Some(b) = self.base.as_mut() {
            b.chat = Arc::new(chat.clone());
        }
        Ok((self.take_outbox()?, removed, chat))
    }

    /// The whole document as one update (snapshots, recovery).
    pub fn full_state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    // ------------------------------------------------------------ positions

    /// `pos` (in the synced document) as a Yjs relative position, encoded: it stays with its
    /// character while others edit. `None` when the position isn't in a shared paragraph.
    pub fn sticky(&self, pos: &Pos) -> Option<Vec<u8>> {
        let base = self.base.as_ref()?;
        let p = base.para_at(pos)?;
        let txn = self.doc.transact();
        let text = text_at(&txn, pos.story, &pos.path)?;
        let y = y_of_byte(p, pos.off);
        let len = text.len(&txn);
        let s = if y >= len && len > 0 { text.sticky_index(&txn, len, Assoc::Before) } else { text.sticky_index(&txn, y.min(len), Assoc::After) };
        s.or_else(|| StickyIndex::at(&txn, yrs::branch::BranchPtr::from(AsRef::<yrs::branch::Branch>::as_ref(&text)), 0, Assoc::Before))
            .map(|s| s.encode_v1())
    }

    /// Where an encoded relative position is now, in the synced document (after
    /// [`Replica::refresh`]). `None` when its paragraph is gone.
    pub fn resolve(&self, sticky: &[u8]) -> Option<Pos> {
        let base = self.base.as_ref()?;
        let s = std::panic::catch_unwind(|| StickyIndex::decode_v1(sticky)).ok()?.ok()?;
        let txn = self.doc.transact();
        let off = s.get_offset(&txn)?;
        let text = XmlTextRef::from(off.branch);
        let (story, path) = path_of(&txn, &text)?;
        let p = base.para(story, &path)?;
        Some(Pos { story, path, off: byte_of_y(p, off.index) })
    }
}

// ---------------------------------------------------------------- positions in the shared document

/// The XmlText of the paragraph at `path` in `story`.
fn text_at<T: ReadTxn>(txn: &T, story: StoryRef, path: &Path) -> Option<XmlTextRef> {
    let f = story_fragment(txn, story)?;
    let (first, rest) = path.0.split_first()?;
    let mut el = block_slots(txn, &f).get(*first as usize)?.1.clone();
    let mut rest = rest;
    while let [r, c, b, more @ ..] = rest {
        let tr = children_tagged(txn, &el, "tr").get(*r as usize)?.1.clone();
        let tc = children_tagged(txn, &tr, "tc").get(*c as usize)?.1.clone();
        el = block_slots(txn, &tc).get(*b as usize)?.1.clone();
        rest = more;
    }
    if !rest.is_empty() || el.try_tag().map(|t| t.as_ref()) != Some("p") {
        return None;
    }
    text_of(txn, &el)
}

/// A story's fragment.
fn story_fragment<T: ReadTxn>(txn: &T, story: StoryRef) -> Option<XmlFragmentRef> {
    match story {
        StoryRef::Body => txn.get_xml_fragment("body"),
        StoryRef::Part(id) => {
            let key = id.to_string();
            for root in STORY_ROOTS {
                if let Some(m) = txn.get_map(root).and_then(|r| sub_map(txn, &r, &key)) {
                    return sub_fragment(txn, &m, "content");
                }
            }
            let comments = txn.get_map("comments")?;
            for (_, v) in comments.iter(txn) {
                if let Out::YMap(m) = v
                    && out_json(m.get(txn, "story")).and_then(|s| s.as_u64()) == Some(u64::from(id))
                {
                    return sub_fragment(txn, &m, "content");
                }
            }
            None
        }
    }
}

/// Which story a fragment is.
fn story_of<T: ReadTxn>(txn: &T, f: &XmlFragmentRef) -> Option<StoryRef> {
    let id = branch_id(f);
    if txn.get_xml_fragment("body").is_some_and(|b| branch_id(&b) == id) {
        return Some(StoryRef::Body);
    }
    for root in STORY_ROOTS {
        let Some(r) = txn.get_map(root) else { continue };
        for (k, v) in r.iter(txn) {
            if let Out::YMap(m) = v
                && sub_fragment(txn, &m, "content").is_some_and(|c| branch_id(&c) == id)
            {
                return k.parse().ok().map(StoryRef::Part);
            }
        }
    }
    let comments = txn.get_map("comments")?;
    for (_, v) in comments.iter(txn) {
        if let Out::YMap(m) = v
            && sub_fragment(txn, &m, "content").is_some_and(|c| branch_id(&c) == id)
        {
            return out_json(m.get(txn, "story")).and_then(|s| s.as_u64()).and_then(|s| u32::try_from(s).ok()).map(StoryRef::Part);
        }
    }
    None
}

fn index_in<T: ReadTxn>(list: &[(u32, XmlElementRef)], el: &XmlElementRef, _txn: &T) -> Option<u32> {
    let id = branch_id(el);
    list.iter().position(|(_, e)| branch_id(e) == id).map(|i| i as u32)
}

/// The story and block path of a paragraph's text.
fn path_of<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> Option<(StoryRef, Path)> {
    let XmlOut::Element(mut el) = text.parent()? else { return None };
    let mut rev: Vec<u32> = Vec::new();
    for _ in 0..=MAX_DEPTH {
        match el.parent()? {
            XmlOut::Fragment(f) => {
                rev.push(index_in(&block_slots(txn, &f), &el, txn)?);
                rev.reverse();
                return Some((story_of(txn, &f)?, Path(rev)));
            }
            XmlOut::Element(tc) => {
                rev.push(index_in(&block_slots(txn, &tc), &el, txn)?);
                let XmlOut::Element(tr) = tc.parent()? else { return None };
                rev.push(index_in(&children_tagged(txn, &tr, "tc"), &tc, txn)?);
                let XmlOut::Element(table) = tr.parent()? else { return None };
                rev.push(index_in(&children_tagged(txn, &table, "tr"), &tr, txn)?);
                el = table;
            }
            XmlOut::Text(_) => return None,
        }
    }
    None
}

// ---------------------------------------------------------------- reading the shared document

/// The body; unchanged top-level blocks (by element, not in `dirty`) are `base`'s own. Also the
/// indexes of the blocks read afresh.
fn read_body<T: ReadTxn>(txn: &T, base: Option<&Blocks>, ids: &HashMap<BranchID, Arc<Block>>, dirty: &HashSet<BranchID>) -> (Blocks, Vec<usize>) {
    let Some(body) = txn.get_xml_fragment("body") else { return (Vec::new(), Vec::new()) };
    let mut out = Vec::new();
    let mut fresh = Vec::new();
    for (_, el) in block_slots(txn, &body) {
        let id = branch_id(&el);
        let hint = ids.get(&id);
        if let Some(b) = hint
            && !dirty.contains(&id)
        {
            out.push(b.clone());
            continue;
        }
        let hint = hint.or_else(|| base.and_then(|b| b.get(out.len())));
        if let Some(b) = read_block(txn, &el, hint, 0) {
            if !hint.is_some_and(|h| Arc::ptr_eq(h, &b)) {
                fresh.push(out.len());
            }
            out.push(b);
        }
    }
    (out, fresh)
}

/// Reads the given roots into `d` (stories, comments, styles, lists, settings, media, files).
/// A root the room doesn't have yet leaves `d`'s value.
fn read_roots<T: ReadTxn>(txn: &T, d: &mut Document, base: Option<&Document>, roots: &[&str]) {
    let hint_part = |id: u32| base.and_then(|b| b.parts.get(&id));
    for root in STORY_ROOTS {
        if !roots.contains(&root) {
            continue;
        }
        let Some(r) = txn.get_map(root) else { continue };
        d.parts.retain(|_, p| story_root(p.kind) != Some(root));
        for (k, v) in r.iter(txn) {
            let (Ok(id), Out::YMap(m)) = (k.parse::<u32>(), v) else { continue };
            let Some(kind) = out_str(m.get(txn, "kind")).and_then(|k| kind_of(&k)) else { continue };
            if story_root(kind) != Some(root) {
                continue;
            }
            let hint = hint_part(id);
            let mut blocks = sub_fragment(txn, &m, "content").map(|f| read_blocks(txn, &f, hint.map(|p| &p.blocks), 0)).unwrap_or_default();
            if blocks.is_empty() {
                blocks.push(Arc::new(Block::Para(Paragraph::new())));
            }
            let body: TextBody = out_json(m.get(txn, "body")).and_then(from_json).unwrap_or_default();
            d.parts.insert(id, Part { kind, blocks, body });
        }
    }
    if roots.contains(&"comments")
        && let Some(r) = txn.get_map("comments")
    {
        d.parts.retain(|_, p| p.kind != PartKind::Comment);
        d.comments.clear();
        for (k, v) in r.iter(txn) {
            let (Ok(cid), Out::YMap(m)) = (k.parse::<u32>(), v) else { continue };
            let get = |key: &str| out_json(m.get(txn, key));
            let s = |key: &str| get(key).and_then(|v| v.as_str().map(str::to_string));
            let Some(story) = get("story").and_then(|v| v.as_u64()).and_then(|v| u32::try_from(v).ok()) else { continue };
            if d.parts.get(&story).is_some_and(|p| p.kind != PartKind::Comment) {
                continue;
            }
            let c = Comment {
                author: s("author").unwrap_or_default(),
                initials: s("initials").unwrap_or_default(),
                date: s("date").unwrap_or_default(),
                parent: get("parent").and_then(|v| v.as_u64()).and_then(|v| u32::try_from(v).ok()),
                resolved: get("resolved").and_then(|v| v.as_bool()).unwrap_or(false),
                part: story,
                durable_id: s("durableId"),
                date_utc: s("dateUtc"),
            };
            let hint = hint_part(story);
            let mut blocks = sub_fragment(txn, &m, "content").map(|f| read_blocks(txn, &f, hint.map(|p| &p.blocks), 0)).unwrap_or_default();
            if blocks.is_empty() {
                blocks.push(Arc::new(Block::Para(Paragraph::new())));
            }
            d.parts.insert(story, Part { kind: PartKind::Comment, blocks, body: TextBody::default() });
            d.comments.insert(cid, c);
        }
    }
    if roots.contains(&"styles")
        && let Some(r) = txn.get_map("styles")
        && r.len(txn) > 0
    {
        let mut sheet = d.styles.clone();
        if let Some(Json::Object(def)) = out_json(r.get(txn, STYLE_DEFAULTS)) {
            if let Some(v) = def.get("defaultChr") {
                sheet.default_chr = from_json(v.clone()).unwrap_or_default();
            }
            if let Some(v) = def.get("defaultPara") {
                sheet.default_para = from_json(v.clone()).unwrap_or_default();
            }
            sheet.default_para_present = def.get("defaultParaPresent").and_then(Json::as_bool).unwrap_or(false);
            sheet.latent_styles = def.get("latentStyles").and_then(|v| v.as_str().map(str::to_string));
        }
        let mut by_id: BTreeMap<String, wordcraft_doc::Style> = BTreeMap::new();
        for (k, v) in r.iter(txn) {
            if k == STYLE_DEFAULTS {
                continue;
            }
            if let Out::Any(a) = v
                && let Some(mut st) = from_json::<wordcraft_doc::Style>(to_json(&a))
            {
                st.id = k.to_string();
                by_id.insert(k.to_string(), st);
            }
        }
        let mut styles = Vec::with_capacity(by_id.len());
        if let Some(order) = txn.get_array("styleOrder") {
            for v in order.iter(txn) {
                if let Some(id) = out_str(Some(v))
                    && let Some(st) = by_id.remove(&*id)
                {
                    styles.push(st);
                }
            }
        }
        styles.extend(by_id.into_values());
        sheet.styles = styles;
        d.styles = sheet;
    }
    if roots.contains(&"numbering")
        && let Some(r) = txn.get_map("numbering")
        && r.len(txn) > 0
    {
        let mut abstracts = Vec::new();
        let mut nums = Vec::new();
        for (k, v) in r.iter(txn) {
            let Out::Any(a) = v else { continue };
            if k.starts_with('a') {
                if let Some(x) = from_json::<wordcraft_doc::numbering::AbstractNum>(to_json(&a)) {
                    abstracts.push(x);
                }
            } else if k.starts_with('n')
                && let Some(x) = from_json::<wordcraft_doc::numbering::Num>(to_json(&a))
            {
                nums.push(x);
            }
        }
        abstracts.sort_by_key(|a| a.id);
        nums.sort_by_key(|n| n.id);
        d.numbering.abstracts = abstracts;
        d.numbering.nums = nums;
    }
    if roots.contains(&"settings")
        && let Some(r) = txn.get_map("settings")
    {
        for key in SETTINGS_KEYS {
            if let Some(v) = out_json(r.get(txn, key)) {
                set_settings_value(d, key, v);
            } else if key == "ribbon" && r.contains_key(txn, key) {
                d.ribbon = None;
            }
        }
    }
    for (root, files) in [("media", false), ("files", true)] {
        if !roots.contains(&root) {
            continue;
        }
        let Some(r) = txn.get_map(root) else { continue };
        let mut out = BTreeMap::new();
        for (k, v) in r.iter(txn) {
            if let Out::Any(Any::Buffer(b)) = v {
                out.insert(k.to_string(), Arc::new(b.to_vec()));
            }
        }
        if files {
            d.passthrough = out;
        } else {
            d.media = out;
        }
    }
}

// ---------------------------------------------------------------- writing the shared document

/// Writes `cur` into the shared document: all of it (`base` `None`, seeding), or what changed
/// since `base`.
fn write_doc(txn: &mut TransactionMut, base: Option<&Document>, cur: &Document) {
    let body = txn.get_or_insert_xml_fragment("body");
    match base {
        None => write_blocks(txn, &body, &cur.body),
        Some(b) => {
            let same = b.body.len() == cur.body.len() && b.body.iter().zip(&cur.body).all(|(x, y)| Arc::ptr_eq(x, y));
            if !same {
                update_blocks(txn, &body, &b.body, &cur.body, 0);
            }
        }
    }
    write_parts(txn, base, cur);
    write_comments(txn, base, cur);
    if base.is_none_or(|b| b.styles != cur.styles) {
        write_styles(txn, base, cur);
    }
    if base.is_none_or(|b| b.numbering != cur.numbering) {
        write_numbering(txn, base, cur);
    }
    let settings = txn.get_or_insert_map("settings");
    let unchanged = base.is_some_and(|b| same_settings(b, cur));
    let now = if unchanged { BTreeMap::new() } else { settings_values(cur) };
    let before = if unchanged { None } else { base.map(settings_values) };
    for (k, v) in now {
        if before.as_ref().is_none_or(|b| b.get(k) != Some(&v)) {
            if v.is_null() {
                settings.remove(txn, k);
            } else {
                settings.insert(txn, k, to_any(&v));
            }
        }
    }
    for (root, now, before) in [("media", &cur.media, base.map(|b| &b.media)), ("files", &cur.passthrough, base.map(|b| &b.passthrough))] {
        if before.is_some_and(|b| b == now) {
            continue;
        }
        let m = txn.get_or_insert_map(root);
        for (k, v) in now {
            if before.is_none_or(|b| b.get(k).is_none_or(|x| !Arc::ptr_eq(x, v) && x != v)) {
                m.insert(txn, k.as_str(), Any::Buffer(Arc::from(v.as_slice())));
            }
        }
        if let Some(b) = before {
            for k in b.keys() {
                if !now.contains_key(k) {
                    m.remove(txn, k);
                }
            }
        }
    }
}

fn new_story(txn: &mut TransactionMut, root: &MapRef, id: u32, part: &Part) {
    let m: MapRef = root.insert(txn, id.to_string(), MapPrelim::default());
    m.insert(txn, "kind", kind_name(part.kind));
    let content: XmlFragmentRef = m.insert(txn, "content", XmlFragmentPrelim::new::<_, ()>(Vec::new()));
    write_blocks(txn, &content, &part.blocks);
    if !part.body.is_default()
        && let Ok(v) = serde_json::to_value(part.body)
    {
        m.insert(txn, "body", to_any(&v));
    }
}

/// Whether two block lists are the same (shared blocks first: an edit copies only what it
/// touches).
fn same_blocks(a: &Blocks, b: &Blocks) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y) || x == y)
}

fn same_part(a: &Part, b: &Part) -> bool {
    a.kind == b.kind && a.body == b.body && same_blocks(&a.blocks, &b.blocks)
}

fn same_parts(a: &Document, b: &Document) -> bool {
    a.parts.len() == b.parts.len() && a.parts.iter().zip(&b.parts).all(|((i, x), (j, y))| i == j && same_part(x, y))
}

/// Whether the `settings` root's values are the same.
fn same_settings(a: &Document, b: &Document) -> bool {
    a.settings == b.settings
        && a.last_section == b.last_section
        && a.core == b.core
        && a.sources == b.sources
        && a.custom_props == b.custom_props
        && a.revisions == b.revisions
        && a.footnote_separators == b.footnote_separators
        && a.endnote_separators == b.endnote_separators
        && a.ribbon == b.ribbon
}

fn write_parts(txn: &mut TransactionMut, base: Option<&Document>, cur: &Document) {
    if base.is_some_and(|b| same_parts(b, cur)) {
        return;
    }
    for (id, part) in &cur.parts {
        let Some(root_name) = story_root(part.kind) else { continue };
        let root = txn.get_or_insert_map(root_name);
        let old = base.and_then(|b| b.parts.get(id));
        let key = id.to_string();
        match old {
            Some(o) if same_part(o, part) => {}
            Some(o) if o.kind == part.kind && sub_map(txn, &root, &key).and_then(|m| sub_fragment(txn, &m, "content")).is_some() => {
                let Some(m) = sub_map(txn, &root, &key) else { continue };
                if !same_blocks(&o.blocks, &part.blocks)
                    && let Some(content) = sub_fragment(txn, &m, "content")
                {
                    update_blocks(txn, &content, &o.blocks, &part.blocks, 0);
                }
                if o.body != part.body {
                    if part.body.is_default() {
                        m.remove(txn, "body");
                    } else if let Ok(v) = serde_json::to_value(part.body) {
                        m.insert(txn, "body", to_any(&v));
                    }
                }
            }
            _ => {
                if let Some(o) = old
                    && let Some(old_root) = story_root(o.kind)
                    && old_root != root_name
                {
                    let r = txn.get_or_insert_map(old_root);
                    r.remove(txn, &key);
                }
                new_story(txn, &root, *id, part);
            }
        }
    }
    if let Some(b) = base {
        for (id, part) in &b.parts {
            if cur.parts.contains_key(id) {
                continue;
            }
            if let Some(root_name) = story_root(part.kind) {
                let root = txn.get_or_insert_map(root_name);
                root.remove(txn, &id.to_string());
            }
        }
    }
}

fn write_comments(txn: &mut TransactionMut, base: Option<&Document>, cur: &Document) {
    let text_of =
        |d: &Document, c: &Comment| -> Option<Blocks> { d.parts.get(&c.part).filter(|p| p.kind == PartKind::Comment).map(|p| p.blocks.clone()) };
    if base.is_some_and(|b| {
        b.comments == cur.comments
            && b.parts.iter().filter(|(_, p)| p.kind == PartKind::Comment).eq(cur.parts.iter().filter(|(_, p)| p.kind == PartKind::Comment))
    }) {
        return;
    }
    let root = txn.get_or_insert_map("comments");
    for (cid, c) in &cur.comments {
        let key = cid.to_string();
        let blocks = text_of(cur, c).unwrap_or_else(|| vec![Arc::new(Block::Para(Paragraph::new()))]);
        let old = base.and_then(|b| b.comments.get(cid).map(|oc| (oc, text_of(b, oc))));
        let existing = sub_map(txn, &root, &key);
        match (old, existing) {
            (Some((oc, ob)), Some(m)) => {
                let (before, now) = (comment_fields(oc), comment_fields(c));
                for (k, v) in &now {
                    if before.get(k) != Some(v) {
                        m.insert(txn, k.as_str(), to_any(v));
                    }
                }
                for k in before.keys() {
                    if !now.contains_key(k) {
                        m.remove(txn, k);
                    }
                }
                match (ob, sub_fragment(txn, &m, "content")) {
                    (Some(ob), Some(content)) => {
                        if !same_blocks(&ob, &blocks) {
                            update_blocks(txn, &content, &ob, &blocks, 0);
                        }
                    }
                    _ => {
                        let content: XmlFragmentRef = m.insert(txn, "content", XmlFragmentPrelim::new::<_, ()>(Vec::new()));
                        write_blocks(txn, &content, &blocks);
                    }
                }
            }
            _ => {
                let m: MapRef = root.insert(txn, key, MapPrelim::default());
                for (k, v) in comment_fields(c) {
                    m.insert(txn, k, to_any(&v));
                }
                let content: XmlFragmentRef = m.insert(txn, "content", XmlFragmentPrelim::new::<_, ()>(Vec::new()));
                write_blocks(txn, &content, &blocks);
            }
        }
    }
    if let Some(b) = base {
        for cid in b.comments.keys() {
            if !cur.comments.contains_key(cid) {
                root.remove(txn, &cid.to_string());
            }
        }
    }
}

fn write_styles(txn: &mut TransactionMut, base: Option<&Document>, cur: &Document) {
    let root = txn.get_or_insert_map("styles");
    let defaults = |d: &Document| {
        json!({"defaultChr": d.styles.default_chr, "defaultPara": d.styles.default_para,
               "defaultParaPresent": d.styles.default_para_present, "latentStyles": d.styles.latent_styles})
    };
    let now = defaults(cur);
    if base.is_none_or(|b| defaults(b) != now) {
        root.insert(txn, STYLE_DEFAULTS, to_any(&now));
    }
    let before: HashMap<&str, &wordcraft_doc::Style> = base.map(|b| b.styles.styles.iter().map(|s| (s.id.as_str(), s)).collect()).unwrap_or_default();
    for st in &cur.styles.styles {
        if before.get(st.id.as_str()).is_none_or(|o| *o != st)
            && let Ok(v) = serde_json::to_value(st)
        {
            root.insert(txn, st.id.as_str(), to_any(&v));
        }
    }
    let ids: HashSet<&str> = cur.styles.styles.iter().map(|s| s.id.as_str()).collect();
    for id in before.keys() {
        if !ids.contains(id) {
            root.remove(txn, id);
        }
    }
    let order: Vec<&str> = cur.styles.styles.iter().map(|s| s.id.as_str()).collect();
    let old_order: Option<Vec<&str>> = base.map(|b| b.styles.styles.iter().map(|s| s.id.as_str()).collect());
    if old_order.as_ref() != Some(&order) {
        let arr: ArrayRef = txn.get_or_insert_array("styleOrder");
        let n = arr.len(txn);
        if n > 0 {
            arr.remove_range(txn, 0, n);
        }
        arr.insert_range(txn, 0, order.iter().map(|s| Any::String(Arc::from(*s))).collect::<Vec<_>>());
    }
}

fn write_numbering(txn: &mut TransactionMut, base: Option<&Document>, cur: &Document) {
    let root = txn.get_or_insert_map("numbering");
    let entries = |d: &Document| -> BTreeMap<String, Json> {
        let mut m = BTreeMap::new();
        for a in &d.numbering.abstracts {
            if let Ok(v) = serde_json::to_value(a) {
                m.insert(format!("a{}", a.id), v);
            }
        }
        for n in &d.numbering.nums {
            if let Ok(v) = serde_json::to_value(n) {
                m.insert(format!("n{}", n.id), v);
            }
        }
        m
    };
    let now = entries(cur);
    let before = base.map(entries).unwrap_or_default();
    for (k, v) in &now {
        if before.get(k) != Some(v) {
            root.insert(txn, k.as_str(), to_any(v));
        }
    }
    for k in before.keys() {
        if !now.contains_key(k) {
            root.remove(txn, k);
        }
    }
}
