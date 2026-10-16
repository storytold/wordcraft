//! craft-live mode: the web build as a live editor inside Nextcloud Office.
//!
//! craft-live (a WOPI editor that takes Collabora Online's place in Nextcloud) frames this build
//! as `index.html#craft-live=1&ws=…&room=…&token=…` and expects the client side of its embed
//! contract (craft-live `docs/design.md` §9): seed the room from the Nextcloud file or join it,
//! speak the y-websocket binding of plugin protocol 2 (Appendix B: lib0 framing, plus craft-live's
//! control messages, type 100) over a WebSocket, upload the file when the room saves, and keep
//! the page around it (the shell) informed with `craft-live:*` messages.
//!
//! This module is the whole protocol, without I/O, so it runs and is tested natively: [`Live`]
//! takes [`Event`]s (socket opened, a frame, the file fetched, a shell message…) and gives
//! [`Action`]s (open the socket, send a frame, fetch, upload, post to the shell…). The browser
//! glue (`web/live_io.rs`) only carries them out. The document side is the engine's own
//! co-editing (`wordcraft_engine::collab`), the same that desktop transports drive: the room is
//! joined through transport [`TRANSPORT`], served in memory, and its `collab/*` messages map one
//! to one onto y-websocket frames.
//!
//! Everything arriving here (the fragment, frames, control messages, HTTP answers) is hostile:
//! decoding is bounded and never panics.

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_plugin::push::{Incoming, Pushed};
use wordcraft_ui_egui::{i18n, tl};

/// The transport id the room is joined through (served in memory: this module is the transport).
pub const TRANSPORT: &str = "craft-live";
/// craft-live's control message type.
pub const CONTROL: u64 = 100;
/// Largest frame accepted (craft-live's limit: a 16 MiB update and some framing).
pub const MAX_FRAME: usize = 17 << 20;
/// Frames kept while the socket is down; beyond this they are dropped and the next sync step 1
/// brings the room up to date anyway.
const MAX_QUEUED_BYTES: usize = 64 << 20;
/// Reconnection delays (milliseconds), then the last one again.
const BACKOFF_MS: [u64; 6] = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000];
/// Most peers kept from a `peers` message.
const MAX_PEERS: usize = 1_000;

// ------------------------------------------------------------------------------------ framing

/// A y-websocket message (protocol 2, Appendix B) or a craft-live control message.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    /// `0/0`: sync step 1, a state vector.
    Sync1(Vec<u8>),
    /// `0/1`: sync step 2, an update.
    Sync2(Vec<u8>),
    /// `0/2`: an update.
    Update(Vec<u8>),
    /// `1`: a y-protocols awareness update.
    Awareness(Vec<u8>),
    /// `2/0`: permission denied, with the reason.
    Denied(String),
    /// `3`: asks for everyone's awareness.
    QueryAwareness,
    /// `100`: a craft-live control message (JSON).
    Control(Json),
}

/// Appends a lib0 varuint.
pub fn write_var(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Appends lib0 varbytes: the length, then the bytes.
pub fn write_bytes(out: &mut Vec<u8>, data: &[u8]) {
    write_var(out, data.len() as u64);
    out.extend_from_slice(data);
}

/// Reads lib0 values from a frame; `None` for anything that runs past the end.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn var(&mut self) -> Option<u64> {
        let mut n: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = *self.data.get(self.pos)?;
            self.pos += 1;
            if shift > 63 {
                return None;
            }
            let part = u64::from(b & 0x7f);
            if shift == 63 && part > 1 {
                return None;
            }
            n |= part << shift;
            if b & 0x80 == 0 {
                return Some(n);
            }
            shift += 7;
        }
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.var()?).ok()?;
        let end = self.pos.checked_add(len)?;
        let out = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Frame::Sync1(d) => {
                out.extend_from_slice(&[0, 0]);
                write_bytes(&mut out, d);
            }
            Frame::Sync2(d) => {
                out.extend_from_slice(&[0, 1]);
                write_bytes(&mut out, d);
            }
            Frame::Update(d) => {
                out.extend_from_slice(&[0, 2]);
                write_bytes(&mut out, d);
            }
            Frame::Awareness(d) => {
                out.push(1);
                write_bytes(&mut out, d);
            }
            Frame::Denied(reason) => {
                out.extend_from_slice(&[2, 0]);
                write_bytes(&mut out, reason.as_bytes());
            }
            Frame::QueryAwareness => out.push(3),
            Frame::Control(v) => {
                write_var(&mut out, CONTROL);
                write_bytes(&mut out, v.to_string().as_bytes());
            }
        }
        out
    }

    /// `None` for anything malformed, unknown or too large.
    pub fn decode(data: &[u8]) -> Option<Frame> {
        if data.len() > MAX_FRAME {
            return None;
        }
        let mut r = Reader { data, pos: 0 };
        Some(match r.var()? {
            0 => {
                let step = r.var()?;
                let payload = r.bytes()?.to_vec();
                match step {
                    0 => Frame::Sync1(payload),
                    1 => Frame::Sync2(payload),
                    2 => Frame::Update(payload),
                    _ => return None,
                }
            }
            1 => Frame::Awareness(r.bytes()?.to_vec()),
            2 => {
                if r.var()? != 0 {
                    return None;
                }
                Frame::Denied(String::from_utf8_lossy(r.bytes()?).chars().take(300).collect())
            }
            3 => Frame::QueryAwareness,
            CONTROL => {
                let v: Json = serde_json::from_slice(r.bytes()?).ok()?;
                if !v.is_object() {
                    return None;
                }
                Frame::Control(v)
            }
            _ => return None,
        })
    }
}

