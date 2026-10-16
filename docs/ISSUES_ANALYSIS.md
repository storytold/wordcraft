# WordCraft GitHub Issues Analysis

*2026-10-13 · 271 open issues fetched via `gh issue list` (all of `storytold/wordcraft`, no PRs;
full bodies in the fetch; zero labels on all). Status legend: **FIXED** (fixed + tested on this
branch), **ALREADY FIXED** (verified in current code with file:line evidence), **MITIGATED**
(partial), **OBSOLETE** (only meaningful on a removed build target), **NOT REPRODUCIBLE** (what
was tried), **NEEDS DECISION** (feature request deferred with rationale), **NEEDS FIX** (open,
not yet addressed, root-cause hypothesis included where one was established). Nothing was
closed or commented on GitHub; commits reference numbers.*

## Deep-dives (investigated with code evidence)

### #1265 Cut does not write to the clipboard — FIXED (commit `6817fb7`, extended `5ee4e70`)
Copy worked, Cut didn't: ribbon `cut` (`ui-egui/ribbon.rs:510`) and the canvas context-menu Cut
(`ui-egui/canvas.rs:2484`) ran `edit.cut` (which correctly fills `Session.clipboard` via
`engine/cmd/edit.rs:copy`) but never called `copy_text`, unlike the Copy buttons next to them.
Only the Ctrl+X keyboard path (`keys.rs:317`) wrote the system clipboard. Fix: both buttons now
push `session.clipboard_text` (Cut only when non-empty); all three sites route through
`system_clipboard::copied`, which also writes the HTML rendering (see #1243). Verified by code
inspection; headless clipboard-write test added for the HTML path.

### #1268 `file.close` over the control channel quits on unsaved changes — FIXED (`618903f`)
`WordApp::run` (user path) stops discarding commands on a dirty document with a SaveChanges
dialog (`ui-egui/lib.rs:605`, `discards_document` at `:1773`, added for #151 in `802ba30`), but
the control channel goes through `WordApp::execute`, which called `session.run` directly and
quit. Fix: `execute` returns `the document has unsaved changes (pass {"force": true} …)` when
dirty + discarding and no `force:true`; the answered-dialog path (`ui.saveChanges` → Don't Save)
injects `force` so normal flows are unaffected. Tests: `execute_refuses_to_discard_unsaved_changes`
plus the pre-existing `new/open_asks…` suites pass.

### #1243 Rich text lost on copy to other apps — FIXED (`5ee4e70`)
`edit.copy` filled only the internal `Fragment`; every copy site called egui's text-only
`copy_text`; `apps/wordcraft/clipboard.rs` only ever *read* the system clipboard. Fix:
`Fragment::to_html` (`doc/edit.rs`: escaped text, bold/italic/underline/strike/color/highlight/
size/sub-superscript/alignment/tables, final-shown runs, 4 MiB cap) + `Services::clipboard_write`
hook + `system_clipboard::copied` used at all three copy sites + arboard `set_html(html,
Some(text))` in the desktop app (failures warn, text copy preserved). Unit tests in `doc/edit.rs`
and `system_clipboard.rs`. Limitation: pictures/fields have no HTML here (see #375).

### #1242 Table Height/Width hit one row/column — FIXED (`76a5524`)
`table.columnWidth`/`table.rowHeight` (`engine/cmd/table.rs`) applied only to the caret cell's
row/column (`cell(s)`), ignoring `selected_cells` (which already handled cell ranges). Fix:
resolve all selected grid columns/rows (immutable borrow first, grid-index math panic-safe via
`get`/`min`, mirroring `grid_col` at `doc/table.rs:93` + `sync_cell_widths`), then apply in
`with_table`. `columnWidth` also clears the table preferred width so layout keeps it. Test:
`cell_size_boxes_apply_to_the_whole_selection` (whole-table select → uniform 90 pt / 25 pt exact).

### #981 CLI render silently defaults bad --page/--scale — FIXED (`f77f536`)
`arg_value(...).parse().ok().unwrap_or(default)` in `apps/wordcraft-cli/main.rs:render`/`run`.
Fix: `page_scale()` helper — malformed/missing/duplicated/non-positive/non-finite values are
`Err` (non-zero exit); validated only where used (`render`, `run --save *.png`). Tests added.

### #968 `--help` treated as a file / rejected — FIXED (`f77f536`)
Top-level `wordcraft-cli --help` already worked (`main.rs` match arm); subcommands rejected it
via `validate_options`, and the GUI opened `--help` as a file. Fix: early `--help`/`-h` check in
`run()` prints `USAGE`; GUI prints options and exits before logging/window. Tests added.

### #1148 Dark-theme ruler shading inverted — FIXED (`67681d9`)
`canvas.rs:2037/2039` fills the margin band with `ruler_margin` and the text band with `ruler`;
dark tokens had text (`0x2E`) darker than margins (`0x45`), opposite of the light theme
(white text band, gray margins). Fix: swapped the two dark values (`ui-egui/theme.rs`), matching
the reporter's expectation. No test (theme tokens); visual check via `ui_shot`.

### #866/#1080 Deleted text inline; `text` drops list labels — FIXED (`76a5524`, `b622409`)
`document.text` (MCP `get_text`) and CLI `text` used `Document::plain_text`, which includes
tracked-deleted runs inline; CLI `text` additionally skipped the list-label logic the txt
exporter has (`formats/txt.rs:100`). Fix: new `Document::final_text` (`doc/lib.rs`, via the
pre-existing but unused `Paragraph::final_text`) for `document.text`; CLI `text` now prints the
txt export (labels + final-shown + dropped deletions, CRLF→LF for the terminal). Tests:
`document_text_shows_tracked_changes_as_final`, `txt_export_keeps_list_labels`.

### #679 `javascript:` hrefs in exports — FIXED (`b622409`, completing pre-existing work)
HTML/MD export already filtered via `doc::link::is_safe_href` (`formats/html.rs:1007,1208`,
`markdown.rs:1129`, test `exported_links_keep_only_safe_targets`). ODT (`odt.rs:224`) and LaTeX
(`latex.rs:321`) did not. Fix: same filter in both; extended the test to `tex` plus a new ODT
content.xml test.

### #187 Open replaces an edited doc silently — ALREADY FIXED
`WordApp::run` stops `file.new`/`file.open` (with path)/`file.close` on dirty docs with the
SaveChanges dialog (`lib.rs:605-610`, `discards_document`), committed in `802ba30` (#151), with
tests (`new_asks…`, `open_asks…`) that pass on this branch. Web path shares the guard.

### #168 Exports clear the Edited indicator — ALREADY FIXED
Same commit: the download path only clears `dirty` for formats that keep everything
(`lib.rs:698-702`, `keeps_everything` at `:1690`: docx/docm/dotx/dotm/odt/rtf/json/xml); engine
`save()` likewise (`engine/cmd/file.rs:204-210`), and `export_png` never touches `dirty`.

### #1212 Cannot drag-and-drop pictures — ALREADY FIXED
Drop handling is present: `lib.rs:1391` drains `dropped_files` into `insert.picture`/`file.open`
paths (also `:1388`, `:1572`, test at `:4169` for CSV drops). Caveat: winit DnD needs X11, so on
Wayland the app runs via XWayland unless `WORDCRAFT_WAYLAND=1` (README documents this).

### #1169 Typing stops on XWayland + fcitx5 — ALREADY FIXED (code-level)
The alleged cause (`request_focus` on every interaction) is not present: focus is requested only
on click/drag-started/explicit `want_focus` (`canvas.rs:1226-1230`) with a focus-lock filter.
Cannot be reproduced headlessly here; if reports persist, suspect IME preedit handling, not focus.

### #1232 Outline of a large document quits the app — MITIGATED (`4f6df7b`)
Navigation panes built unbounded widget lists per frame; now `show_rows`-virtualized (pages,
find results, headings). This bounds per-frame UI work but is *not proven* to fix the reported
`wgpu` validation error (likely oversized texture); needs a `ui_shot` repro with a large
outline before claiming more.

### #1160 Broken `storytold/craftrules` links — NEEDS DECISION
Repo is private/invisible to contributors (404 for outsiders); only the org can fix by publishing
or vendoring the standards. Not touched (owner's call).

### #764 LanguageTool API key in plaintext `ui.json` — NEEDS DECISION
Real issue; fix needs per-OS credential-store integration (Secret Service on Linux-only scope)
plus migration. Deferred as medium-risk feature work, documented here.

## Scope-obsolete (removed build targets — not fixed, closed by scope reduction)

| # | Note |
|---|---|
| 1229 | macOS-only text-box rendering |
| 1111 | macOS pinch-zoom half (narrow-ribbon half survives → see #1228/#897, still open) |
| 1103 | macOS LaunchServices default-app corruption |
| 979, 644 | macOS Intel GPU/Metal crashes |
| 954 | macOS Cmd+arrow keybinding |
| 845 | Windows Explorer thumbnails (shell extension) |
| 670 | macOS Palatino substitution table |
| 533, 73, 51, 64 | Windows MSI installer issues (incl. ProductCode conflicts) |
| 525 | macOS DMG signature/notarization |
| 170 | Windows Intel Vulkan driver crash |
| 1119 | Windows Magnifier caret tracking |
| 1122 | Windows console window during TTS |
| 984 | Android port request |
| 39 | EXE installer request |
| 738, 808 | Windows installer/portable launch failures |
| 996, 285 | macOS-only env reports (suggestion-pick, CJK boxes); need Linux repro to reopen |
| 324 | Author withdrew |

## Full triage table (all 271)

| # | Title | Status | Note |
|---|---|---|---|
| 1269 | [Bug] Colour emoji are not drawn on screen, although the PDF export has them | NEEDS FIX | open bug, not yet addressed |
| 1268 | [Bug] `file.close` over the control channel quits and discards unsaved changes without a p | FIXED | execute() refuses discarding dirty docs without force:true |
| 1266 | File ▸ Options: a pane label wider than the nav column (170 px) runs over the divider — ru | NEEDS DECISION | feature request, deferred |
| 1265 | Cut does not write to the clipboard: Paste inserts the last copied content (data loss). Co | FIXED | ribbon + context-menu Cut now write system clipboard |
| 1264 | Persistent default formatting for new documents (Normal template), and RTF losing the defa | NEEDS DECISION | feature request, deferred |
| 1263 | FR - Tabbed browsing | NEEDS FIX | open bug, not yet addressed |
| 1252 | [WordCraft] HIghlight in darkmode not visible for black text. | NEEDS FIX | open bug, not yet addressed |
| 1251 | Feature request: Built-in citation and reference management (like EndNote/Zotero) | NEEDS DECISION | feature request, deferred |
| 1246 | Web: Save does nothing in a cross-origin iframe on Chromium (showSaveFilePicker rejects as | NEEDS FIX | open bug, not yet addressed |
| 1243 | Copy: Rich text formatting is not placed on the clipboard, so pasting into other apps lose | FIXED | HTML+text now placed on system clipboard (Fragment::to_html + arboard set_html) |
| 1242 | Table Layout: Height/Width changes apply only to the last row/column when the entire table | FIXED | columnWidth/rowHeight apply to all selected rows/cols |
| 1241 | [Bug] Right-to-left text direction only seems to work for numbers. | NEEDS FIX | open bug, not yet addressed |
| 1240 | Save dialog window is 1222x1307 (taller than screen) when running via XWayland on niri; fi | NEEDS FIX | open bug, not yet addressed |
| 1238 | [Bug] Styles selector becomes empty with attached sample docx file | NEEDS FIX | open bug, not yet addressed |
| 1237 | [feature] ctrl+ ctrl- for zooming | NEEDS DECISION | feature request, deferred |
| 1236 | Bookmark Delete doesn't exist | NEEDS FIX | open bug, not yet addressed |
| 1234 | Link > Place in This Document (does not exist) | NEEDS FIX | open bug, not yet addressed |
| 1233 | installing error | NOT REPRODUCIBLE | install error, images only |
| 1232 | WordCraft quits when attempting to display a large document's outline | MITIGATED | nav panes virtualized (mitigation; crash repro still wanted) |
| 1230 | Object names and certain automatic numbering features are missing. | NEEDS DECISION | feature request, deferred |
| 1229 | Text Box Feature bug view - [Mac OS] [WordCraft] | OBSOLETE | macOS-only text-box rendering |
| 1228 | Tab row: Comments / Editing / Share buttons overlap and hide ribbon tabs when the window i | NEEDS FIX | open bug, not yet addressed |
| 1227 | Feature request: Create with templates | NEEDS DECISION | feature request, deferred |
| 1225 | Proofing: help users get a Hunspell dictionary (a link and the dictionaries folder now, an | NEEDS FIX | open bug, not yet addressed |
| 1223 | [Bug] Zoom via ctrl+middle mouse causes scroll and zoom at the same time | NEEDS FIX | open bug, not yet addressed |
| 1218 | Ctrl+Q clears paragraph formatting but also opens a save dialog | NEEDS FIX | open bug, not yet addressed |
| 1215 | Bug in scrollbar when dragged | NEEDS FIX | open bug, not yet addressed |
| 1213 | [WordCraft] Can't type anything under Xwayland in GNOME 50 + arboard not supporting waylan | NEEDS DECISION | feature request, deferred |
| 1212 | Cannot drag n drop picture | ALREADY FIXED | drop handling present in lib.rs:1391 |
| 1211 | [WordCraft] Print Option Doesn't show my printers, it converts the word document into pdf  | NEEDS DECISION | feature request, deferred |
| 1208 | [Bug] Laggy Cursor | NEEDS FIX | open bug, not yet addressed |
| 1207 | [Feature] Document Tabs & [Bug] importing doc with tabs fails | NEEDS FIX | import failure is a bug; doc-tabs feature needs decision |
| 1206 | Margins, Orientation, Size, and Columns drop down dialog missing icon and the active setti | NEEDS FIX | menu checkmarks/icons missing |
| 1201 | Table Layout →  Formula Dialog is missing | NEEDS FIX | open bug, not yet addressed |
| 1200 | Table  Layout →  Sort Dialog is missing | NEEDS FIX | open bug, not yet addressed |
| 1199 | Navigation Pane content should align to left | NEEDS FIX | open bug, not yet addressed |
| 1198 | Tables don't Delete correctly | NEEDS FIX | open bug, not yet addressed |
| 1197 | TOC: Update table of contents missing | NEEDS FIX | open bug, not yet addressed |
| 1196 | Provide pre-built Docker images on GHCR or Docker Hub | NEEDS DECISION | feature request, deferred |
| 1192 | Proposal: LawCraft, legal drafting tools for WordCraft | NEEDS DECISION | feature request, deferred |
| 1189 | Ten catalogues are ~397 keys behind the leaders (pt-br among them): equation, citation and | NEEDS DECISION | feature request, deferred |
| 1188 | Bug Ctrl+A | NEEDS FIX | open bug, not yet addressed |
| 1186 | [WordCraft] It's flickering after been updated to version 0.6.1 this morning. Thank You. | NOT REPRODUCIBLE | “flickering”, no OS/hw info |
| 1183 | [Feature Request] Need KeyTips for Table Design and Table Format Ribbon | NEEDS DECISION | feature request, deferred |
| 1182 | Random placeholder profile icon at the top right | NEEDS FIX | open bug, not yet addressed |
| 1181 | [WordCraft] Facing (mirror) margins do not load correctly from a .docx file. I also have S | NEEDS FIX | open bug, not yet addressed |
| 1178 | Modify styles page does not have a font selector | NEEDS FIX | open bug, not yet addressed |
| 1174 | DOCX: legacy form fields (FORMCHECKBOX/FORMTEXT) are not drawn and lose ffData on save; ro | NEEDS FIX | open bug, not yet addressed |
| 1173 | Picture Adjustments compound instead of re-applying from the original (transparency 80% →  | NEEDS FIX | open bug, not yet addressed |
| 1171 | Fonts embedded in a .docx are not used for layout or rendering (text falls back to Liberat | NEEDS FIX | open bug, not yet addressed |
| 1169 | [bug] Typing stops after a few characters on XWayland when fcitx5 XIM is connected | ALREADY FIXED | focus guard already present (canvas.rs:1226-30) |
| 1167 | [Feature Request] Surface a visible hint when a proofing dictionary isn't installed | NEEDS DECISION | feature request, deferred |
| 1160 | AGENTS.md / CLAUDE.md link to `storytold/craftrules`, which returns 404 for contributors | NEEDS FIX | open bug, not yet addressed |
| 1159 | Bulleted lists: bullet doesn't move with the text when paragraph alignment is changed (cen | NEEDS FIX | open bug, not yet addressed |
| 1158 | Layout: text in an earlier column doesn't wrap around a shape anchored in a later column | NEEDS FIX | open bug, not yet addressed |
| 1156 | File › Print: preview is small and can't be zoomed; it should fill the pane, with a zoom s | NEEDS FIX | open bug, not yet addressed |
| 1155 | Finnish spelling, grammar and hyphenation through Rust Voikko | NEEDS FIX | open bug, not yet addressed |
| 1148 | Dark theme: ruler margin and text areas have inverted shading (text area is darker than th | FIXED | dark ruler/ruler_margin tokens swapped back |
| 1129 | Startup: a blank document is created behind the File (Backstage) Home page; Back shows it  | NEEDS FIX | open bug, not yet addressed |
| 1128 | The interface-translation audit is red on main: 2 Mailings labels bypass translation, and  | NEEDS FIX | open bug, not yet addressed |
| 1126 | White mouse cursor (I-Beam) on a white background | NEEDS FIX | open bug, not yet addressed |
| 1124 | File (Backstage): add a "Close" item to the side menu, after "Export" | NEEDS DECISION | feature request, deferred |
| 1122 | [WordCraft] Read Aloud on the Review Tab opens a CLI window during TTS. | OBSOLETE | Windows console window during TTS |
| 1119 | [WordCraft] Windows Magnifier now follows the text insertion point, but ignores the Return | OBSOLETE | Windows Magnifier integration |
| 1116 | Layout: a paragraph ending above footnotes needs its space after clear of the separator (p | NEEDS FIX | open bug, not yet addressed |
| 1113 | Feature Request: Add Advanced Page Numbering and Formatting Options | NEEDS DECISION | feature request, deferred |
| 1111 | macOS: pinch zoom isn't anchored at the pointer; ribbon groups don't collapse in narrow wi | OBSOLETE | macOS pinch part (ribbon-collapse half stays, see 1228/897) |
| 1110 | Narrow window: ribbon tabs run under the Comments / Editing / Share buttons | NEEDS FIX | open bug, not yet addressed |
| 1109 | Perf: every edit and zoom step re-renders whole pages on the CPU | NEEDS FIX | open bug, not yet addressed |
| 1103 | MacOS: File is considered corrupt after setting WordCraft as the default application | OBSOLETE | macOS LaunchServices default-app behavior |
| 1093 | Disappearing blinking cursor when certain UI elements are clicked | NEEDS FIX | open bug, not yet addressed |
| 1091 | Wordcraft lags when the window enlarged | NEEDS FIX | open bug, not yet addressed |
| 1086 | Add clear hover hints (tooltips) to UI controls | NEEDS DECISION | feature request, deferred |
| 1085 | [WordCraft] When creating empty comment, one cannot delete it. | NEEDS FIX | open bug, not yet addressed |
| 1082 | [WordCraft] Comment function - not able to reply, resolve or delete | NEEDS FIX | open bug, not yet addressed |
| 1081 | Parity: save as Word 2003 XML Document (.xml, WordprocessingML 2003) | NEEDS DECISION | feature request, deferred |
| 1080 | wordcraft-cli text drops bullet and number labels (the #434 fix covered convert to TXT onl | FIXED | CLI text uses txt exporter (labels) |
| 1079 | Parity: encrypted .doc — open password-protected Word 97–2003 files (RC4, XOR) | NEEDS DECISION | feature request, deferred |
| 1078 | Mouse pointer disappears over table selection areas while dragging/scrolling | NEEDS FIX | open bug, not yet addressed |
| 1077 | support for forms, read-only parts, selectors, etc | NEEDS DECISION | feature request, deferred |
| 1073 | lenovo Chromebook, IME not working on Chromebook Crostini Wayland - fcitx5 works in mousep | NEEDS FIX | open bug, not yet addressed |
| 1067 | =rand() / =lorem() sample text like in Word | NEEDS FIX | open bug, not yet addressed |
| 1065 | Middle Click Document Scrolling (Autoscrolling) | NEEDS DECISION | feature request, deferred |
| 1063 | No setting for a Hunspell dictionaries folder (WORDCRAFT_DICTIONARIES doesn't reach apps s | NEEDS DECISION | feature request, deferred |
| 1062 | Saving ignores "remove personal information" / "remove date and time" (authors, dates and  | NEEDS FIX | open bug, not yet addressed |
| 1058 | [WordCraft] Feature request: Mendeley support for citations (import .bib/.ris, Mendeley Ci | NEEDS DECISION | feature request, deferred |
| 1055 | Comments: an option to show a comment only for the selected text (default stays as in Word | NEEDS DECISION | feature request, deferred |
| 1054 | Parity: ODT — footnotes, endnotes and comments (text:note, office:annotation) | NEEDS DECISION | feature request, deferred |
| 1053 | Parity: RTF — footnotes, endnotes and comments (\footnote, \annotation) | NEEDS DECISION | feature request, deferred |
| 1047 | Font Import | NEEDS FIX | open bug, not yet addressed |
| 1045 | [WordCraft] Proof language locked to English US | NEEDS FIX | open bug, not yet addressed |
| 1039 | Eraser: borders beside vertically merged cells merge or hide, as in Word | NEEDS FIX | open bug, not yet addressed |
| 1037 | Track Changes: a tracked deletion inside a field result (e.g. a =SUM(ABOVE) subtotal) is r | NEEDS FIX | open bug, not yet addressed |
| 1034 | Saving a .docx silently drops settings and parts the model doesn't represent (compat setti | NEEDS FIX | silent compat-settings drop (data-loss class) |
| 1031 | Multi-select text with Ctrl | NEEDS FIX | open bug, not yet addressed |
| 1026 | Double-click doesn't open header/footer editing until one exists, and the footer is unreac | NEEDS FIX | open bug, not yet addressed |
| 1022 | Paperpile plugin use | NEEDS DECISION | feature request, deferred |
| 1019 | Clicking on (and holding) the ToolBar makes the document scroll while you are holding | NEEDS FIX | open bug, not yet addressed |
| 1018 | Hebrew not fully supported with rtl | NEEDS DECISION | feature request, deferred |
| 1015 | Accessibility: Save Changes dialog needs screen-reader announcements and reliable focus ma | NEEDS FIX | open bug, not yet addressed |
| 1014 | Ctrl + click and drag should copy the object | NEEDS FIX | open bug, not yet addressed |
| 1013 | 2 clicks on a comment box makes it unselected instead of moving the cursor inside of the e | NEEDS FIX | open bug, not yet addressed |
| 1012 | Accessibility: expose ribbon tabs and Alt/F10 KeyTips to screen readers | NEEDS FIX | open bug, not yet addressed |
| 1011 | Fix duplicate title bars on Linux | NEEDS FIX | open bug, not yet addressed |
| 1010 | [insert] Dragging a shape retains the text, equations, tables, and shapes behind it in dra | NEEDS FIX | open bug, not yet addressed |
| 1008 | Markdown table round-trip moves or drops content in code-span cells | NEEDS FIX | open bug, not yet addressed |
| 1007 | Down arrow from the start of the line above a table skips the table | NEEDS FIX | open bug, not yet addressed |
| 1002 | [ui] Equation elements have mismatched and errant background colors | NEEDS FIX | open bug, not yet addressed |
| 1001 | Require a test, or a stated reason for none, in every pull request | NEEDS DECISION | feature request, deferred |
| 999 | Page background picture/texture fill is not drawn, and is removed on save | NEEDS FIX | open bug, not yet addressed |
| 998 | Radar and stock charts render as blank space, with no placeholder or warning | NEEDS FIX | open bug, not yet addressed |
| 997 | Saving replaces Office 2016 charts (cx:chart) with their fallback pictures and drops the c | NEEDS FIX | open bug, not yet addressed |
| 996 | Spellchecker doesn't work on woodcraft Mac OS Sillicon | OBSOLETE | macOS-only env; suggestion-pick needs Linux repro |
| 995 | Single-page view setting is not preserved when reopening a document | NEEDS DECISION | feature request, deferred |
| 993 | i18n: Add Italian language support (UI, spell checker, thesaurus) | NEEDS DECISION | feature request, deferred |
| 992 | Superscript is bugged | NEEDS FIX | open bug, not yet addressed |
| 991 | Focus button doesn't work. | NOT REPRODUCIBLE | one line, no repro |
| 989 | [WordCraft] Quick panel overlap text size/font selector | NEEDS FIX | open bug, not yet addressed |
| 988 | Increase the size of toolbar icons in the ribbon | NEEDS FIX | open bug, not yet addressed |
| 987 | Flaky test: native print PDFs differ by their creation time | NEEDS FIX | open bug, not yet addressed |
| 984 | [WordCraft] Request for android version of the wordcraft and gridcraft | OBSOLETE | Android port request (no mobile target) |
| 981 | CLI: render silently accepts invalid or duplicate page and scale options | FIXED | strict --page/--scale parsing + tests |
| 980 | [WordCraft] F11 (full screen mode) doesn't exist | NEEDS FIX | open bug, not yet addressed |
| 979 | WordCraft 0.5.0 crashes on macOS 12.7.6 (Intel Mac) | OBSOLETE | macOS 12 Intel GPU crash |
| 975 | Ruler: tab stops can't be set, moved or cleared with the pointer; Tabs dialog loses un-Set | NEEDS FIX | open bug, not yet addressed |
| 974 | No CI on most pull requests: `cargo xtask ci` isn't enforced, and releases ship without ru | NEEDS DECISION | feature request, deferred |
| 971 | Tables: AutoFit in a nested table, whole-cell shading for selections through a table, Shif | NEEDS FIX | open bug, not yet addressed |
| 969 | Question: should AutoCorrect be off by default in headless runs? | NEEDS FIX | open bug, not yet addressed |
| 968 | `--help` on the GUI binary and on CLI subcommands is treated as a file or rejected | FIXED | subcommand --help prints usage; GUI --help added |
| 967 | para.keepNext, keepLines, pageBreakBefore, widowControl: accept {"value": bool} like forma | NEEDS FIX | open bug, not yet addressed |
| 966 | Layout: keep with next leaves a heading alone at the bottom of a page when the next paragr | NEEDS FIX | open bug, not yet addressed |
| 965 | insert.picture: the next typing command deletes the picture | NEEDS FIX | open bug, not yet addressed |
| 963 | [WordCraft] Document content does not load correctly | NOT REPRODUCIBLE | no repro info |
| 962 | Tables: dragging a selected cell's border moves it in that cell's row only | NEEDS FIX | open bug, not yet addressed |
| 961 | Typing: Alt+numpad decimal character entry (ANSI/OEM Alt codes) | NEEDS FIX | open bug, not yet addressed |
| 960 | [WordCraft] Double click on top of page to create header | NEEDS FIX | open bug, not yet addressed |
| 959 | Bring back 'Clippy' and other assistants pretty please😭 | NEEDS DECISION | feature request, deferred |
| 957 | Ticket to close tickets: already done, duplicates, or off-topic | NEEDS FIX | open bug, not yet addressed |
| 955 | WordCraft language | NEEDS FIX | open bug, not yet addressed |
| 954 | macos: CMD + arrow left/right behaves incorrectly | OBSOLETE | macOS Cmd+arrow keybinding |
| 951 | [Feature Request] Add Italian (it-IT) proofing dictionary | NEEDS DECISION | feature request, deferred |
| 949 | [WordCraft] Dictate and transcribe | NEEDS DECISION | feature request, deferred |
| 938 | BUG | NOT REPRODUCIBLE | no repro info |
| 918 | Missing feature: Autosave control and reopening last open documents (and page positions) a | NEEDS DECISION | feature request, deferred |
| 917 | [bug] Page shakes while typing | NOT REPRODUCIBLE | no repro info |
| 916 | [WordCraft] cross-references don't open | NEEDS FIX | open bug, not yet addressed |
| 911 | [WordCraft] the menu next to undo/redo is not working | NOT REPRODUCIBLE | no repro info |
| 905 | Bug Report: Automatic Scrolling Does Not Work When Selecting Text by Dragging the Mouse | NOT REPRODUCIBLE | no repro info |
| 903 | [Feature Request] Add Italian (it-IT) UI language support | NEEDS DECISION | feature request, deferred |
| 900 | [WordCraft] ... doesnt work and theme | NOT REPRODUCIBLE | no repro info |
| 898 | Zotero citation fields buggy (not closed) when introduced at the end of a document and typ | NEEDS DECISION | feature request, deferred |
| 897 | Ribbon tab strip: tabs that don't fit hide under Comments / Editing / Share, so Table Desi | NEEDS FIX | open bug, not yet addressed |
| 895 | [REQUEST] - Support for FRA Language | NEEDS DECISION | feature request, deferred |
| 886 | Saving a .docx silently drops DrawingML shapes (data loss, no warning) | NEEDS FIX | open bug, not yet addressed |
| 884 | Shapes with square wrap type not wrapping properly | NEEDS FIX | open bug, not yet addressed |
| 883 | [Feature request] Add add-ins | NEEDS DECISION | feature request, deferred |
| 880 | [WordCraft] .doc files import bug, images does not load. | NEEDS FIX | open bug, not yet addressed |
| 878 | fix(references): updating an imported TOC field writes the entries outside the field and k | NEEDS FIX | open bug, not yet addressed |
| 877 | fix(docx): saving drops w:dirty from field begin markers, so Word no longer refreshes an i | NEEDS FIX | open bug, not yet addressed |
| 867 | [Feature Request] Portable Mode: write preferences and settings in the application folder  | NEEDS DECISION | feature request, deferred |
| 866 | get_text / wordcraft-cli text include deleted tracked text inline | FIXED | document.text/CLI text read as final |
| 865 | MCP/commands: styles.modify rejects hex colours that format.color accepts; CharProps/ParaP | NEEDS FIX | open bug, not yet addressed |
| 863 | .odt Heading of a document is missing. | NEEDS FIX | open bug, not yet addressed |
| 860 | Language-related commands (proofing language / text language) are disabled; document defau | NEEDS FIX | open bug, not yet addressed |
| 851 | WordPerfect-Style Reveal Codes for Precise Document Formatting and Troubleshooting | NEEDS DECISION | feature request, deferred |
| 845 | Windows Explorer thumbnails for files (and folder previews) | OBSOLETE | Windows Explorer shell extension |
| 834 | Package for arch linux AUR | NEEDS FIX | open bug, not yet addressed |
| 824 | Supporting Farsi fonts | NEEDS DECISION | feature request, deferred |
| 822 | 100% scaling is incorrect | NEEDS FIX | open bug, not yet addressed |
| 820 | [ui] Add floating Paste Options widget (Ctrl) after paste | NEEDS DECISION | feature request, deferred |
| 816 | [clipboard] Copying equations exports plain linear text instead of OMML/MathML to Word | NEEDS FIX | open bug, not yet addressed |
| 808 | [WordCraft] Cannot open the program | OBSOLETE | Windows craft-apps launch |
| 799 | Harper grammar checker integration | NEEDS DECISION | feature request, deferred |
| 794 | 中文乱码问题，期望修复 | NOT REPRODUCIBLE | no repro info |
| 793 | [REQUEST]: add some man-made C, and stop using AI! | NEEDS DECISION | feature request, deferred |
| 789 | Top Ribbon and Spell Check Issues | NOT REPRODUCIBLE | no repro info |
| 788 | Selecting multiple non-contiguous blocks of text | NEEDS FIX | open bug, not yet addressed |
| 786 | pt_BR spell check: pt module reading the installed VERO dictionary | NEEDS FIX | open bug, not yet addressed |
| 785 | [Feature Request] enable voice dictation | NEEDS DECISION | feature request, deferred |
| 784 | Reduce flatpak permissions | NEEDS FIX | open bug, not yet addressed |
| 781 | [Feature Request] MathType-like WYSIWYG Math Formula Editor | NEEDS DECISION | feature request, deferred |
| 780 | Images also display incorrectly | NOT REPRODUCIBLE | image only |
| 778 | Existing list does not support XLSX/XLS, recipient list shows garbled text, and filter fun | NEEDS DECISION | feature request, deferred |
| 777 | [WordCraft] Incorrect paragraph breaks when pressing Enter | NEEDS FIX | open bug, not yet addressed |
| 775 | Styleref in mergefield | NEEDS FIX | open bug, not yet addressed |
| 767 | UI rendering issue after opening Word docx files | NEEDS FIX | open bug, not yet addressed |
| 764 | LanguageTool: keep the API key in the OS credential store, not ui.json | NEEDS FIX | open bug, not yet addressed |
| 763 | LanguageTool: check mixed-language paragraphs in each run's language | NEEDS FIX | open bug, not yet addressed |
| 762 | LanguageTool: check headers, footers, notes and text boxes too | NEEDS FIX | open bug, not yet addressed |
| 754 | CI: run cargo xtask ci on pull requests and main | NEEDS DECISION | feature request, deferred |
| 753 | Spellcheck dictionary needs to be expanded | NEEDS FIX | open bug, not yet addressed |
| 749 | Page size looks wrong when set to small margins, doesn't render properly | NEEDS FIX | open bug, not yet addressed |
| 738 | WordCraft does not open | OBSOLETE | Windows installable/portable launch |
| 735 | Per-language rules for typing, dates and proofing, keyed by the text's language (w:lang) | NEEDS FIX | open bug, not yet addressed |
| 714 | [Feature request] Options in table style editor | NEEDS DECISION | feature request, deferred |
| 685 | Insert a chart backed by an embedded .xlsx workbook (like Word's Insert > Chart) | NEEDS FIX | open bug, not yet addressed |
| 680 | Voice Typing not working. | NEEDS FIX | open bug, not yet addressed |
| 679 | HTML and Markdown export write javascript: and other non-web hyperlink targets into href | FIXED | HTML/MD filtered before; ODT/LaTeX filtered now |
| 670 | Palatino Linotype / Book Antiqua fall back to Times New Roman on macOS instead of Palatino | OBSOLETE | macOS font-substitution table |
| 662 | Home tab overflows at common window widths | NEEDS FIX | open bug, not yet addressed |
| 660 | Icons painted at off-grid sizes outside the ribbon | NEEDS FIX | open bug, not yet addressed |
| 650 | Browser / cloud version (e.g., integrated with nextcloud) | NEEDS DECISION | feature request, deferred |
| 646 | Cannot open two documents at once | NEEDS FIX | open bug, not yet addressed |
| 644 | macOS launch failure on Intel MacBook Pro — Metal shader compilation and surface creation  | OBSOLETE | macOS Metal launch failure |
| 641 | File > Print: add Word's print settings (Current Page, custom page range, copies, printer, | NEEDS DECISION | feature request, deferred |
| 638 | Design tab: style set gallery overlaps the Colors and Fonts buttons in Document Formatting | NEEDS FIX | open bug, not yet addressed |
| 634 | support for mobile screens | NEEDS DECISION | feature request, deferred |
| 619 | A compact single-line ribbon | NEEDS FIX | open bug, not yet addressed |
| 618 | Integrate with Himmelblau, for full o365 like experience? | NEEDS DECISION | feature request, deferred |
| 613 | Printing and export is not working under Linux | NEEDS FIX | open bug, not yet addressed |
| 602 | ~50ms stroke/ink rendering delay on canvas (shared render pipeline) | NEEDS FIX | open bug, not yet addressed |
| 599 | Unable to select partial/full text/terms inside equation blocks using mouse or Shift + Arr | NEEDS FIX | open bug, not yet addressed |
| 590 | The link to the wordcraft website is broken | NEEDS FIX | open bug, not yet addressed |
| 585 | wordcraft ui is stiff. there is no fluid animation | NEEDS DECISION | feature request, deferred |
| 582 | Let an end user add a font without rebuilding | NEEDS DECISION | feature request, deferred |
| 575 | Languagetool integration for spell and grammar checking | NEEDS DECISION | feature request, deferred |
| 554 | Performance: large documents (500+ pages, many pictures and tables) stay responsive | NEEDS FIX | open bug, not yet addressed |
| 537 | Header color is still black even in light theme in linux | NEEDS FIX | open bug, not yet addressed |
| 534 | Font, typesetting, and paragraph issues | NEEDS DECISION | feature request, deferred |
| 533 | Cannot write anything with release 0.5.0 Windows x64 installer | OBSOLETE | Windows MSI installed build only |
| 529 | Add RTL for the tables | NEEDS DECISION | feature request, deferred |
| 525 | fix(macos): shipped DMG app fails strict signature verification with FinderInfo metadata | OBSOLETE | macOS DMG signature |
| 523 | Window decorations and title bar inconsistent with VectorCraft / PhotoCraft | NEEDS FIX | open bug, not yet addressed |
| 518 | Feature request: AutoSave & version history to a storage target you own (local/NAS, Google | NEEDS DECISION | feature request, deferred |
| 498 | Request: Add support for Indonesian in Proofing Language | NEEDS DECISION | feature request, deferred |
| 403 | Feature Request: Extensible Plugin System for WordCraft | NEEDS DECISION | feature request, deferred |
| 375 | Copy/past feature doesn't work with Images | NEEDS DECISION | feature request, deferred |
| 374 | feat(distribution): provide reliable CLI access with desktop packages | NEEDS DECISION | feature request, deferred |
| 373 | feat(agents): guide desktop users through CLI and MCP setup | NEEDS FIX | open bug, not yet addressed |
| 358 | Word documents containing images become misformatted. | NEEDS FIX | open bug, not yet addressed |
| 336 | UI localization: translation mechanism (and Polish as a first community language?) | NEEDS FIX | open bug, not yet addressed |
| 324 | won't start | OBSOLETE | author withdrew (wrong version) |
| 322 | Parity: View › Window — New Window, Arrange All, View Side by Side, Synchronous Scrolling, | NEEDS DECISION | feature request, deferred |
| 299 | Cursor corlor not changin, is not visible. PDF EDITOR and WORDCRAFT // no languaje changes | NEEDS FIX | open bug, not yet addressed |
| 285 | [Bug] Software interface garbled characters | OBSOLETE | macOS CJK font env; needs Linux repro |
| 279 | WordCraft cannot open files in the "Word Document" format. | NEEDS FIX | open bug, not yet addressed |
| 211 | Arabic Letter (Unicode not rendering) | NEEDS FIX | open bug, not yet addressed |
| 201 | Linux (deb): no update reminder and no one-click update | NEEDS FIX | open bug, not yet addressed |
| 199 | arabic support | NEEDS DECISION | feature request, deferred |
| 197 | How about Wincraft? | NEEDS FIX | open bug, not yet addressed |
| 187 | [Bug] Open silently replaces an edited document and clears its Undo history | ALREADY FIXED | guarded since 802ba30 (SaveChanges prompt) |
| 184 | AutoSave: off per document by default, with an "AutoSave all documents" option | NEEDS DECISION | feature request, deferred |
| 170 | Crash on startup on Windows with Intel UHD Graphics (Access Violation 0xc0000005 in igvk64 | OBSOLETE | Windows Intel Vulkan driver crash |
| 168 | [Bug] PDF, PNG and TXT exports clear the editable document’s Edited indicator | ALREADY FIXED | guarded since 802ba30 (keeps_everything) |
| 167 | still needs margin and margin comments implementation same as word | NEEDS DECISION | feature request, deferred |
| 142 | Broken shape tool | NEEDS FIX | open bug, not yet addressed |
| 118 | Change font | NEEDS FIX | open bug, not yet addressed |
| 106 | "a newer version of wordcraft is installed" error | NEEDS FIX | open bug, not yet addressed |
| 105 | [Feature Request] Support for "CraftCloud", decentralized encrypted cloud extension | NEEDS DECISION | feature request, deferred |
| 100 | [Feature Request] Support Hunspell / system dictionaries and configurable proofing languag | NEEDS DECISION | feature request, deferred |
| 97 | PDF export: REF cross-reference field prints U+FFFC instead of its result | NEEDS FIX | open bug, not yet addressed |
| 95 | Cant exit bullet list using enter or backspace | NEEDS FIX | open bug, not yet addressed |
| 80 | macros not working | NEEDS FIX | open bug, not yet addressed |
| 78 | Linux (GNOME Wayland): window title bar doesn't match the desktop theme | NEEDS FIX | open bug, not yet addressed |
| 73 | Windows intaller won't work if you have other Art apps already installed | OBSOLETE | Windows MSI ProductCode conflict |
| 70 | Missing feature: Show all | NEEDS DECISION | feature request, deferred |
| 68 | Scaling issue on fedora 1080p. | NOT REPRODUCIBLE | no repro info |
| 66 | Full Arabic and RTL Support | NOT REPRODUCIBLE | no repro info |
| 64 | App won't install. Says that a newer version is already installed, but I never installed i | OBSOLETE | Windows MSI install error |
| 63 | Arabic text not displaying correctly | NEEDS FIX | open bug, not yet addressed |
| 56 | Language pack issue | NOT REPRODUCIBLE | no repro info |
| 51 | Big problem | OBSOLETE | Windows MSI ProductCode conflict |
| 48 | rtl texts format is cursed | NOT REPRODUCIBLE | no repro info |
| 47 | just dont open itself and files | NOT REPRODUCIBLE | empty body |
| 42 | Section breaks not displaying | NOT REPRODUCIBLE | no repro info |
| 40 | English (UK) spelling dictionary | NEEDS FIX | open bug, not yet addressed |
| 39 | EXE Installer | OBSOLETE | EXE installer (no Windows target) |
| 25 | Choice of language for correction not implemented yet | NOT REPRODUCIBLE | no repro info |
| 19 | Bug Report: Incorrect RTL (Right-to-Left) Text Rendering and Alignment | NOT REPRODUCIBLE | no repro info |
| 18 | The styles not working on the portable | NOT REPRODUCIBLE | no repro info |
| 15 | Print dialog and text render | NOT REPRODUCIBLE | no repro info |

## Owner decisions (2026-10-13)

Owner decided every NEEDS DECISION item plus #1155:

### Implement now (40)
- #70
- #100
- #167
- #184
- #199
- #374
- #375
- #529
- #582
- #641
- #714
- #754
- #778
- #820
- #824
- #867
- #898
- #918
- #974
- #995
- #1001
- #1018
- #1053
- #1054
- #1063
- #1065
- #1086
- #1113
- #1124
- #1155
- #1167
- #1183
- #1189
- #1211
- #1213
- #1225
- #1227
- #1237
- #1264
- #1266

### Deferred (26)
- #322
- #403
- #498
- #518
- #534
- #575
- #585
- #650
- #781
- #785
- #799
- #851
- #883
- #895
- #903
- #949
- #951
- #993
- #1055
- #1058
- #1077
- #1079
- #1081
- #1196
- #1230
- #1251

### Rejected (10)
- #105 (stays closed: out of scope / not actionable)
- #618 (stays closed: out of scope / not actionable)
- #634 (stays closed: out of scope / not actionable)
- #793 (stays closed: out of scope / not actionable)
- #959 (stays closed: out of scope / not actionable)
- #1022 (stays closed: out of scope / not actionable)
- #1192 (stays closed: out of scope / not actionable)

Note: the triage table above still shows the pre-decision `NEEDS DECISION` label; this section overrides it.
