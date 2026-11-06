//! Server tests: token checks and a real WebSocket round trip with two clients.

use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{EncodingKey, Header};
use serde_json::json;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;

use wordcraft_collab::wire::{ClientMsg, Op, OpId, OpKind, ServerMsg, Target};

use crate::{AppState, Claims, Config, app};

fn token(cfg: &Config, sub: &str, role: &str, app: &str, ttl: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    let claims = Claims {
        sub: sub.into(),
        aud: "collab".into(),
        doc: "doc-1".into(),
        app: app.into(),
        role: role.into(),
        sid: "session-1".into(),
        iat: now,
        exp: if ttl >= 0 { now + ttl as u64 } else { now.saturating_sub(ttl.unsigned_abs()) },
    };
    let secret = cfg.jwt_secret.as_deref().unwrap_or_default();
    jsonwebtoken::encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes())).expect("encode")
}

fn dev_config() -> Config {
    let dir = std::env::temp_dir().join(format!("wordcraft-collab-test-{}", std::process::id()));
    Config { bind: "127.0.0.1:0".into(), jwt_secret: Some("test-secret".into()), jwt_pem: None, data_dir: dir }
}

#[test]
fn a_valid_token_is_accepted() {
    let cfg = dev_config();
    let claims = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "wordcraft", 600)).expect("valid token");
    assert_eq!(claims.sub, "alice");
    assert_eq!(claims.role, "editor");
}

#[test]
fn an_expired_token_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "wordcraft", -3600)).expect_err("expired");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

#[test]
fn a_token_for_another_app_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "gridcraft", 600)).expect_err("wrong app");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

#[test]
fn an_unknown_role_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "admin", "wordcraft", 600)).expect_err("bad role");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

async fn spawn_server() -> (String, Config) {
    let cfg = dev_config();
    let state = AppState::new(cfg.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app(state)).await;
    });
    (format!("ws://{addr}/ws"), cfg)
}

async fn connect(
    url: &str,
    cfg: &Config,
    role: &str,
    client: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    let auth = ClientMsg::Auth { token: token(cfg, client, role, "wordcraft", 600), client_id: client.into(), last_version: None };
    ws.send(Message::Text(serde_json::to_string(&auth).expect("auth json").into())).await.expect("send auth");
    ws
}

async fn next_msg(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> ServerMsg {
    loop {
        let frame = timeout(Duration::from_secs(5), ws.next()).await.expect("no timeout").expect("frame").expect("ok frame");
        match frame {
            Message::Text(text) => return serde_json::from_str(&text).expect("server message"),
            // Control frames may arrive at any time.
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("expected text frame, got {other:?}"),
        }
    }
}

async fn node_of(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> u32 {
    loop {
        match next_msg(ws).await {
            ServerMsg::Nodes { nodes } => {
                return nodes.iter().find(|n| n.story == "body").map(|n| n.id).expect("a body node");
            }
            ServerMsg::Welcome { .. } | ServerMsg::Snapshot { .. } | ServerMsg::Tail { .. } => {}
            other => panic!("unexpected before nodes: {other:?}"),
        }
    }
}

fn insert(client: &str, seq: u64, base: u64, node: u32, off: usize, text: &str) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, off),
        payload: json!({"op": "insert_text", "text": text}),
    }
}

#[tokio::test]
async fn two_clients_edit_through_the_server() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let mut bob = connect(&url, &cfg, "editor", "bob").await;

    let welcome = next_msg(&mut alice).await;
    assert!(matches!(welcome, ServerMsg::Welcome { .. }));
    let node_alice = node_of(&mut alice).await;
    let _ = next_msg(&mut bob).await;
    let node_bob = node_of(&mut bob).await;
    assert_eq!(node_alice, node_bob, "both clients see the same session node");

    alice
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 0, node_alice, 0, "Hello") }).expect("json").into()))
        .await
        .expect("send op");
    let applied = next_msg(&mut alice).await;
    assert!(matches!(applied, ServerMsg::Applied { .. }), "{applied:?}");
    let echoed = next_msg(&mut bob).await;
    assert!(matches!(echoed, ServerMsg::Applied { .. }), "{echoed:?}");

    bob.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("bob", 1, 1, node_bob, 5, " world") }).expect("json").into()))
        .await
        .expect("send op");
    let applied = next_msg(&mut alice).await;
    match applied {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 2),
        other => panic!("expected applied, got {other:?}"),
    }
}