/// Percent-encodes everything but the unreserved characters (`X-Craft-File-Name`, the token in
/// the WebSocket URL).
pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` and `+` (a space, as in form encoding); `None` for a bad escape or bad UTF-8.
pub fn unpct(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'%' => {
                let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

// ------------------------------------------------------------------------------ configuration

/// The embed configuration (craft-live `docs/design.md` §9.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The room's WebSocket URL (`ws://` or `wss://`); connect to `<ws>?token=<token>`.
    pub ws: String,
    pub room: String,
    /// The session token (secret): the WebSocket's `token` and the API's bearer token.
    pub token: String,
    /// The person, `{id, name, color}`, made valid for a room (`color` `#RRGGBB`).
    pub user: Json,
    /// `role=view`: the document is read-only.
    pub view: bool,
    /// This client seeds the room from the file.
    pub seed: bool,
    /// Where the file's bytes are fetched (seeders).
    pub file: String,
    /// The file's name (`Report.docx`).
    pub name: String,
    /// Its format (extension), to save in.
    pub format: String,
    /// The API base (`https://live.example.com/api/v1`).
    pub api: String,
    /// Nextcloud's interface language (may be empty).
    pub lang: String,
}

/// A person's colour from their id, for a `user` without a usable one.
fn color_for(id: &str) -> &'static str {
    const PALETTE: [&str; 8] = ["#3B7DD8", "#D8553B", "#2E9E5B", "#9B4FD0", "#D89B1F", "#1F9BB0", "#C23B7A", "#6B7F2A"];
    let h = id.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(usize::from(b)));
    PALETTE.get(h % PALETTE.len()).copied().unwrap_or("#3B7DD8")
}

/// Up to `max` characters of `s`, without control characters, trimmed.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

fn http_url(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://")) && s.len() <= 4096
}

impl Config {
    /// Reads `#craft-live=1&ws=…&…` (the page's fragment, with or without `#`). `None`: not a
    /// craft-live page; `Some(Err)`: one that can't be used.
    pub fn from_fragment(hash: &str) -> Option<Result<Config, String>> {
        let hash = hash.strip_prefix('#').unwrap_or(hash);
        let mut map = serde_json::Map::new();
        for part in hash.split('&').take(64) {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let (Some(k), Some(v)) = (unpct(k), unpct(v)) else { continue };
            map.insert(k, Json::String(v));
        }
        if map.get("craft-live").and_then(Json::as_str) != Some("1") {
            return None;
        }
        // `user` is JSON text, `seed` 1 or 0.
        if let Some(u) = map.get("user").and_then(Json::as_str).and_then(|t| serde_json::from_str::<Json>(t).ok()) {
            map.insert("user".into(), u);
        }
        let seed = map.get("seed").and_then(Json::as_str) == Some("1");
        map.insert("seed".into(), Json::Bool(seed));
        Some(Config::from_json(&Json::Object(map)))
    }

