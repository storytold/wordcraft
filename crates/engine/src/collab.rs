//! Co-editing (plugin protocol 2): the room, the `collab.*` commands, and the pump the app's main
//! loop runs between frames ([`Session::collab_poll`]).
//!
//! A room ties the open document to a transport plugin. The document is mirrored in a Yjs
//! document ([`wordcraft_collab::Replica`], WordCraft schema 1): every committed command becomes
//! one transaction sent as `collab/update` (typing as character inserts, formatting as Yjs
//! `format`); remote updates are applied between commands and mapped back into the model, only
//! the paragraphs they touched read again, and the caret stays with its text. While the
//! document is in a room, Undo and Redo are the person's own (they revert only their changes).
//! Transports only move bytes; with a room key every payload is encrypted end to end
//! (AES-256-GCM).

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use wordcraft_collab::awareness::{self, Awareness};
use wordcraft_collab::crypto::{self, Key};
use wordcraft_collab::{CollabError, Replica};
use wordcraft_doc::{Path, Pos, StoryRef};
use wordcraft_plugin::push::{Incoming, Pushed};

use crate::session::Selection;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The transport of rooms joined with [`Session::collab_join_in_memory`]: messages wait in the
/// room for the caller ([`Session::collab_take_outgoing`]) instead of going to a plugin. Tests
/// connect sessions with it.
pub const IN_MEMORY: &str = "(in-memory)";

/// Longest room id (protocol 2, section 2).
const MAX_ROOM: usize = 256;
/// After this many messages that don't decrypt, the status bar asks about the key.
const WRONG_KEY_AFTER: u32 = 10;
/// Awareness at most this often (20 a second).
const AWARENESS_MS: u64 = 50;
/// A transport that exited is started again after this long.
const RESTART_MS: u64 = 3_000;
/// Others' edits kept for the interface to highlight.
const MAX_REMOTE_EDITS: usize = 2_000;

/// A person in a room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub color: String,
}

impl User {
    fn parse(v: &Json) -> Option<User> {
        let s = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
        let u = User { id: s("id")?, name: s("name")?, color: s("color").unwrap_or_else(|| "#808080".into()) };
        u.valid().then_some(u)
    }
    fn valid(&self) -> bool {
        let len = |t: &str| (1..=128).contains(&t.chars().count());
        len(&self.id) && len(&self.name) && valid_color(&self.color)
    }
    /// One or two letters for the person's circle.
    pub fn initials(&self) -> String {
        let mut words = self.name.split_whitespace().filter_map(|w| w.chars().next());
        let first: String = words.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let last: String = words.next_back().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        if first.is_empty() { "?".into() } else { first + &last }
    }
}

/// `#RRGGBB`.
pub fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c.get(1..).is_some_and(|h| h.chars().all(|ch| ch.is_ascii_hexdigit()))
}

/// What the transport says about the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomState {
    Connecting,
    Online,
    Offline,
    Ended,
}

impl RoomState {
    fn parse(s: &str) -> Option<RoomState> {
        Some(match s {
            "connecting" => RoomState::Connecting,
            "online" => RoomState::Online,
            "offline" => RoomState::Offline,
            "ended" => RoomState::Ended,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            RoomState::Connecting => "connecting",
            RoomState::Online => "online",
            RoomState::Offline => "offline",
            RoomState::Ended => "ended",
        }
    }
}

/// Someone else in the room, as their awareness state says.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub client_id: u64,
    pub user: User,
    /// Their selection's ends in our document (the caret is `focus`), when it resolves.
    pub anchor: Option<Pos>,
    pub focus: Option<Pos>,
}

/// The shared document's live session.
pub struct Room {
    pub room: String,
    pub plugin: String,
    /// The session's `document_id` when it joined: another document ends the session.
    pub document: u64,
    pub state: RoomState,
    /// Waiting for the room's document (`mode: "join"`): the document is read-only.
    pub joining: bool,
    /// `view`: the document is read-only.
    pub view_only: bool,
    pub user: User,
    /// Peers the transport listed (`collab/status`).
    pub listed: Vec<User>,
    /// The transport's or the app's message for the status bar.
    pub message: Option<String>,
    /// Another app's or schema's room: nothing syncs.
    pub refused: Option<String>,
    /// The invitation link the transport gave, for Copy Invitation.
    pub invitation: Option<String>,
    key: Option<Key>,
    replica: Replica,
    awareness: Awareness,
    decrypt_failures: u32,
    last_awareness: Option<Json>,
    last_awareness_at: u64,
    /// The transport exited at this time (it's started again after [`RESTART_MS`]).
    exited_at: Option<u64>,
    /// The session's `rev` when the document was last synced.
    synced_rev: u64,
    /// The selection as Yjs relative positions, while remote changes are applied.
    pinned: Option<(Vec<u8>, Vec<u8>)>,
    outbox: RefCell<Vec<Json>>,
    in_memory: bool,
    /// Status-bar messages not shown yet ([`Session::collab_take_notices`]).
    notices: Vec<String>,
    /// This person is typing in the chat (awareness `typing: true`).
    typing: bool,
}

impl Room {
    pub fn client_id(&self) -> u64 {
        self.replica.client_id()
    }
    /// The room's title (`meta.title`).
    pub fn title(&self) -> Option<String> {
        self.replica.title()
    }
    pub fn encrypted(&self) -> bool {
        self.key.is_some()
    }
    /// Others present, from their awareness states.
    pub fn peers(&self) -> Vec<Peer> {
        self.awareness
            .present()
            .filter_map(|(id, st)| {
                let user = User::parse(st.get("user")?)?;
                let sel = st.get("selection");
                let pos = |k: &str| -> Option<Pos> {
                    let text = sel?.get(k)?.as_str()?;
                    let bytes = wordcraft_collab::b64_decode(text).ok()?;
                    self.replica.resolve(&bytes)
                };
                Some(Peer { client_id: id, user, anchor: pos("anchor"), focus: pos("focus") })
            })
            .collect()
    }
    /// The names of the people typing in the chat now (awareness `typing: true`), sorted.
    pub fn typing(&self) -> Vec<String> {
        let states: Vec<&Json> = self.awareness.present().map(|(_, st)| st).collect();
        wordcraft_collab::chat::typing(states)
    }
    fn info(&self) -> Json {
        let people: Vec<Json> = self.peers().iter().map(|p| json!({"clientId": p.client_id, "user": p.user})).collect();
        json!({
            "room": self.room,
            "plugin": self.plugin,
            "state": if self.joining { "joining" } else { self.state.as_str() },
            "peers": people,
            "listed": self.listed,
            "role": if self.view_only { "view" } else { "edit" },
            "encrypted": self.encrypted(),
            "clientId": self.client_id(),
            "user": self.user,
            "title": self.replica.title(),
            "invitation": self.invitation,
            "message": self.refused.clone().or_else(|| self.message.clone()),
            "canUndo": self.replica.can_undo(),
            "canRedo": self.replica.can_redo(),
        })
    }
}

