//! Cooperative TTL leases: safe sharing without a server (issue #518, Phase 3).
//!
//! A lease is a small JSON sidecar next to the document (`<doc>.lease`, hidden from
//! [`list`][crate::StorageBackend::list] like the other bookkeeping files) naming who holds
//! the document, on which device, and until when:
//!
//! ```json
//! {"holder":"alice","device":"studio-mac","expires":1791991200,"lease_secs":120}
//! ```
//!
//! The expiry is the whole point: unlike Word's `~$` owner files, a crashed holder can't leave
//! a phantom lock — anyone may take over an expired lease. Holders that stay alive renew
//! (`heartbeat`) well before the TTL runs out; the recommended cadence is a third of the TTL.
//!
//! The protocol needs nothing the backends don't already have (plain `read` / `write` /
//! `remove`, `If-Match` for takeovers and renewals), so it works identically on a local folder
//! and on oCIS over WebDAV, and every app using this crate interoperates. Races are closed by
//! read-back verification: after writing a lease the writer reads it back and only wins when
//! its own holder/device record is the one stored — two writers racing an absent lease can't
//! both succeed, with no compare-and-swap primitive required.
//!
//! Leases are cooperative, not enforced: nothing stops a client from writing without one.
//! The engine pairs them with conditional writes (`If-Match` on the document's ETag), so a
//! writer that skipped the lease still can't silently overwrite — it gets
//! [`StorageError::Conflict`][crate::StorageError::Conflict] instead. Lease release is
//! read-check-then-remove: two holders racing release/renew may briefly disagree, but the TTL
//! bounds the damage and the loser of a renew finds out on its next heartbeat.
//!
//! Clocks: expiry is compared against each holder's own clock, so keep the TTL comfortably
//! above the worst clock skew you expect (the default is two minutes; LAN peers are
//! milliseconds apart). Times are whole Unix seconds.

use serde::{Deserialize, Serialize};

use crate::{StorageBackend, StorageError, WriteOptions, clean_path};

/// What a lease file holds: who, where, until when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// Who holds the document (a user name, an e-mail — free text, shown to rivals).
    pub holder: String,
    /// Which device holds it (shown beside the holder so people can find the laptop).
    pub device: String,
    /// Unix seconds when the lease lapses unless renewed.
    pub expires: u64,
    /// The TTL this lease was granted with (advisory: renew at a third of this).
    pub lease_secs: u64,
}

/// A lease this process holds: the record plus the version tag renewals condition on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseHold {
    /// The document path the lease guards (not the lease file's own path).
    pub path: String,
    /// The record as stored.
    pub lease: Lease,
    /// The lease file's version tag when acquired (`None` where the backend tracks none).
    pub etag: Option<String>,
}

/// What [`status_at`] found next to a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseStatus {
    /// No lease file stored.
    Absent,
    /// Held and unexpired at `now`.
    Held(Lease),
    /// Stored but lapsed at `now`: anyone may take it over with [`acquire`].
    Expired(Lease),
    /// A lease file is stored but doesn't parse: take it over with [`acquire`] (which
    /// replaces the bytes) rather than trusting it; never renew or release it.
    Corrupt,
}

/// Shortest grantable TTL, in seconds: below this heartbeats can't reliably outrun latency.
pub const MIN_TTL_SECS: u64 = 10;
/// Longest grantable TTL, in seconds (a day): leases are for editing sessions, not ownership.
pub const MAX_TTL_SECS: u64 = 86_400;
/// Longest holder/device label, in characters.
pub const MAX_LABEL_LEN: usize = 128;

/// The lease file's backend-relative path for a document path.
pub fn lease_path(path: &str) -> Result<String, StorageError> {
    Ok(format!("{}.lease", clean_path(path)?))
}

fn check_label(kind: &str, value: &str) -> Result<(), StorageError> {
    if value.is_empty() || value.chars().count() > MAX_LABEL_LEN {
        return Err(StorageError::BadPath(format!("lease {kind} must be 1-{MAX_LABEL_LEN} characters")));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(StorageError::BadPath(format!("lease {kind} must not hold control characters")));
    }
    Ok(())
}

