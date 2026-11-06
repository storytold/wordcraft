# Host bridge: collaboration messages

The web build talks to the cabinet through `craft-web-host` (storytold/craft-libs),
which owns the message contract and both transports (same-origin `postMessage` for
an iframe, DOM `CustomEvent`s on the canvas). That crate's `Message` enum and
`Host` methods are closed: today they cover `ready/open/save/dirty/failed/handoff`
only. Collaboration needs six more messages (`INTEGRATION.md` §6).

## What is blocked, and why

WordCraft's `apps/wordcraft-web/src/host.rs` cannot add the messages itself:
the envelope name, origin checks, `CustomEvent` details and the parent window are
private to `craft_web_host`. Duplicating them in the app would fork the protocol
that all five apps share.

A change is needed in `craft-libs/crates/web-host` (a different repository):

```rust
pub enum Message {
    // … existing variants …
    /// host → app: open a collaboration session
    Session { session_id: String, ws_url: String, token: String, role: String },
    /// host → app: the session is committed by the cabinet
    Committed { version: u64, etag: String },
    /// app → host: session state for the cabinet UI
    SessionState { status: String, participants: Vec<Participant> },
    /// app → host: presence for the cabinet UI
    Presence { participants: Vec<Participant> },
    /// app → host: ask the cabinet to commit (when the host owns commit)
    Commit { session_id: String, bytes: Option<Vec<u8>> },
}
```

with matching `post_*` methods and prefixes `<app>:session`, `<app>:session-state`,
`<app>:presence`, `<app>:commit`, `<app>:committed`. The contract version stays
`"1"`: an older host never sends the new messages, so the app simply stays in
single-user mode (the rule from `INTEGRATION.md` §6).

## WordCraft side, once the crate has them

1. `host.rs` handles `Message::Session`: store `session_id/ws_url/token/role` in
   `WordApp`, and only then connect `wordcraft-collab-client` (the token never
   reaches a URL, a log or `localStorage`).
2. `post_session_state` on join/leave/reconnect; `post_presence` coalesced to the
   same 5–10 Hz as the WebSocket presence (ADR-0006).
3. When the cabinet owns commit, `post_commit` carries the bytes the server
   assembled; otherwise the server posts them to the cabinet's callback and the
   app only shows the result.
4. Without `Message::Session` the app makes no network call: the feature flag
   `collab` stays off (TASK.md §1).

## Until then

The desktпop build can connect directly to the sync server (the flag and the
address are in `Prefs.collab`, `collab.enable`/`collab.disable`/`collab.status`
exist in the engine). The web build stays single-user; this is a missing external
contract, not an untested code path.
