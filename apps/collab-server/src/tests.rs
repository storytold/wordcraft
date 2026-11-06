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
    Config { bind: "127.0.0.1:0".into(), jwt_secret: Some("test-secret".into()), jwt_pem: None }
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
