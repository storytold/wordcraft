# Document tabs

> **Last reviewed:** 2026-10-26 · **Last updated:** 2026-10-26 · **Change:** minor (browser picker dismissal and save-authority review fixes) · **Target:** WordCraft desktop and web

New, Open, templates, Recent and dropped documents use the current window. Opening a
saved file already open in that window selects its tab. The Start screen's placeholder
is replaced by the first document. Closing the final document leaves an empty window;
Quit is a separate operation. File > New > New Window opens a separate desktop process.

The strip above the ribbon supports selection, pinning, dragging within the pinned or
unpinned group, Close, Close Others, Close All and reopening a saved file from disk.
The strip reveals the selected tab when selection or order changes, while allowing manual
scrolling. Close Others retains pinned tabs. Reopening never resurrects discarded edits. A window
holds at most 128 documents; recently closed history holds 20 saved paths.

## Document state and safety

Each document retains its engine session, undo/redo, selection, canvas and scroll state,
zoom, protection, comment/reply drafts and permission for AutoSave to write its path.
Application preferences follow the current settings across existing and newly created
tabs, including the dictionary persistence path and effective editing language. Merely opening a file does not authorize AutoSave to rewrite it.

Dirty content, unfinished comment drafts and unconfirmed browser saves all count as
unsaved work. Tab Close, Close Others, Close All and Quit use Save / Don't Save / Cancel.
Cancel stops the remaining batch; failed saves keep the document open with the reason.
Repeated window-close requests preserve the current prompt. An explicit automation close
during Quit cancels that review; subsequent close requests review every remaining dirty
document again. Save, password, encryption
and recovery-discard confirmations prevent switching or opening tabs. Zotero operations
also block those actions while they require the current session.

Every desktop tab has its own recovery worker. Active and inactive sessions tick, and
close/exit finishes their pending work. Recovery failures appear on the status bar.
Restore creates a new unsaved tab and cannot overwrite current writing. Typing before
its recovery copy has loaded cancels restoration rather than replacing the new text. Existing
password protections against plaintext recovery apply to every tab.

Browser save tickets retain the originating document and saved revision. A failed or
cancelled save restores that document's dirty state, including while it is inactive.
Save As picker results save their originating tab. Other document-specific picker
results cancel with an explanation if their document is no longer active. A save needed
to continue a pending close is document modal until its picker finishes. Browser pickers
have dedicated result channels carrying complete batches, separate from dropped files.
One picker may be pending at a time, with visible refusal of a second request; cancellation
or disconnection clears it. Picture and recipient results retain their originating tab
and operation, including when another tab is selected before the read finishes.
Host adapters implement `Services::open_async` by returning a `Receiver<PickedFiles>`:
one complete batch (empty for cancellation), followed by a UI wakeup. They deliver
independent drops and host-initiated files through `Services::inbox`.

Duplicate-file and Save As destination checks apply within each window. Separate
windows do not synchronize edits or prevent each other writing the same file. Their
application preferences use the existing atomic, last-writer-wins publication.

## Keyboard and accessibility

Customize Keyboard records shortcuts without executing them. Explicit user bindings take precedence. Ctrl+Tab / Ctrl+Shift+Tab cycle documents outside
tables when multiple tabs exist and the keys are available. A single document does not
consume cycling. Ordinary Tab / Shift+Tab cell navigation and the platform literal-tab
binding remain available in tables. Ctrl+Shift+T reopens a closed saved file only when
no existing binding owns it. Menus display effective shortcuts with the platform formatter.

Tab names, selected state, unsaved status and individual close controls have accessibility
metadata. Switching interrupts the previous document's input-method composition. Interface strings go through the shared translation catalog; new entries may
fall back to English until translators add them in `storytold/craft-i18n`. No external
translation repository was changed. Real VoiceOver/NVDA/Orca verification remains open.

## Automation

`engine.commands` lists the UI commands. Buttons, keyboard handling and control/MCP
commands use the same IDs:

| Command | Parameters |
|---|---|
| `ui.documentTabs` | none; returns `active`, `activeId` and tabs with stable `id`, display `index`, name, dirty, active and pinned state |
| `ui.switchDocument` | `id` or `index` |
| `ui.closeDocument` | `id` or `index`, optional boolean `force` |
| `ui.closeOtherDocuments` | `id` or `index`, optional boolean `force`; pinned neighbors stay open |
| `ui.closeAllDocuments` | optional boolean `force` |
| `ui.pinDocument` | `id` or `index`, boolean `pinned` |
| `ui.moveDocument` | `from`, `to` display indexes in the same pin group |
| `ui.nextDocument`, `ui.previousDocument` | none |
| `ui.reopenClosedDocument` | none |
| `ui.newWindow` | none; desktop host required |

Indexes and IDs must be nonnegative integers in range; no numeric truncation is used.
Programmatic destructive commands return an error on unsaved work unless `force` is
explicitly `true`. This includes `file.close`, tab batch closes and control-channel
`app.quit`. Scripts never receive a Save Changes dialog. `ui.confirmQuitTab` is internal
and cannot be executed through automation.

## Workspace restoration

The existing `ui.json` format adds a defaulted `workspace` containing tab paths, order,
pinning and selected display index. Ordinary desktop launches restore it. Explicit
startup files, `--sample`, `--control` and `--new-window` take precedence. Missing or
unreadable files are skipped with visible feedback. Restoration processes at most 128
entries and reports skipped entries above that limit. Untitled entries reopen blank;
their text is recovered through recovery copies, not preferences. With no saved workspace,
the existing Start screen preference applies. Explicit Close removes tabs from the saved
workspace; Quit retains it. A secondary `--new-window` process preserves the latest
workspace on disk when saving preferences, so closing it later cannot replace the primary
window's saved tab list. Ordinary formatting/search dialogs close when switching tabs.

## Source revisions and attribution

Adapted from these local, fetched revisions; no upstream changes were published:

- PR [#528](https://github.com/storytold/wordcraft/pull/528),
  `23c107a59378e2a7bdbd3d2c8028eb3ef398ebcb`, authored by
  EvilDemonFluffball (`evildemonfluffball@gmail.com`): tab strip, session slots, pinning,
  reordering, closed-file history, workspace structures and initial regression tests.
- PR [#3059](https://github.com/storytold/wordcraft/pull/3059),
  `02c83a841c00b2f1e3c6c2a18e6f46a82f9e1135`: integration reference. The launcher is adapted
  from dcomp1977 (`dominic@streeem.com`), commit
  `cb9c688c9b5e7ef238b54b387ba3f00c614a658d`, with an explicit New Window request.
- Integration base: upstream `main` at
  `b70c746f` (2026-10-25 checkout), reconciled with `a029d8ae` on 2026-10-26
  before publication. Clipboard changes from the PR stack were excluded.

Cross-window editing synchronization, tab tear-off and tab groups are deferred.
See [document-tabs-validation.md](document-tabs-validation.md) for verification evidence.

A successful export never confirms an editable-document save. Selecting an already
open file by dropping it preserves that tab's existing AutoSave authority.

Browser file pickers also finish on the next page click or keypress after dismissal
when the browser emits neither `change` nor `cancel`. The input and its listeners
are removed before another request. Without transient user activation, a visible
native file input remains available for manual selection.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | minor | Browser picker dismissal, Escape, export/save confirmation and drop AutoSave safeguards |
| 2026-10-26 | minor | Quit retry, dictionary persistence, effective editing language and keyboard recording review fixes |
| 2026-10-26 | minor | PR review fixes: secondary-window workspace preservation, Start preference, dialog guards, search and dirty reporting |
| 2026-10-26 | major | Tabs adapted with document-bound saves, pickers, recovery, drafts, guarded closes and workspace restoration |