/// Co-editing state of a session.
#[derive(Default)]
pub struct Collab {
    /// The document's live session, if any (a document is in at most one room).
    pub room: Option<Room>,
    /// Paragraphs others changed, for the interface to highlight briefly.
    pub remote_edits: Vec<(StoryRef, Path)>,
    /// Where room states are kept for recovery (the app's data folder); `None`: not kept.
    pub recovery_dir: Option<std::path::PathBuf>,
    /// The main loop's clock (milliseconds), as [`Session::collab_poll`] got it.
    now: u64,
    /// Transport ids the caller serves itself, in memory ([`Session::collab_serve_in_memory`]).
    in_process: Vec<String>,
    /// Answers to transports' requests and messages of rooms left, for
    /// [`Session::collab_take_outgoing`].
    answers: Vec<Json>,
}

impl Collab {
    /// Others' edits since the last call.
    pub fn take_remote_edits(&mut self) -> Vec<(StoryRef, Path)> {
        std::mem::take(&mut self.remote_edits)
    }
}

const SHARE: &str = "Title bar › Share";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("collab.share", "Start Live Session", SHARE, share)
            .params("{plugin, room, user?, role?, key?, title?, invitation?} → the room. Shares the open document in a live session through a transport plugin (protocol 2); with no params, the interface asks")
            .pure(),
        CommandSpec::new("collab.join", "Join Live Session", SHARE, join)
            .params("{plugin, room, mode: share|join, user?: {id, name, color}, role?: edit|view, key?: base64 of 32 bytes, path?, title?, invitation?} → {room, clientId, state}. `share` starts a room from the open document (or `path`); `join` replaces the document (unless it has unsaved changes) with the room's, or merges `path` into it")
            .pure(),
        CommandSpec::new("collab.leave", "Leave Live Session", SHARE, leave).params("{room?} → {left}. The document stays open with every edit, as an ordinary document").pure(),
        CommandSpec::new("collab.status", "Live Session", SHARE, status)
            .params("{} → {rooms: [{room, plugin, state, peers, role, encrypted, invitation, …}], transports: [{id, transport, label}], user}")
            .pure(),
        CommandSpec::new("collab.setUser", "Live Session Name and Colour", SHARE, set_user)
            .params("{name?, color?: #RRGGBB} → the user. How others see you in live sessions")
            .pure(),
    ]
}

fn bad(m: impl Into<String>) -> CmdError {
    CmdError::Params(m.into())
}

/// Who writes chat messages: the room's user in a live session, the app's own user otherwise.
pub(crate) fn chat_user(s: &mut Session) -> User {
    match &s.collab.room {
        Some(r) if r.document == s.document_id() => r.user.clone(),
        _ => user_from_prefs(s),
    }
}

/// A chat edit in a live session: applied to the room's shared document and sent. `None`: not
/// in a room (the document's own chat takes it). The messages a clear removed, and the chat.
pub(crate) fn chat_op(s: &mut Session, op: &wordcraft_collab::chat::ChatOp) -> Option<Result<(usize, wordcraft_doc::chat::Chat), CmdError>> {
    let doc_id = s.document_id();
    let r = s.collab.room.as_mut().filter(|r| r.document == doc_id)?;
    if let Some(why) = &r.refused {
        return Some(Err(CmdError::Disabled(why.clone())));
    }
    Some(match r.replica.chat(op) {
        Ok((update, removed, chat)) => {
            if let Some(u) = update {
                send_update(r, &u);
            }
            Ok((removed, chat))
        }
        Err(CollabError::Other(m)) => Err(CmdError::Params(m)),
        Err(e) => Err(CmdError::Failed(e.to_string())),
    })
}

fn user_from_prefs(s: &mut Session) -> User {
    if s.prefs.collab_user_id.is_empty() {
        let mut b = [0u8; 8];
        let v = if wordcraft_collab::random_bytes(&mut b).is_ok() { u64::from_le_bytes(b) } else { wordcraft_collab::new_client_id() };
        s.prefs.collab_user_id = format!("u-{v:016x}");
    }
    let palette = ["#3B7DD8", "#D8553B", "#2E9E5B", "#9B4FD0", "#D89B1F", "#1F9BB0", "#C23B7A", "#6B7F2A"];
    if !valid_color(&s.prefs.collab_color) {
        let i = s.prefs.collab_user_id.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) % palette.len();
        s.prefs.collab_color = palette.get(i).copied().unwrap_or("#3B7DD8").to_string();
    }
    let name: String = s.author.trim().chars().take(128).collect();
    User {
        id: s.prefs.collab_user_id.clone(),
        name: if name.is_empty() { "WordCraft User".into() } else { name },
        color: s.prefs.collab_color.clone(),
    }
}

fn share(s: &mut Session, p: &Json) -> CmdResult {
    if p.as_object().is_none_or(|o| o.is_empty()) {
        s.ui_requests.push(json!({"open": "collabShare"}));
        return Ok(json!({}));
    }
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.insert("mode".into(), json!("share"));
    }
    join(s, &p)
}

fn join(s: &mut Session, p: &Json) -> CmdResult {
    if p.as_object().is_none_or(|o| o.is_empty()) {
        s.ui_requests.push(json!({"open": "collabShare"}));
        return Ok(json!({}));
    }
    join_with(s, p, false)
}

