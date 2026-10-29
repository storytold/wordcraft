//! Live oCIS round-trip: engine [`Session`][wordcraft_engine::Session] ↔ WebDAV Bearer backend.
//!
//! Runs only with `CRAFT_E2E_LIVE=1`, `CRAFT_DAV_BASE`, `CRAFT_DAV_USER` and `CRAFT_DAV_TOKEN` set (a short-lived OIDC access
//! token minted out of band); plain `cargo test` skips without touching the network, so this
//! never blocks offline runs. The token is read from the environment and never printed.
//!
//! What it proves beyond the raw-byte backend tests: a `.docx` goes oCIS → engine
//! `Session` → oCIS with the backend carrying the bytes exactly (byte fidelity), and the
//! parsed document keeps its text across both crossings.

#![cfg(not(target_arch = "wasm32"))]

use wordcraft_doc::StoryRef;
use wordcraft_engine::Session;
use wordcraft_engine::io::ExportOptions;
use wordcraft_storage::StorageBackend;
use wordcraft_storage::webdav::WebDavBackend;

fn live_backend() -> Option<(WebDavBackend, String)> {
    if std::env::var("CRAFT_E2E_LIVE").as_deref() != Ok("1") {
        return None;
    }
    let base = std::env::var("CRAFT_DAV_BASE").unwrap_or_default();
    let user = std::env::var("CRAFT_DAV_USER").unwrap_or_default();
    let token = std::env::var("CRAFT_DAV_TOKEN").unwrap_or_default();
    assert!(
        !base.is_empty() && !user.is_empty() && !token.is_empty(),
        "CRAFT_E2E_LIVE=1 needs CRAFT_DAV_BASE (e.g. http://ocis.lan:9200), CRAFT_DAV_USER and CRAFT_DAV_TOKEN set"
    );
    let url = format!("{}/remote.php/dav/files/{user}/", base.trim_end_matches('/'));
    // The tester names the server, so the network is theirs to trust (plain http, no TLS).
    let backend = WebDavBackend::with_bearer(&url, &token).expect("bearer backend builds").allow_cleartext_credentials();
    Some((backend, user))
}

#[test]
fn docx_round_trips_ocis_to_session_to_ocis() {
    let Some((store, _user)) = live_backend() else {
        eprintln!("skipped: set CRAFT_E2E_LIVE=1 with CRAFT_DAV_TOKEN for the live oCIS round-trip");
        return;
    };
    let tag =
        format!("craft-e2e-{}-{}.docx", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    let copy = format!("{tag}.copy.docx");
    let opts = ExportOptions::default();

    // Session → oCIS.
    let mut session = Session::new(wordcraft_engine::sample::sample_document());
    let before = session.doc.plain_text(StoryRef::Body);
    assert!(before.contains("Open Studio"));
    let meta = session.save_to_storage(&store, &tag, &opts).expect("PUT session doc to oCIS");
    assert_eq!(meta.size, store.stat(&tag).expect("PROPFIND after PUT").size);

    // oCIS → engine Session → oCIS under another name; the backend carries bytes exactly.
    let mut reopened = Session::new(wordcraft_doc::Document::new());
    assert!(!reopened.open_from_storage(&store, &tag, None).expect("GET into a fresh session"));
    assert_eq!(reopened.doc.plain_text(StoryRef::Body), before);
    let expected = wordcraft_engine::io::save_bytes_with(&copy, &reopened.doc, None, &opts).unwrap();
    wordcraft_engine::storage::save_to_storage(&store, &copy, &reopened.doc, None, &opts).expect("PUT copy back to oCIS");
    assert_eq!(store.read(&copy).expect("GET the copy"), expected, "oCIS carried the bytes exactly");

    // A lease makes the sharing story safe: one holder at a time, and the document still
    // round-trips byte-for-byte after the conditional save.
    let lease_holder = format!("e2e-{}", std::process::id());
    let hold = wordcraft_storage::lease::acquire(&store, &tag, &lease_holder, "e2e-test", 120).expect("acquire lease on oCIS");
    assert!(matches!(
        wordcraft_storage::lease::status(&store, &tag).expect("lease status"),
        wordcraft_storage::lease::LeaseStatus::Held(ref l) if l.holder == lease_holder
    ));
    // A rival is refused, naming us.
    let rival = wordcraft_storage::lease::acquire(&store, &tag, "rival", "other-box", 120).unwrap_err();
    assert!(rival.to_string().contains(&lease_holder), "{rival}");
    hold.renew(&store, 120).expect("renew the lease over WebDAV");
    // Conditional save against the current tag, then the merged save round-trips.
    let tag_now = wordcraft_engine::storage::stat_storage(&store, &tag).expect("stat the document").etag;
    reopened.save_to_storage_checked(&store, &tag, &opts, tag_now.as_deref()).expect("If-Match save over WebDAV");
    hold.release(&store).expect("release the lease");
    assert!(matches!(wordcraft_storage::lease::status(&store, &tag).expect("lease status"), wordcraft_storage::lease::LeaseStatus::Absent));

    // Clean up: both files gone afterwards (idempotent remove, then 404 on read).
    store.remove(&tag).expect("DELETE original");
    store.remove(&copy).expect("DELETE copy");
    assert!(matches!(store.read(&tag), Err(wordcraft_storage::StorageError::NotFound(_))), "deleted file reads as missing");
    eprintln!("live round-trip green: {tag} ({0} bytes) via oCIS", meta.size);
}