#[tokio::test]
async fn a_viewer_cannot_write() {
    let (url, cfg) = spawn_server().await;
    let mut viewer = connect(&url, &cfg, "viewer", "carol").await;
    let _ = next_msg(&mut viewer).await;
    let node = node_of(&mut viewer).await;

    viewer
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("carol", 1, 0, node, 0, "x") }).expect("json").into()))
        .await
        .expect("send op");
    match next_msg(&mut viewer).await {
        ServerMsg::Rejected { reject } => {
            assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Forbidden);
        }
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn a_room_is_restored_from_disk() {
    let cfg = dev_config();
    let doc = "doc-restore";
    let state = AppState::new(cfg.clone());
    let room = state.room_for(doc);
    let node = {
        let mut guard = room.lock().expect("room lock");
        guard.room.join(None, false).nodes.first().expect("a node").id
    };
    let op = insert("alice", 1, 0, node, 0, "Hi");
    room.lock().expect("room lock").room.submit("alice", wordcraft_collab::room::Role::Editor, op).expect("apply");
    state.persist(doc, &room, None).expect("persist");
    drop(room);
    drop(state);

    let restored = AppState::new(cfg);
    let room = restored.room_for(doc);
    let guard = room.lock().expect("room lock");
    assert_eq!(guard.room.version(), 1, "the snapshot restores the version");
    let text = guard.room.model().document().body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hi");
}

#[test]
fn a_second_session_needs_the_owner_role() {
    let state = AppState::new(dev_config());
    let room = state.room_for("doc-lease");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    assert!(super::register(&room, "alice", "sid-1", wordcraft_collab::room::Role::Editor, &tx, None).is_ok());

    let denied = super::register(&room, "bob", "sid-2", wordcraft_collab::room::Role::Editor, &tx, None);
    assert_eq!(denied.expect_err("a foreign session needs the owner").code, wordcraft_collab::wire::RejectCode::Lease);

    assert!(super::register(&room, "carol", "sid-3", wordcraft_collab::room::Role::Owner, &tx, None).is_ok());
}

/// The acceptance gate from TASK.md §8: the committed file opens in LibreOffice
/// without a repair prompt. Ignored by default because it needs `soffice`.
#[tokio::test]
#[ignore = "needs LibreOffice (soffice) on PATH"]
async fn the_committed_docx_opens_in_libreoffice() {
    let (url, cfg) = spawn_server().await;
    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Hello collaboration") }).expect("json").into(),
        ))
        .await
        .expect("send op");
    let _ = next_msg(&mut owner).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: wordcraft_collab::wire::OpId::new("owner", 2), expected_version: 1, etag: None })
                .expect("json")
                .into(),
        ))
        .await
        .expect("commit");
    let committed = loop {
        match next_msg(&mut owner).await {
            ServerMsg::Committed { version, sha256, warnings } => break (version, sha256, warnings),
            ServerMsg::Applied { .. } => continue,
            other => panic!("expected committed, got {other:?}"),
        }
    };
    assert_eq!(committed.0, 1);
    assert!(!committed.1.is_empty());

    let docx = cfg.data_dir.join("doc-1").join("source.docx");
    let bytes = std::fs::read(&docx).expect("the server saved the committed file");
    assert!(bytes.starts_with(b"PK"), "a .docx is a zip package");

    let work = std::env::temp_dir().join(format!("wordcraft-collab-lo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    let status = std::process::Command::new("soffice")
        .args(["--headless", "--convert-to", "txt:Text", "--outdir"])
        .arg(&work)
        .arg(&docx)
        .status()
        .expect("run soffice");
    assert!(status.success(), "LibreOffice converts the committed document");

    let txt = std::fs::read_dir(&work)
        .expect("converted files")
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "txt"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
        .expect("a converted text file");
    assert!(txt.contains("Hello collaboration"), "converted text: {txt:?}");
    let _ = std::fs::remove_dir_all(&work);
}
