# WordCraft user guide

[Deutsch](usage-de.md) · [All languages / translate](README.md) · [Project README](../../README.md)

English is the source text; German is the initial example translation. Further languages can
follow the same structure (see the language index).

This guide collects everyday workflows and keyboard shortcuts. English command names are used
so you can find the same actions in the ribbon or command search; the interface may translate them.
Checked against source revision [2cf1d6c](https://github.com/storytold/wordcraft/tree/2cf1d6c7ae94723a45bd6e7e8c884fe375148750)
on 2026-10-27. A downloaded release may differ.

## Contents

- [Start, save and export](#start-save-and-export)
- [Write, select, copy and paste](#write-select-copy-and-paste)
- [Format and structure a document](#format-and-structure-a-document)
- [Review, references and mail merge](#review-references-and-mail-merge)
- [Settings and keyboard customization](#settings-and-keyboard-customization)
- [Keyboard reference](#keyboard-reference)
- [Context-sensitive controls](#context-sensitive-controls)
- [Further help and coverage](#further-help-and-coverage)

## Start, save and export

1. Choose **File › New** for a blank document or a template, or **File › Open** for an existing file.
2. Write in the document canvas. Use **File › Save As** to choose a name, location and format;
   use **Save** for subsequent changes. To keep an input file, save under a new name.
3. Choose **File › Export › Export PDF** for a PDF copy, or **File › Print** to review print settings.
   Check **Print Markup** when deciding whether comments and tracked changes appear in output.

WordCraft reads and writes DOCX, ODT, RTF, HTML, Markdown and text, among other formats;
legacy DOC is an import format. Format conversion can change content or layout: consult the
[file-format reference](../file-format-parity.md) before relying on a round trip, and inspect the
saved/exported file. Installation and build instructions remain in the [README](../../README.md).
For interrupted work, use the existing [recovery guide](../recovery.md).

## Write, select, copy and paste

Click to place the caret, drag to select text, or hold **Shift** while moving the caret to extend
a selection. **Enter** starts a paragraph; **Shift+Enter** inserts a line break. In a list, Enter
continues the list and Tab changes its level; in a table, Tab moves between cells. See
[typing behavior](../typing-parity.md) for the detailed rules and exceptions.

Select text before **Home › Clipboard › Copy** or **Cut**, place the caret at the destination,
then choose **Paste**. Normal paste uses available clipboard formatting; the result depends on
the formats supplied by the source application. **Paste: Keep Text Only** inserts plain text;
**Paste Special** offers the available clipboard representations.

The defaults distinguish **text content** from **formatting**:

| Action | Windows / Linux / BSD | macOS |
|---|---|---|
| Copy content | Ctrl+C / Ctrl+Insert | Cmd+C / Cmd+Insert |
| Paste content | Ctrl+V / Shift+Insert | Cmd+V / Shift+Insert |
| Paste: Keep Text Only | Ctrl+Shift+Alt+V | Cmd+Shift+Alt+V |
| Copy Formatting | Ctrl+Shift+C | Cmd+Shift+C |
| Paste Formatting | Ctrl+Shift+V | Cmd+Shift+V |
| Paste Special | Ctrl+Alt+V | Cmd+Alt+V |

**Copy Formatting / Paste Formatting** transfer the source selection's formatting to the target;
they do not copy or paste plain text. There is no default plain-text-copy binding, nor a default
Ctrl+Alt+Insert or Shift+Alt+Insert binding in the checked registry.

Use **Home › Editing › Find** to search, **Find and Replace** to change matches, and **Go To**
to navigate. **Undo** reverses an edit; **Redo** restores it. **Repeat** repeats the last repeatable
action and is distinct from Redo.

## Format and structure a document

Select the text or place the caret in the relevant paragraph before using these controls.

| Task | Where and how | Result / context |
|---|---|---|
| Change character appearance | **Home › Font**: choose the face, size, emphasis, color or highlight | Applies character formatting to the selection or subsequent typing |
| Set paragraph layout | **Home › Paragraph**: choose alignment, spacing, indents or a list | Changes the selected paragraphs |
| Use consistent headings | **Home › Styles**: apply Normal or a Heading style | Headings provide document structure; use the Styles pane to manage styles |
| Set up pages | **Layout**: set margins, orientation, paper size, columns and breaks | Controls page/section layout; [layout coverage](../layout-parity.md) records limitations |
| Add a table | **Insert › Table**, choose dimensions, then edit cells | Table controls act on the current table/cell selection; Tab navigates cells |
| Add pictures, shapes or charts | Choose the matching **Insert** command, then select the inserted object | Use its contextual controls for size, position and wrapping |
| Add a link or bookmark | **Insert › Links**: select text, choose Link or Bookmark and supply its destination/name | Creates a navigable reference |
| Add headers, footers or fields | Use **Insert** and the corresponding header/footer or **Text › Quick Parts** controls | Fields can be updated; headers/footers belong to sections |
| Change the view | **View**: Print Layout, Draft, Outline, Read Mode or split view | Changes how you work with the document, not its saved page size |

## Review, references and mail merge

- **Review:** enable **Track Changes** before edits you want recorded. Select text and choose
  **New Comment** to discuss it. Use the review controls to accept/reject changes and navigate
  comments. Hiding markup does not accept revisions. Run **Spelling & Grammar** for proofing;
  [spelling dictionaries](../spelling-dictionaries.md) explains dictionary installation and selection.
- **References:** apply heading styles before inserting a table of contents. Use **Insert Footnote**
  or **Insert Endnote** at the reference position. Refresh fields after changing the document.
  Citations, bibliography, captions and index entries live on **References**;
  [Zotero integration](../zotero.md) has its own setup and usage instructions.
- **Mailings:** choose recipients, insert merge fields into the document, inspect **Preview Results**,
  then use **Finish & Merge** or **Print Documents**. Verify records in the preview before producing
  the merged output. These commands require a configured recipient source.

## Settings and keyboard customization

Use **File › Options** for application settings. To inspect or change shortcuts, open
**Tools › Customize Keyboard**, or **File › Options › Keyboard shortcuts**:

1. Choose a category and command (or use the dialog's filter).
2. Inspect its current keys. Focus **New shortcut** and press the desired combination.
3. Check the displayed assignment and choose **Assign**; select an existing binding and **Remove**
   to remove it. **Reset All** asks for confirmation before restoring defaults.

Assignments are saved in preferences. The dialog accepts modified keys or function keys, rather
than unmodified typing letters. User assignments/removals can change the tables below.
On Windows/Linux/BSD **Ctrl+F1** collapses the ribbon unless assigned to another command;
on macOS use **Cmd+Alt+R**. **Alt+Q** opens command search on non-macOS builds.

## Keyboard reference

These are the **163 command entries with default shortcuts** in the checked engine registry,
grouped by area, followed by UI/context controls. Commands with no default binding remain available
through the UI or command search where supported. This is a source-checked inventory, not a claim
that every combination was exercised on every platform.

- **Ctrl** is Control; **Cmd** is Command (⌘); **Alt** is Option (⌥) on macOS.
  **Shift** is ⇧. A slash separates alternative combinations; **—** means no default for that platform.
- The platform columns describe native builds. The web build uses the non-macOS registry defaults;
  its primary modifier follows the browser platform. Browsers and the operating system may consume
  shortcuts before WordCraft sees them. Function keys may require **Fn** on your keyboard.
- Click the document canvas for editing/navigation keys. A focused text box or dialog owns its
  input. Outline, field, table, review and mail-merge commands require the corresponding context.
  A disabled command does not become available merely because its shortcut is pressed.
- The character names reflect the registered keys; punctuation placement varies with keyboard layout.

### Files

Source: [file](../../crates/engine/src/cmd/file.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| New | `Ctrl+N` | `Cmd+N` |
| Open | `Ctrl+O / Ctrl+F12 / Ctrl+Alt+F2` | `Cmd+O / Cmd+F12 / Cmd+Alt+F2` |
| Save | `Ctrl+S / Shift+F12 / Alt+Shift+F2` | `Cmd+S / Shift+F12 / Alt+Shift+F2` |
| Save As | `F12 / Alt+F2` | `F12 / Cmd+Shift+S` |
| Print | `Ctrl+P / Ctrl+Shift+F12 / Ctrl+F2 / Ctrl+Alt+I` | `Cmd+P / Cmd+Shift+F12 / Cmd+F2 / Cmd+Alt+I` |
| Close | `Ctrl+W / Ctrl+F4` | `Cmd+W / Cmd+F4` |

### Typing and special characters

Source: [text](../../crates/engine/src/cmd/text.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| New Paragraph | `Enter` | `Enter` |
| Line Break | `Shift+Enter` | `Shift+Enter` |
| Page Break | `Ctrl+Enter` | `Cmd+Enter` |
| Column Break | `Ctrl+Shift+Enter` | `Cmd+Shift+Enter` |
| Tab | `Tab` | `Tab` |
| Shift+Tab | `Shift+Tab` | `Shift+Tab` |
| Tab Character | `Ctrl+Tab` | `Alt+Tab` |
| Backspace | `Backspace` | `Backspace` |
| Delete | `Delete` | `Delete` |
| Delete Previous Word | `Ctrl+Backspace / Alt+Backspace` | `Cmd+Backspace / Alt+Backspace` |
| Delete Next Word | `Ctrl+Delete / Alt+Delete` | `Cmd+Delete / Alt+Delete` |
| Nonbreaking Space | `Ctrl+Shift+Space` | `Cmd+Shift+Space` |
| Nonbreaking Hyphen | `Ctrl+Shift+-` | `Cmd+Shift+-` |
| Optional Hyphen | `Ctrl+-` | `Cmd+-` |
| Toggle Unicode Character | `Alt+X` | — |
| Em Dash | `Ctrl+Alt+-` | `Cmd+Alt+-` |
| Copyright Sign | `Ctrl+Alt+C` | — |
| Registered Sign | `Ctrl+Alt+R` | — |
| Trademark Sign | `Ctrl+Alt+T` | — |
| Ellipsis | `Ctrl+Alt+.` | — |

### Navigation and selection

Source: [caret](../../crates/engine/src/cmd/caret.rs), [column](../../crates/engine/src/cmd/column.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Column Selection | `Ctrl+Shift+F8` | `Cmd+Shift+F8` |
| Left | `Left` | `Left` |
| Right | `Right` | `Right` |
| Up | `Up` | `Up` |
| Down | `Down` | `Down` |
| Word Left | `Ctrl+Left / Alt+Left` | `Alt+Left` |
| Word Right | `Ctrl+Right / Alt+Right` | `Alt+Right` |
| Line Start | `Home` | `Home / Cmd+Left` |
| Line End | `End` | `End / Cmd+Right` |
| Paragraph Up | `Ctrl+Up / Alt+Up` | `Cmd+Up / Alt+Up` |
| Paragraph Down | `Ctrl+Down / Alt+Down` | `Cmd+Down / Alt+Down` |
| Document Start | `Ctrl+Home` | `Cmd+Home` |
| Document End | `Ctrl+End` | `Cmd+End` |
| Page Up | `PageUp` | `PageUp` |
| Page Down | `PageDown` | `PageDown` |
| Select All | `Ctrl+A` | `Cmd+A` |
| Collapse Selection | `Escape` | `Escape` |

### Editing and clipboard

Source: [edit](../../crates/engine/src/cmd/edit.rs), [paste](../../crates/engine/src/cmd/paste.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Undo | `Ctrl+Z` | `Cmd+Z` |
| Redo | `Ctrl+Y / Ctrl+Shift+Z` | `Cmd+Y / Cmd+Shift+Z` |
| Cut | `Ctrl+X` | `Cmd+X` |
| Copy | `Ctrl+C / Ctrl+Insert` | `Cmd+C / Cmd+Insert` |
| Spike | `Ctrl+F3` | `Cmd+F3` |
| Insert Spike | `Ctrl+Shift+F3` | `Cmd+Shift+F3` |
| Paste | `Ctrl+V / Shift+Insert` | `Cmd+V / Shift+Insert` |
| Paste: Keep Text Only | `Ctrl+Shift+Alt+V` | `Cmd+Shift+Alt+V` |
| Find | `Ctrl+F` | `Cmd+F` |
| Find Next | `Ctrl+G / F3 / Shift+F4 / Ctrl+PageDown` | `Cmd+G / F3 / Shift+F4 / Cmd+PageDown` |
| Find Previous | `Ctrl+PageUp` | `Cmd+Shift+G / Cmd+PageUp` |
| Find and Replace | `Ctrl+H` | `Cmd+H / Ctrl+H` |
| Go To | `Ctrl+Alt+G / F5` | `Cmd+Alt+G / F5` |
| Go Back | `Shift+F5 / Ctrl+Alt+Z` | `Shift+F5 / Cmd+Alt+Z` |
| Copy Formatting | `Ctrl+Shift+C` | `Cmd+Shift+C` |
| Paste Formatting | `Ctrl+Shift+V` | `Cmd+Shift+V` |
| Paste Special | `Ctrl+Alt+V` | `Cmd+Alt+V` |

### Character formatting

Source: [format](../../crates/engine/src/cmd/format.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Bold | `Ctrl+B` | `Cmd+B` |
| Italic | `Ctrl+I` | `Cmd+I` |
| Underline | `Ctrl+U` | `Cmd+U` |
| Double Underline | `Ctrl+Shift+D` | `Cmd+Shift+D` |
| Underline Words Only | `Ctrl+Shift+W` | `Cmd+Shift+W` |
| Strikethrough | `Ctrl+Shift+X` | `Cmd+Shift+X` |
| Subscript | `Ctrl+=` | `Cmd+=` |
| Superscript | `Ctrl+Shift+=` | `Cmd+Shift+=` |
| All Caps | `Ctrl+Shift+A` | `Cmd+Shift+A` |
| Small Caps | `Ctrl+Shift+K` | `Cmd+Shift+K` |
| Hidden | `Ctrl+Shift+H` | `Cmd+Shift+H` |
| Font | `Ctrl+Shift+F` | `Cmd+Shift+F` |
| Font Size | `Ctrl+Shift+P` | `Cmd+Shift+P` |
| Increase Font Size | `Ctrl+Shift+. / Ctrl+Shift+>` | `Cmd+Shift+. / Cmd+Shift+>` |
| Decrease Font Size | `Ctrl+Shift+, / Ctrl+Shift+<` | `Cmd+Shift+, / Cmd+Shift+<` |
| Grow Font 1 Point | `Ctrl+]` | `Cmd+]` |
| Shrink Font 1 Point | `Ctrl+[` | `Cmd+[` |
| Text Highlight Color | `Ctrl+Alt+H` | — |
| Change Case | `Shift+F3` | `Shift+F3` |
| Reset Character Formatting | `Ctrl+Space` | `Ctrl+Space` |
| Font Dialog | `Ctrl+D` | `Cmd+D` |

### Paragraphs and styles

Source: [para](../../crates/engine/src/cmd/para.rs), [inspector](../../crates/engine/src/cmd/inspector.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Style Inspector | `Shift+F1` | `Shift+F1` |
| Align Left | `Ctrl+L` | `Cmd+L` |
| Center | `Ctrl+E` | `Cmd+E` |
| Align Right | `Ctrl+R` | `Cmd+R` |
| Justify | `Ctrl+J` | `Cmd+J` |
| Distributed | `Ctrl+Shift+J` | `Cmd+Shift+J` |
| Increase Indent | `Ctrl+M` | `Cmd+M` |
| Decrease Indent | `Ctrl+Shift+M` | `Cmd+Shift+M` |
| Hanging Indent | `Ctrl+T` | `Cmd+T` |
| Reduce Hanging Indent | `Ctrl+Shift+T` | `Cmd+Shift+T` |
| Single Spacing | `Ctrl+1` | `Cmd+1` |
| Double Spacing | `Ctrl+2` | `Cmd+2` |
| 1.5 Line Spacing | `Ctrl+5` | `Cmd+5` |
| Add Space Before Paragraph | `Ctrl+0` | `Cmd+0` |
| Reset Paragraph Formatting | `Ctrl+Q` | `Ctrl+Q` |
| Normal Style | `Ctrl+Shift+N` | `Cmd+Shift+N` |
| Heading 1 | `Ctrl+Alt+1` | `Cmd+Alt+1` |
| Heading 2 | `Ctrl+Alt+2` | `Cmd+Alt+2` |
| Heading 3 | `Ctrl+Alt+3` | `Cmd+Alt+3` |
| Bullets | `Ctrl+Shift+L` | `Cmd+Shift+L` |
| Move Paragraph Up | `Alt+Shift+Up` | `Ctrl+Shift+Up` |
| Move Paragraph Down | `Alt+Shift+Down` | `Ctrl+Shift+Down` |
| Promote | `Alt+Shift+Left` | `Ctrl+Shift+Left` |
| Demote | `Alt+Shift+Right` | `Ctrl+Shift+Right` |
| Styles Pane | `Ctrl+Alt+Shift+S` | `Cmd+Alt+Shift+S` |
| Apply Styles | `Ctrl+Shift+S` | — |

### Insert

Source: [insert](../../crates/engine/src/cmd/insert.rs), [equation](../../crates/engine/src/cmd/equation.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Equation | `Alt+=` | `Alt+=` |
| Link | `Ctrl+K` | `Cmd+K` |
| Bookmark | `Ctrl+Shift+F5` | `Cmd+Shift+F5` |
| Field | `Ctrl+F9` | `Cmd+F9` |
| Insert Date Field | `Alt+Shift+D` | `Ctrl+Shift+D` |
| Insert Time Field | `Alt+Shift+T` | `Ctrl+Shift+T` |
| Insert Page Field | `Alt+Shift+P` | `Ctrl+Shift+P` |

### Views and outline

Source: [view](../../crates/engine/src/cmd/view.rs), [outline](../../crates/engine/src/cmd/outline.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Show/Hide ¶ | `Ctrl+Shift+8 / Ctrl+8` | `Cmd+Shift+8 / Cmd+8` |
| Print Layout | `Ctrl+Alt+P` | `Cmd+Alt+P` |
| Draft | `Ctrl+Alt+N` | `Cmd+Alt+N` |
| Outline | `Ctrl+Alt+O` | `Cmd+Alt+O` |
| Split | `Ctrl+Alt+S` | `Cmd+Alt+S` |
| Full Screen | `F11` | `Cmd+Ctrl+F / F11` |
| Expand | `Alt+Shift+=` | `Alt+Shift+=` |
| Collapse | `Alt+Shift+-` | `Alt+Shift+-` |
| Show Level 1 | `Alt+Shift+1` | `Alt+Shift+1` |
| Show Level 2 | `Alt+Shift+2` | `Alt+Shift+2` |
| Show Level 3 | `Alt+Shift+3` | `Alt+Shift+3` |
| Show Level 4 | `Alt+Shift+4` | `Alt+Shift+4` |
| Show Level 5 | `Alt+Shift+5` | `Alt+Shift+5` |
| Show Level 6 | `Alt+Shift+6` | `Alt+Shift+6` |
| Show Level 7 | `Alt+Shift+7` | `Alt+Shift+7` |
| Show Level 8 | `Alt+Shift+8` | `Alt+Shift+8` |
| Show Level 9 | `Alt+Shift+9` | `Alt+Shift+9` |
| Show All Levels | `Alt+Shift+A` | `Alt+Shift+A` |
| Show First Line Only | `Alt+Shift+L` | — |

### References and fields

Source: [references](../../crates/engine/src/cmd/references.rs), [fields](../../crates/engine/src/cmd/fields.rs), [index](../../crates/engine/src/cmd/index.rs), [citations](../../crates/engine/src/cmd/citations.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Mark Entry | `Alt+Shift+X` | — |
| Field Codes | `Alt+F9` | `Alt+F9` |
| Toggle Field Codes | `Shift+F9` | `Shift+F9` |
| Unlink Fields | `Ctrl+Shift+F9` | `Cmd+Shift+F9` |
| Lock Fields | `Ctrl+F11` | `Cmd+F11` |
| Unlock Fields | `Ctrl+Shift+F11` | `Cmd+Shift+F11` |
| Next Field | `Alt+F1` | `Alt+F1` |
| Previous Field | `Shift+F11 / Alt+Shift+F1` | `Shift+F11 / Alt+Shift+F1` |
| Mark Citation | `Alt+Shift+I` | — |
| Insert Footnote | `Ctrl+Alt+F` | `Cmd+Alt+F` |
| Insert Endnote | `Ctrl+Alt+D` | `Cmd+Alt+E` |
| Next Footnote | `Alt+Shift+> / Alt+Shift+.` | — |
| Update Field | `F9` | `F9` |

### Review

Source: [review](../../crates/engine/src/cmd/review.rs), [speech](../../crates/engine/src/cmd/speech.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Read Aloud | `Ctrl+Alt+Space` | `Cmd+Alt+Space` |
| New Comment | `Ctrl+Alt+M` | `Cmd+Alt+A / Cmd+Alt+M` |
| Track Changes | `Ctrl+Shift+E` | `Cmd+Shift+E` |
| Word Count | `Ctrl+Shift+G` | — |
| Spelling & Grammar | `F7 / Alt+F7` | `F7 / Alt+F7 / Cmd+Alt+L` |
| Thesaurus | `Shift+F7` | `Shift+F7` |

### Mail merge

Source: [mailings](../../crates/engine/src/cmd/mailings.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Edit Recipient List | `Alt+Shift+E` | — |
| Insert Merge Field | `Alt+Shift+F` | — |
| Preview Results | `Alt+Shift+K` | — |
| Finish & Merge | `Alt+Shift+N` | — |
| Print Documents | `Alt+Shift+M` | — |

### Other tools

Source: [tools/specs](../../crates/engine/src/cmd/tools/specs.rs).

| Command | Windows / Linux / BSD | macOS |
|---|---|---|
| Repeat | `F4` | `F4` |
| Macros | `Alt+F8` | `Alt+F8` |
| Search | `Alt+Q` | — |
| AutoText | `Alt+F3` | — |
| Link to Previous | `Alt+Shift+R` | — |
| Extend Selection | `F8` | `F8` |

## Context-sensitive controls

These controls supplement the registry. A focused dialog, menu, selected object or active tool can
take precedence over ordinary text editing.

| Context | Keys / gesture | Effect |
|---|---|---|
| Text navigation | Shift + a caret-movement shortcut | Extends the selection when that combination has no explicit assignment |
| Selected text | Shift+Delete | Cuts the selection; does nothing without a selection |
| Document | Shift+Backspace | Deletes backward |
| Document | Insert | Toggles overtype only if enabled in **File › Options › Advanced** |
| Ribbon | F10; tap and release Alt on non-macOS | Shows KeyTips; follow the displayed letters. Escape backs out |
| Ribbon | Ctrl+F1 (non-macOS); Cmd+Alt+R (macOS) | Collapses/expands the ribbon; a custom Ctrl+F1 assignment takes precedence |
| Document | Shift+F10 | Opens the context menu |
| Read Mode | PageDown / Right; PageUp / Left | Next/previous reading page |
| Read Mode / full-screen views | Escape | Exits after nearer contexts such as selections, tools or menus have handled Escape |
| Equation editing | Arrows, Home, End | Moves inside the equation |
| Equation editing | Tab / Shift+Tab | Next/previous equation part |
| Equation editing | Backspace / Delete; Enter; Escape | Deletes within the equation; builds up the equation, or starts a new paragraph when there is nothing to build; exits equation editing |
| Text box / WordArt text | Escape | Selects the containing object; Delete can then remove it |
| Header/footer with a collapsed selection | Escape | Returns to the main document |
| Format Painter, pen/eraser or table drawing | Escape | Ends the active tool |

Sources: [UI shortcut dispatch](../../crates/ui-egui/src/keys/shortcuts.rs),
[canvas input](../../crates/ui-egui/src/keys/canvas.rs), [KeyTips](../../crates/ui-egui/src/keytips.rs).

## Further help and coverage

| Topic | Existing documentation |
|---|---|
| Recovery and unsaved work | [Recovery](../recovery.md) |
| Proofing setup | [Spelling dictionaries](../spelling-dictionaries.md) |
| Reference manager | [Zotero](../zotero.md) |
| Formats, layout and typing details | [File formats](../file-format-parity.md), [layout](../layout-parity.md), [typing](../typing-parity.md) |
| Interface/platform behavior | [UI](../ui-parity.md), [localization](../localization-parity.md), [hardware/platforms](../hardware-parity.md) |
| Automation | [MCP](../mcp.md), [control protocol](../control-protocol.md), [README CLI examples](../../README.md#for-agents-cli-and-mcp) |
| Known gaps and planned work | [Gaps](../gaps.md), [roadmap](../roadmap.md) |

This is an initial handbook, not a complete tutorial for every command. Detailed walkthroughs for
advanced tables and objects, drawing, equations, document protection, macros and every individual
dialog remain to be expanded. The shortcut tables cover all default bindings in the checked engine
registry plus the contextual controls listed above; additional object/tool/dialog gestures still
need a systematic walkthrough. Existing topic pages remain authoritative for their details and
are currently mostly English. Planned capabilities in roadmap/parity pages are not instructions
for available features.

Contributors: keep the English guide and existing translations aligned and verify behavior against the implementation;
see [AGENTS.md](../../AGENTS.md#user-documentation).
