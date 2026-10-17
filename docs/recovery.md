# Local document recovery

Desktop WordCraft periodically keeps a local copy of unsaved work, including new drafts.
File › Recover lists available copies. The page opens at startup when copies are found.
Restore opens an unsaved document with no file path. Save As chooses where to keep it.
Recovery never writes to the original document. AutoSave's permission to write to an earlier
file is cleared on restore.

Continue without restoring leaves copies available. Discard asks for confirmation and deletes
the selected copy, including its retained generation. It does not delete the editing document.
After discarding that document's copy, another copy is made only after a further edit.
A successful save removes that document's recovery copy on the next worker cycle while it
remains open. Copies left after switching documents can still be explicitly discarded.

Documents with a password do not get plaintext recovery copies. Applying a password waits for
any pending snapshot, removes the current document's recovery generations, and fails if cleanup
fails. Encrypted recovery is not implemented. Documents opened with a password keep recovery
disabled for that session. Removing the password allows future recovery again.

## Storage and timing

Copies live in `recovery/` under the app's profile folder. `WORDCRAFT_PROFILE_DIR` selects a
separate profile for isolated runs, including preferences, logs and control keys.
`WORDCRAFT_NO_PREFS` disables recovery as well as preferences. Headless engine/CLI sessions
and the browser do not create recovery files implicitly.

The desktop host checks for a dirty document every frame but snapshots at most once per ten
seconds, and only when its revision changes. It clones the shared document model for a bounded
worker queue. Encoding, file I/O, full-document validation and discovery run on the worker.
An unchanged document does not schedule repeated recovery repaints. The most recent edits can
be lost if a crash happens before the next snapshot or while its write is still pending.

Version 1 snapshots wrap the native JSON document with an id, document name, timestamp and
monotonic generation. Native JSON now includes media bytes, opaque preserved parts, and
chart/SmartArt/OLE source markup and relationships. Older JSON without these fields still opens.
Paragraph cache revisions are transient and regenerated on load; undo history and selection are
not part of the document snapshot. Recovery preserves the current in-memory model. It cannot
recover content already lost during import or make later export more faithful than the exporter.

Each generation is written exclusively to a temporary file, flushed and synchronized, decoded
and compared with the source document, then renamed to a new generation. The last valid
snapshot is retained until its successor is valid. Discovery falls back to an older valid copy
if the newest is corrupt and reports unreadable files. Abandoned temporary files are cleaned
only after acquiring that record's lock. Native file locks prevent other running instances from
restoring or deleting a live record. Unix folders/files use modes 0700/0600; Windows uses the
profile's inherited permissions. Directory synchronization runs on Unix; Windows synchronizes
the file but does not yet explicitly synchronize directory metadata. Power-loss durability on
Windows is therefore unverified.

Limits are 256 MiB of encoded JSON per snapshot, 2 GiB of snapshot files per profile, 128 records
and 16 generations per record during discovery. Ordinarily only two valid generations remain.
Copies are retained until saved or explicitly discarded; there is no automatic age-based deletion.
Over-limit writes report an error and retain the previous valid copy. A failure caused by the
snapshot itself (the size limit, the folder or disk budget, a value the format cannot keep) is
not retried until the document changes; other failures, such as a lock or I/O error, are tried
again at the next interval. JSON stores binary parts
as byte arrays, so it can be substantially larger than DOCX. This first version has a fixed
interval, English fallback for new labels, and no cloud, version-history or browser recovery.

## Commands and verification

`file.recover={}` reads cached `{enabled,busy,documents,error}`. Each document has `id`, `name`
and `savedAt`. `{"refresh":true}`, `{"restore":"id"}` and `{"discard":"id"}` queue worker
operations. Poll until `busy` is false; inspect `error` for failure. A restore is cancelled if the
document id or revision changes while it loads. Programmatic restores do not open dialogs;
user restores over dirty work use the existing Save Changes prompt.
`ui.recoveryDiscard={"id":"id"}` opens confirmation; `{"answer":"discard"}` or
`{"answer":"cancel"}` answers it.

Focused tests are `cargo test -p wordcraft-engine recovery`,
`cargo test -p wordcraft-engine --test recovery_format`, and
`cargo test -p wordcraft-ui-egui recovery`. They cover lifecycle, preservation, original-file
protection, failed writes, corruption, locks, password cleanup and the actual recovery buttons.
The contribution also uses a native SIGKILL/restart replay in temporary profiles for both new
drafts and edited existing files. It uses synthetic inputs and hashes the original before and
after. No Microsoft Word operation is needed for this reliability test.

## Maintenance validation

The 2026-10-12 rebase retains main’s editable graphic models, native font setup, current
options module, fixed-width Backstage and filename-aware Save As. The SIGKILL/restart
observations above belong to the original contribution, not a rerun on this branch. Current
validation executes the focused lifecycle and preservation tests in temporary profiles and
all repository CI gates; no daily-driver recovery folder or native app is operated.

The desktop host chooses the private recovery profile. Recovery commands accept known
snapshot IDs, never file paths or a profile override. This internal store is separate from
the automation roots that constrain opening and saving user documents.
