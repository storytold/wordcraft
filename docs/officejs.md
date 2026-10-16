# Office add-ins (Office.js)

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-15 · **Change:** major (first version: what WordCraft's Office.js covers, the bridge, Microsoft's `office.js` neutralised in the page, function commands, colour names, events, limits) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

WordCraft runs **Office add-ins**, the web add-ins built with the Office JavaScript API
(`Office.js`), on top of its own add-ins ([plugins.md](plugins.md#office-add-ins)). An Office
add-in is a web app on its vendor's server, described by an XML manifest. WordCraft reads the
manifest into a plugin: its ribbon buttons and right-click items are ordinary plugin items, listed
in File › Options › Add-ins with the others, and its commands open its **task pane** or call a
**function** of its function file instead of starting a program.

The task pane shows the add-in's page in the system web view (WKWebView on macOS, WebView2 on
Windows; none on Linux yet). Before the page's own scripts run, WordCraft gives it **WordCraft's
Office.js** (`crates/office-js`), written from Microsoft's published API reference (clean room).
Each `context.sync()` of the page becomes one `office.sync` command against the document.

```
 add-in page (vendor's https)              WordCraft
 ┌────────────────────────────────┐      ┌──────────────────────────────────────────┐
 │ the add-in's own scripts       │      │ office_pane (ui-egui): the pane, pump,   │
 │   Word.run(ctx => …)           │ JSON │   function commands, events              │
 │   ctx.sync()                   │ ───► │ office.sync (engine): paths → objects,   │
 │ bootstrap.js + WordCraft's     │ ◄─── │   WordCraft commands, one undo step,     │
 │ Office.js (wasm)               │      │   the manifest's permissions             │
 └────────────────────────────────┘      └──────────────────────────────────────────┘
   wry web view over the task pane (apps/wordcraft/src/office_web.rs)
```

## Installing an Office add-in

File › Options › Add-ins › Load Add-in… and pick the add-in's `manifest.xml` (or
`plugins.load {"path": ".../manifest.xml"}`). The manifest's host must be `Document` and its pages
`https://`. The library itself is built by `cargo xtask office-js` (it needs `wasm-bindgen-cli` at
the version in `Cargo.lock`); the macOS package carries it in `Resources/office-js`, or point
`WORDCRAFT_OFFICE_JS` at the folder.

## What the API covers

WordCraft reports **WordApi 1.3** (plus Settings, Selection, TextCoercion, DocumentEvents and
AddinCommands 1.1) to `isSetSupported` and to the manifest's `<Requirements>` check, with **only
these objects** of WordApi. That's what add-ins check before they start, and what
the other craft apps do with their partial ExcelApi and PowerPointApi; the honest picture is the
table below. Anything else either isn't defined (a `TypeError` in the page) or answers
`NotImplemented`.

| Object | Properties | Methods |
|---|---|---|
| `Word` | | `run(batch)`, `new Word.RequestContext()`; enums `InsertLocation`, `RangeLocation`, `BreakType`, `Alignment`, `UnderlineType`, `SelectionMode`, `Style`/`BuiltInStyleName`, `ErrorCodes` |
| `document` | | `body`, `properties`, `getSelection()`, `save()` |
| `DocumentProperties` | `title`, `subject`, `author`, `keywords`, `comments` | |
| `Body` | `text` (r), `style` | `paragraphs`, `font`, `getRange(Whole/Start/End/Content)`, `insertText(text, Start/End/Replace)`, `insertParagraph(text, Start/End)`, `insertBreak(type, Start/End)`, `clear()`, `search(text, {matchCase, matchWholeWord})` |
| `ParagraphCollection` | `items` | `getFirst()`, `getLast()` |
| `Paragraph` | `text` (r), `style`, `styleBuiltIn`, `alignment`, `firstLineIndent`, `leftIndent`, `rightIndent`, `lineSpacing`, `spaceAfter`, `spaceBefore`, `isListItem` (r) | `font`, `getRange()`, `getNext()`, `getPrevious()`, `insertText(text, Start/End/Replace)`, `insertParagraph(text, Before/After)`, `insertBreak(type, Before/After)`, `delete()`, `select()` |
| `Range` | `text` (r), `isEmpty` (r), `style` | `font`, `paragraphs`, `insertText(text, Start/End/Before/After/Replace)`, `insertParagraph(text, Before/After)`, `insertBreak(type, Before/After)`, `delete()`, `select(Select/Start/End)` |
| `Font` | `bold`, `italic`, `underline`, `size`, `name`, `color`, `highlightColor`, `strikeThrough`, `doubleStrikeThrough`, `subscript`, `superscript` (`null` where the text is mixed) | |