fn join_with(s: &mut Session, prm: &Json, in_memory: bool) -> CmdResult {
    let plugin_id = match p::str(prm, "plugin") {
        Some(id) => id.to_string(),
        None if in_memory => IN_MEMORY.to_string(),
        None => return Err(bad("`plugin` (the transport) is required")),
    };
    let in_memory = in_memory || plugin_id == IN_MEMORY || s.collab.in_process.contains(&plugin_id);
    let room = p::req_str(prm, "room")?.to_string();
    if room.is_empty() || room.chars().count() > MAX_ROOM || room.chars().any(char::is_control) {
        return Err(bad("`room` needs 1–256 characters"));
    }
    let mode = p::str(prm, "mode").unwrap_or("join");
    if mode != "share" && mode != "join" {
        return Err(bad("`mode` is `share` or `join`"));
    }
    let view_only = match p::str(prm, "role").unwrap_or("edit") {
        "edit" => false,
        "view" => true,
        _ => return Err(bad("`role` is `edit` or `view`")),
    };
    let key = match p::str(prm, "key").filter(|k| !k.trim().is_empty()) {
        Some(k) => Some(Key::from_base64(k).map_err(|e| bad(format!("`key`: {e}")))?),
        None => None,
    };
    let mut user = user_from_prefs(s);
    if let Some(u) = prm.get("user").filter(|u| !u.is_null()) {
        user = User::parse(u).ok_or_else(|| bad("`user` needs `id` and `name` (1–128 characters) and `color` (#RRGGBB)"))?;
    }
    let title = p::str(prm, "title").map(|t| t.chars().take(256).collect::<String>());
    let path = p::str(prm, "path").filter(|x| !x.trim().is_empty()).map(str::to_string);
    let invitation = p::str(prm, "invitation").or_else(|| p::str(prm, "link")).map(|t| t.chars().take(4096).collect::<String>());
    if !in_memory {
        let plugin = wordcraft_plugin::get(&plugin_id).ok_or_else(|| CmdError::Failed(format!("no add-in `{plugin_id}` is loaded")))?;
        if !plugin.enabled() {
            return Err(CmdError::Failed(format!("add-in {plugin_id} is turned off (File › Options › Add-ins)")));
        }
        if let Some(b) = plugin.blocked_by().first() {
            return Err(CmdError::Failed(format!("add-in {plugin_id} is blocked: {}", b.message)));
        }
        if !plugin.is_transport() {
            return Err(CmdError::Failed(format!("add-in {plugin_id} isn't a co-editing transport (its manifest has no `collab`)")));
        }
    }
    if let Some(r) = &s.collab.room {
        return Err(CmdError::Failed(if r.room == room {
            format!("already in room {room}")
        } else {
            "this document is already in a live session".into()
        }));
    }
    if key.is_some() {
        // Encryption needs random nonces: refuse now rather than at the first edit.
        wordcraft_collab::random_bytes(&mut [0u8; 4]).map_err(|e| CmdError::Failed(e.to_string()))?;
    }
    // The document: the open one (or `path`) to share; a new one (or `path`) to join.
    match (&path, mode) {
        (Some(path), _) => {
            s.run("file.open", &json!({"path": path}))?;
        }
        (None, "join") => {
            if s.dirty {
                return Err(CmdError::Failed("the open document has unsaved changes: save it, or join from a new window".into()));
            }
            let blank = s.blank_document();
            s.set_document(blank);
            s.path = None;
            s.dirty = false;
        }
        (None, _) => {}
    }
    if !in_memory && let Some(plugin) = wordcraft_plugin::get(&plugin_id) {
        plugin.ensure_running().map_err(|e| CmdError::Failed(format!("can't start {plugin_id}: {e}")))?;
    }

    let client_id = wordcraft_collab::new_client_id();
    let mut replica = Replica::new(client_id);
    let mut joining = mode == "join";
    if mode == "share" {
        // The title: as given, else the file's name; an unsaved document has none.
        let title =
            title.clone().unwrap_or_else(|| s.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        replica.seed(&s.doc, &title).map_err(|e| CmdError::Failed(e.to_string()))?;
        s.doc.set_id_space(replica.id_space());
    } else if let Some(state) = read_recovery(s, &room) {
        // Our own edits from before a crash: merged with the room's when it answers.
        if replica.apply_update(&state).is_ok()
            && let Ok(Some(r)) = replica.refresh(&s.doc)
        {
            s.doc = r.doc;
            s.touch();
            s.clamp_selection();
            joining = false;
        }
    }
    let encrypted = key.is_some();
    let mut r = Room {
        room: room.clone(),
        plugin: plugin_id,
        document: s.document_id(),
        state: RoomState::Connecting,
        joining,
        view_only,
        user,
        listed: vec![],
        message: None,
        refused: None,
        invitation,
        key,
        awareness: Awareness::new(client_id),
        replica,
        decrypt_failures: 0,
        last_awareness: None,
        last_awareness_at: 0,
        exited_at: None,
        synced_rev: s.rev(),
        pinned: None,
        outbox: Default::default(),
        in_memory,
        notices: vec![],
        typing: false,
    };
    // A room's undo is the person's own: the document's history starts here.
    s.reset_history();
    announce(&mut r).map_err(CmdError::Failed)?;
    let out =
        json!({"room": room, "clientId": client_id, "mode": mode, "state": if r.joining { "joining" } else { "connecting" }, "encrypted": encrypted});
    s.collab.room = Some(r);
    send_awareness(s, true);
    Ok(out)
}

/// `collab/joined`, then sync step 1.
fn announce(r: &mut Room) -> Result<(), String> {
    let joined = json!({"room": r.room, "clientId": r.client_id(), "user": r.user, "app": wordcraft_collab::APP, "schema": wordcraft_collab::SCHEMA, "encrypted": r.encrypted()});
    let sync1 = json!({"room": r.room, "stateVector": wordcraft_collab::b64_encode(&r.replica.state_vector())});
    if r.in_memory {
        notify(r, "collab/joined", joined);
        notify(r, "collab/sync1", sync1);
    } else {
        let plugin = wordcraft_plugin::get(&r.plugin).ok_or("the transport was unloaded")?;
        plugin.notify("collab/joined", joined)?;
        plugin.notify("collab/sync1", sync1)?;
    }
    Ok(())
}

fn seal(r: &Room, method: &str, bytes: &[u8]) -> Result<String, CollabError> {
    let data = match &r.key {
        Some(k) => crypto::encrypt(k, &r.room, method, bytes)?,
        None => bytes.to_vec(),
    };
    Ok(wordcraft_collab::b64_encode(&data))
}

fn unseal(r: &mut Room, method: &str, text: &str) -> Result<Vec<u8>, CollabError> {
    let data = wordcraft_collab::b64_decode(text)?;
    match &r.key {
        Some(k) => crypto::decrypt(k, &r.room, method, &data).inspect_err(|_| {
            r.decrypt_failures += 1;
            if r.decrypt_failures >= WRONG_KEY_AFTER {
                r.message = Some("Wrong room key?".into());
                if r.decrypt_failures == WRONG_KEY_AFTER {
                    r.notices.push("Wrong room key?".into());
                }
            }
        }),
        None => Ok(data),
    }
}

fn notify(r: &Room, method: &str, params: Json) {
    if r.in_memory {
        r.outbox.borrow_mut().push(json!({"jsonrpc": "2.0", "method": method, "params": params}));
        return;
    }
    if let Some(p) = wordcraft_plugin::get(&r.plugin)
        && let Err(e) = p.notify(method, params)
    {
        log::warn!("collab {}: {method}: {e}", r.room);
    }
}

fn sync1(r: &Room) {
    notify(r, "collab/sync1", json!({"room": r.room, "stateVector": wordcraft_collab::b64_encode(&r.replica.state_vector())}));
}

fn leave(s: &mut Session, prm: &Json) -> CmdResult {
    let Some(r) = &s.collab.room else { return Err(CmdError::Failed("this document isn't in a live session".into())) };
    if let Some(room) = p::str(prm, "room")
        && room != r.room
    {
        return Err(CmdError::Failed(format!("not in room {room}")));
    }
    let room = r.room.clone();
    leave_room(s, None, true);
    Ok(json!({"left": room}))
}

/// Leaves the room: tells the transport (unless it asked), keeps the document as it is.
fn leave_room(s: &mut Session, reason: Option<&str>, tell: bool) {
    let Some(mut r) = s.collab.room.take() else { return };
    // The last change goes first, unless another document replaced the room's.
    if s.document_id() == r.document {
        flush(s, &mut r, false);
    }
    if tell {
        let bye = r.awareness.set_local(None);
        if let Ok(u) = seal(&r, "collab/awareness", &bye) {
            notify(&r, "collab/awareness", json!({"room": r.room, "update": u}));
        }
        notify(&r, "collab/leave", json!({"room": r.room, "reason": reason.unwrap_or("left")}));
    }
    s.collab.answers.extend(std::mem::take(&mut *r.outbox.borrow_mut()));
    remove_recovery(s, &r.room);
    // Back to an ordinary document: its own undo starts afresh; new ids as usual.
    s.reset_history();
    s.doc.set_id_space(wordcraft_doc::IdSpace::default());
    if let Some(why) = reason
        && !tell
    {
        s.status = format!("The live session ended: {why}");
        s.ui_requests.push(json!({"status": s.status}));
    }
}

fn status(s: &mut Session, _p: &Json) -> CmdResult {
    let rooms: Vec<Json> = s.collab.room.iter().map(Room::info).collect();
    let transports: Vec<Json> = wordcraft_plugin::transports()
        .iter()
        .map(|p| {
            let c = p.manifest.collab.clone().unwrap_or_default();
            json!({"id": p.manifest.id, "transport": c.transport, "label": if c.label.is_empty() { p.manifest.name.clone() } else { c.label }})
        })
        .collect();
    Ok(json!({"rooms": rooms, "transports": transports, "user": user_from_prefs(s)}))
}

fn set_user(s: &mut Session, prm: &Json) -> CmdResult {
    if let Some(n) = p::str(prm, "name") {
        let n = n.trim();
        if n.is_empty() || n.chars().count() > 128 {
            return Err(bad("`name` needs 1–128 characters"));
        }
        s.author = n.to_string();
    }
    if let Some(c) = p::str(prm, "color") {
        if !valid_color(c) {
            return Err(bad("`color` is #RRGGBB"));
        }
        s.prefs.collab_color = c.to_ascii_uppercase();
    }
    let user = user_from_prefs(s);
    if let Some(r) = s.collab.room.as_mut() {
        r.user = User { id: r.user.id.clone(), ..user.clone() };
        r.last_awareness = None;
    }
    send_awareness(s, true);
    Ok(serde_json::to_value(&user).unwrap_or(Json::Null))
}

// ---------------------------------------------------------------- the engine's side

/// Why the document can't be edited now (in a room while joining, as a viewer, or in a room for
/// another app).
pub fn read_only(s: &Session) -> Option<String> {
    let r = s.collab.room.as_ref()?;
    if let Some(why) = &r.refused {
        return Some(why.clone());
    }
    if r.joining {
        return Some("Joining…".into());
    }
    r.view_only.then(|| "You can view this document (live session, view only)".into())
}

/// `edit.undo`, `edit.undoTo` and `edit.redo` in a room: the person's own changes. `None`: not
/// in a room.
pub(crate) fn intercept(s: &mut Session, id: &str, prm: &Json) -> Option<CmdResult> {
    let undo = match id {
        "edit.undo" | "edit.undoTo" => true,
        "edit.redo" => false,
        _ => return None,
    };
    let mut r = s.collab.room.take()?;
    if let Some(why) = read_only_room(&r) {
        s.collab.room = Some(r);
        return Some(Err(CmdError::Disabled(format!("{id}: {why}"))));
    }
    let steps = prm.get("count").or_else(|| prm.get("steps")).and_then(Json::as_u64).unwrap_or(1).clamp(1, 100);
    // Anything not yet sent goes first (it becomes the step being undone); the selection is
    // kept as relative positions before the shared document changes.
    flush(s, &mut r, false);
    pin(&mut r, s);
    let mut did = 0;
    let mut result = Ok(());
    for _ in 0..steps {
        let step = if undo { r.replica.undo() } else { r.replica.redo() };
        match step {
            Ok(Some(update)) => {
                did += 1;
                send_update(&r, &update);
            }
            Ok(None) => break,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    if did > 0 {
        match r.replica.refresh(&s.doc) {
            Ok(Some(out)) => apply_refreshed(s, &mut r, out, false),
            Ok(None) => {}
            Err(e) => result = Err(e),
        }
    }
    r.pinned = None;
    s.collab.room = Some(r);
    Some(match result {
        Err(e) => Err(CmdError::Failed(e.to_string())),
        Ok(()) if did == 0 => Err(CmdError::Disabled(format!("{id}: {}", if undo { "nothing to undo" } else { "nothing to redo" }))),
        Ok(()) => {
            let mut out = crate::cmd::sel_result(s).unwrap_or(Json::Null);
            if let Some(o) = out.as_object_mut() {
                o.insert("perUser".into(), json!(true));
                o.insert(if undo { "undone" } else { "redone" }.into(), json!(did));
            }
            Ok(out)
        }
    })
}

fn read_only_room(r: &Room) -> Option<String> {
    if let Some(why) = &r.refused {
        return Some(why.clone());
    }
    if r.joining {
        return Some("Joining…".into());
    }
    r.view_only.then(|| "You can view this document (live session, view only)".into())
}

/// Whether Undo / Redo have something to do in the room. `None`: not in one.
pub fn room_undo(s: &Session, redo: bool) -> Option<bool> {
    let r = s.collab.room.as_ref()?;
    Some(r.replica.is_ready() && if redo { r.replica.can_redo() } else { r.replica.can_undo() })
}

fn send_update(r: &Room, update: &[u8]) {
    if update.len() > wordcraft_collab::MAX_UPDATE {
        log::warn!("collab {}: an update of {} bytes is larger than 16 MiB; peers will fetch it with a sync", r.room, update.len());
        sync1(r);
        return;
    }
    match seal(r, "collab/update", update) {
        Ok(u) => notify(r, "collab/update", json!({"room": r.room, "update": u})),
        Err(e) => log::warn!("collab {}: {e}", r.room),
    }
}

/// Sends the change since the last sync, if any. `join`: it continues the last undo step.
fn flush(s: &mut Session, r: &mut Room, join: bool) {
    if !r.replica.is_ready() || r.refused.is_some() || s.rev() == r.synced_rev {
        return;
    }
    if r.view_only || r.joining {
        // A viewer's edits never leave: back to the room's state.
        if let Some(b) = r.replica.base() {
            s.doc = b.clone();
            s.touch();
            s.clamp_selection();
        }
        r.synced_rev = s.rev();
        return;
    }
    match r.replica.commit(&s.doc, join) {
        Ok(Some(update)) => send_update(r, &update),
        Ok(None) => {}
        Err(e) => {
            log::warn!("collab {}: {e}", r.room);
            r.message = Some(e.to_string());
        }
    }
    r.synced_rev = s.rev();
}

/// After every top-level command: the document's change becomes one update. `join`: the
/// command continued typing (one undo step).
pub(crate) fn after_command(s: &mut Session, join: bool) {
    let Some(mut r) = s.collab.room.take() else { return };
    if s.document_id() != r.document {
        // Another document was opened: the session ends (the old one isn't open any more).
        s.collab.room = Some(r);
        leave_room(s, Some("the document was closed"), true);
        return;
    }
    flush(s, &mut r, join);
    s.collab.room = Some(r);
}

/// The selection as relative positions, so it stays with its text while remote changes come in.
fn pin(r: &mut Room, s: &Session) {
    if !r.replica.is_ready() || r.pinned.is_some() {
        return;
    }
    r.pinned = r.replica.sticky(&s.sel.anchor).zip(r.replica.sticky(&s.sel.focus));
}

/// Puts a refreshed document (remote changes, an undo) in place, the selection where its text
/// went.
fn apply_refreshed(s: &mut Session, r: &mut Room, out: wordcraft_collab::Refreshed, remote: bool) {
    s.doc = out.doc;
    // Others' changes (and an undo) aren't in the saved file yet.
    s.touch();
    match r.pinned.take() {
        Some((a, f)) => match (r.replica.resolve(&a), r.replica.resolve(&f)) {
            (Some(anchor), Some(focus)) => s.sel = Selection { anchor, focus },
            _ => s.clamp_selection(),
        },
        None => s.clamp_selection(),
    }
    s.clamp_selection();
    r.synced_rev = s.rev();
    if remote {
        for c in out.changed {
            if s.collab.remote_edits.len() >= MAX_REMOTE_EDITS {
                s.collab.remote_edits.remove(0);
            }
            s.collab.remote_edits.push(c);
        }
    }
}

impl Session {
    /// Runs co-editing between frames: takes what transports pushed, applies remote changes,
    /// answers snapshots and sends this person's selection. `now`: the caller's clock in
    /// milliseconds (the engine reads no clock: it would trap in the browser). Returns whether
    /// anything changed that the interface should show.
    pub fn collab_poll(&mut self, now: u64) -> bool {
        let pushed = wordcraft_plugin::push::take();
        self.collab_poll_pushed(now, pushed)
    }

    /// [`Session::collab_poll`] with the messages given (several sessions in one process share
    /// the plugins' queue: the caller splits it; [`IN_MEMORY`] rooms get theirs this way).
    pub fn collab_poll_pushed(&mut self, now: u64, pushed: Vec<Pushed>) -> bool {
        self.collab.now = now;
        let mut changed = false;
        if let Some(mut r) = self.collab.room.take() {
            flush(self, &mut r, false);
            if !r.replica.has_changes() {
                pin(&mut r, self);
            }
            self.collab.room = Some(r);
        }
        for p in pushed {
            changed |= handle(self, p);
        }
        if let Some(mut r) = self.collab.room.take() {
            if r.refused.is_none() && r.replica.has_changes() {
                let doc = self.doc.clone();
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.replica.refresh(&doc)));
                match res {
                    Ok(Ok(Some(out))) => {
                        let was_joining = r.joining;
                        r.joining = false;
                        apply_refreshed(self, &mut r, out, !was_joining);
                        if was_joining {
                            self.dirty = false;
                            self.sel = Selection::caret(self.doc.start_of(StoryRef::Body));
                            if let Some(t) = r.replica.title().filter(|t| !t.trim().is_empty()) {
                                self.file_name = Some(t);
                            }
                            r.last_awareness = None;
                        }
                        changed = true;
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(e @ CollabError::Mismatch { .. })) => {
                        r.notices.push(e.to_string());
                        r.refused = Some(e.to_string());
                        changed = true;
                    }
                    Ok(Err(e)) => {
                        r.message = Some(e.to_string());
                        changed = true;
                    }
                    Err(_) => {
                        r.message = Some("A change from the room couldn't be applied".into());
                        changed = true;
                    }
                }
            }
            r.pinned = None;
            if r.replica.has_gaps() && r.state == RoomState::Online {
                sync1(&r);
            }
            // A transport that exited: started again, then the room rejoins.
            if let Some(at) = r.exited_at
                && now.saturating_sub(at) >= RESTART_MS
            {
                r.exited_at = None;
                if let Some(plugin) = wordcraft_plugin::get(&r.plugin) {
                    match plugin.ensure_running() {
                        Ok(()) => {
                            r.state = RoomState::Connecting;
                            r.message = None;
                            if let Err(e) = announce(&mut r) {
                                r.message = Some(e);
                            }
                        }
                        Err(e) => {
                            r.message = Some(format!("{}: {e}", r.plugin));
                            if !e.starts_with("stopped after") {
                                r.exited_at = Some(now);
                            }
                        }
                    }
                }
                changed = true;
            }
            changed |= r.awareness.prune(now);
            self.collab.room = Some(r);
        }
        send_awareness(self, false);
        changed
    }

    /// Serves transport `id` in memory: rooms joined through it (`collab.join` with `plugin: id`)
    /// work like [`Session::collab_join_in_memory`]'s. A conformance replayer stands in for a
    /// transport this way.
    pub fn collab_serve_in_memory(&mut self, id: &str) {
        if !self.collab.in_process.iter().any(|x| x == id) {
            self.collab.in_process.push(id.to_string());
        }
    }

    /// Joins a room whose transport is the caller ([`IN_MEMORY`]): `collab.join`'s params
    /// without `plugin`. Its messages are taken with [`Session::collab_take_outgoing`]; what
    /// arrives is handed to [`Session::collab_poll_pushed`] as coming from [`IN_MEMORY`].
    pub fn collab_join_in_memory(&mut self, params: Json) -> CmdResult {
        join_with(self, &params, true)
    }

    /// Messages for the status bar since the last call ("This room is for gridcraft 1",
    /// "Wrong room key?").
    pub fn collab_take_notices(&mut self) -> Vec<String> {
        self.collab.room.as_mut().map(|r| std::mem::take(&mut r.notices)).unwrap_or_default()
    }

    /// Messages [`IN_MEMORY`] rooms sent, oldest first (JSON-RPC notifications and answers).
    pub fn collab_take_outgoing(&mut self) -> Vec<Json> {
        let mut out: Vec<Json> = std::mem::take(&mut self.collab.answers);
        if let Some(r) = &self.collab.room {
            out.extend(std::mem::take(&mut *r.outbox.borrow_mut()));
        }
        out
    }

    /// Whether this person is typing in the chat: others see "Name is typing…" (awareness
    /// `typing: true`). The chat panel calls it as the draft changes; sending ends it.
    pub fn chat_typing(&mut self, on: bool) {
        let Some(r) = self.collab.room.as_mut() else { return };
        if r.typing == on {
            return;
        }
        r.typing = on;
        send_awareness(self, true);
    }

    /// The people typing in the chat now (none outside a live session).
    pub fn chat_typing_names(&self) -> Vec<String> {
        self.collab.room.as_ref().map(Room::typing).unwrap_or_default()
    }

    /// The chat's own user id: the room's user in a live session, the app's own user otherwise
    /// (`None` before one was made).
    pub fn chat_user_id(&self) -> Option<String> {
        match &self.collab.room {
            Some(r) => Some(r.user.id.clone()),
            None => Some(self.prefs.collab_user_id.clone()).filter(|id| !id.is_empty()),
        }
    }

    /// The document's live session.
    pub fn active_room(&self) -> Option<&Room> {
        self.collab.room.as_ref()
    }

    /// The room's whole state, for recovery: `(room, plugin, update)`.
    pub fn collab_recovery(&self) -> Option<(String, String, Vec<u8>)> {
        let r = self.collab.room.as_ref().filter(|r| r.replica.is_ready())?;
        Some((r.room.clone(), r.plugin.clone(), r.replica.full_state()))
    }

    /// Writes [`Session::collab_recovery`] into the recovery folder (the app does this with its
    /// recovery timer).
    pub fn save_collab_recovery(&self) {
        let Some(dir) = &self.collab.recovery_dir else { return };
        let Some((room, plugin, state)) = self.collab_recovery() else { return };
        let body = json!({"room": room, "plugin": plugin, "update": wordcraft_collab::b64_encode(&state)});
        if let Err(e) = room_file::write(dir, &recovery_name(&room), &body.to_string()) {
            log::warn!("collab: can't keep the room's state for recovery: {e}");
        }
    }

    /// The document's text shared in the room (diagnostics, tests): see
    /// [`wordcraft_collab::canonical`].
    pub fn collab_fingerprint(&self) -> Json {
        wordcraft_collab::canonical(&self.doc)
    }
}

/// Recovery files: the desktop's folder; the browser keeps none.
#[cfg(not(target_arch = "wasm32"))]
use crate::io::rooms as room_file;

#[cfg(target_arch = "wasm32")]
mod room_file {
    use std::path::Path;
    pub fn write(_: &Path, _: &str, _: &str) -> Result<(), String> {
        Err("the browser keeps no recovery files".into())
    }
    pub fn read(_: &Path, _: &str) -> Option<String> {
        None
    }
    pub fn remove(_: &Path, _: &str) {}
}

/// A file name for a room's recovery state (two FNV-1a hashes of the room id).
fn recovery_name(room: &str) -> String {
    let h = |seed: u64| room.bytes().fold(seed, |a, b| (a ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!("room-{:016x}{:08x}.json", h(0xcbf2_9ce4_8422_2325), h(0x6c62_272e_07bb_0142) as u32)
}

fn read_recovery(s: &Session, room: &str) -> Option<Vec<u8>> {
    let dir = s.collab.recovery_dir.as_ref()?;
    let text = room_file::read(dir, &recovery_name(room))?;
    let v: Json = serde_json::from_str(&text).ok()?;
    if v.get("room").and_then(Json::as_str) != Some(room) {
        return None;
    }
    wordcraft_collab::b64_decode(v.get("update")?.as_str()?).ok()
}

fn remove_recovery(s: &Session, room: &str) {
    if let Some(dir) = &s.collab.recovery_dir {
        room_file::remove(dir, &recovery_name(room));
    }
}

/// The story as awareness names it.
fn story_name(s: StoryRef) -> String {
    match s {
        StoryRef::Body => "body".into(),
        StoryRef::Part(id) => id.to_string(),
    }
}

/// This person's selection to the room (at most 20 a second, renewed every 15 s).
fn send_awareness(s: &mut Session, force: bool) {
    let now = s.collab.now;
    let Some(r) = s.collab.room.as_mut() else { return };
    if !r.replica.is_ready() || r.refused.is_some() || r.replica.has_changes() {
        return;
    }
    let mut state = json!({"user": r.user});
    if r.typing {
        state["typing"] = json!(true);
    }
    if let (Some(a), Some(f)) = (r.replica.sticky(&s.sel.anchor), r.replica.sticky(&s.sel.focus)) {
        state["selection"] = json!({
            "story": story_name(s.sel.focus.story),
            "anchor": wordcraft_collab::b64_encode(&a),
            "focus": wordcraft_collab::b64_encode(&f),
        });
    }
    let same = r.last_awareness.as_ref() == Some(&state);
    let due = now.saturating_sub(r.last_awareness_at) >= awareness::RENEW_MS;
    let allowed = force || now.saturating_sub(r.last_awareness_at) >= AWARENESS_MS || now < r.last_awareness_at;
    if (same && !due) || !allowed {
        return;
    }
    let update = r.awareness.set_local(Some(state.clone()));
    r.last_awareness = Some(state);
    r.last_awareness_at = now;
    if let Ok(u) = seal(r, "collab/awareness", &update) {
        notify(r, "collab/awareness", json!({"room": r.room, "update": u}));
    }
}

/// Handles one pushed message. Returns whether the interface should repaint.
fn handle(s: &mut Session, p: Pushed) -> bool {
    let msg = match p.incoming {
        Incoming::Message(m) => m,
        Incoming::Exited(why) => {
            let now = s.collab.now;
            let mut any = false;
            if let Some(r) = s.collab.room.as_mut().filter(|r| r.plugin == p.plugin) {
                r.state = RoomState::Offline;
                r.message = Some(format!("Offline: {why}"));
                r.exited_at = Some(now);
                any = true;
            }
            if let Some(plugin) = wordcraft_plugin::get(&p.plugin) {
                plugin.exited();
                if any {
                    plugin.count_failure(&why);
                }
            }
            return any;
        }
        Incoming::Overlong => {
            if let Some(plugin) = wordcraft_plugin::get(&p.plugin) {
                plugin.count_failure("a message larger than 64 MiB was dropped");
            }
            return false;
        }
        Incoming::Flood => {
            if let Some(plugin) = wordcraft_plugin::get(&p.plugin) {
                plugin.fail("stopped: it sent more than 1,000 messages a second");
            }
            if let Some(r) = s.collab.room.as_mut().filter(|r| r.plugin == p.plugin) {
                r.state = RoomState::Offline;
                r.message = Some(format!("{} was stopped: it flooded the app", p.plugin));
            }
            return true;
        }
    };
    let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
    let params = msg.get("params").cloned().unwrap_or(Json::Null);
    // Requests: answered now, in any order.
    if let Some(id) = msg.get("id").cloned() {
        let answer = match method.as_str() {
            "host/log" => {
                let text: String = params.get("text").and_then(Json::as_str).unwrap_or("").chars().take(4000).collect();
                log::info!("plugin {}: {text}", p.plugin);
                Ok(json!({}))
            }
            "collab/snapshot" => snapshot(s, &p.plugin, &params),
            // Protocol 2.2's invalidation is for formula functions, which WordCraft doesn't have:
            // unknown here as everywhere (not "not now").
            "host/functions/invalidate" => Err(wordcraft_plugin::HostError::unknown()),
            m if m.starts_with("host/") => Err(wordcraft_plugin::HostError {
                code: -32000,
                message: format!("{m} is only available while the add-in runs a command"),
                data: json!({"code": "NotNow"}),
            }),
            _ => Err(wordcraft_plugin::HostError::unknown()),
        };
        if p.plugin == IN_MEMORY || s.collab.in_process.contains(&p.plugin) {
            let msg = match answer {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": e.code, "message": e.message, "data": e.data}}),
            };
            s.collab.answers.push(msg);
        } else if let Some(plugin) = wordcraft_plugin::get(&p.plugin)
            && let Err(e) = plugin.respond(&id, answer)
        {
            log::warn!("plugin {}: answering {method}: {e}", p.plugin);
        }
        return false;
    }
    let Some(room) = params.get("room").and_then(Json::as_str).map(str::to_string) else {
        if !method.is_empty() && !method.starts_with("host/") {
            log::debug!("plugin {}: ignored notification {method}", p.plugin);
        }
        return false;
    };
    let now = s.collab.now;
    let Some(r) = s.collab.room.as_mut().filter(|r| r.room == room) else {
        log::debug!("plugin {}: {method} for a room we're not in", p.plugin);
        return false;
    };
    if r.plugin != p.plugin {
        log::warn!("plugin {}: {method} for room {room}, which belongs to {}", p.plugin, r.plugin);
        return false;
    }
    match method.as_str() {
        "collab/sync1" => {
            let Some(sv) = params.get("stateVector").and_then(Json::as_str).and_then(|t| wordcraft_collab::b64_decode(t).ok()) else {
                return false;
            };
            if r.refused.is_some() {
                return false;
            }
            match r.replica.diff_for(&sv) {
                Ok(diff) => match seal(r, "collab/sync2", &diff) {
                    Ok(u) => {
                        let mut out = json!({"room": r.room, "update": u});
                        if let Some(to) = params.get("from").or_else(|| params.get("clientId")).filter(|c| c.is_u64()) {
                            out["to"] = to.clone();
                        }
                        notify(r, "collab/sync2", out);
                        // The asker has changes we lack: ask back.
                        if r.replica.lacks(&sv) {
                            sync1(r);
                        }
                    }
                    Err(e) => log::warn!("collab {room}: {e}"),
                },
                Err(e) => log::warn!("collab {room}: sync1: {e}"),
            }
            false
        }
        "collab/sync2" | "collab/update" => {
            if let Some(to) = params.get("to").and_then(Json::as_u64)
                && to != r.client_id()
            {
                return false;
            }
            if r.refused.is_some() {
                return false;
            }
            let Some(text) = params.get("update").and_then(Json::as_str) else { return false };
            // A stored snapshot (the room's whole state) is sealed as `collab/snapshot`.
            let sealed_as = if method == "collab/sync2" && params.get("snapshot").and_then(Json::as_bool) == Some(true) {
                "collab/snapshot"
            } else {
                method.as_str()
            };
            match unseal(r, sealed_as, text).and_then(|u| r.replica.apply_update(&u)) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("collab {room}: {method}: {e}");
                    match e {
                        CollabError::Decrypt => {}
                        CollabError::Mismatch { .. } => {
                            r.message = Some(e.to_string());
                            r.notices.push(e.to_string());
                        }
                        e => r.message = Some(e.to_string()),
                    }
                    true
                }
            }
        }
        "collab/awareness" => {
            let Some(text) = params.get("update").and_then(Json::as_str) else { return false };
            match unseal(r, "collab/awareness", text).and_then(|u| r.awareness.apply(&u, now)) {
                Ok(changed) => changed,
                Err(e) => {
                    log::debug!("collab {room}: awareness: {e}");
                    false
                }
            }
        }
        "collab/status" => {
            if let Some(st) = params.get("state").and_then(Json::as_str).and_then(RoomState::parse) {
                let back_online = st == RoomState::Online && r.state != RoomState::Online;
                r.state = st;
                if back_online {
                    sync1(r);
                    r.last_awareness = None;
                    r.last_awareness_at = 0;
                }
            }
            if let Some(peers) = params.get("peers").and_then(Json::as_array) {
                r.listed = peers.iter().take(1000).filter_map(|p| p.get("user").and_then(User::parse)).collect();
            }
            r.message = params.get("message").and_then(Json::as_str).map(|m| m.chars().take(300).collect());
            if r.state == RoomState::Ended {
                let why = r.message.clone().unwrap_or_else(|| "the room was closed".into());
                leave_room(s, Some(&why), false);
            }
            true
        }
        "collab/leave" => {
            let why = params.get("reason").and_then(Json::as_str).unwrap_or("the transport closed the room").to_string();
            leave_room(s, Some(&why), false);
            true
        }
        _ => {
            log::debug!("plugin {}: ignored {method}", p.plugin);
            false
        }
    }
}

/// The formats `collab/snapshot {file: true, format}` writes: Word's packages, ODT and RTF.
pub const SNAPSHOT_FORMATS: &[&str] = &["docx", "docm", "dotx", "dotm", "odt", "rtf"];

/// The format a snapshot's file is written in: `format` (an extension, with or without the dot,
/// any case) when it is one of [`SNAPSHOT_FORMATS`], else `docx`.
pub fn snapshot_format(format: Option<&str>) -> &'static str {
    let f = format.map(|f| f.trim().trim_start_matches('.').to_ascii_lowercase()).unwrap_or_default();
    SNAPSHOT_FORMATS.iter().copied().find(|x| *x == f).unwrap_or("docx")
}

