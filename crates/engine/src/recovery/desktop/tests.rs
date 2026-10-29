use super::*;
use crate::Session;
use serde_json::json;
use wordcraft_doc::StoryRef;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wordcraft-recovery-test-{}", new_id().unwrap()));
        private_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// A store reopened on `root` whose scan reports an unreadable record. The record's lock can be
/// briefly held by a process another test is starting (a forked child holds the descriptor
/// until it execs), and a record that can't be locked is skipped, not reported: so try again
/// for up to a second rather than read a skipped record as a clean scan.
fn reopen_reporting_unreadable(root: &std::path::Path) -> Store {
    for _ in 0..50 {
        let mut store = Store::new(root.to_path_buf()).unwrap();
        if store.scan().is_err() {
            return store;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the scan never reported the unreadable record");
}
/// `store.load(id)`, tried again for up to a second while the record can't be locked: a process
/// another test is starting holds the lock for a moment (see [`reopen_reporting_unreadable`]).
fn load_retrying(store: &mut Store, id: &str) -> Snapshot {
    let mut last = String::new();
    for _ in 0..50 {
        match store.load(id) {
            Ok(snapshot) => return snapshot,
            Err(e) => last = e,
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the record never loaded: {last}");
}
fn snapshot(id: &str, text: &str) -> Snapshot {
    Snapshot {
        version: 1,
        info: RecoveryInfo { id: id.into(), name: "Synthetic draft".into(), saved_at: crate::cmd::now_iso() },
        generation: 0,
        document: Document::from_text(text),
    }
}
fn settle(s: &mut Session) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        s.recovery_tick();
        if s.recovery_ready() && !s.recovery_busy() {
            break;
        }
        assert!(Instant::now() < end, "recovery did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn session(root: &Path) -> Session {
    let mut s = Session::new(Document::new());
    s.enable_recovery(root.to_owned(), Arc::new(|| {})).unwrap();
    settle(&mut s);
    s
}
/// Every open document's interval has passed.
fn due_now(s: &mut Session) {
    for d in s.recovery_docs() {
        d.slot.due = Instant::now();
    }
}
fn save_now(s: &mut Session) {
    due_now(s);
    s.recovery_tick();
    settle(s);
}
/// The records in the profile folder: the copies on disk.
fn records(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

/// #3140: a save that lands right before the app closes can race the process exiting before
/// the now-redundant recovery copy is actually removed, so the next launch wrongly offers to
/// recover a document that was in fact saved cleanly. `finish_recovery` (desktop hosts call it
/// on exit, the same as they wait for pending print jobs) must discard it synchronously
/// instead, with no regular tick in between — simulating closing right after Save, not after
/// however long it'd take the next normal autosave tick to notice and clean up.
#[test]
fn finish_recovery_discards_its_own_copy_for_a_document_saved_right_before_exit() {
    let profile = Temp::new();
    let mut s = session(&profile.0);
    s.run("document.setText", &json!({"text": "About to be saved"})).unwrap();
    save_now(&mut s);
    assert_eq!(records(&profile.0).len(), 1, "a recovery copy exists for the dirty document");
    // A real Save does this, then the app can close on the very next frame, with no regular
    // recovery_tick() in between to notice `!s.dirty` and queue the discard itself.
    s.dirty = false;
    assert_eq!(records(&profile.0).len(), 1, "confirms the race: setting dirty=false alone (what Save does) doesn't discard anything by itself");
    s.finish_recovery();
    assert!(records(&profile.0).is_empty(), "finish_recovery should have discarded the now-redundant copy before returning");
}

/// `tick` resets `self.id` the moment the session's document no longer matches what it last
/// saw (so it never discards under a stale identity); `finish_recovery` must guard the same
/// way. Switching documents and exiting immediately, with no regular tick in between to reset
/// that identity, must not let the *new* (clean, blank) document's state discard the
/// *previous* document's still-live recovery copy.
#[test]
fn finish_recovery_does_not_discard_a_previous_document_s_copy_after_switching() {
    let profile = Temp::new();
    let mut s = session(&profile.0);
    s.run("document.setText", &json!({"text": "The document the user is still working on"})).unwrap();
    save_now(&mut s);
    assert_eq!(records(&profile.0).len(), 1, "a recovery copy exists for the dirty document");
    s.set_document(Document::new()); // the document is replaced, with no tick() in between
    assert!(!s.dirty, "a freshly switched-to document starts clean");
    s.finish_recovery();
    assert_eq!(records(&profile.0).len(), 1, "replacing the document must not discard the previous one's recovery copy");
    assert_eq!(s.recovery_documents().len(), 1, "no open document keeps it: it is listed");
}

#[test]
fn recovery_commands_only_address_known_profile_records() {
    use crate::file_access::{AutomationRoots, confine};
    let profile = Temp::new();
    let files = Temp::new();
    let original = files.0.join("original.txt");
    fs::write(&original, b"External synthetic original").unwrap();
    let roots = AutomationRoots::new(Some(&files.0), Some(&files.0)).unwrap().unwrap();
    let mut s = session(&profile.0);
    s.run("document.setText", &json!({"text": "Recoverable synthetic draft"})).unwrap();
    save_now(&mut s);
    drop(s);
    let mut s = session(&profile.0);
    let id = s.recovery_documents()[0].id.clone();
    confine(Some(&roots), || {
        // A command cannot use a file path or choose a different storage folder.
        for bad in [original.to_string_lossy().into_owned(), "../original.txt".into()] {
            for action in ["restore", "discard"] {
                let err = s.run("file.recover", &json!({action: bad, "root": files.0})).unwrap_err();
                assert!(err.to_string().contains("no such recovery copy"), "{err}");
            }
        }
        assert_eq!(s.recovery_documents().len(), 1);
        s.run("file.recover", &json!({"restore": id})).unwrap();
        settle(&mut s);
        assert!(s.path.is_none());
        assert!(s.dirty);
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Recoverable synthetic draft");
        // The restored document keeps its copy there: it isn't listed, and can't be discarded.
        assert!(s.recovery_documents().is_empty());
        let err = s.run("file.recover", &json!({"discard": id})).unwrap_err();
        assert!(err.to_string().contains("no such recovery copy"), "{err}");
        settle(&mut s);
        assert_eq!(records(&profile.0), std::slice::from_ref(&id));
    });
    assert_eq!(fs::read(&original).unwrap(), b"External synthetic original");
    assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn failed_write_keeps_last_valid_generation_and_discovery_falls_back() {
    let t = Temp::new();
    let id = new_id().unwrap();
    let mut store = Store::new(t.0.clone()).unwrap();
    store.save(snapshot(&id, "First valid copy")).unwrap();
    let first = store.entries[&id].path.clone();
    let bytes = fs::read(&first).unwrap();
    // Simulate a final rename failure without relying on permissions (root can bypass them).
    let obstruction = t.0.join(&id).join("snapshot-00000000000000000002.json");
    fs::create_dir(&obstruction).unwrap();
    assert!(store.save(snapshot(&id, "Failed copy")).is_err());
    assert_eq!(fs::read(&first).unwrap(), bytes);
    fs::remove_dir(obstruction).unwrap();
    store.save(snapshot(&id, "Second valid copy")).unwrap();
    let second = store.entries[&id].path.clone();
    fs::write(second, b"{truncated").unwrap();
    drop(store);
    let mut reopened = reopen_reporting_unreadable(&t.0);
    assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "First valid copy");
    // Never reuse the corrupt generation's name or prune the valid fallback.
    reopened.save(snapshot(&id, "Third valid copy")).unwrap();
    assert_eq!(fs::read(first).unwrap(), bytes);
    assert_eq!(Store::files(&t.0.join(&id)).unwrap().len(), 2);
}

#[test]
fn size_limit_and_lossy_values_fail_without_committing() {
    let mut out = Limited { inner: Vec::new(), remaining: 3, exceeded: false };
    assert!(out.write_all(b"four").is_err());
    assert!(out.inner.is_empty());
    assert!(out.exceeded);
    let t = Temp::new();
    let id = new_id().unwrap();
    let mut store = Store::new(t.0.clone()).unwrap();
    store.save(snapshot(&id, "valid")).unwrap();
    let first = store.entries[&id].path.clone();
    let bytes = fs::read(&first).unwrap();
    let mut invalid = snapshot(&id, "invalid");
    invalid.document.last_section.page_w = f32::NAN;
    assert!(store.save(invalid).is_err_and(|e| e.permanent));
    assert_eq!(fs::read(first).unwrap(), bytes);
    assert_eq!(Store::files(&t.0.join(id)).unwrap().len(), 1);
}

/// Copies are written with picture bytes in base64 (#586 follow-up); those written before kept
/// them as arrays of numbers and are still listed and restored, bytes intact.
#[test]
fn copies_with_byte_arrays_written_before_still_restore() {
    let t = Temp::new();
    let id = new_id().unwrap();
    let mut s = snapshot(&id, "Synthetic draft with a picture");
    s.generation = 1;
    s.document.media.insert("media/image1.png".into(), Arc::new(vec![0, 1, 127, 128, 254, 255]));
    let mut value = serde_json::to_value(&s).unwrap();
    assert_eq!(value["document"]["media"]["media/image1.png"], "AAF/gP7/");
    value["document"]["media"]["media/image1.png"] = json!([0, 1, 127, 128, 254, 255]);
    private_dir(&t.0.join(&id)).unwrap();
    fs::write(t.0.join(&id).join("snapshot-00000000000000000001.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    let mut store = Store::new(t.0.clone()).unwrap();
    store.scan().unwrap();
    assert_eq!(store.inventory().len(), 1);
    assert_eq!(store.load(&id).unwrap().document, s.document);
}

#[test]
fn snapshot_that_cannot_be_copied_is_not_retried_until_the_document_changes() {
    // #2625: a failure caused by the snapshot itself (here a value the format cannot keep;
    // in the field, the 256 MiB limit) used to be rewritten every interval while dirty.
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("document.setText", &json!({"text": "Synthetic draft"})).unwrap();
    s.doc.last_section.page_w = f32::NAN;
    save_now(&mut s);
    assert!(s.recovery_error().is_some());
    assert!(s.recovery_wait().is_none());
    due_now(&mut s);
    s.recovery_tick();
    assert!(!s.recovery_busy(), "the unchanged document was snapshotted again");
    assert!(s.recovery_error().is_some());
    // An edit tries again.
    s.doc.last_section.page_w = 612.0;
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    save_now(&mut s);
    assert!(s.recovery_error().is_none());
    assert_eq!(records(&t.0).len(), 1);
}

#[test]
fn failed_pruning_cannot_push_a_record_beyond_the_discovery_limit() {
    let t = Temp::new();
    let id = new_id().unwrap();
    let mut store = Store::new(t.0.clone()).unwrap();
    store.save(snapshot(&id, "Last valid copy")).unwrap();
    let first = store.entries[&id].path.clone();
    let bytes = fs::read(&first).unwrap();
    // A failed cleanup must not allow the next write to exceed the reader's bound.
    for n in 2..=MAX_FILES {
        fs::write(t.0.join(&id).join(format!("snapshot-{n:020}.json")), b"{truncated").unwrap();
    }
    assert!(store.save(snapshot(&id, "New copy")).is_err());
    assert_eq!(fs::read(first).unwrap(), bytes);
    drop(store);
    let mut reopened = reopen_reporting_unreadable(&t.0);
    assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "Last valid copy");
}

#[test]
fn failed_first_write_does_not_consume_a_record_slot() {
    let t = Temp::new();
    let mut store = Store::new(t.0.clone()).unwrap();
    let mut invalid = snapshot(&new_id().unwrap(), "Synthetic draft");
    invalid.document.last_section.page_w = f32::NAN;
    assert!(store.save(invalid).is_err());
    assert!(store.directories().unwrap().is_empty());
}

#[test]
fn discovery_reclaims_a_crashed_first_write_but_keeps_valid_records() {
    let t = Temp::new();
    let id = new_id().unwrap();
    let empty = new_id().unwrap();
    let mut store = Store::new(t.0.clone()).unwrap();
    store.save(snapshot(&id, "Valid synthetic draft")).unwrap();
    drop(store);
    let dir = t.0.join(&empty);
    private_dir(&dir).unwrap();
    fs::write(dir.join(format!("pending-{}.tmp", new_id().unwrap())), b"{unfinished").unwrap();
    let mut reopened = Store::new(t.0.clone()).unwrap();
    reopened.scan().unwrap();
    assert!(!dir.exists());
    assert_eq!(load_retrying(&mut reopened, &id).document.plain_text(StoryRef::Body), "Valid synthetic draft");
}

#[test]
fn live_records_are_locked_and_discard_releases_storage() {
    let t = Temp::new();
    let id = new_id().unwrap();
    let mut owner = Store::new(t.0.clone()).unwrap();
    owner.save(snapshot(&id, "live")).unwrap();
    let mut other = Store::new(t.0.clone()).unwrap();
    other.scan().unwrap();
    assert!(other.inventory().is_empty());
    assert!(other.discard(&id).is_err());
    assert!(other.discard("../outside").is_err());
    drop(owner);
    other.scan().unwrap();
    assert_eq!(other.inventory().len(), 1);
    other.discard(&id).unwrap();
    assert!(!t.0.join(id).exists(), "discard must release the record slot");
}

#[test]
fn new_draft_restarts_restores_unsaved_and_explicitly_discards() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Unsaved synthetic draft"})).unwrap();
    save_now(&mut s);
    assert!(s.recovery_wait().is_none(), "unchanged documents must not schedule idle work");
    let expected = s.doc.clone();
    drop(s);
    let mut s = session(&t.0);
    let id = s.recovery_documents()[0].id.clone();
    s.run("file.recover", &json!({"restore": id})).unwrap();
    settle(&mut s);
    assert_eq!(s.doc, expected);
    assert!(s.dirty && s.path.is_none());
    assert!(s.recovery_documents().is_empty(), "the restored document keeps the copy");
    // A crash leaves the restored document's copy, in the same record.
    drop(s);
    let mut s = session(&t.0);
    assert_eq!(s.recovery_documents().len(), 1);
    s.run("file.recover", &json!({"discard": id})).unwrap();
    settle(&mut s);
    save_now(&mut s);
    assert!(s.recovery_documents().is_empty());
    assert!(records(&t.0).is_empty());
}

#[test]
fn existing_file_is_unchanged_and_successful_save_removes_copy() {
    let t = Temp::new();
    let file = t.0.join("original.docx");
    let root = t.0.join("profile");
    crate::io::save_path(&file, &Document::from_text("Original business note")).unwrap();
    let original = fs::read(&file).unwrap();
    let mut s = session(&root);
    s.run("file.open", &json!({"path": file})).unwrap();
    s.run("text.insert", &json!({"text": "Unsaved edit. "})).unwrap();
    save_now(&mut s);
    let expected = s.doc.clone();
    drop(s);
    let mut s = session(&root);
    let id = s.recovery_documents()[0].id.clone();
    s.run("file.recover", &json!({"restore": id})).unwrap();
    settle(&mut s);
    assert_eq!(s.doc, expected);
    assert!(s.path.is_none() && s.dirty);
    assert_eq!(fs::read(&file).unwrap(), original);
    let saved = t.0.join("recovered.json");
    s.run("file.save", &json!({"path": saved})).unwrap();
    settle(&mut s);
    assert!(records(&root).is_empty());
    assert_eq!(fs::read(file).unwrap(), original);
}

#[test]
fn password_barrier_clears_in_flight_plaintext_and_blocks_new_copies() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Confidential synthetic draft"})).unwrap();
    due_now(&mut s);
    s.recovery_tick();
    assert!(s.recovery_busy());
    s.run("file.encrypt", &json!({"password": "synthetic-password"})).unwrap();
    assert!(records(&t.0).is_empty());
    s.run("text.insert", &json!({"text": "More confidential edits"})).unwrap();
    save_now(&mut s);
    assert!(fs::read_dir(&t.0).unwrap().next().is_none());
    // Removing protection makes future recovery possible again.
    s.run("file.encrypt", &json!({"password": null})).unwrap();
    save_now(&mut s);
    assert_eq!(records(&t.0).len(), 1);
}

#[test]
fn failed_password_cleanup_rejects_protection_and_preserves_snapshot() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Synthetic text"})).unwrap();
    save_now(&mut s);
    let id = records(&t.0).remove(0);
    let dir = t.0.join(id);
    // An unknown file prevents removal: don't report that plaintext was purged.
    let first = Store::files(&dir).unwrap()[0].clone();
    let bytes = fs::read(&first).unwrap();
    fs::write(dir.join("unexpected"), b"unexpected").unwrap();
    assert!(s.run("file.encrypt", &json!({"password": "synthetic-password"})).is_err());
    assert!(s.password.is_none());
    assert_eq!(fs::read(first).unwrap(), bytes);
}

#[test]
fn delayed_restore_cannot_replace_new_edits() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Saved draft"})).unwrap();
    save_now(&mut s);
    drop(s);
    let mut s = session(&t.0);
    let id = s.recovery_documents()[0].id.clone();
    s.run("file.recover", &json!({"restore": id})).unwrap();
    s.run("text.insert", &json!({"text": "New work"})).unwrap();
    settle(&mut s);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "New work");
    assert!(s.recovery_error().unwrap().contains("document changed"));
}

/// A restore asked for in one document lands in that document, even when another document
/// is active while the copy loads.
#[test]
fn delayed_restore_lands_in_the_document_that_asked_for_it() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Saved draft"})).unwrap();
    save_now(&mut s);
    drop(s);
    let mut s = session(&t.0);
    let first = s.document_id();
    let id = s.recovery_documents()[0].id.clone();
    s.run("file.recover", &json!({"restore": id})).unwrap();
    s.run("file.new", &json!({})).unwrap();
    let second = s.document_id();
    s.run("text.insert", &json!({"text": "Other work"})).unwrap();
    settle(&mut s);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Other work", "the active document is left alone");
    s.activate(first).unwrap();
    settle(&mut s);
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Saved draft");
    assert_eq!(s.with_document(second, |s| s.doc.plain_text(StoryRef::Body)).unwrap(), "Other work");
    assert_eq!(s.documents().len(), 2);
}