fn check_ttl(ttl_secs: u64) -> Result<(), StorageError> {
    if !(MIN_TTL_SECS..=MAX_TTL_SECS).contains(&ttl_secs) {
        return Err(StorageError::BadPath(format!("lease TTL must be {MIN_TTL_SECS}-{MAX_TTL_SECS} seconds")));
    }
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_lease(backend: &dyn StorageBackend, lease: &str) -> Result<Option<Lease>, StorageError> {
    match backend.read(lease) {
        Ok(bytes) => match serde_json::from_slice::<Lease>(&bytes) {
            Ok(l) => Ok(Some(l)),
            Err(_) => Err(StorageError::Protocol(format!("{lease}: stored lease does not parse"))),
        },
        Err(StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn is_live(lease: &Lease, now: u64) -> bool {
    lease.expires > now
}

fn held_by_other(lease: &Lease, holder: &str, device: &str) -> bool {
    lease.holder != holder || lease.device != device
}

/// Take the lease on `path` for (`holder`, `device`) with a TTL of `ttl_secs`.
///
/// Succeeds when no lease is stored, when the stored lease has lapsed, or when the stored
/// lease file doesn't parse (a crashed writer's residue — it is replaced, never trusted). A
/// live lease held by anyone else fails with [`StorageError::Conflict`] naming the holder and
/// its expiry, so a person knows who to ask. Renew with [`LeaseHold::renew`] before the TTL
/// runs out; give up the lease with [`LeaseHold::release`] when done.
pub fn acquire(backend: &dyn StorageBackend, path: &str, holder: &str, device: &str, ttl_secs: u64) -> Result<LeaseHold, StorageError> {
    check_label("holder", holder)?;
    check_label("device", device)?;
    check_ttl(ttl_secs)?;
    let lease_file = lease_path(path)?;
    let now = now_secs();
    // A live rival lease ends the attempt here; everything else goes through the write below
    // (absent, lapsed, or unparseable), where read-back verification settles races.
    let takeover_etag: Option<String> = match read_lease(backend, &lease_file) {
        Ok(None) => None,
        Ok(Some(stored)) if !is_live(&stored, now) => backend.stat(&lease_file).ok().and_then(|m| m.etag),
        Ok(Some(stored)) if !held_by_other(&stored, holder, device) => {
            // Our own record (a previous acquire whose reply we lost, or a second handle):
            // fall through and re-grant rather than reporting a conflict with ourselves.
            backend.stat(&lease_file).ok().and_then(|m| m.etag)
        }
        Ok(Some(stored)) => {
            return Err(StorageError::Conflict(format!("{path}: held by {} on {} until {}", stored.holder, stored.device, stored.expires)));
        }
        Err(_) => None,
    };
    let granted = Lease { holder: holder.into(), device: device.into(), expires: now.saturating_add(ttl_secs), lease_secs: ttl_secs };
    let bytes = serde_json::to_vec(&granted).map_err(|e| StorageError::Protocol(format!("{lease_file}: {e}")))?;
    let write_opts = WriteOptions { if_match: takeover_etag, atomic: backend.capabilities().atomic_write };
    // A conditional write can still fail where the version tag rotated under us (a rival took
    // over between our read and this write): that is a lost race, i.e. a conflict.
    if backend.write(&lease_file, &bytes, &write_opts).is_err() {
        return Err(StorageError::Conflict(format!("{path}: lost the race for the lease")));
    }
    confirm(backend, path, &lease_file, &granted)
}

/// Re-read the lease file and win only when our own record is the one stored.
fn confirm(backend: &dyn StorageBackend, path: &str, lease_file: &str, granted: &Lease) -> Result<LeaseHold, StorageError> {
    let etag = backend.stat(lease_file).ok().and_then(|m| m.etag);
    match read_lease(backend, lease_file) {
        Ok(Some(stored)) if stored.holder == granted.holder && stored.device == granted.device => {
            Ok(LeaseHold { path: path.into(), lease: stored, etag })
        }
        _ => Err(StorageError::Conflict(format!("{path}: lost the race for the lease"))),
    }
}

impl LeaseHold {
    /// Push the expiry `ttl_secs` into the future. Fails with [`StorageError::Conflict`] when
    /// someone else holds the lease now (it lapsed and was taken over, or was released): the
    /// holder must [`acquire`] again rather than keep writing under a dead lease.
    ///
    /// The stored lease decides, never this handle's own claim: the holder and device are read
    /// back first, and the write conditions on that version, so a handle can't renew — or
    /// overwrite — a lease someone else took over.
    pub fn renew(&self, backend: &dyn StorageBackend, ttl_secs: u64) -> Result<LeaseHold, StorageError> {
        check_ttl(ttl_secs)?;
        let lease_file = lease_path(&self.path)?;
        let (etag, holder, device) = match read_lease(backend, &lease_file) {
            Ok(Some(stored)) if !held_by_other(&stored, &self.lease.holder, &self.lease.device) => {
                let etag = backend.stat(&lease_file).ok().and_then(|m| m.etag);
                (etag, stored.holder, stored.device)
            }
            Ok(Some(stored)) => {
                return Err(StorageError::Conflict(format!("{}: lease now held by {} on {}", self.path, stored.holder, stored.device)));
            }
            _ => return Err(StorageError::Conflict(format!("{}: our lease is gone (released or taken over)", self.path))),
        };
        let granted = Lease { holder, device, expires: now_secs().saturating_add(ttl_secs), lease_secs: ttl_secs };
        let bytes = serde_json::to_vec(&granted).map_err(|e| StorageError::Protocol(format!("{lease_file}: {e}")))?;
        let write_opts = WriteOptions { if_match: etag, atomic: backend.capabilities().atomic_write };
        // A failed conditional write means the lease rotated under us: someone else's now.
        backend
            .write(&lease_file, &bytes, &write_opts)
            .map_err(|_| StorageError::Conflict(format!("{}: lost the race renewing the lease", self.path)))?;
        confirm(backend, &self.path, &lease_file, &granted)
    }

    /// Give the lease up. Succeeds when no lease file is stored (already gone); fails with
    /// [`StorageError::Conflict`] when someone else holds it — never delete a rival's lease.
    pub fn release(&self, backend: &dyn StorageBackend) -> Result<(), StorageError> {
        let lease_file = lease_path(&self.path)?;
        match read_lease(backend, &lease_file) {
            Ok(None) => Ok(()),
            Ok(Some(stored)) if held_by_other(&stored, &self.lease.holder, &self.lease.device) => {
                Err(StorageError::Conflict(format!("{}: lease held by {} on {}; not ours to release", self.path, stored.holder, stored.device)))
            }
            Ok(Some(_)) => backend.remove(&lease_file),
            Err(_) => Err(StorageError::Conflict(format!("{}: stored lease does not parse; not ours to release", self.path))),
        }
    }
}

/// What the lease file next to `path` holds, judged at this moment.
pub fn status(backend: &dyn StorageBackend, path: &str) -> Result<LeaseStatus, StorageError> {
    status_at(backend, path, now_secs())
}

/// [`status`] against an explicit clock (tests pin `now`; production passes the wall clock).
pub fn status_at(backend: &dyn StorageBackend, path: &str, now: u64) -> Result<LeaseStatus, StorageError> {
    let lease_file = lease_path(path)?;
    match backend.read(&lease_file) {
        Ok(bytes) => match serde_json::from_slice::<Lease>(&bytes) {
            Ok(l) if is_live(&l, now) => Ok(LeaseStatus::Held(l)),
            Ok(l) => Ok(LeaseStatus::Expired(l)),
            Err(_) => Ok(LeaseStatus::Corrupt),
        },
        Err(StorageError::NotFound(_)) => Ok(LeaseStatus::Absent),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalBackend;

    struct Scratch {
        path: std::path::PathBuf,
    }

    impl Scratch {
        fn create(name: &str) -> Scratch {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("wordcraft-lease-{name}-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratch { path }
        }

        fn backend(&self) -> LocalBackend {
            LocalBackend::open(&self.path).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn plant_expired(store: &LocalBackend, path: &str, holder: &str) {
        let stale = Lease { holder: holder.into(), device: "old-laptop".into(), expires: 1, lease_secs: 120 };
        let bytes = serde_json::to_vec(&stale).unwrap();
        store.write(&lease_path(path).unwrap(), &bytes, &WriteOptions::default()).unwrap();
    }

    #[test]
    fn full_lifecycle_against_a_local_backend() {
        let scratch = Scratch::create("life");
        let store = scratch.backend();
        assert_eq!(status(&store, "memo.docx").unwrap(), LeaseStatus::Absent);
        let hold = acquire(&store, "memo.docx", "alice", "studio-mac", 120).unwrap();
        assert_eq!(hold.lease.holder, "alice");
        assert!(hold.etag.is_some(), "local backend versions its files");
        match status(&store, "memo.docx").unwrap() {
            LeaseStatus::Held(l) => assert_eq!(l.device, "studio-mac"),
            s => panic!("expected Held, got {s:?}"),
        }
        let renewed = hold.renew(&store, 120).unwrap();
        assert!(renewed.lease.expires >= hold.lease.expires);
        // Re-acquiring our own lease re-grants instead of conflicting with ourselves.
        acquire(&store, "memo.docx", "alice", "studio-mac", 120).unwrap();
        hold.release(&store).unwrap();
        assert_eq!(status(&store, "memo.docx").unwrap(), LeaseStatus::Absent);
        // Releasing twice is fine (idempotent, like remove).
        hold.release(&store).unwrap();
    }

    #[test]
    fn rival_holder_gets_a_named_conflict() {
        let scratch = Scratch::create("rival");
        let store = scratch.backend();
        let alice = acquire(&store, "memo.docx", "alice", "studio-mac", 120).unwrap();
        let err = acquire(&store, "memo.docx", "bob", "field-tablet", 120).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("alice") && msg.contains("studio-mac"), "{msg}");
        // Bob can't renew or release Alice's lease either.
        let bob_hold = LeaseHold {
            path: "memo.docx".into(),
            lease: Lease { holder: "bob".into(), device: "field-tablet".into(), expires: u64::MAX, lease_secs: 120 },
            etag: None,
        };
        assert!(bob_hold.renew(&store, 120).is_err());
        assert!(bob_hold.release(&store).is_err());
        alice.release(&store).unwrap();
        // And once Alice leaves, Bob takes it cleanly.
        acquire(&store, "memo.docx", "bob", "field-tablet", 120).unwrap();
    }

    #[test]
    fn expired_and_corrupt_leases_are_takeable() {
        let scratch = Scratch::create("takeover");
        let store = scratch.backend();
        plant_expired(&store, "old.docx", "crashed-carol");
        match status(&store, "old.docx").unwrap() {
            LeaseStatus::Expired(l) => assert_eq!(l.holder, "crashed-carol"),
            s => panic!("expected Expired, got {s:?}"),
        }
        let hold = acquire(&store, "old.docx", "dave", "desk", 120).unwrap();
        assert_eq!(hold.lease.holder, "dave");
        hold.release(&store).unwrap();

        store.write(&lease_path("garbage.docx").unwrap(), b"{not json", &WriteOptions::default()).unwrap();
        assert_eq!(status(&store, "garbage.docx").unwrap(), LeaseStatus::Corrupt);
        acquire(&store, "garbage.docx", "dave", "desk", 120).unwrap();
    }

    #[test]
    fn lease_sidecars_stay_hidden_from_listing() {
        let scratch = Scratch::create("hidden");
        let store = scratch.backend();
        let bytes = b"PK\x03\x04docx".to_vec();
        store.write("memo.docx", &bytes, &WriteOptions::default()).unwrap();
        acquire(&store, "memo.docx", "alice", "studio-mac", 120).unwrap();
        let listed = store.list("").unwrap();
        assert_eq!(listed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), vec!["memo.docx"], "{listed:?}");
        // The sidecar is readable through the API even though listing hides it.
        assert!(matches!(status(&store, "memo.docx").unwrap(), LeaseStatus::Held(_)));
    }

    #[test]
    fn bad_inputs_are_rejected() {
        let scratch = Scratch::create("inputs");
        let store = scratch.backend();
        assert!(acquire(&store, "m.docx", "", "d", 120).is_err());
        assert!(acquire(&store, "m.docx", "h", "", 120).is_err());
        assert!(acquire(&store, "m.docx", "h", "d", 0).is_err());
        assert!(acquire(&store, "m.docx", "h", "d", MAX_TTL_SECS + 1).is_err());
        assert!(acquire(&store, "../escape.docx", "h", "d", 120).is_err());
        assert_eq!(status(&store, "m.docx").unwrap(), LeaseStatus::Absent);
    }
}