/// `base` named for `format`: kept when it already has that extension, its document extension
/// replaced (`Report.odt` → `Report.docx`), or the extension added.
pub fn file_name_as(base: &str, format: &str) -> String {
    const DOC_EXTS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "rtf", "txt", "md", "html", "htm", "xml", "json"];
    match base.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case(format) && !stem.is_empty() => base.to_string(),
        Some((stem, ext)) if !stem.is_empty() && DOC_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) => format!("{stem}.{format}"),
        _ => format!("{base}.{format}"),
    }
}

/// `collab/snapshot`: the room's whole state, and the document as a file when asked (not in an
/// encrypted room: the transport would see the content).
fn snapshot(s: &mut Session, plugin: &str, params: &Json) -> Result<Json, wordcraft_plugin::HostError> {
    let err = |m: String| wordcraft_plugin::HostError { code: -32000, message: m, data: Json::Null };
    let room = params.get("room").and_then(Json::as_str).ok_or_else(|| err("missing `room`".into()))?.to_string();
    let Some(mut r) = s.collab.room.take() else { return Err(err(format!("not in room {room}"))) };
    if r.room != room || r.plugin != plugin {
        s.collab.room = Some(r);
        return Err(err(format!("not in room {room}")));
    }
    flush(s, &mut r, false);
    let out = (|| {
        if !r.replica.is_ready() {
            return Err(err("the room's document hasn't arrived yet".into()));
        }
        let update = seal(&r, "collab/snapshot", &r.replica.full_state()).map_err(|e| err(e.to_string()))?;
        let mut out = json!({"update": update});
        if params.get("file").and_then(Json::as_bool) == Some(true) {
            if r.key.is_some() {
                out["fileRefused"] = json!("the room is encrypted: the document stays with the people in it");
            } else {
                let base = s.file_name.clone().or_else(|| r.replica.title()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Document".into());
                // The format the transport asks for (the file's own, e.g. `.odt`), when WordCraft
                // writes it; `.docx` otherwise (craft-live then saves a copy beside the file).
                let format = snapshot_format(params.get("format").and_then(Json::as_str));
                let name = file_name_as(&base, format);
                let bytes = crate::io::save_bytes(&name, &s.doc).map_err(err)?;
                out["file"] = json!(wordcraft_collab::b64_encode(&bytes));
                out["name"] = json!(name);
                out["format"] = json!(format);
            }
        }
        Ok(out)
    })();
    s.collab.room = Some(r);
    out
}
