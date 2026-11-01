# Optional external file providers — WordCraft

The web editor exposes a small trusted same-origin capability. **It is disconnected and
inactive by default.** Serving the normal build, a query parameter, a stored preference or
receiving an unsolicited message does not activate it. Desktop, CLI and engine/control-channel
file commands retain their existing behavior. No storage server, account database, launcher,
cloud product, domain or authentication system is added.

An embedding page can opt in by explicitly calling
`window.wordcraftFileProviders.registerProvider(id, label, namespace)` after the editor starts.
The object is frozen, has `protocolVersion: 1`, and announces readiness on the
`wordcraft-file-provider` CustomEvent. An integrator can also discover it directly if the ready
event has already fired. The matching API/event names are the only app-specific transport
names; the protocol matches PhotoCraft's optional provider bridge.

## Host authority and activation

The provider is trusted code on the editor's origin, not an untrusted message sender. It owns
storage, authentication, per-user authorization, namespace selection and the destination
chooser. There is no cross-origin listener and no provider-origin URL to configure inside the
editor. Resource IDs are opaque identities, never filesystem paths or arbitrary engine commands.
The backend must enforce ownership on every list/read/write; browser namespaces do not replace
server authorization.

`registerProvider(id, label, namespace)` returns a fresh connection ID. Re-registering the exact
identity is idempotent. Changing account requires `unregisterProvider()` first, which refuses
while a chooser, serialization, upload acknowledgement or read is unresolved. Unregistering
clears resources and bindings without closing documents. `setEnabled(false)` stops new
provider choices and hides provider Recents while allowing outstanding operations to settle;
`setEnabled(true)` restores availability. There is no editor preference or end-user cloud switch.
Disabling choices preserves existing document bindings and pending saves: local copies still
cannot acknowledge those provider checkpoints, including after a browser picker finishes.
Never reuse an account namespace for a different account.

## Save protocol

1. A user presses Save or Save As in the editor. The editor emits `saveDialog` with
   `requestId`, `documentId`, `revision`, `name`, `saveAs` and an optional `resourceId`.
   Serialization has not happened and nothing has been marked saved. Repeated Save while
   this document already has an outstanding operation reuses that operation.
2. The host asks for **Provider**, **This device**, or **Cancel** every time, then calls
   `resolveSaveDialog(requestId, destination, name)`. Destinations are `provider`, `device`,
   `cancel`. An empty name retains the suggestion. Cancel needs no further reply.
3. On a subsequent logic frame, the editor encodes a complete package in the selected format.
   Provider selection emits `save` with a fresh owned `Uint8Array` in `bytes`, its
   `bytesLength`, the exact captured `documentId`/`revision`, `name`, `resourceId` and request
   identity. Later edits cannot mutate those bytes. Save As never infers overwrite authority
   from a matching filename.
4. Persist that exact package durably, then call `saveResult(requestId, "saved", resourceId)`.
   Only a matching outstanding request can acknowledge the captured revision. Edits made after
   capture stay dirty, closed document IDs are ignored, and another tab/document is never
   marked saved. Other open documents bound to the same overwritten resource lose their stale
   binding and clean state; unrelated local files with the same name are untouched.
5. On failure call `saveResult(requestId, "failed", "")`, or use `cancel` for cancellation.
   `download` is a terminal fallback result that does not acknowledge provider persistence.
   These outcomes keep unsaved work open. A serialization/filename rejection emits `saveFailed`.

A device choice uses the browser's local download service. A device copy of a provider-bound
project does not acknowledge provider persistence or replace that binding. As with the normal
browser fallback, accepting a download does not prove the browser wrote it to physical disk.
Device copies therefore retain the document's unsaved-work state, including for documents
without a provider binding. Explicit-path saves and template/export pickers keep their local route.
For a provider-bound document, explicit-path Save/Save As and template/export copies preserve
its unsaved provider state, including comment drafts committed by Save. A browser copy is never
an acknowledgement of provider durability. Unbound native local saves keep their original behavior.

Supported Save packages: **DOCX, DOCM, DOTX and DOTM**.

The ordinary Word package serializer includes the document, embedded pictures, headers/footers, comments and preserved package parts. Existing document encryption is retained. Export remains the route for PDF, images and text. An encrypted provider read requiring a password fails explicitly; it never binds the previously active document. A host must supply bytes the editor can open.

## Recent files and opening

`setResources(connectionId, resources)` replaces a bounded list for the current connection.
Each resource is `{ id, name, updatedAt? }`; IDs must be unique. `updatedAt` is display text,
not a sorting instruction or trusted timestamp. The editor shows these entries in its native
Home/Open Recent section under the provider's label, separately from **Recent on this device**.
No provider resource is inserted into the device's recent filesystem paths. A successful save
also refreshes the provider entry in this session; the host supplies persistence across visits.

Clicking a provider Recent or calling `requestOpen(resourceId)` emits `openRequest` and returns
its correlated request ID. Read bytes from authorized storage and call
`openResource(requestId, name, Uint8Array)`. Only requests the editor issued are accepted.
`cancelOpen(requestId, message)` finishes a cancelled or failed read. After decoding/admitting
an actual document, the editor emits `opened`; malformed/unsupported/password-required bytes
emit `openFailed`. Bytes are not dropped into an uncorrelated general file-picker inbox.
Resource identity comes from the outstanding read, not a filename sent by the host.

## Identity, limits and lifecycle

Every event carries `providerId`, `namespace` and `connectionId`. Request IDs contain a random
per-WASM-instance nonce plus a checked monotonic counter. Late replies from a previous page,
connection, account, read or completed save cannot address a newer request. Metadata never
contains credentials. `snapshot()` reports connection state, current provider resources,
document IDs/dirty state and outstanding work; `setResources` rejects stale connection IDs.

- 80 MiB per file and aggregate queued open/capture event bytes.
- 16 pending operations; 64 queued events; 100 listed resources.
- IDs: 1–160 ASCII letters/digits, hyphen or underscore.
- Filenames: 1–255 UTF-8 bytes; no controls, slash, backslash, `.` or `..`.
- Labels and update text: at most 100 UTF-8 bytes; read-error text at most 1024 bytes.
- The WASM adapter checks JavaScript string/array/byte lengths before copying them into Rust;
  arbitrary extra host object properties are not stringified or copied.

The host should register listeners before requesting work and always settle each request.
A failing event listener cannot silently acknowledge a save. The editor keeps authoritative
captures pending until the host reports an explicit terminal outcome. This contract does not
provide autosave, a background synchronization daemon, storage conflict resolution or remote
session authentication. Those are host/backend decisions.

## Validation sources

`crates/ui-egui/src/file_providers/tests.rs` covers opt-in defaults, metadata bounds,
coalescing, choices, exact captures, cancellation, identity changes, aliases and correlated
reads. `crates/ui-egui/src/app/provider_files.rs` includes native package round-trip and
late-edit/failure regressions. They are source fixtures; execution results belong in the PR's
validation report after running the repository's checks.

Comment/reply drafts are committed again when a destination is resolved, including text typed
while the choice was open. A refused draft fails the save; an inactive document with parked
comment drafts must be activated and finished first. ACKs do not clear newer comment drafts.
Resource overwrites advance alias revisions so delayed ACKs cannot clean an invalidated tab.
Provider saves with opaque VBA require a macro-enabled .docm/.dotm filename; Device retains the
official writer and format behavior.

Opening provider bytes preserves the complete device Recent list, including when decoding fails;
a provider filename is never admitted as a local path. Provider event consumers select a pending
save dialog by event type rather than assuming the event queue contains only that dialog.