#[test]
fn opening_protected_file_and_password_save_never_leave_plaintext_copies() {
    let t = Temp::new();
    let original = t.0.join("protected.docx");
    let root = t.0.join("profile");
    crate::io::save_path_with(
        &original,
        &Document::from_text("Synthetic confidential note"),
        Some("synthetic-password"),
        &crate::io::ExportOptions::default(),
    )
    .unwrap();
    let bytes = fs::read(&original).unwrap();
    let mut s = session(&root);
    s.run("file.open", &json!({"path": original, "password": "synthetic-password"})).unwrap();
    s.run("text.insert", &json!({"text": "Confidential edit"})).unwrap();
    save_now(&mut s);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
    assert_eq!(fs::read(&original).unwrap(), bytes);
    s.run("file.new", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "New confidential draft"})).unwrap();
    save_now(&mut s);
    assert_eq!(records(&root).len(), 1);
    let saved = t.0.join("saved.docx");
    s.run("file.save", &json!({"path": saved, "password": "synthetic-password"})).unwrap();
    settle(&mut s);
    assert!(fs::read_dir(root).unwrap().next().is_none());
    assert_eq!(fs::read(original).unwrap(), bytes);
}

/// Each open document keeps its own recovery copy: switching between documents doesn't start
/// a new copy of either.
#[test]
fn each_open_document_keeps_its_own_copy() {
    let t = Temp::new();
    let mut s = session(&t.0);
    let first = s.document_id();
    s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
    save_now(&mut s);
    s.run("file.new", &json!({})).unwrap();
    settle(&mut s);
    s.run("text.insert", &json!({"text": "Second synthetic draft"})).unwrap();
    save_now(&mut s);
    s.activate(first).unwrap();
    s.run("text.insert", &json!({"text": ", edited"})).unwrap();
    save_now(&mut s);
    assert_eq!(records(&t.0).len(), 2, "one copy per document");
    assert!(s.recovery_documents().is_empty(), "the copies open documents keep are not listed");
}