Every object has `load()`, `toJSON()`, `track()`/`untrack()`, `context`, and **`set(properties)`**:
`font.set({name: "Courier New", bold: true, size: 18})`, or an object's own through it
(`paragraph.set({alignment: "Centered", font: {italic: true}})`). Break types are `Page`, `Line`,
and the section breaks `SectionNext`, `SectionContinuous`, `SectionEven`, `SectionOdd` (main text
only).

**Colours.** Wherever a colour is set, it can be `#RRGGBB`, `RRGGBB`, `#RGB`, `#RRGGBBAA` (text
has no transparency: the alpha is dropped) or any of the 148 CSS colour names, in any case (`"blue"`, `"DarkSlateGray"`), as Office.js allows. `highlightColor` also takes Word's
highlight names (`Yellow`, `BrightGreen`, `Pink` is Word's `#FF00FF`) and `null` for none; a
highlight must be one of Word's 16 highlight colours. Reads answer `#RRGGBB`.

**Common API.**

| Member | What it does |
|---|---|
| `Office.onReady(cb)`, `Office.initialize` | after the library and the add-in's settings are ready; `initialize` set late is called at once, as in Office |
| `Office.context` | `host` (`"Word"`), `platform`, `diagnostics`, `displayLanguage`, `contentLanguage`, `requirements.isSetSupported` |
| `Office.context.document` | `getSelectedDataAsync`/`setSelectedDataAsync` (text), `settings` (`get`, `set`, `remove`, `saveAsync`; kept per open document while WordCraft runs, not in the file), `addHandlerAsync`/`removeHandlerAsync` (below), `mode` |
| `Office.actions.associate(id, fn)` | function commands (below); `areShortcutsInUse`, `getShortcuts`, `replaceShortcuts` reject with `NotImplemented` |
| `Office.context.document.getFileAsync`, `getFilePropertiesAsync`, `bindings.*` | call back with a failed `AsyncResult`, code `NotSupported` |
| `Office.context.ui.displayDialogAsync` | calls back with `NotImplemented`; `messageParent` does nothing |
| `Office.addin.*`, `Office.ribbon.*` | reject with `NotImplemented` |
| Enums | `AsyncResultStatus`, `CoercionType`, `HostType`, `PlatformType`, `EventType`, `Index`, `GoToType`, `BindingType`, `StartupBehavior`, `VisibilityMode`, `InitializationReason`, `ValueFormat`, `FilterType`, `SelectionMode`, `DocumentMode`, `ActiveView`, `FileType`, `Table` |
| `OfficeExtension` | `Error` (every error is an `Error` named `RichApi.Error` with `code` and `debugInfo`), `ErrorCodes` |
| `OfficeRuntime` | defined (empty) |

**Events.** `addHandlerAsync(Office.EventType.DocumentSelectionChanged, handler)` works: the
handler gets `{type, document}` once each time the selection changes (checked once a frame;
WordCraft tells every page whose library has started, and the library calls the handlers).
Every other event type answers, asynchronously as every `*Async` call does, a failed
`AsyncResult` with the code `NotSupported`; it never throws.

## The bridge (`office.sync`)

The library queues the page's actions and sends each `context.sync()` as one JSON batch
`{"id", "actions": [{"op": "load"|"set"|"call", "path": [...], …}]}`; WordCraft runs it as the
`office.sync` command: one undo step, all or nothing (a sync that fails changes nothing and
rejects with the failing action's `code`, `message` and index). Word's API has no ids; WordCraft
hands out opaque ones (`p:<path>`, `r:<path>:<from>-<path>:<to>`) so an object keeps naming the
same thing across syncs. The wire format and the id rules are in [plugins.md](plugins.md#office-add-ins)
and `crates/office-js/src/lib.rs`.

Besides syncs, the page and WordCraft exchange:

| From | Message | Meaning |
|---|---|---|
| page | `{"event": "ready"}` | the library has started and loaded the add-in's settings |
| page | `{"event": "functionDone", "call": n, "error"?}` | function command `n` called `event.completed()`, or couldn't run or failed |
| WordCraft | `__craftOfficeReply(answer)` | a sync's answer |
| WordCraft | `__craftOfficeRunFunction({"call": n, "name", "control"})` | run a function command |
| WordCraft | `__craftOfficeEvent({"type": "documentSelectionChanged"})` | the selection changed |

## Microsoft's `office.js` in the page

Add-in pages load Microsoft's `office.js` themselves, from `appsforoffice.microsoft.com` or
`officeapis.public.onecdn.static.microsoft`. WordCraft never ships it, and it must not take over
from WordCraft's library: outside Office it never reports a host, so the page's buttons would
never be wired. The defences, the same in GridCraft and DeckCraft:

1. **The script never runs** (`crates/office-js/bootstrap.js`, all platforms). A `<script>` whose
   source is on Microsoft's office.js CDNs (or an npm copy of `@microsoft/office-js`) gets an
   empty `data:` script instead: parser-inserted ones through a mutation observer, installed
   before the page's HTML is parsed (observers run before the parser runs a script), and ones a
   page's code makes through the `src` setter and `setAttribute`.
2. **Windows: the hosts don't resolve.** WebView2 is started with wry's default arguments plus
   `--host-resolver-rules` mapping both CDN hosts to nothing, so the request never leaves the
   machine (`apps/wordcraft/src/office_web.rs`).
3. **The globals can't be replaced.** `Office`, `Word`, `OfficeExtension` and `OfficeRuntime` are
   accessors on the global object that ignore assignment and can't be redefined or deleted. Until
   the wasm library has started they are stand-ins that hold the page's early calls
   (`Office.onReady`, `Word.run`, `Office.initialize = …`, `Office.actions.associate`); then they
   are the library's objects (handed over with `__craftOfficeAdopt`), whose members are
   read-only. A script that assigns its own `Office` anyway is ignored, except that an
   `initialize` function on it is kept. So `Office.onReady` always waits for WordCraft's library.

Why not block requests in the web view on macOS too: wry (0.57) has no portable hook for a page's
sub-resource requests (custom-protocol handlers only), and WKWebView's content rule lists are
Objective-C calls this repository can't make (`unsafe` is forbidden). There the bootstrap is the
only guard; the browser's preload scanner may still fetch the file, which never runs.

## Function commands (`ExecuteFunction`)

A button or menu item whose action is `ExecuteFunction` calls a function of the add-in's
**function file** (`<FunctionFile>`), a page without a pane:

1. The command carries the function's name, the function file and the manifest's **control id**.
2. WordCraft loads the function file in a hidden web view (one per add-in, kept for its next
   commands) and waits for its library's `{"event": "ready"}`.
3. It calls `__craftOfficeRunFunction({"call", "name", "control"})`. The library waits until the
   page has loaded (its top-level `Office.actions.associate(...)` calls have run), finds the
   function associated under `name` (else, as older function files have it, a global function of
   that name, `a.b.name` too) and calls it with an event whose `source.id` is the control id and
   whose `completed()` tells WordCraft `{"event": "functionDone", "call"}`.
4. WordCraft waits for `completed()` for up to **5 minutes**. Commands aren't serialised: several
   can run at once. A missing function, or one that throws or rejects before completing, is
   reported in the status bar with its message; one that never completes is given up on ("The
   add-in … didn't finish its command."), and its page is closed so the next command loads it
   afresh.

The function's own `Word.run` and common-API calls work as in a task pane (each sync is an undo
step of its own).

## Security

The page runs in the system web view, navigates only to `https://` pages, and opens links that
want a new window in the system browser. What it may do to the document is bounded by the
manifest's `<Permissions>` (`Restricted`, `ReadDocument`, `ReadWriteDocument`…), checked for every
action. Settings are kept per add-in and per open document.

## Limits and next steps

- **Word API:** no content controls, inline pictures, tables, lists API, sections, headers and
  footers, footnotes/endnotes, comments, HTML/OOXML in and out (`insertHtml`, `insertOoxml`,
  `getOoxml`, the `Ooxml` coercion), `insertFileFromBase64`, custom properties.
- **Common API:** no dialog API, no `Office.addin` / `Office.ribbon`, no bindings, no events but
  `DocumentSelectionChanged`, no `getFileAsync`, no `OfficeRuntime.storage`, no SSO
  (`Office.auth`).
- **Manifest:** the XML add-in-only manifest only (no unified JSON manifest); no shared runtime
  (`<Runtimes>`), keyboard shortcuts (`ExtendedOverrides`), `LaunchEvent`s or contextual tabs;
  settings aren't written into the `.docx`, so `Office.AutoShowTaskpaneWithDocument` isn't
  honoured.
- **Platforms:** no web view on Linux or in the web app.

**Checked by hand** (on a Mac or Windows machine, with the web inspector open), because no test
here runs a real web view: that the `office.js` script gets the empty script (and, on Windows, that the request fails) and `Office.onReady`
reports `host: "Word"` (Microsoft's `hello-world/word-hello-world` sample); that a function
command runs once the function file has loaded (`office-add-in-commands/word`: its menu item
inserts "ExecuteFunction works. Button ID=…" in blue); that the task pane's `insertParagraph` with
`font.color = "blue"` works.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version: the API WordCraft's Office.js covers (WordApi 1.3 reported, subset listed; `set()`, `insertBreak`, strikethrough, sub/superscript), CSS colour names, the bridge's messages, Microsoft's `office.js` turned off in the page (and unresolvable on Windows) and the globals protected, function commands end to end (`Office.actions.associate`, `event.completed()`, 5-minute limit), `DocumentSelectionChanged`, common-API enums, `NotSupported`/`NotImplemented` for the rest, late `Office.initialize`, limits; aligned with GridCraft and DeckCraft |
