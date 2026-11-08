# Host bridge: collaboration messages

The web build talks to the cabinet through `craft-web-host` (storytold/craft-libs),
which owns the message contract and both transports (same-origin `postMessage` for
an iframe, DOM `CustomEvent`s on the canvas). The collaboration messages live on
the crate's `feat/collab-session-messages` branch; the upstream pin does not have
them yet. Collaboration needs six more messages (`INTEGRATION.md` §6).

## What the crate change adds

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

## WordCraft side (implemented)

1. `host.rs` handles `Message::Session`: it stores `session_id/ws_url/token/role`
   in `WordApp` (`host_session`) and only then connects
   `wordcraft-collab-web::Collab`, which drives `wordcraft-collab-client` over
   the browser's `WebSocket`. The token never reaches a URL, a log,
   `localStorage` or preferences (`HostSession`'s `Debug` redacts it).
2. `post_session_state` on join/leave/reconnect/status change; `post_presence`
   coalesced to the same 5–10 Hz as the WebSocket presence (ADR-0006).
3. When the cabinet owns commit, `post_commit` carries no `bytes` (the contract's
   `bytes?` is optional; the cabinet assembles/fetches the file itself) and
   `Message::Committed` shows the result. Otherwise the server commits and the
   panel shows its sha256/warnings.
4. Without `Message::Session` no network call is made: the feature flag `collab`
   stays off (TASK.md §1).

## Build note

The six messages are on the `feat/collab-session-messages` branch of
`storytold/craft-libs` (commit `7666e2b`, pushed to `copoka/craft-libs`), not in
the upstream pin. For a local build point `craft-web-host` at that checkout (or a
git pin to the branch); the committed manifest keeps the upstream pin until the
craft-libs change lands.

## Until then

The desktop build connects directly to the sync server: `collab.enable` stores the
address, `WORDCRAFT_COLLAB_TOKEN` supplies the JWT, and `crates/collab-app` +
`ui-egui/collab.rs` provide the session, participants panel, remote carets, viewer
gate and commit button (`README.md`). The web build stays single-user; this is a
missing external contract, not an untested code path.
