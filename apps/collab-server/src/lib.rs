//! Authoritative sync server for WordCraft collaboration.
//!
//! One room per document; clients authenticate with a JWT in the first
//! WebSocket message (ADR-0008), send normalized operations (ADR-0001) and
//! receive the server's authoritative answer. Presence is ephemeral and never
//! journaled (ADR-0006). The file is assembled by the server on commit
//! (ADR-0009) and, when `COLLAB_CABINET_URL` names one, posted to the cabinet
//! as its next file version (`cabinet.rs`, M87).
//!
//! A stored snapshot that cannot be opened refuses the join instead of starting
//! a blank room that would be written over it (ADR WC-0003).
//!
//! The room logic itself lives in `wordcraft_collab`; this crate is the
//! transport: WebSocket framing, tokens, rooms, presence fan-out, snapshots,
//! metrics and the commit call.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval, timeout};

use cabinet::{Cabinet, CabinetConfig, Publication, PublishError, Published};
use sha2::Digest;
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{CabinetCommit, ClientMsg, Participant, Reject, RejectCode, ServerMsg, Version};
use wordcraft_collab_engine::SessionModel;

pub mod cabinet;
pub mod store;

use store::{Corrupt, MAX_MODEL, Snapshot, Store};

/// Largest WebSocket frame the server accepts (RFC §9).
pub const MAX_FRAME: usize = 1 << 20;
/// A document the server will hold and commit (matches the reader's budget).
pub const MAX_DOCUMENT: usize = 64 << 20;
/// Snapshot chunk size: a large document is sent in several frames.
pub const SNAPSHOT_CHUNK: usize = 512 * 1024;
/// A client that does not authenticate within this window is disconnected.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Presence is dropped after this without a heartbeat (ADR-0006).
pub const PRESENCE_TTL: Duration = Duration::from_secs(30);
/// Frames a peer's connection may hold before the peer counts as not reading
/// (ADR WC-0002).
pub const PEER_QUEUE: usize = 512;

/// Server configuration from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    /// Shared secret for HS256 tokens (local development and tests).
    pub jwt_secret: Option<String>,
    /// PEM public key for ES256 tokens (production, per ADR-0008).
    pub jwt_pem: Option<String>,
    /// Where snapshots are written (ADR-0005, ADR-0011).
    pub data_dir: PathBuf,
    /// How long a cabinet session may be silent before another session may take
    /// the lease (ADR-0008). `0` for tests.
    pub lease_ttl_secs: u64,
    /// The cabinet commit callback (M87). `None` keeps the commit server-side
    /// only: the file is built and the host owns the storage.
    pub cabinet: Option<CabinetConfig>,
    /// How often a connected peer is pinged, and the longest any single write
    /// to it may take before the peer counts as gone (ADR WC-0002). Also the
    /// interval at which a peer whose outbox overflowed is closed.
    pub heartbeat: Duration,
    /// Frames one peer's outbox holds before the peer counts as not reading
    /// (ADR WC-0002). 512 in production; tests keep it small so the overflow is
    /// reachable without a stalled megabyte-sized socket buffer.
    pub peer_queue: usize,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let bind = std::env::var("COLLAB_BIND").unwrap_or_else(|_| "127.0.0.1:8087".to_string());
        let jwt_secret = std::env::var("COLLAB_JWT_SECRET").ok();
        let jwt_pem = std::env::var("COLLAB_JWT_PEM").ok();
        if jwt_secret.is_none() && jwt_pem.is_none() {
            return Err("set COLLAB_JWT_SECRET (HS256, local) or COLLAB_JWT_PEM (ES256 public key)".into());
        }
        let data_dir = std::env::var("COLLAB_DATA_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("collab-data"));
        let lease_ttl_secs = std::env::var("COLLAB_LEASE_TTL_SECS").ok().and_then(|value| value.parse().ok()).unwrap_or(120);
        let cabinet = cabinet_from_env()?;
        Ok(Config { bind, jwt_secret, jwt_pem, data_dir, lease_ttl_secs, cabinet, heartbeat: PRESENCE_TTL / 3, peer_queue: PEER_QUEUE })
    }
}

/// The commit callback from `COLLAB_CABINET_URL` / `COLLAB_CABINET_TOKEN`.
/// The token is a secret: it is read here, kept in `CabinetConfig` (whose
/// `Debug` redacts it) and never written to a log, a metric or a URL.
fn cabinet_from_env() -> Result<Option<CabinetConfig>, String> {
    let url = std::env::var("COLLAB_CABINET_URL").ok().map(|url| url.trim().to_string()).filter(|url| !url.is_empty());
    let Some(url) = url else {
        return Ok(None);
    };
    if !url.starts_with("http://") {
        return Err("COLLAB_CABINET_URL must start with http:// (TLS is terminated by the ingress; https:// is not supported)".into());
    }
    if !url.contains("{sessionId}") {
        return Err("COLLAB_CABINET_URL must contain {sessionId}: the endpoint is /api/v1/internal/office/sessions/{sessionId}/commit".into());
    }
    let token = std::env::var("COLLAB_CABINET_TOKEN").ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty());
    let Some(token) = token else {
        return Err("COLLAB_CABINET_TOKEN is required with COLLAB_CABINET_URL".into());
    };
    let timeout_ms = std::env::var("COLLAB_CABINET_TIMEOUT_MS").ok().and_then(|value| value.parse().ok()).unwrap_or(cabinet::DEFAULT_TIMEOUT_MS);
    let attempts = std::env::var("COLLAB_CABINET_ATTEMPTS").ok().and_then(|value| value.parse().ok()).unwrap_or(cabinet::DEFAULT_ATTEMPTS).max(1);
    Ok(Some(CabinetConfig { url, token, timeout: Duration::from_millis(timeout_ms.max(1)), attempts }))
}

/// JWT claims the cabinet issues (ADR-0008).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub aud: String,
    pub doc: String,
    pub app: String,
    pub role: String,
    pub sid: String,
    pub iat: u64,
    pub exp: u64,
}

/// One connected client.
#[derive(Clone)]
struct Peer {
    /// Server-assigned identity of *this connection*, not of the `client_id`. A
    /// connection removes a peer only while this id matches, so the `leave` of a
    /// socket the server closed cannot remove the connection that replaced it
    /// (ADR WC-0005).
    conn: u64,
    /// The account and the cabinet session the connection authenticated as. A
    /// second connection may take a `client_id` over only when both match: that
    /// is the same client reconnecting, never another participant (ADR WC-0005).
    sub: String,
    sid: String,
    role: Role,
    out: mpsc::Sender<ServerMsg>,
    /// Set when a document frame could not be queued for this peer: its outbox
    /// is full, so it has already missed an accepted operation. Its connection
    /// ends instead of staying open with a document the room has moved past
    /// (ADR WC-0002).
    stale: Arc<Retire>,
    /// Set when a newer connection of the same client registered this
    /// `client_id`. This connection ends so two sockets never share one
    /// `(clientId, seq)` space, and its `leave` is a no-op (ADR WC-0005).
    superseded: Arc<Retire>,
}