#[test]
fn protection_after_document_switch_does_not_discard_another_draft() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
    save_now(&mut s);
    let first = s.document_id();
    s.run("file.new", &json!({})).unwrap();
    // Protection can be commanded before the host's next recovery tick.
    s.run("file.encrypt", &json!({"password": "synthetic-password"})).unwrap();
    settle(&mut s);
    assert!(s.with_document(first, |s| s.recovery_slot.id.is_some()).unwrap());
    assert_eq!(records(&t.0).len(), 1, "the first draft is kept");
}

/// Two dirty documents, one of them parked since before its first copy: both get copies once
/// their interval has passed, without being made active.
#[test]
fn parked_documents_keep_their_copies() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "Parked synthetic draft"})).unwrap();
    let parked = s.document_id();
    s.run("file.new", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Active synthetic draft"})).unwrap();
    let active = s.document_id();
    assert_ne!(parked, active);
    save_now(&mut s);
    assert_eq!(s.document_id(), active, "the parked document was not made active");
    assert_eq!(records(&t.0).len(), 2, "both documents have copies");
    // An edit to the parked document reaches its copy too.
    s.with_document(parked, |s| s.run("text.insert", &json!({"text": ", edited"})).unwrap()).unwrap();
    let id = s.with_document(parked, |s| s.recovery_slot.id.clone()).unwrap().unwrap();
    save_now(&mut s);
    drop(s);
    let mut store = Store::new(t.0.clone()).unwrap();
    store.scan().unwrap();
    assert_eq!(load_retrying(&mut store, &id).document.plain_text(StoryRef::Body), "Parked synthetic draft, edited");
}