    /// Reads the configuration as an object (`craft-live:config`'s `config`, or the fragment's).
    pub fn from_json(v: &Json) -> Result<Config, String> {
        let s = |k: &str| v.get(k).and_then(Json::as_str).unwrap_or("").trim().to_string();
        let ws = s("ws");
        if !(ws.starts_with("wss://") || ws.starts_with("ws://")) || ws.len() > 4096 {
            return Err("craft-live: `ws` isn't a WebSocket URL".into());
        }
        let room = s("room");
        if room.is_empty() || room.chars().count() > 256 || room.chars().any(char::is_control) {
            return Err("craft-live: `room` needs 1–256 characters".into());
        }
        let token = s("token");
        if token.is_empty() || token.len() > 4096 || token.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("craft-live: no session token".into());
        }
        let u = v.get("user").cloned().unwrap_or(Json::Null);
        let field = |k: &str| u.get(k).and_then(Json::as_str).map(|t| clean(t, 128)).unwrap_or_default();
        let mut id = field("id");
        if id.is_empty() {
            id = format!("guest-{}", pct(&token).chars().rev().take(8).collect::<String>());
        }
        let mut name = field("name");
        if name.is_empty() {
            name = id.clone();
        }
        let color = Some(field("color")).filter(|c| wordcraft_engine::collab::valid_color(c)).unwrap_or_else(|| color_for(&id).to_string());
        let seed = v.get("seed").and_then(|b| b.as_bool().or_else(|| b.as_str().map(|t| t == "1"))).unwrap_or(false);
        let file = s("file");
        if seed && !http_url(&file) {
            return Err("craft-live: a seeding client needs the file's URL".into());
        }
        let api = s("api");
        Ok(Config {
            ws,
            room,
            token,
            user: json!({"id": id, "name": name, "color": color}),
            // Anything but `edit` is read-only: the server drops a viewer's updates anyway.
            view: s("role") != "edit",
            seed,
            file,
            name: clean(&s("name"), 255),
            format: clean(&s("format"), 16).trim_start_matches('.').to_ascii_lowercase(),
            api: if http_url(&api) { api.trim_end_matches('/').to_string() } else { String::new() },
            lang: clean(&s("lang"), 16),
        })
    }

    /// `<ws>?token=<token>`.
    pub fn socket_url(&self) -> String {
        format!("{}{}token={}", self.ws, if self.ws.contains('?') { '&' } else { '?' }, pct(&self.token))
    }

    /// Nextcloud's language as an interface language code (`pt_BR` → `pt-br`), when WordCraft has
    /// it ([`i18n::normalize_pref`]); the language alone (`de_AT` → `de`) otherwise.
    pub fn interface_language(&self) -> Option<String> {
        let l = self.lang.replace('_', "-").to_ascii_lowercase();
        if l.is_empty() {
            return None;
        }
        let first = l.split('-').next().unwrap_or("").to_string();
        i18n::normalize_pref(&l).or_else(|| i18n::normalize_pref(&first)).filter(|c| *c != i18n::AUTO).map(str::to_string)
    }
}

// ---------------------------------------------------------------------------- events, actions

/// What happened, for [`Live::handle`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The WebSocket opened.
    Open,
    /// A binary message arrived.
    Frame(Vec<u8>),
    /// The WebSocket closed (or failed to open: code 1006).
    Closed { code: u16, reason: String },
    /// The file's bytes, or why they couldn't be fetched.
    File(Result<Vec<u8>, String>),
    /// An upload's answer: HTTP status and body, or why it didn't reach the server.
    Uploaded { save: u64, result: Result<(u16, String), String> },
    /// `POST <api>/prepare`'s answer.
    Prepared(Result<Json, String>),
    /// A message from the shell (the page embedding the app), already checked to be same-origin.
    Shell(Json),
    /// The person pressed Save (File › Save, Ctrl+S).
    Save,
    /// The person closed the document (File › Close, the title bar's Close).
    Close,
}

/// What the browser glue does next.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open the WebSocket (binary frames) at this URL.
    Connect(String),
    /// Send a binary frame.
    Send(Vec<u8>),
    /// Close the WebSocket.
    Disconnect(u16, String),
    /// `postMessage` this object to the shell (`targetOrigin` = our own origin).
    Shell(Json),
    /// `GET url` with `Authorization: Bearer token`; the answer comes back as [`Event::File`].
    Fetch { url: String, token: String },
    /// `PUT url` with the bearer token, `X-Craft-File-Name: name` (already percent-encoded) and
    /// `bytes`; the answer comes back as [`Event::Uploaded`].
    Upload { save: u64, url: String, token: String, name: String, bytes: Vec<u8> },
    /// `POST url` with the bearer token; the answer comes back as [`Event::Prepared`].
    Prepare { url: String, token: String },
    /// A word for the status bar.
    Status(String),
    /// Switch the interface to this language.
    Language(String),
}

/// Where the live session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not started.
    Idle,
    /// Fetching the file to seed the room.
    Fetching,
    /// In the room, waiting for its document ("Joining…", read-only).
    Waiting,
    /// The document is shown and edited live.
    Live,
    /// The person closed the document (or another one replaced it).
    Closed,
    /// The session expired (close 4401): the file must be opened again.
    Expired,
    /// The document can't be opened (or craft-live refused the connection).
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Socket {
    Down,
    Connecting,
    Open,
}

/// A `save` waiting for the engine's file bytes.
struct PendingSave {
    request: String,
    save: u64,
    url: String,
}

/// The client side of craft-live's embed contract (see the module's documentation).
pub struct Live {
    pub cfg: Config,
    pub phase: Phase,
    socket: Socket,
    /// The server's `welcome` arrived on this connection: frames may go out.
    welcomed: bool,
    /// We closed the socket ourselves: no reconnection when it says so.
    closing: bool,
    /// Frames waiting for the socket (and its `welcome`).
    queue: Vec<Vec<u8>>,
    queued_bytes: usize,
    attempts: u32,
    retry_at: Option<u64>,
    /// The room's document was built here from the file (`collab.share`) and the server hasn't
    /// confirmed it (`seeded`) yet.
    seeding: bool,
    /// The last `modified` told to the shell.
    modified: Option<bool>,
    /// The others in the room (`peers`).
    others: Vec<Json>,
    /// What's wrong, for the session's status (a save that failed, a copy, the session expired).
    message: Option<String>,
    /// The person (or Nextcloud) asked for a save: the shell hears how it went.
    save_asked: bool,
    /// The last save went to a copy: the next Save writes over the file (`saveNow {force}`).
    copied: bool,
    pending: Vec<PendingSave>,
    loaded: bool,
    /// The reason in a permission-denied message, for the close that follows it.
    denied: Option<String>,
    now: u64,
    out: Vec<Action>,
}