impl Peer {
    fn new(conn: u64, sub: &str, role: Role, sid: &str, out: mpsc::Sender<ServerMsg>) -> Peer {
        Peer {
            conn,
            sub: sub.to_string(),
            sid: sid.to_string(),
            role,
            out,
            stale: Arc::new(Retire::default()),
            superseded: Arc::new(Retire::default()),
        }
    }
}

/// A one-shot signal that ends one connection: its outbox overflowed and it
/// missed an accepted operation (ADR WC-0002), or a newer connection took its
/// `client_id` over (ADR WC-0005). The flag is what the connection loop reads,
/// the notification is what wakes it out of a write that is stuck because the
/// peer stopped reading. Without the notification the loop is inside that write
/// and cannot look at the flag.
#[derive(Default)]
struct Retire {
    flag: AtomicBool,
    wake: tokio::sync::Notify,
}

impl Retire {
    /// Set the signal; `true` the first time only.
    fn mark(&self) -> bool {
        let first = !self.flag.swap(true, Ordering::Relaxed);
        self.wake.notify_one();
        first
    }
    fn is_set(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Resolves when the signal is set. The permit `notify_one` leaves behind is
    /// taken here, so a mark that lands between two writes is not lost.
    async fn wait(&self) {
        self.wake.notified().await;
    }
}

impl std::fmt::Debug for Retire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Notify` is not `Debug`; the flag is what matters in a failure message.
        f.debug_struct("Retire").field("set", &self.is_set()).finish()
    }
}

/// Source of connection identities. Peers are keyed by the `client_id` the
/// client owns, so the server needs an id of its own to tell one connection from
/// the connection that replaced it (ADR WC-0005).
static CONNECTIONS: AtomicU64 = AtomicU64::new(1);

fn next_connection() -> u64 {
    CONNECTIONS.fetch_add(1, Ordering::Relaxed)
}

/// A document room and its connected peers.
struct RoomState {
    room: Room<SessionModel>,
    peers: HashMap<String, Peer>,
    /// Cabinet session that holds the lease; a different session may join only
    /// as `owner` or after the lease went silent (ADR-0008).
    session: String,
    lease_seen: std::time::Instant,
    /// Lease TTL for this room.
    lease_ttl_secs: u64,
    /// Fixed-window operation counter, so one client cannot flood a room.
    window: std::time::Instant,
    ops_in_window: u32,
    /// The cabinet's copy of the last accepted commit: the etag the next
    /// commit is checked against (M87). Stored with the snapshot, so a restart
    /// keeps the conflict check.
    published: Option<Published>,
    /// The file the room built for a commit the cabinet refused with `409`, so
    /// the owner can save a copy without overwriting the cabinet (TASK.md §6).
    /// Kept in memory only: it is not part of the document and a restart loses
    /// nothing the cabinet did not already refuse.
    refused: Option<Refused>,
}

/// A built `.docx` the cabinet refused with `409`, kept for the owner's
/// "save a copy" action (TASK.md §6).
#[derive(Clone)]
struct Refused {
    version: Version,
    sha256: String,
    warnings: Vec<String>,
    /// Shared so a request for the copy does not copy the whole file.
    bytes: Arc<Vec<u8>>,
}

/// Why a join did not open a room (ADR WC-0003, ADR WC-0004).
#[derive(Debug)]
enum Refusal {
    /// A snapshot is stored and cannot be opened.
    Corrupt(Corrupt),
    /// The document has no state on this server and the client brought no copy.
    Unseeded,
    /// The document the client offered is not a model this server can hold.
    BadSeed(String),
}

/// Read a document from the model JSON a snapshot or a joining client holds.
/// The message is the reason, not the content: it goes into a log line or a
/// refusal, never into an answer that carries the document.
fn read_model(bytes: &[u8]) -> Result<wordcraft_doc::Document, String> {
    serde_json::from_slice::<wordcraft_doc::Document>(bytes).map_err(|err| format!("is not a document: {err}"))
}

impl RoomState {
    fn new(room: Room<SessionModel>, lease_ttl_secs: u64, published: Option<Published>) -> RoomState {
        RoomState {
            room,
            peers: HashMap::new(),
            session: String::new(),
            lease_seen: std::time::Instant::now(),
            lease_ttl_secs,
            window: std::time::Instant::now(),
            ops_in_window: 0,
            published,
            refused: None,
        }
    }
}

/// Counters for `/metrics`.
#[derive(Default)]
pub struct Metrics {
    connections: AtomicU64,
    ops: AtomicU64,
    rejects: AtomicU64,
    commits: AtomicU64,
    presence: AtomicU64,
    /// Commits the cabinet accepted (M87).
    published: AtomicU64,
    /// Commits the cabinet refused with `409` (the file changed elsewhere).
    publish_conflicts: AtomicU64,
    /// Commits not stored by the cabinet: a wrong token, a size refusal, an
    /// exhausted retry or any other refusal.
    publish_errors: AtomicU64,
    /// Refused copies the owner took to save (TASK.md §6).
    copies: AtomicU64,
    /// Peer outboxes that overflowed: each one missed a document frame and was
    /// closed (ADR WC-0002).
    stale: AtomicU64,
    /// Joins refused because the stored snapshot of the document cannot be
    /// opened (ADR WC-0003). Non-zero means an operator has a file to restore.
    corrupt: AtomicU64,
    /// Joins refused because the document has no state here and the client
    /// brought no copy (ADR WC-0004). Non-zero means the first client to open
    /// the document was older than the seeding step, or a read-only session
    /// arrived before a writer did.
    unseeded: AtomicU64,
    /// Joins refused because the `client_id` they registered belongs to another
    /// participant (ADR WC-0005). Non-zero means a client is using an id that is
    /// not its own.
    client_ids: AtomicU64,
}

impl Metrics {
    fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self, rooms: usize) -> String {
        let mut out = String::from("# HELP wordcraft_collab_rooms Open document rooms\n# TYPE wordcraft_collab_rooms gauge\n");
        out.push_str(&format!("wordcraft_collab_rooms {rooms}\n"));
        for (name, value) in [
            ("connections_total", self.connections.load(Ordering::Relaxed)),
            ("ops_applied_total", self.ops.load(Ordering::Relaxed)),
            ("rejects_total", self.rejects.load(Ordering::Relaxed)),
            ("commits_total", self.commits.load(Ordering::Relaxed)),
            ("presence_total", self.presence.load(Ordering::Relaxed)),
            ("cabinet_published_total", self.published.load(Ordering::Relaxed)),
            ("cabinet_conflicts_total", self.publish_conflicts.load(Ordering::Relaxed)),
            ("cabinet_errors_total", self.publish_errors.load(Ordering::Relaxed)),
            ("save_copies_total", self.copies.load(Ordering::Relaxed)),
            ("stale_peers_total", self.stale.load(Ordering::Relaxed)),
            ("corrupt_snapshots_total", self.corrupt.load(Ordering::Relaxed)),
            ("unseeded_joins_total", self.unseeded.load(Ordering::Relaxed)),
            ("client_id_conflicts_total", self.client_ids.load(Ordering::Relaxed)),
        ] {
            out.push_str(&format!("# TYPE wordcraft_collab_{name} counter\nwordcraft_collab_{name} {value}\n"));
        }
        out
    }
}

/// Shared state of the server.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    rooms: Arc<Mutex<HashMap<String, Arc<Mutex<RoomState>>>>>,
    pub metrics: Arc<Metrics>,
    store: Store,
    /// The cabinet commit callback, when configured.
    cabinet: Option<Cabinet>,
}

impl AppState {
    pub fn new(cfg: Config) -> AppState {
        let store = Store::new(&cfg.data_dir);
        let cabinet = cfg.cabinet.as_ref().map(Cabinet::new);
        AppState { cfg: Arc::new(cfg), rooms: Arc::new(Mutex::new(HashMap::new())), metrics: Arc::new(Metrics::default()), store, cabinet }
    }

    /// Write the room's snapshot (ADR-0005): model JSON plus the source `.docx`
    /// when one is known, plus the `opId` window so retries survive a restart,
    /// plus the cabinet etag when the room published a commit (M87).
    fn persist(&self, doc: &str, room: &Arc<Mutex<RoomState>>, source: Option<Vec<u8>>) -> Result<(), String> {
        let (version, epoch, model, dedup, cabinet) = {
            let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            (guard.room.version(), guard.room.epoch(), guard.room.model().snapshot(), guard.room.dedup(), guard.published.clone())
        };
        self.store.save(doc, &Snapshot { version, epoch, model, source, dedup, cabinet })
    }

    /// The room of `doc`, restored from its stored snapshot when one exists, or
    /// built from the document the joining client offered when it never had one.
    ///
    /// The room is never started blank (ADR WC-0004). `Err(Refusal::Corrupt)`
    /// means a snapshot is stored and cannot be opened: a blank room would be
    /// persisted over that file on the first operation or on the next shutdown
    /// and would destroy the only copy of the document (ADR WC-0003).
    /// `Err(Refusal::Unseeded)` means this server holds nothing for the document
    /// and the client brought no copy, so any room it could open would be an
    /// empty page whose snapshot replaces the document the client has open.
    /// The caller refuses the join and leaves the file for an operator.
    ///
    /// The flag says the room was built from this client's seed; it answers the
    /// join without a snapshot, because the client already holds that state.
    fn room_for(&self, doc: &str, seed: Option<Vec<u8>>, role: Role) -> Result<(Arc<Mutex<RoomState>>, bool), Refusal> {
        // A room that is already open costs one lock and no disk work.
        if let Some(room) = self.open_room(doc) {
            return Ok((room, false));
        }
        // Loading and parsing happen with no lock of the room map held: the
        // model is read under a 64 MiB bound and parsed, and doing that inside
        // the map would stall every other join, `/metrics` and `persist_all`
        // (CodeRabbit 4533747484). The map is taken again only to publish the
        // room, and a join that won the race is reused instead of replaced.
        let built = self.build_room(doc, seed, role);
        let mut rooms = self.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(room) = rooms.get(doc) {
            return Ok((room.clone(), false));
        }
        let (state, seeded) = built?;
        let room = Arc::new(Mutex::new(state));
        rooms.insert(doc.to_string(), room.clone());
        Ok((room, seeded))
    }

    /// The room of `doc` if one is open already.
    fn open_room(&self, doc: &str) -> Option<Arc<Mutex<RoomState>>> {
        self.rooms.lock().unwrap_or_else(|poison| poison.into_inner()).get(doc).cloned()
    }

    /// Build the room of `doc` from its stored snapshot, or from the copy the
    /// joining client offered when the document was never snapshotted.
    ///
    /// Touches no shared lock beyond the store's own reads: the caller decides
    /// whether the result is published. Two joins of a never-opened document
    /// can therefore both build it; the loser's copy is dropped and the room
    /// that is already in the map is the one they share.
    fn build_room(&self, doc: &str, seed: Option<Vec<u8>>, role: Role) -> Result<(RoomState, bool), Refusal> {
        let (state, seeded) = match self.store.load(doc) {
            Ok(Some(snapshot)) => {
                let mut document =
                    // The store already checked; this keeps the room correct on
                    // its own instead of trusting that.
                    read_model(&snapshot.model).map_err(|reason| Refusal::Corrupt(Corrupt::new("model.json", reason)))?;
                // `Document::kept` is not serialized (ADR-0011): take it from
                // the source `.docx` so unchanged paragraphs still write their
                // original markup. Fingerprints only match paragraphs that
                // were not edited since the file was read.
                if let Some(source) = &snapshot.source
                    && let Ok(from_file) = wordcraft_docx::read_with_password(source, None)
                {
                    document.kept = from_file.kept;
                }
                (
                    RoomState::new(
                        Room::restore(SessionModel::from_document(document), snapshot.version, snapshot.epoch, snapshot.dedup),
                        self.cfg.lease_ttl_secs,
                        snapshot.cabinet,
                    ),
                    false,
                )
            }
            Ok(None) => {
                // Only a client that may write can create the room: a read-only
                // session arriving first waits for one that may edit.
                let Some(model) = seed.filter(|_| role.can_write()) else { return Err(Refusal::Unseeded) };
                let document = read_model(&model).map_err(Refusal::BadSeed)?;
                log::info!("opening the room for {doc} from a joining client's copy ({} bytes of model)", model.len());
                (RoomState::new(Room::new(SessionModel::from_document(document)), self.cfg.lease_ttl_secs, None), true)
            }
            Err(corrupt) => {
                // The refusal is counted here, where the damage is found; the
                // join path logs it with the path an operator has to look at.
                Metrics::inc(&self.metrics.corrupt);
                return Err(Refusal::Corrupt(corrupt));
            }
        };
        Ok((state, seeded))
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().unwrap_or_else(|poison| poison.into_inner()).len()
    }

    /// Write every open room before the process exits. A container gets
    /// SIGTERM; without this the last operations since the periodic snapshot
    /// would be lost.
    pub fn persist_all(&self) -> Result<(), String> {
        let rooms: Vec<(String, Arc<Mutex<RoomState>>)> = {
            let guard = self.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.iter().map(|(doc, room)| (doc.clone(), room.clone())).collect()
        };
        for (doc, room) in rooms {
            self.persist(&doc, &room, None)?;
        }
        Ok(())
    }
}

/// The HTTP application: health, metrics and the WebSocket endpoint.
pub fn app(state: AppState) -> Router {
    Router::new().route("/healthz", get(healthz)).route("/metrics", get(metrics)).route("/ws", get(ws_upgrade)).with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics(State(state): State<AppState>) -> String {
    state.metrics.render(state.room_count())
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME).on_upgrade(move |socket| handle_socket(socket, state))
}

/// Verify a token and return its claims.
pub fn verify_token(cfg: &Config, token: &str) -> Result<Claims, Reject> {
    let invalid = |message: &str| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Forbidden, message);
    let mut validation = Validation::new(if cfg.jwt_pem.is_some() { Algorithm::ES256 } else { Algorithm::HS256 });
    validation.set_audience(&["collab", "wordcraft"]);
    validation.set_required_spec_claims(&["exp", "aud", "sub"]);

    let key = match (&cfg.jwt_pem, &cfg.jwt_secret) {
        (Some(pem), _) => DecodingKey::from_ec_pem(pem.as_bytes()).map_err(|_| invalid("bad public key"))?,
        (None, Some(secret)) => DecodingKey::from_secret(secret.as_bytes()),
        (None, None) => return Err(invalid("server has no token key")),
    };
    let data = decode::<Claims>(token, &key, &validation).map_err(|_| invalid("token is invalid or expired"))?;
    let claims = data.claims;
    if claims.app != "wordcraft" {
        return Err(invalid("token is for another application"));
    }
    if Role::parse(&claims.role).is_none() {
        return Err(invalid("token role is unknown"));
    }
    Ok(claims)
}

/// Refuse a join to a document whose stored snapshot cannot be opened, and end
/// the connection (ADR WC-0003).
///
/// The message names the situation, not the server's paths: the client cannot
/// repair the file. The detail goes to the log, without document content.
async fn refuse_corrupt(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, state: &AppState, doc: &str, corrupt: &Corrupt) {
    log::error!(
        "refusing a join for {doc}: its stored snapshot cannot be opened ({corrupt}); restore or remove {}",
        state.store.doc_dir(doc).display()
    );
    refuse(sink, state, "the stored snapshot of this document cannot be opened; an operator must restore or remove it").await;
}

/// Refuse a join that cannot open a room (ADR WC-0003, ADR WC-0004) and end the
/// connection. The answer names the situation, not the server's paths: the
/// client cannot repair either case, and the detail goes to the log without
/// document content.
async fn refuse_join(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, state: &AppState, doc: &str, refusal: Refusal) {
    match refusal {
        Refusal::Corrupt(corrupt) => refuse_corrupt(sink, state, doc, &corrupt).await,
        Refusal::Unseeded => {
            Metrics::inc(&state.metrics.unseeded);
            log::warn!("refusing a join for {doc}: this server has no state for it and the client offered none");
            refuse(
                sink,
                state,
                "this document has no state on this server yet; it must be opened by a client that may edit it and has the document open",
            )
            .await
        }
        Refusal::BadSeed(reason) => {
            log::warn!("refusing a join for {doc}: the document it offered cannot be used ({reason})");
            refuse(sink, state, "the document this client offered could not be read").await
        }
    }
}

/// Answer a join with a refusal and end the connection. The code is `invalid`
/// and not retryable: the client cannot fix what the server or an operator has
/// to.
async fn refuse(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, state: &AppState, message: &str) {
    let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, message);
    let _ = send(sink, &ServerMsg::Rejected { reject }, state.cfg.heartbeat).await;
    let _ = sink.close().await;
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    Metrics::inc(&state.metrics.connections);
    let (mut sink, mut stream) = socket.split();

    let Some(Handshake { client_id, claims, role, wants_version, seed }) = authenticate(&mut sink, &mut stream, &state).await else {
        return;
    };
    let opened = {
        // Building a room reads and parses the snapshot: blocking file work on a
        // 64 MiB bound that must not run on an async worker either
        // (CodeRabbit 4533747484).
        let state = state.clone();
        let doc = claims.doc.clone();
        tokio::task::spawn_blocking(move || state.room_for(&doc, seed, role)).await
    };
    let (room, seeded) = match opened {
        Ok(Ok(parts)) => parts,
        // Joining would open an empty room whose snapshot overwrites the stored
        // file (ADR WC-0003) or replaces the document the client has open
        // (ADR WC-0004).
        Ok(Err(refusal)) => return refuse_join(&mut sink, &state, &claims.doc, refusal).await,
        Err(_) => {
            log::error!("the room of {} could not be opened: the task ended unexpectedly", claims.doc);
            return;
        }
    };
    // Every write to this peer is bounded by one heartbeat period: a peer that
    // stopped reading is closed instead of holding the connection open with a
    // document the room no longer has (ADR WC-0002).
    let limit = state.cfg.heartbeat;
    let (tx, rx) = mpsc::channel::<ServerMsg>(state.cfg.peer_queue.max(1));
    let registration = match register(&state, &room, &client_id, &claims, role, &tx, wants_version) {
        Ok(registration) => registration,
        Err(reject) => {
            let _ = send(&mut sink, &ServerMsg::Rejected { reject }, limit).await;
            let _ = sink.close().await;
            return;
        }
    };
    if !send_join(&mut sink, &claims, role, &registration.plan, registration.peers, limit, seeded).await {
        leave(&state, &claims.doc, &room, &client_id, registration.conn);
        return;
    }
    announce_join(&room, &client_id, role);

    let conn = Conn { state: &state, room: &room, claims: &claims, role, client_id: &client_id, limit };
    let session = Session { conn, sink, stream, rx, stale: registration.stale, superseded: registration.superseded };
    session.run().await;
    leave(&state, &claims.doc, &room, &client_id, registration.conn);
}

/// One connection after the join: the frames a client sends, the frames the room
/// queues for it, and the two signals that end it — its outbox overflowed
/// (ADR WC-0002) or a newer connection took its `client_id` over (ADR WC-0005).
/// Kept out of `handle_socket` so the handshake stays one screen.
struct Session<'a> {
    conn: Conn<'a>,
    sink: futures_util::stream::SplitSink<WebSocket, Message>,
    stream: futures_util::stream::SplitStream<WebSocket>,
    rx: mpsc::Receiver<ServerMsg>,
    stale: Arc<Retire>,
    superseded: Arc<Retire>,
}

impl Session<'_> {
    async fn run(mut self) {
        let limit = self.conn.limit;
        let mut heartbeat = interval(self.conn.state.cfg.heartbeat);
        heartbeat.tick().await; // the first tick is immediate; the first frame stays an answer
        loop {
            tokio::select! {
                // A newer connection of the same client registered this
                // `client_id`: two sockets must not share one `(clientId, seq)`
                // space, and this connection's `leave` must not remove the
                // replacement (ADR WC-0005).
                _ = self.superseded.wait() => {
                    close_retired(&mut self.sink, limit, "a newer connection registered the same client id").await;
                    break;
                }
                incoming = self.stream.next() => {
                    let text = match incoming {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => continue,
                    };
                    if !process_frame(&self.conn, &mut self.sink, &text).await {
                        break;
                    }
                }
                outgoing = self.rx.recv() => {
                    let Some(msg) = outgoing else { break };
                    // The outbox overflowed while this peer was not reading: the
                    // accepted operation it missed is gone from its queue, so the
                    // connection ends here and the client reconnects and resyncs.
                    // `stale.wait()` cancels a write that is stuck on that peer, so
                    // the close does not wait for the write bound.
                    if self.stale.is_set() {
                        close_retired(&mut self.sink, limit, "its outbox overflowed and it missed an accepted operation").await;
                        break;
                    }
                    tokio::select! {
                        _ = self.stale.wait() => {
                            close_retired(&mut self.sink, limit, "its outbox overflowed and it missed an accepted operation").await;
                            break;
                        }
                        sent = send(&mut self.sink, &msg, limit) => {
                            if sent.is_err() {
                                break;
                            }
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    // The queue drop happens while this loop is blocked on a write,
                    // so the flag can only be read here or on the next frame; the
                    // heartbeat is what guarantees a stale peer is noticed even
                    // when nothing else arrives.
                    if self.stale.is_set() {
                        close_retired(&mut self.sink, limit, "its outbox overflowed and it missed an accepted operation").await;
                        break;
                    }
                    let ping = timeout(limit, self.sink.send(Message::Ping(Vec::new().into()))).await;
                    if !matches!(ping, Ok(Ok(()))) {
                        break;
                    }
                }
            }
        }
    }
}

/// End a connection the room has retired: the peer's outbox overflowed and it
/// missed an accepted operation (ADR WC-0002), or a newer connection of the same
/// client took its `client_id` over (ADR WC-0005). Closing makes the client
/// reconnect with its `lastVersion`; the join answers with the journal tail or a
/// snapshot.
async fn close_retired(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, limit: Duration, reason: &str) {
    log::warn!("closing a peer connection: {reason}");
    if timeout(limit, sink.send(Message::Close(None))).await.is_err() {
        log::warn!("the retired peer did not take the close frame; dropping the socket");
    }
}

/// What the first frames of a connection established (ADR-0008, ADR WC-0004).
struct Handshake {
    client_id: String,
    claims: Claims,
    role: Role,
    wants_version: Option<Version>,
    /// The document the client offered, when it announced a seed. `None` means
    /// it brought none: this server can only join it to a room that already has
    /// state.
    seed: Option<Vec<u8>>,
}

async fn authenticate(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &AppState,
) -> Option<Handshake> {
    let first = match timeout(AUTH_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            let _ = sink.close().await;
            return None;
        }
    };
    let (client_id, claims, wants_version, seeds) = match parse_auth(&state.cfg, &first) {
        Ok(parts) => parts,
        Err(reject) => {
            let _ = send(sink, &ServerMsg::Rejected { reject }, state.cfg.heartbeat).await;
            let _ = sink.close().await;
            return None;
        }
    };
    let role = Role::parse(&claims.role)?;
    let seed = if seeds {
        // The token has been checked, so the stream that follows is bounded work
        // for an authenticated peer only.
        match read_seed(stream).await {
            Ok(model) => Some(model),
            Err(reject) => {
                let _ = send(sink, &ServerMsg::Rejected { reject }, state.cfg.heartbeat).await;
                let _ = sink.close().await;
                return None;
            }
        }
    } else {
        None
    };
    Some(Handshake { client_id, claims, role, wants_version, seed })
}

/// Read the `Seed` frames that follow `Auth { seed: true }` (ADR WC-0004).
///
/// The client sends the document it has open in chunks; the frame with `last`
/// ends the stream. The handshake is refused when a frame is not a seed, when
/// the stream stops before its end, or when the document passes the model bound
/// the store enforces: a seed that would not fit a snapshot is not a room state.
async fn read_seed(stream: &mut futures_util::stream::SplitStream<WebSocket>) -> Result<Vec<u8>, Reject> {
    let invalid = |message: String| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, message);
    let mut model = Vec::new();
    loop {
        let text = match timeout(AUTH_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text,
            _ => return Err(invalid("the document you offered ended before its last frame".into())),
        };
        let ClientMsg::Seed { bytes, last } = parse_client(&text)? else {
            return Err(invalid("a seed frame must follow an auth that announced one".into()));
        };
        if model.len() + bytes.len() > MAX_MODEL {
            return Err(invalid(format!("the document you offered is over the {MAX_MODEL}-byte model bound")));
        }
        model.extend_from_slice(&bytes);
        if last {
            return Ok(model);
        }
    }
}

/// What `register` installed: the answer this connection gets, plus its own
/// identity and the signal that ends it when a newer connection takes the
/// `client_id` over (ADR WC-0005).
struct Registration {
    plan: wordcraft_collab::room::JoinPlan,
    peers: Vec<Participant>,
    conn: u64,
    stale: Arc<Retire>,
    superseded: Arc<Retire>,
}

fn register(
    state: &AppState,
    room: &Arc<Mutex<RoomState>>,
    client_id: &str,
    claims: &Claims,
    role: Role,
    tx: &mpsc::Sender<ServerMsg>,
    wants_version: Option<Version>,
) -> Result<Registration, Reject> {
    let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
    const MAX_PARTICIPANTS: usize = 8;
    if let Some(existing) = guard.peers.get(client_id) {
        // The id is taken. Only the same account in the same cabinet session may
        // take it over: that is this client reconnecting while the server has
        // not noticed the old socket is gone, and the old connection is retired
        // below. A different `sub` or `sid` is a claim on someone else's id: it
        // is refused before it reaches the map, so no one can displace a peer
        // from the room or inherit its presence (ADR WC-0005).
        if existing.sub != claims.sub || existing.sid != claims.sid {
            Metrics::inc(&state.metrics.client_ids);
            log::warn!("refusing a join: the client id it registered belongs to another participant");
            let reject =
                Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "this client id belongs to another participant");
            return Err(reject);
        }
    } else if guard.peers.len() >= MAX_PARTICIPANTS {
        let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::TooLarge, "the room is full");
        return Err(reject);
    }
    if guard.session.is_empty() {
        guard.session = claims.sid.clone();
        guard.lease_seen = std::time::Instant::now();
    } else if guard.session != claims.sid {
        let lease_alive = guard.peers.values().any(|peer| peer.sid == guard.session);
        let stale = guard.lease_seen.elapsed().as_secs() >= guard.lease_ttl_secs;
        if role != Role::Owner && (lease_alive || !stale) {
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Lease, "another session holds this document");
            return Err(reject);
        }
        guard.session = claims.sid.clone();
        guard.lease_seen = std::time::Instant::now();
    }
    guard.lease_seen = std::time::Instant::now();
    // Only now, with every refusal behind us, does a takeover retire the old
    // connection: a join the lease check refuses must leave the working socket
    // in place, or the client loses a live connection to a refusal that changes
    // nothing (CodeRabbit 4533747488).
    if let Some(existing) = guard.peers.get(client_id) {
        existing.superseded.mark();
    }
    let plan = guard.room.join(wants_version, false);
    let conn = next_connection();
    let peer = Peer::new(conn, &claims.sub, role, &claims.sid, tx.clone());
    // The connection keeps both signals, so a queue drop (ADR WC-0002) and a
    // takeover (ADR WC-0005) end *this* connection and no other.
    let stale = peer.stale.clone();
    let superseded = peer.superseded.clone();
    guard.peers.insert(client_id.to_string(), peer);
    let peers = guard
        .peers
        .iter()
        .map(|(id, peer)| Participant { client_id: id.clone(), role: peer.role.as_str().to_string(), color: None, presence: None })
        .collect();
    Ok(Registration { plan, peers, conn, stale, superseded })
}

async fn send_join(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    claims: &Claims,
    role: Role,
    plan: &wordcraft_collab::room::JoinPlan,
    peers: Vec<Participant>,
    limit: Duration,
    seeded: bool,
) -> bool {
    // The room was built from this client's own copy, so there is nothing to
    // send back: the snapshot would be the document the client just offered, and
    // `resync` stays false so the client keeps it (ADR WC-0004).
    let snapshot = if seeded { None } else { plan.snapshot.clone() };
    let welcome = ServerMsg::Welcome {
        session_id: claims.sid.clone(),
        role: role.as_str().to_string(),
        version: plan.version,
        epoch: plan.epoch,
        participants: peers,
        resync: snapshot.is_some(),
    };
    if send(sink, &welcome, limit).await.is_err() {
        return false;
    }
    if let Some(snapshot) = snapshot {
        for msg in snapshot_frames(plan.version, snapshot) {
            if send(sink, &msg, limit).await.is_err() {
                return false;
            }
        }
    } else if let Some(tail) = plan.replay.clone()
        && send(sink, &ServerMsg::Tail { entries: tail }, limit).await.is_err()
    {
        return false;
    }
    send(sink, &ServerMsg::Nodes { nodes: plan.nodes.clone() }, limit).await.is_ok()
}

/// Tell the peers already in the room who just joined and with what role, so
/// their participant list shows the role before the newcomer's first presence
/// frame (TASK.md §3). The frame is an ordinary `presence` with an empty state:
/// the clients already fold it through `apply_presence`, and an older peer that
/// does not know the `role` field still parses it. The role comes from the JWT
/// claim (ADR-0008), never from the wire, and the newcomer is not echoed its own
/// join: it learns the existing peers from `welcome`.
fn announce_join(room: &Arc<Mutex<RoomState>>, client_id: &str, role: Role) {
    let targets: Vec<Peer> = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.iter().filter(|(id, _)| id.as_str() != client_id).map(|(_, peer)| peer.clone()).collect()
    };
    let msg = ServerMsg::Presence { client_id: client_id.to_string(), role: role.as_str().to_string(), state: serde_json::json!({}) };
    for peer in &targets {
        // Presence is droppable; a full queue must not stall the room.
        queue_ephemeral(peer, &msg);
    }
}

/// One authenticated connection: what a frame handler needs to answer it.
#[derive(Clone, Copy)]
struct Conn<'a> {
    state: &'a AppState,
    room: &'a Arc<Mutex<RoomState>>,
    claims: &'a Claims,
    role: Role,
    client_id: &'a str,
    /// The write bound: one heartbeat period (ADR WC-0002).
    limit: Duration,
}

async fn process_frame(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, text: &str) -> bool {
    let Conn { state, room, claims, role, client_id, limit } = *conn;
    let msg = match parse_client(text) {
        Ok(msg) => msg,
        Err(reject) => return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok(),
    };
    {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.session == claims.sid {
            guard.lease_seen = std::time::Instant::now();
        }
    }
    if now_secs() >= claims.exp {
        let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::TokenExpired, "the session token expired");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    match msg {
        ClientMsg::Op { op } => handle_op(conn, sink, op).await,
        ClientMsg::Presence { state: presence } => {
            Metrics::inc(&state.metrics.presence);
            // Ephemeral state goes to the other participants; the author already
            // knows its own caret and the cabinet's UI keeps it locally.
            let targets: Vec<Peer> = {
                let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.peers.iter().filter(|(id, _)| id.as_str() != client_id).map(|(_, peer)| peer.clone()).collect()
            };
            let msg = ServerMsg::Presence { client_id: client_id.to_string(), role: role.as_str().to_string(), state: presence };
            for peer in &targets {
                // Presence is droppable; a full queue must not stall the room.
                queue_ephemeral(peer, &msg);
            }
            true
        }
        ClientMsg::Resync { last_version } => {
            let (plan, peers) = {
                let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                let peers = guard
                    .peers
                    .iter()
                    .map(|(id, peer)| Participant { client_id: id.clone(), role: peer.role.as_str().to_string(), color: None, presence: None })
                    .collect();
                (guard.room.join(Some(last_version), true), peers)
            };
            // The client asked for a snapshot: `seeded` is always false here.
            send_join(sink, claims, role, &plan, peers, limit, false).await
        }
        ClientMsg::Commit { op_id, expected_version, .. } => {
            let call = CommitCall { client_id, op_id, expected_version };
            handle_commit(conn, sink, call).await
        }
        ClientMsg::SaveCopy { op_id } => handle_save_copy(conn, sink, op_id).await,
        ClientMsg::Ping { ts } => send(sink, &ServerMsg::Pong { ts }, limit).await.is_ok(),
        ClientMsg::Auth { .. } => {
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "already authenticated");
            send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok()
        }
        ClientMsg::Seed { .. } => {
            // A seed is read only between an auth that announced one and the
            // welcome (ADR WC-0004); later it belongs to no room state.
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "a seed is only read right after auth");
            send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok()
        }
    }
}

async fn handle_op(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, op: wordcraft_collab::wire::Op) -> bool {
    let Conn { state, room, claims, role, client_id, limit } = *conn;
    // An operation id names its author. A peer that could put another client's
    // id on the wire would make the victim skip the answer as its own echo and
    // then, on its own retry, get a cached answer for an edit it never made.
    if op.op_id.client != client_id {
        Metrics::inc(&state.metrics.rejects);
        let reject = Reject::new(op.op_id.clone(), RejectCode::Invalid, "the operation id belongs to another client");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let structural = op.kind.is_structural();
    let result = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.window.elapsed() >= std::time::Duration::from_secs(1) {
            guard.window = std::time::Instant::now();
            guard.ops_in_window = 0;
        }
        if guard.ops_in_window >= 200 {
            Err(Reject::new(op.op_id.clone(), RejectCode::TooLarge, "too many operations; retry in a moment"))
        } else {
            guard.ops_in_window += 1;
            guard.room.submit(&claims.sub, role, op)
        }
    };
    match result {
        Ok(applied) => {
            Metrics::inc(&state.metrics.ops);
            // A command that allocates ids carries them on the answer; peers
            // apply the delta instead of replacing the whole map.
            let delta_sent = applied.nodes.is_some();
            // The author's own answer: without it the pending operation is never
            // acknowledged on this connection, so a failed write ends it.
            if !broadcast(state, room, client_id, ServerMsg::Applied { applied }, sink).await {
                return false;
            }
            let nodes = {
                let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                let moved = guard.room.take_nodes_changed();
                if !delta_sent && (structural || moved) { Some(guard.room.nodes()) } else { None }
            };
            if let Some(nodes) = nodes
                && !broadcast(state, room, "", ServerMsg::Nodes { nodes }, sink).await
            {
                return false;
            }
            let due = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.should_snapshot();
            if due && let Err(message) = state.persist(&claims.doc, room, None) {
                log::warn!("snapshot failed for {}: {message}", claims.doc);
            }
            true
        }
        Err(reject) => {
            Metrics::inc(&state.metrics.rejects);
            send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok()
        }
    }
}

/// One commit frame from the owner: the caller's client id (so the answer
/// reaches each peer exactly once), the operation id and the room version the
/// commit was taken at.
struct CommitCall<'a> {
    client_id: &'a str,
    op_id: wordcraft_collab::wire::OpId,
    expected_version: Version,
}

async fn handle_commit(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, call: CommitCall<'_>) -> bool {
    let Conn { state, room, claims, role, limit, .. } = *conn;
    let CommitCall { client_id, op_id, expected_version } = call;
    if !role.can_commit() {
        let reject = Reject::new(op_id, RejectCode::Forbidden, "only the owner commits");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let version = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.version();
    if expected_version != version {
        let reject = Reject::new(op_id, RejectCode::Conflict, format!("head is version {version}"));
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let (bytes, warnings) = match build_docx(room) {
        Ok(built) => built,
        Err(message) => {
            let reject = Reject::new(op_id, RejectCode::Invalid, message);
            return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
        }
    };
    if bytes.len() > MAX_DOCUMENT {
        let reject = Reject::new(op_id, RejectCode::TooLarge, format!("the built file is {} bytes", bytes.len()));
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    Metrics::inc(&state.metrics.commits);
    let sha256 = hex(&sha2::Sha256::digest(&bytes));

    // The cabinet owns the stored file: post the built bytes and let it name
    // the version. Without a callback the host owns the commit and only the
    // built file's digest travels back (M87).
    let outcome = match &state.cabinet {
        Some(cabinet) => {
            let expected = room.lock().unwrap_or_else(|poison| poison.into_inner()).published.as_ref().map(|published| published.etag.clone());
            let publication =
                Publication { session_id: &claims.sid, file_id: &claims.doc, bytes: &bytes, sha256: &sha256, version, expected: expected.as_deref() };
            Some(cabinet.publish(&publication).await)
        }
        None => None,
    };

    let cabinet = match outcome {
        Some(Ok(published)) => {
            Metrics::inc(&state.metrics.published);
            let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.published = Some(published.clone());
            // The refused copy belonged to an older attempt; the cabinet now
            // holds this one.
            guard.refused = None;
            Some(CabinetCommit { version: published.version, etag: published.etag })
        }
        Some(Err(PublishError::Conflict(message))) => {
            Metrics::inc(&state.metrics.publish_conflicts);
            // The file changed outside the session: the room must not
            // overwrite it, and every replica resyncs so the refusal and the
            // stale base are visible everywhere. The built file stays in the
            // room for the owner's "save a copy" action (TASK.md §6).
            log::warn!("cabinet refused the commit of {} as a conflict: {message}", claims.doc);
            let refused = Refused { version, sha256: sha256.clone(), warnings: warnings.clone(), bytes: Arc::new(bytes.clone()) };
            room.lock().unwrap_or_else(|poison| poison.into_inner()).refused = Some(refused);
            let notice =
                ServerMsg::Error { code: "cabinetConflict".into(), message: format!("{message}; save a copy or reopen the file"), retryable: true };
            let _ = state.persist(&claims.doc, room, Some(bytes));
            // Every replica needs this notice; the author's own frame decides
            // whether the connection survives it.
            return broadcast(state, room, client_id, notice, sink).await;
        }
        Some(Err(error)) => {
            Metrics::inc(&state.metrics.publish_errors);
            // The token and the body stay out of the log; the status does not.
            log::warn!("cabinet did not store the commit of {}: {error}", claims.doc);
            let code = match &error {
                PublishError::Unauthorized(_) => RejectCode::Forbidden,
                PublishError::TooLarge(_) => RejectCode::TooLarge,
                PublishError::Unavailable(_) | PublishError::Refused(_) | PublishError::Conflict(_) => RejectCode::Invalid,
            };
            let _ = state.persist(&claims.doc, room, Some(bytes));
            let reject = Reject::new(op_id, code, error.to_string());
            return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
        }
        None => None,
    };

    if let Err(message) = state.persist(&claims.doc, room, Some(bytes)) {
        log::warn!("snapshot after commit failed: {message}");
    }
    // The version and etag are the cabinet's; every peer shows them, so the
    // participants panel never claims a save the cabinet did not take. The
    // author's own frame is the answer to its commit: if it cannot be written,
    // the session ends and the client resyncs.
    broadcast(state, room, client_id, ServerMsg::Committed { version, sha256, warnings, cabinet }, sink).await
}

/// The owner asked for the file of a refused commit (`409`): send the bytes the
/// room built, chunked like a snapshot, to this peer only. Nothing is written
/// to the cabinet (TASK.md §6).
async fn handle_save_copy(
    conn: &Conn<'_>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    op_id: wordcraft_collab::wire::OpId,
) -> bool {
    let Conn { state, room, role, limit, .. } = *conn;
    if !role.can_commit() {
        let reject = Reject::new(op_id, RejectCode::Forbidden, "only the owner saves a copy of a refused commit");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let refused = room.lock().unwrap_or_else(|poison| poison.into_inner()).refused.clone();
    let Some(refused) = refused else {
        let reject = Reject::new(op_id, RejectCode::Conflict, "there is no refused commit to save; commit again to see the conflict");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    };
    Metrics::inc(&state.metrics.copies);
    let mut offset = 0;
    loop {
        let end = (offset + SNAPSHOT_CHUNK).min(refused.bytes.len());
        let last = end >= refused.bytes.len();
        let chunk = ServerMsg::SaveCopy {
            version: refused.version,
            sha256: refused.sha256.clone(),
            warnings: refused.warnings.clone(),
            bytes: refused.bytes[offset..end].to_vec(),
            last,
        };
        if send(sink, &chunk, limit).await.is_err() {
            return false;
        }
        if last {
            return true;
        }
        offset = end;
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0)
}

fn parse_auth(cfg: &Config, text: &str) -> Result<(String, Claims, Option<Version>, bool), Reject> {
    let msg: ClientMsg = serde_json::from_str(text)
        .map_err(|_| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "the first message must be auth"))?;
    let ClientMsg::Auth { token, client_id, last_version, seed } = msg else {
        return Err(Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Forbidden, "authenticate first"));
    };
    let claims = verify_token(cfg, &token)?;
    if client_id.is_empty() {
        return Err(Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "clientId is required"));
    }
    Ok((client_id, claims, last_version, seed))
}

fn parse_client(text: &str) -> Result<ClientMsg, Reject> {
    serde_json::from_str(text).map_err(|_| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "malformed message"))
}

/// Write one frame to a peer, bounded by the heartbeat period. A write that
/// does not finish in time means the peer stopped reading: the caller ends the
/// connection and the client reconnects and resyncs (ADR WC-0002).
///
/// Generic over the sink so a test can hand it one whose write fails and see
/// the answer the callers act on (CodeRabbit 4533747492).
async fn send<S>(sink: &mut S, msg: &ServerMsg, limit: Duration) -> Result<(), ()>
where
    S: futures_util::Sink<Message, Error = axum::Error> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    match timeout(limit, sink.send(Message::Text(text.into()))).await {
        Ok(result) => result.map_err(|_| ()),
        Err(_) => Err(()),
    }
}

/// Queue one document frame for a peer. `false` means the peer missed it: its
/// outbox is full or its connection is gone. A full outbox marks the peer
/// stale — it has already missed an accepted operation — and its connection is
/// closed instead of staying open with a document the room moved past
/// (ADR WC-0002). Counted once per peer.
fn queue(state: &AppState, peer: &Peer, msg: &ServerMsg) -> bool {
    match peer.out.try_send(msg.clone()) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            if peer.stale.mark() {
                Metrics::inc(&state.metrics.stale);
                log::warn!("a peer's outbox is full: it missed a document frame and will be closed");
            }
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Queue an ephemeral frame (presence, a leave notice). Droppable by design: a
/// full outbox must not stall the room, and a lost presence frame is repaired
/// by the next one (ADR-0006). It never marks a peer stale, because it carries
/// no document state.
fn queue_ephemeral(peer: &Peer, msg: &ServerMsg) {
    let _ = peer.out.try_send(msg.clone());
}

/// Send to every peer of the room (`broadcast` includes the sender, so the
/// author sees the authoritative version too).
///
/// `false` means the author's own frame could not be written within the
/// heartbeat bound: the answer it needs for the operation it just sent is gone,
/// so the caller ends the session and the client reconnects and resyncs, the way
/// every other failed write in this file does (CodeRabbit 4533747492).
async fn broadcast<S>(state: &AppState, room: &Arc<Mutex<RoomState>>, sender: &str, msg: ServerMsg, sink: &mut S) -> bool
where
    S: futures_util::Sink<Message, Error = axum::Error> + Unpin,
{
    let targets: Vec<Peer> = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.iter().filter(|(id, _)| id.as_str() != sender).map(|(_, peer)| peer.clone()).collect()
    };
    for peer in &targets {
        // Bounded: a client that stops reading loses the frame instead of
        // growing the server, and is closed rather than left stale: the frame
        // it missed is an accepted operation it can no longer derive.
        let _ = queue(state, peer, &msg);
    }
    send(sink, &msg, state.cfg.heartbeat).await.is_ok()
}

/// Remove this connection's peer from the room. The connection id decides: when
/// another connection with the same `client_id` replaced this one, the entry is
/// not ours and stays — no `left` is broadcast and the room is not treated as
/// empty (ADR WC-0005).
fn leave(state: &AppState, doc: &str, room: &Arc<Mutex<RoomState>>, client_id: &str, conn: u64) {
    let gone = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        let ours = guard.peers.get(client_id).is_some_and(|peer| peer.conn == conn);
        if ours { guard.peers.remove(client_id) } else { None }
    };
    if gone.is_some() {
        let targets: Vec<Peer> = {
            let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.peers.values().cloned().collect()
        };
        for peer in &targets {
            queue_ephemeral(peer, &ServerMsg::Left { client_id: client_id.to_string() });
        }
    }
    let empty = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.is_empty()
    };
    if empty {
        // The last peer left: keep the room's state on disk before dropping it.
        if let Err(message) = state.persist(doc, room, None) {
            log::warn!("snapshot on leave failed for {doc}: {message}");
        }
        let mut rooms = state.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
        rooms.retain(|_, existing| !Arc::ptr_eq(existing, room));
    }
}

fn snapshot_frames(version: Version, bytes: Vec<u8>) -> Vec<ServerMsg> {
    if bytes.is_empty() {
        return vec![ServerMsg::Snapshot { version, bytes, last: true }];
    }
    let total = bytes.len();
    let mut frames = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + SNAPSHOT_CHUNK).min(total);
        frames.push(ServerMsg::Snapshot { version, bytes: bytes[start..end].to_vec(), last: end == total });
        start = end;
    }
    frames
}

fn build_docx(room: &Arc<Mutex<RoomState>>) -> Result<(Vec<u8>, Vec<String>), String> {
    let doc = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.room.model().session().doc.clone()
    };
    let mut warnings = Vec::new();
    if doc.kept.get().is_some() {
        warnings.push("unmodelled paragraph markup is kept only for paragraphs unchanged since open".to_string());
    }
    let bytes = wordcraft_docx::write(&doc).map_err(|err| err.to_string())?;
    Ok((bytes, warnings))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests;
