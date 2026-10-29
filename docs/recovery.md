# Local document recovery

Desktop WordCraft periodically keeps a local copy of unsaved work, including new drafts.
File › Recover lists available copies. The page opens at startup when copies are found,
unless a document was explicitly created or opened before the startup scan completed.
In particular, a window launched for New/Open keeps the requested document visible;
older copies remain available through File › Recover. Recovery uses the regular interface
palette so its labels and buttons stay readable in light and dark themes.
Restore opens the copy as another unsaved document with no file path, beside the open
documents. It replaces only an untouched blank document, so it never replaces work and asks
nothing. Save As chooses where to keep it. Recovery never writes to the original document.
The restored document has no AutoSave permission to write to an earlier file, and keeps its
copy in the record it came from.

File › Recover lists copies from earlier runs: crashed sessions, documents replaced in this
one, and copies whose deletion failed. The copies the open documents keep are not listed;
they are not lost. Continue without restoring leaves copies available. Discard asks for
confirmation and deletes the selected copy, including its retained generation.

Documents with a password do not get plaintext recovery copies. Applying a password waits for
any pending snapshot, removes the current document's recovery generations, and fails if cleanup
fails. Encrypted recovery is not implemented. Documents opened with a password keep recovery
disabled for that session. Removing the password allows future recovery again.

## Several open documents

Every dirty open document keeps its own copy, whether it is shown or not. A crash with several
unsaved documents leaves a copy of each, and the next start lists them all.

| Event | That document's copy |
|---|---|
| Save | Deleted on the next worker cycle |
| Close without saving (Don't Save, `file.closeDocument`) | Deleted on the next worker cycle |
| Don't Save when quitting | Deleted before the app exits |
| Clean exit | Waits for the worker, then deletes the copies of documents saved, closed or protected since their last copy; dirty documents keep theirs |
| Crash | Kept |

## Storage and timing

Copies live in `recovery/` under the app's profile folder. `WORDCRAFT_PROFILE_DIR` selects a
separate profile for isolated runs, including preferences, logs and control keys.
`WORDCRAFT_NO_PREFS` disables recovery as well as preferences. Headless engine/CLI sessions
and the browser do not create recovery files implicitly.

One worker thread and one store serve every open document, so a record is locked once per
process. Every frame the desktop host compares each open document's revision and timer without
making it active, and queues at most one job. A document is snapshotted at most once per ten
seconds, and only when its revision changes. Only a document whose copy is due is cloned (the
shared document model) for the worker. Encoding, file I/O, full-document validation and discovery run on the worker.
An unchanged document does not schedule repeated recovery repaints. The most recent edits can
be lost if a crash happens before the next snapshot or while its write is still pending.

Version 1 snapshots wrap the native JSON document with an id, document name, timestamp and
monotonic generation. Native JSON now includes media bytes, opaque preserved parts, and
chart/SmartArt/OLE source markup and relationships. Media and preserved parts are written as one
base64 string each. Older JSON without these fields, or with their bytes as arrays of numbers (as
written before base64), still opens.
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
again at the next interval. Base64 takes 4/3 of a part's size (a 27 MiB picture makes a 36 MiB
copy), and DOCX compresses, so a copy can still be larger than the DOCX it came from. This first
version has a fixed
interval, English fallback for new labels, and no cloud, version-history or browser recovery.

## Commands and verification

`file.recover={}` reads cached `{enabled,busy,documents,error}`. Each document has `id`, `name`
and `savedAt`. `{"refresh":true}`, `{"restore":"id"}` and `{"discard":"id"}` queue worker
operations, and accept only listed ids. Poll until `busy` is false; inspect `error` for failure.
A restore asked for in an untouched blank document replaces it once that document is active,
and is cancelled if the document changes or closes while the copy loads. Restores never open
dialogs.
`ui.recoveryDiscard={"id":"id"}` opens confirmation; `{"answer":"discard"}` or
`{"answer":"cancel"}` answers it.

Focused tests are `cargo test -p wordcraft-engine recovery`,
`cargo test -p wordcraft-engine --test it recovery_format::`, and
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