impl Live {
    pub fn new(cfg: Config) -> Live {
        Live {
            cfg,
            phase: Phase::Idle,
            socket: Socket::Down,
            welcomed: false,
            closing: false,
            queue: Vec::new(),
            queued_bytes: 0,
            attempts: 0,
            retry_at: None,
            seeding: false,
            modified: None,
            others: Vec::new(),
            message: None,
            save_asked: false,
            copied: false,
            pending: Vec::new(),
            loaded: false,
            denied: None,
            now: 0,
            out: Vec::new(),
        }
    }

    /// What to do, oldest first.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.out)
    }

    /// Whether the document has been shown (`craft-live:loaded` sent).
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// The `craft-live:hello` the app sends as soon as its script runs.
    pub fn hello() -> Json {
        json!({"type": "craft-live:hello"})
    }

    /// Starts: joins the room's document read-only ("Joining…"), then fetches the file (seeding)
    /// or connects (joining).
    pub fn start(&mut self, s: &mut Session, now: u64) {
        self.now = now;
        if self.phase != Phase::Idle {
            return;
        }
        if let Some(lang) = self.cfg.interface_language() {
            self.out.push(Action::Language(lang));
        }
        s.collab_serve_in_memory(TRANSPORT);
        if let Err(e) = self.join_engine(s, "join") {
            return self.fail(s, &e);
        }
        if self.cfg.seed {
            self.fetch();
        } else {
            self.phase = Phase::Waiting;
            self.connect();
        }
        self.pump(s);
    }

    /// Handles what happened since the last frame, in order. Consecutive room frames reach the
    /// engine together.
    pub fn handle_all(&mut self, s: &mut Session, events: Vec<Event>, now: u64) {
        self.now = now;
        let mut batch: Vec<Json> = Vec::new();
        for ev in events {
            match ev {
                Event::Frame(bytes) => match Frame::decode(&bytes) {
                    Some(Frame::Control(v)) => {
                        self.deliver(s, std::mem::take(&mut batch));
                        self.control(s, &v);
                    }
                    Some(Frame::Denied(why)) => {
                        log::warn!("craft-live: permission denied: {why}");
                        self.denied = Some(why);
                    }
                    Some(f) => {
                        if let Some(m) = self.to_engine(f) {
                            batch.push(m);
                        }
                    }
                    None => log::debug!("craft-live: dropped a frame that isn't y-websocket"),
                },
                other => {
                    self.deliver(s, std::mem::take(&mut batch));
                    self.handle(s, other);
                }
            }
        }
        self.deliver(s, batch);
        self.pump(s);
    }

    /// Handles one event.
    pub fn handle(&mut self, s: &mut Session, ev: Event) {
        match ev {
            Event::Frame(_) => self.handle_all(s, vec![ev], self.now),
            Event::Open => self.opened(s),
            Event::Closed { code, reason } => self.closed(s, code, reason),
            Event::File(r) => self.file(s, r),
            Event::Uploaded { save, result } => self.uploaded(save, result),
            Event::Prepared(r) => self.prepared(s, r),
            Event::Shell(m) => self.shell(s, &m),
            Event::Save => self.save_now(),
            Event::Close => {
                self.out.push(Action::Shell(json!({"type": "craft-live:close"})));
                self.close(s);
            }
        }
        self.pump(s);
    }

    /// Every frame: reconnects when it's time, and sends what the engine produced.
    pub fn tick(&mut self, s: &mut Session, now: u64) {
        self.now = now;
        if self.socket == Socket::Down
            && let Some(at) = self.retry_at
            && now >= at
        {
            self.retry_at = None;
            self.connect();
        }
        self.pump(s);
    }

    // ------------------------------------------------------------------------ the engine side

    fn join_engine(&mut self, s: &mut Session, mode: &str) -> Result<(), String> {
        let params = json!({
            "plugin": TRANSPORT,
            "room": self.cfg.room,
            "mode": mode,
            "user": self.cfg.user,
            "role": if self.cfg.view { "view" } else { "edit" },
            "title": self.cfg.name,
        });
        s.run("collab.join", &params).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Leaves the engine's room without telling the server (its messages are dropped).
    fn leave_engine(&mut self, s: &mut Session) {
        if s.active_room().is_some() {
            let _ = s.run("collab.leave", &json!({}));
        }
        s.collab_take_outgoing();
        self.queue.clear();
        self.queued_bytes = 0;
    }

    fn deliver(&mut self, s: &mut Session, msgs: Vec<Json>) {
        if msgs.is_empty() || s.active_room().is_none() {
            return;
        }
        let pushed = msgs.into_iter().map(|m| Pushed { plugin: TRANSPORT.into(), incoming: Incoming::Message(m) }).collect();
        s.collab_poll_pushed(self.now, pushed);
    }

    /// A room frame as the `collab/*` message the engine reads.
    fn to_engine(&self, f: Frame) -> Option<Json> {
        let b64 = wordcraft_collab::b64_encode;
        let (method, params) = match f {
            Frame::Sync1(sv) => ("collab/sync1", json!({"room": self.cfg.room, "stateVector": b64(&sv)})),
            Frame::Sync2(u) => ("collab/sync2", json!({"room": self.cfg.room, "update": b64(&u)})),
            Frame::Update(u) => ("collab/update", json!({"room": self.cfg.room, "update": b64(&u)})),
            Frame::Awareness(a) => ("collab/awareness", json!({"room": self.cfg.room, "update": b64(&a)})),
            Frame::Denied(_) | Frame::QueryAwareness | Frame::Control(_) => return None,
        };
        Some(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// The room's state for the engine (`collab/status`): the dot in the Share button, the
    /// people listed, and what's wrong in the status bar.
    fn engine_status(&mut self, s: &mut Session, state: &str) {
        let peers: Vec<Json> = self.others.iter().map(|p| json!({"user": p.get("user").cloned().unwrap_or(Json::Null)})).collect();
        let mut params = json!({"room": self.cfg.room, "state": state, "peers": peers});
        if let Some(m) = &self.message {
            params["message"] = json!(m);
        }
        self.deliver(s, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": params})]);
    }

    /// Sends what the engine produced: its `collab/*` messages as frames, its answers to our
    /// `collab/snapshot` requests as uploads.
    fn pump(&mut self, s: &mut Session) {
        for msg in s.collab_take_outgoing() {
            if let Some(id) = msg.get("id").and_then(Json::as_str) {
                self.snapshot_answer(id, &msg);
                continue;
            }
            let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
            let p = msg.get("params").cloned().unwrap_or(Json::Null);
            let bytes = |k: &str| p.get(k).and_then(Json::as_str).and_then(|t| wordcraft_collab::b64_decode(t).ok());
            let frame = match method {
                "collab/sync1" => bytes("stateVector").map(Frame::Sync1),
                "collab/sync2" => bytes("update").map(Frame::Sync2),
                "collab/update" => bytes("update").map(Frame::Update),
                "collab/awareness" => bytes("update").map(Frame::Awareness),
                "collab/leave" => {
                    // The engine left on its own (another document replaced this one).
                    if matches!(self.phase, Phase::Live | Phase::Waiting) {
                        self.send_now(&Frame::Control(json!({"type": "bye"})));
                        self.disconnect(1000, "left");
                        self.phase = Phase::Closed;
                    }
                    None
                }
                // `hello` (sent when the socket opens) says who we are.
                _ => None,
            };
            if let Some(f) = frame {
                self.send(f.encode());
            }
        }
        if self.phase == Phase::Waiting && s.active_room().is_some_and(|r| !r.joining) {
            self.phase = Phase::Live;
            self.show(s);
        }
    }

    fn show(&mut self, s: &mut Session) {
        if !self.loaded {
            self.loaded = true;
            self.out.push(Action::Shell(json!({"type": "craft-live:loaded"})));
            self.post_peers(s);
        }
    }

    // --------------------------------------------------------------------------- the socket

    fn connect(&mut self) {
        if self.socket != Socket::Down || matches!(self.phase, Phase::Closed | Phase::Expired | Phase::Failed) {
            return;
        }
        self.socket = Socket::Connecting;
        self.welcomed = false;
        self.closing = false;
        self.out.push(Action::Connect(self.cfg.socket_url()));
    }

    fn disconnect(&mut self, code: u16, reason: &str) {
        if self.socket != Socket::Down {
            self.closing = true;
            self.socket = Socket::Down;
            self.welcomed = false;
            self.out.push(Action::Disconnect(code, reason.into()));
        }
    }

    /// A frame for the room: now when the socket is up and welcomed, else queued.
    fn send(&mut self, frame: Vec<u8>) {
        if self.socket == Socket::Open && self.welcomed {
            self.out.push(Action::Send(frame));
            return;
        }
        self.queued_bytes = self.queued_bytes.saturating_add(frame.len());
        if self.queued_bytes > MAX_QUEUED_BYTES {
            // The next sync exchange brings the server whatever is dropped here.
            self.queue.clear();
            self.queued_bytes = frame.len();
        }
        self.queue.push(frame);
    }

    /// A frame that goes only if the socket is open (control messages).
    fn send_now(&mut self, f: &Frame) -> bool {
        if self.socket == Socket::Open {
            self.out.push(Action::Send(f.encode()));
            true
        } else {
            false
        }
    }

    fn opened(&mut self, s: &mut Session) {
        if self.socket != Socket::Connecting {
            return;
        }
        self.socket = Socket::Open;
        self.denied = None;
        let client = s.active_room().map(|r| r.client_id());
        let mut hello = json!({"type": "hello", "can": ["upload"], "client": format!("wordcraft-web/{}", env!("CARGO_PKG_VERSION"))});
        if let Some(c) = client {
            hello["clientId"] = json!(c);
        }
        self.send_now(&Frame::Control(hello));
    }

    fn closed(&mut self, s: &mut Session, code: u16, reason: String) {
        let ours = std::mem::replace(&mut self.closing, false) && self.socket == Socket::Down;
        self.socket = Socket::Down;
        self.welcomed = false;
        if ours || matches!(self.phase, Phase::Closed | Phase::Expired | Phase::Failed) {
            return;
        }
        match code {
            4401 => self.expired(s),
            4403 => {
                let why = self.denied.take().unwrap_or(reason);
                let msg = i18n::fmt(tl!("craft-live refused the connection: {reason}"), &[("reason", &why)]);
                self.fail(s, &msg);
            }
            _ => {
                let delay = BACKOFF_MS.get(self.attempts as usize).or(BACKOFF_MS.last()).copied().unwrap_or(30_000);
                self.attempts = self.attempts.saturating_add(1);
                self.retry_at = Some(self.now.saturating_add(delay));
                log::info!("craft-live: the connection closed ({code} {reason}); again in {} s", delay / 1000);
                if self.phase != Phase::Fetching {
                    self.engine_status(s, "offline");
                }
            }
        }
    }

    // ---------------------------------------------------------------------- control messages

    fn control(&mut self, s: &mut Session, v: &Json) {
        let b = |k: &str| v.get(k).and_then(Json::as_bool).unwrap_or(false);
        let text = |k: &str| v.get(k).and_then(Json::as_str).map(|t| clean(t, 600)).unwrap_or_default();
        match v.get("type").and_then(Json::as_str).unwrap_or("") {
            "welcome" => self.welcome(s, b("seed"), b("seeded"), b("modified")),
            "peers" => {
                self.others = v.get("peers").and_then(Json::as_array).map(|a| a.iter().take(MAX_PEERS).cloned().collect()).unwrap_or_default();
                self.post_peers(s);
                if self.welcomed {
                    self.engine_status(s, "online");
                }
            }
            "save" => self.save_request(s, v),
            "saved" => {
                let modified = b("modified");
                if self.message.take().is_some() {
                    self.engine_status(s, "online");
                }
                self.out.push(Action::Status(tl!("Saved to Nextcloud").to_string()));
                self.saved(true);
                self.set_modified(s, modified);
            }
            "saveFailed" => {
                let msg = i18n::fmt(tl!("Couldn't save to Nextcloud: {error}"), &[("error", &text("message"))]);
                self.out.push(Action::Status(msg.clone()));
                self.message = Some(msg);
                self.engine_status(s, "online");
                self.saved(false);
            }
            "modified" => self.set_modified(s, b("modified")),
            "seeded" => {
                self.seeding = false;
                self.set_modified(s, b("modified"));
            }
            "notice" => {
                let t = text("text");
                if t.is_empty() {
                    return;
                }
                self.out.push(Action::Status(t.clone()));
                if v.get("level").and_then(Json::as_str).is_some_and(|l| l != "info") {
                    // craft-live's words for a save that went to a copy (its text is English).
                    if t.contains("saved as") {
                        self.copied = true;
                        self.saved(true);
                    }
                    self.message = Some(t);
                    self.engine_status(s, "online");
                }
            }
            "reseed" => {
                let t = text("message");
                if !t.is_empty() {
                    self.out.push(Action::Status(t));
                }
                if self.cfg.view || self.cfg.api.is_empty() {
                    return;
                }
                self.out.push(Action::Prepare { url: format!("{}/prepare", self.cfg.api), token: self.cfg.token.clone() });
            }
            "expired" => self.expired(s),
            other => log::debug!("craft-live: ignored control `{other}`"),
        }
    }

    fn welcome(&mut self, s: &mut Session, seed: bool, seeded: bool, modified: bool) {
        if self.socket != Socket::Open {
            return;
        }
        self.attempts = 0;
        if self.seeding && !seed {
            // Our seed isn't wanted: the lease went to someone else, or the room already has a
            // document. Merging ours in would duplicate the content: join instead.
            log::info!("craft-live: the room is seeded by someone else; joining it");
            self.seeding = false;
            self.leave_engine(s);
            s.dirty = false;
            if let Err(e) = self.join_engine(s, "join") {
                return self.fail(s, &e);
            }
            self.phase = Phase::Waiting;
            self.loaded = false;
        } else if !self.seeding && seed && !seeded && self.phase == Phase::Waiting {
            // We hold the seed lease after all: load the file into this connection.
            self.welcomed = true;
            self.fetch();
        }
        self.welcomed = true;
        for f in std::mem::take(&mut self.queue) {
            self.out.push(Action::Send(f));
        }
        self.queued_bytes = 0;
        self.engine_status(s, "online");
        self.set_modified(s, modified && seeded);
    }

    fn set_modified(&mut self, s: &mut Session, modified: bool) {
        if !modified && s.active_room().is_some() {
            // The file in Nextcloud has every edit.
            s.dirty = false;
        }
        if self.modified != Some(modified) {
            self.modified = Some(modified);
            self.out.push(Action::Shell(json!({"type": "craft-live:modified", "modified": modified})));
        }
    }

    fn saved(&mut self, success: bool) {
        if std::mem::take(&mut self.save_asked) {
            self.out.push(Action::Shell(json!({"type": "craft-live:saved", "success": success})));
        }
    }

    fn expired(&mut self, s: &mut Session) {
        if self.phase == Phase::Expired {
            return;
        }
        let msg = tl!("The editing session expired. Close the document and open it again from Nextcloud.").to_string();
        self.out.push(Action::Status(msg.clone()));
        self.message = Some(msg);
        self.engine_status(s, "offline");
        self.phase = Phase::Expired;
        self.retry_at = None;
        self.disconnect(4401, "expired");
    }

    fn fail(&mut self, s: &mut Session, msg: &str) {
        log::error!("craft-live: {msg}");
        self.out.push(Action::Status(msg.to_string()));
        if !self.loaded {
            self.out.push(Action::Shell(json!({"type": "craft-live:error", "message": msg})));
        }
        self.message = Some(msg.to_string());
        self.engine_status(s, "offline");
        self.phase = Phase::Failed;
        self.retry_at = None;
        self.disconnect(1000, "failed");
    }

    // ------------------------------------------------------------------------------ seeding

    fn fetch(&mut self) {
        self.phase = Phase::Fetching;
        self.out.push(Action::Fetch { url: self.cfg.file.clone(), token: self.cfg.token.clone() });
    }

    /// The file arrived: it becomes the document and seeds the room (`collab.share`).
    fn file(&mut self, s: &mut Session, r: Result<Vec<u8>, String>) {
        if self.phase != Phase::Fetching {
            return;
        }
        let opened = r.and_then(|bytes| {
            if bytes.is_empty() {
                // A 0-byte file is a new, empty document.
                Ok(s.blank_document())
            } else {
                let region = s.system_region.clone();
                wordcraft_engine::io::open_bytes_in(&self.cfg.name, &bytes, None, &region).map(|(doc, _)| doc)
            }
        });
        let doc = match opened {
            Ok(d) => d,
            Err(e) => {
                let msg = i18n::fmt(tl!("Couldn't open the document from Nextcloud: {error}"), &[("error", &e)]);
                return self.fail(s, &msg);
            }
        };
        self.leave_engine(s);
        s.set_document(doc);
        s.file_name = Some(self.cfg.name.clone()).filter(|n| !n.is_empty());
        s.path = None;
        s.dirty = false;
        if let Err(e) = self.join_engine(s, "share") {
            return self.fail(s, &e);
        }
        self.seeding = true;
        self.phase = Phase::Live;
        self.show(s);
        if self.socket == Socket::Open && self.welcomed {
            // The connection holds the lease already and the server asked for our state before
            // we had one: send it whole (the first update makes the room seeded).
            if let Some((_, _, state)) = s.collab_recovery() {
                self.out.push(Action::Send(Frame::Update(state).encode()));
            }
        } else {
            self.connect();
        }
    }

    /// `POST <api>/prepare` answered after a `reseed`.
    fn prepared(&mut self, s: &mut Session, r: Result<Json, String>) {
        match r {
            Ok(v) if v.get("seed").and_then(Json::as_bool) == Some(true) && self.phase == Phase::Waiting => {
                // The lease is ours: a new connection seeds the room.
                self.disconnect(1000, "reseed");
                self.fetch();
            }
            Ok(_) => {}
            Err(e) => {
                log::warn!("craft-live: prepare: {e}");
                let _ = s;
            }
        }
    }

    // ------------------------------------------------------------------------------- saving

    /// `save {id, mode, url, name, format}`: the engine makes the file (`collab/snapshot {file:
    /// true, format}`), then it's uploaded.
    fn save_request(&mut self, s: &mut Session, v: &Json) {
        let Some(id) = v.get("id").and_then(Json::as_u64) else { return };
        let url = v.get("url").and_then(Json::as_str).unwrap_or("").to_string();
        let upload = v.get("mode").and_then(Json::as_str) == Some("upload");
        if !upload || !http_url(&url) || self.cfg.view || s.active_room().is_none() {
            self.save_result(id, "this client can only upload the file");
            return;
        }
        let format = v.get("format").and_then(Json::as_str).filter(|f| !f.is_empty()).unwrap_or(&self.cfg.format).to_string();
        let request = format!("craft-live-save-{id}");
        self.pending.retain(|p| p.save != id);
        self.pending.push(PendingSave { request: request.clone(), save: id, url });
        let req =
            json!({"jsonrpc": "2.0", "id": request, "method": "collab/snapshot", "params": {"room": self.cfg.room, "file": true, "format": format}});
        self.deliver(s, vec![req]);
    }

    fn snapshot_answer(&mut self, id: &str, msg: &Json) {
        let Some(i) = self.pending.iter().position(|p| p.request == id) else { return };
        let p = self.pending.remove(i);
        let r = msg.get("result");
        let file = r.and_then(|r| r.get("file")).and_then(Json::as_str).and_then(|t| wordcraft_collab::b64_decode(t).ok());
        let name = r.and_then(|r| r.get("name")).and_then(Json::as_str).unwrap_or(&self.cfg.name).to_string();
        match file {
            Some(bytes) => self.out.push(Action::Upload { save: p.save, url: p.url, token: self.cfg.token.clone(), name: pct(&name), bytes }),
            None => {
                let why = msg
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Json::as_str)
                    .or_else(|| r.and_then(|r| r.get("fileRefused")).and_then(Json::as_str))
                    .unwrap_or("no file")
                    .to_string();
                self.save_result(p.save, &why);
            }
        }
    }

    fn save_result(&mut self, id: u64, error: &str) {
        self.send_now(&Frame::Control(json!({"type": "saveResult", "id": id, "ok": false, "error": clean(error, 300)})));
    }

    fn uploaded(&mut self, save: u64, r: Result<(u16, String), String>) {
        match r {
            Ok((200, body)) => {
                let v: Json = serde_json::from_str(&body).unwrap_or(Json::Null);
                if v.get("copy").and_then(Json::as_str).is_some() {
                    self.copied = true;
                    self.saved(true);
                }
            }
            Ok((status, body)) => {
                let v: Json = serde_json::from_str(&body).unwrap_or(Json::Null);
                let why = v.get("error").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| format!("HTTP {status}"));
                log::warn!("craft-live: upload {save}: {why}");
                self.save_result(save, &why);
            }
            Err(e) => {
                log::warn!("craft-live: upload {save}: {e}");
                self.save_result(save, &e);
            }
        }
    }

    /// The person pressed Save: the room saves now (over the file after a copy).
    fn save_now(&mut self) {
        if self.cfg.view {
            self.out.push(Action::Status(tl!("This document is view only").to_string()));
            return;
        }
        let mut msg = json!({"type": "saveNow"});
        if self.copied {
            msg["force"] = json!(true);
        }
        if self.welcomed && self.send_now(&Frame::Control(msg)) {
            self.copied = false;
            self.save_asked = true;
            self.out.push(Action::Status(tl!("Saving to Nextcloud…").to_string()));
        } else {
            self.out.push(Action::Status(tl!("Not connected: Nextcloud saves the document once the connection is back").to_string()));
        }
    }

    // -------------------------------------------------------------------------------- shell

    fn shell(&mut self, s: &mut Session, m: &Json) {
        match m.get("type").and_then(Json::as_str).unwrap_or("") {
            // The shell already told the server: nothing to do but show it.
            "craft-live:save" => {
                if !self.cfg.view {
                    self.save_asked = true;
                    self.out.push(Action::Status(tl!("Saving to Nextcloud…").to_string()));
                }
            }
            "craft-live:close" => self.close(s),
            "craft-live:getPeers" => self.post_peers(s),
            // `craft-live:config` is read before starting; `craft-live:host` (Nextcloud's other
            // PostMessages) asks for nothing WordCraft does.
            _ => {}
        }
    }

    /// `craft-live:peers`: the others, and ourselves (`self: true`).
    fn post_peers(&mut self, s: &Session) {
        let mut peers: Vec<Json> = self.others.clone();
        let mut me = json!({"user": self.cfg.user, "role": if self.cfg.view { "view" } else { "edit" }, "self": true});
        if let Some(r) = s.active_room() {
            me["clientId"] = json!(r.client_id());
        }
        peers.push(me);
        self.out.push(Action::Shell(json!({"type": "craft-live:peers", "peers": peers})));
    }

    /// Leaves the room: `bye`, the socket closed, the document kept as it is.
    pub fn close(&mut self, s: &mut Session) {
        if matches!(self.phase, Phase::Closed | Phase::Idle) {
            return;
        }
        self.send_now(&Frame::Control(json!({"type": "bye"})));
        self.disconnect(1000, "closed");
        self.leave_engine(s);
        self.retry_at = None;
        self.phase = Phase::Closed;
    }
}

#[cfg(test)]
mod tests;