/// Closing a dirty document and saving another delete both their copies; a third document
/// keeps its own.
#[test]
fn closing_or_saving_a_document_deletes_only_its_copy() {
    let t = Temp::new();
    let files = Temp::new();
    let mut s = session(&t.0);
    let mut ids = Vec::new();
    for text in ["Closed draft", "Saved draft", "Kept draft"] {
        if !ids.is_empty() {
            s.run("file.new", &json!({})).unwrap();
        }
        s.run("text.insert", &json!({"text": text})).unwrap();
        ids.push(s.document_id());
    }
    save_now(&mut s);
    assert_eq!(records(&t.0).len(), 3);
    let kept = s.recovery_slot.id.clone().unwrap();
    s.close_document(ids[0]).unwrap();
    s.with_document(ids[1], |s| s.run("file.save", &json!({"path": files.0.join("saved.json")})).unwrap()).unwrap();
    settle(&mut s);
    assert_eq!(records(&t.0), [kept], "only the open dirty document's copy is left");
    assert!(s.recovery_documents().is_empty());
}

/// A crash with two dirty documents leaves two copies; the next run lists both and restores
/// each into a document of its own, beside the work already there.
#[test]
fn a_crash_leaves_every_dirty_document_s_copy() {
    let t = Temp::new();
    let mut s = session(&t.0);
    s.run("text.insert", &json!({"text": "First synthetic draft"})).unwrap();
    s.run("file.new", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Second synthetic draft"})).unwrap();
    save_now(&mut s);
    drop(s); // no finish_recovery: the process died
    let mut s = session(&t.0);
    let copies = s.recovery_documents();
    assert_eq!(copies.len(), 2);
    s.run("text.insert", &json!({"text": "Work in progress"})).unwrap();
    let work = s.document_id();
    for copy in &copies {
        s.run("file.recover", &json!({"restore": copy.id})).unwrap();
        settle(&mut s);
    }
    assert!(s.recovery_documents().is_empty(), "the restored documents keep their copies");
    let mut texts: Vec<String> = s.documents().iter().map(|d| s.with_document(d.id, |s| s.doc.plain_text(StoryRef::Body)).unwrap()).collect();
    texts.sort();
    assert_eq!(texts, ["First synthetic draft", "Second synthetic draft", "Work in progress"]);
    assert!(s.with_document(work, |s| s.dirty).unwrap());
}

/// A clean exit deletes the copies of documents saved or closed since their last copy, the
/// parked ones' too, and keeps the dirty ones'.
#[test]
fn finish_recovery_covers_every_open_document() {
    let t = Temp::new();
    let files = Temp::new();
    let mut s = session(&t.0);
    let mut ids = Vec::new();
    for text in ["Saved draft", "Closed draft", "Dirty draft"] {
        if !ids.is_empty() {
            s.run("file.new", &json!({})).unwrap();
        }
        s.run("text.insert", &json!({"text": text})).unwrap();
        ids.push(s.document_id());
    }
    save_now(&mut s);
    assert_eq!(records(&t.0).len(), 3);
    let dirty = s.recovery_slot.id.clone().unwrap();
    // No tick between these and the exit.
    s.with_document(ids[0], |s| s.run("file.save", &json!({"path": files.0.join("saved.json")})).unwrap()).unwrap();
    s.close_document(ids[1]).unwrap();
    s.finish_recovery();
    assert_eq!(records(&t.0), [dirty]);
}
