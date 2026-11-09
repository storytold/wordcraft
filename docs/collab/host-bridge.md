# Host bridge: collaboration messages

The web build talks to the cabinet through `craft-web-host` (craft-libs),
which owns the message contract and both transports (same-origin `postMessage` for
an iframe, DOM `CustomEvent`s on the canvas). The collaboration messages live on
the crate's `feat/collab-session-messages` branch (`copoka/craft-libs`
`7666e2b1`, pinned by ADR-0016 §3); `storytold/craft-libs` `main` does not have
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
3. Commit goes through the room. With `COLLAB_CABINET_URL` set, the server
   builds the `.docx` and posts it to the cabinet itself (M87,
   `production.md`); every peer receives `committed` with the cabinet's
   `version`/`etag` and the panel shows them. `wordcraft:commit` (no `bytes`:
   the host may assemble or fetch the file itself) is used only when the app has
   no live session to commit through, and `Message::Committed` from the host
   remains a result the app records. The server's own commit path reports its
   digest and warnings when no callback is configured.
4. Without `Message::Session` no network call is made: the feature flag `collab`
   stays off (TASK.md §1).

## Build note

The six messages live on `feat/collab-session-messages` in `copoka/craft-libs`,
commit **`7666e2b100d8cf12998f9b57d3b036964cc3189c`** ("craft-web-host: carry a
collaboration session over the host bridge"), which is not in
`storytold/craft-libs` `main` yet. Per ADR-0016 §3 the app pins that fork
revision instead of a path dependency, so `apps/wordcraft-web/Cargo.toml` reads:

```toml
craft-web-host = { git = "https://github.com/copoka/craft-libs", rev = "7666e2b100d8cf12998f9b57d3b036964cc3189c", version = "0.1.0" }
```

`Cargo.lock` records the same source and revision
(`git+https://github.com/copoka/craft-libs?rev=7666e2b100d8…`). It is the only
fork dependency in the workspace: `craft-ui`, `craft-i18n`, `craft-window`,
`craft-print`, `craft-proof`, `craft-font-*`, `craft-pdf` and `craft-update`
still point at `storytold/craft-libs` revisions.

Checked on the pin (2026-11-08, HEAD `60f30f0a` + this change):

| Step | Result |
|---|---|
| `cargo check -p wordcraft-web --target wasm32-unknown-unknown` | ok |
| `cargo xtask wasm` | ok, 22 crates |
| `trunk build` in `apps/wordcraft-web` (trunk 0.22.1) | ok — `dist/web` with `index.html`, the JS glue and a 76.9 MB `_bg.wasm` (dev profile) |
| `cargo test -p wordcraft-collab-web -p wordcraft-web` | 14 tests (5 + 9) green |
| `cargo fmt --all --check` | ok |
| `cargo clippy -p wordcraft-collab-web -p wordcraft-web --all-targets -- -D warnings` | ok (native) |

Around the pinned crate the build only prints two pre-existing
`wordcraft-ui-egui` wasm warnings (`unused imports: block_behind/forget_nudge`,
`unused variable: system_arabic`) and no errors; `cargo xtask wasm` stays green
with them.

After the craft-libs PR merges, change the URL back to
`storytold/craft-libs` and pin the merge commit; nothing else in the app moves.

## Checked with two clients (2026-11-09)

The trunk build was hosted by a page that speaks this contract and driven
headlessly in Chrome: two same-origin iframes of
`index.html?host=parent&webgl&author=<name>`, `session` in, typing in both
panes, `session-state` and `presence` out, `commit` out and `committed` in.
The whole chain worked on one origin; what is not covered (two machines, the
real cabinet's storage, the visible commit confirmation) is listed in
`STATUS.md` §5.1.

That run also showed that a client id made from the document id alone is not
unique: a document id is a per-process counter, so both panes joined as
`web-1`, one peer replaced the other and the second client's operation ids
were deduplicated as the first's. The id now comes from
`wordcraft_collab_client::session_client_id` (per session, not per document;
`ui-…` on the desktop, `web-…` in the browser).

## Until then

The desktop build connects directly to the sync server: `collab.enable` stores the
address, `WORDCRAFT_COLLAB_TOKEN` supplies the JWT, and `crates/collab-app` +
`ui-egui/collab.rs` provide the session, participants panel, remote carets, viewer
gate and commit button (`README.md`). The web build opens a session only when the
host page sends `wordcraft:session`; the pinned crate supplies the messages, and
without that message the app stays single-user — the contract rule from
`INTEGRATION.md` §6, not an untested code path.
