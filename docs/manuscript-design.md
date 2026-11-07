# Manuscript projects: design

> **Last updated:** 2026-11-07 · **Status:** proposal, nothing implemented. Describes a
> project-based manuscript and thesis workflow (a binder of sections, compiled to DOCX or PDF) built
> on the existing engine. Open questions at the end need a maintainer decision before work starts.

## Why

Long documents such as theses, books and reports are hard to manage as one Word file. Writers
reorder chapters, keep notes and research beside the text, track how much they have written, and
want to see the structure without scrolling through 300 pages. Applications built for this split
the manuscript into a tree of small sections, then **compile** them into one formatted document at
the end.

WordCraft already has most of what that final document needs. It has headings, outline view,
captions, cross-references, tables of contents and figures, footnotes, citations with a
bibliography, Zotero, and DOCX and PDF export. What it lacks is the project layer around it.
This document designs that layer.

## Goals

1. **A project is a folder of ordinary documents.** Each section is a `.docx` that opens in Word,
   LibreOffice or WordCraft on its own, so a supervisor can review one chapter with tracked
   changes and send it back.
2. **A binder.** A tree of folders and sections, reordered by drag or by command, with a title,
   synopsis, label, status and word-count target on each.
3. **Compile.** One command combines the included sections in binder order into a single
   document, applies a format preset, rebuilds the generated content and exports DOCX or PDF.
   It works the same in the app, the CLI and over MCP.
4. **Numbering, cross-references and citations that span sections** come out right in the
   compiled document.
5. **Snapshots.** Saved versions of a section that can be compared and restored.
6. **Never lose work.** Same rules as the rest of WordCraft: no panics, hostile input is bounded,
   and a damaged project file never stops the sections from opening.

## Non-goals (for the first version)

- Real-time collaboration or cloud sync. The project is a folder, so users can sync it with git,
  Syncthing or a cloud drive themselves.
- A new file format for section text. Sections are `.docx`.
- LaTeX output. Markdown and HTML export exist per document; whole-project export to them can
  come later through the same compile pipeline.
- Editing many sections in one scrolling view (Scrivenings-style). See Milestone 4.

## Concepts

| Term | Meaning |
|---|---|
| **Project** | A folder ending in `.wcproj` holding `project.toml` and the subfolders below |
| **Binder** | The tree of items in `project.toml` |
| **Item** | A folder (groups items, may carry its own text) or a section |
| **Section** | One `.docx` under `sections/`, named by the item's id, never by its title |
| **Research** | Files kept with the project but never compiled (PDFs, images, notes) |
| **Snapshot** | A saved copy of a section at a moment, with an optional label |
| **Compile preset** | Rules that turn the binder into one document: template, which items, separators, title page, front matter, output format |

## Project format

```
thesis.wcproj/
  project.toml          # binder, metadata, targets, presets (the only file WordCraft must parse to list the project)
  sections/
    s0001.docx
    s0002.docx
  research/             # anything; shown in the binder's Research folder
  snapshots/
    s0002/
      20261107T143000Z.docx
      20261107T143000Z.toml   # label, word count
  templates/
    university.dotx     # optional; a preset may also name a template outside the project
  .wcproj-lock          # who has the project open (host, pid, time); advisory
```

### `project.toml`

```toml
format = 1                    # project format version; newer files open read-only with a message
title = "Ink and Iron: Metallurgy in Early Print"
author = ""                   # left blank by default; never guessed from git or the OS
next_id = 7

[targets]
project_words = 80000
deadline = "2027-03-31"      # parsed with chrono
session_words = 1000

[[item]]
id = "s0001"
kind = "folder"
title = "Front matter"
include = true

  [[item.children]]
  id = "s0002"
  kind = "section"
  title = "Abstract"
  synopsis = "300 words on the question, method and finding."
  label = "front"
  status = "draft"            # free text; the UI offers todo / draft / revised / final
  target_words = 300
  include = true

[[preset]]
name = "University submission"
template = "templates/university.dotx"
output = "pdf"                 # pdf | docx | odt
title_page = "s0003"           # an item to place first, or omitted
separator = "section-break"    # between top-level folders: section-break | page-break | none
section_separator = "none"     # between sections inside a folder
heading_from_title = false     # true: insert each item's title as a heading at its level
include_labels = []            # empty = every included item
toc = true
```

Why TOML and ids:
- `project.toml` is readable and diffable, so a project kept in git shows clean changes when
  the binder is reordered.
- Section files are named by id, so renaming an item never moves a file, and a broken
  `project.toml` still leaves every section intact and openable.
- `toml` is already in `Cargo.lock`; its licence must be confirmed against the allowlist before
  the new crate depends on it directly.

### Reading `project.toml` safely

The file is hostile input like any other:
- The total file size is capped (for example 4 MiB), along with the item count (for example
  10,000) and nesting depth (for example 32). An item over a cap is reported, not loaded.
- Ids must match `^[a-z0-9]{1,32}$`. Paths in `template` and `title_page` must stay inside the
  project folder, or be absolute paths the user chose. `..` is refused.
- Unknown keys are kept and written back, so a newer version's settings survive an older
  WordCraft.
- If the file can't be parsed, the project opens in **recovery mode**: every `sections/*.docx`
  is listed in a flat binder ordered by id, and the broken file is kept as
  `project.toml.broken-<time>` before anything is written.
- Saves go through the existing `safe_save` path (write to a temporary file, then rename).

## Where the code goes

| Piece | Crate | Layer | Notes |
|---|---|---|---|
| Project model, `project.toml` I/O, binder operations, snapshots, compile | new `crates/manuscript` | L5 (beside `mcp`, `zotero`) | Depends on `engine`, `doc`, `chrono`, `toml`. No UI dependencies. Builds for wasm (no direct file access there; see "Web") |
| `project.*`, `binder.*`, `compile.*` commands | `crates/manuscript` | L5 | Registered into the session's command registry the same way other crates' commands are, so the CLI, control channel and MCP get them for free |
| Binder sidebar, corkboard, compile dialog | `crates/ui-egui` | L6 | Thin: reads the project state and dispatches commands |
| CLI | `apps/wordcraft-cli` | apps | `wordcraft-cli project compile thesis.wcproj --preset "University submission" -o thesis.pdf` |

`xtask/src/layers.rs` needs one new row: `("manuscript", Class::Layer(5))`. The crate root
carries the standard `#![deny(clippy::unwrap_used, …)]` attribute, and `Cargo.toml` uses
`[lints] workspace = true`.

The engine already holds several open documents (`Session::with_document`,
`crates/engine/src/session/documents.rs`). The binder uses that, so opening a section is an
ordinary open, and editing, undo, autosave and recovery work unchanged.

## Commands

Everything is a command, as elsewhere in WordCraft. Programmatic calls never open dialogs.

| Id | Params | Does |
|---|---|---|
| `project.new` | `{"path", "title"?, "template"?}` | Create the folder, `project.toml` and one empty section |
| `project.open` | `{"path"}` | Load the binder; open the last edited section |
| `project.close` | `{}` | Save open sections and the binder |
| `project.info` | `{}` | Binder tree, word counts, targets, open section (pure) |
| `binder.add` | `{"parent"?, "after"?, "kind", "title"}` | New item; a section gets an empty `.docx` made from the default preset's template |
| `binder.import` | `{"path", "parent"?, "split"?: "none"\|"heading1"\|"heading2"}` | Bring in an existing document, optionally split at headings into one section each |
| `binder.move` | `{"id", "parent", "index"}` | Reorder; refuses moving a folder into itself |
| `binder.rename`, `binder.setMeta` | `{"id", …}` | Title, synopsis, label, status, target, include |
| `binder.delete` | `{"id"}` | Moves the item to a Trash folder in the binder; its file stays until `binder.emptyTrash` |
| `binder.split` | `{}` | Split the open section at the caret into two sections |
| `binder.merge` | `{"ids"}` | Join adjacent sections into the first |
| `binder.openItem` | `{"id"}` | Open the section in the editor |
| `snapshot.take` | `{"id"?, "label"?}` | Copy the section's current state |
| `snapshot.list`, `snapshot.restore`, `snapshot.compare` | `{"id", "at"?}` | `compare` uses the existing `review.compare` command |
| `compile.run` | `{"preset", "path"?}` | Compile and export (see below); returns warnings |
| `compile.preview` | `{"preset"}` | Compile into a new unsaved document in the editor, so the user can inspect it before export |
| `compile.presets` | `{}` | List presets (pure) |

`crates/engine/src/tests.rs::hostile_params_never_panic` must cover the new commands. If the
registry it walks doesn't include commands from L5 crates, the crate gets its own equivalent test.

## Compile

Compile is the core of this design. It reuses engine primitives that already exist and are tested.

### What already exists

- **`Document::adopt`** (`crates/doc/src/edit.rs:1041`) takes another document's body as a
  fragment of this one. Its media, lists, styles, notes, comments and tracked changes come along,
  with lists, notes, comments and permission ranges renumbered so they can't collide. A style we
  already have by id stays ours. That is exactly what compile wants: the template's styles win.
- **`insert.textFromFile`** (`crates/engine/src/cmd/insert.rs:134`) already does
  open → adopt → unlink the inserted file's sections → insert the fragment. Compile does the same
  for each section, without a selection.
- **Section breaks** (`layout.break`), **TOC** (`references.toc`, `references.updateToc`),
  **table of figures** (`references.updateFigures`), **index** (`references.updateIndex`) and
  **field update** (`references.updateFields`) exist as commands.
- **Template styles:** `tools.updateStyles` ("Update Styles from Template") and
  `styles.organizer` exist.
- **Export:** `crate::io::save_path_with` writes DOCX, ODT, RTF, HTML, Markdown and PDF through
  one call (`crates/engine/src/io.rs:844`).

### What compile has to add

`adopt` was built for pasting one document into another. A compiled manuscript joins dozens, so
two things it doesn't yet do have to be handled. Both were checked in the code, not assumed:

1. **Bookmarks aren't renamed.** `adopt` renumbers notes and comments but copies `BookmarkStart`
   and `BookmarkEnd` names as they are. If two sections both have `_Ref123` (Word makes these for
   cross-references), the compiled document has duplicates. Cross-references within a section
   are fine. References between sections are handled separately below.
2. **Bibliography sources aren't merged.** `Document::sources` (`crates/doc/src/lib.rs:115`,
   used by `CITATION` fields and `references.bibliography`) isn't touched by `adopt`. Compile
   must union every section's sources by `tag`. When two sections have the same tag with
   different content, keep the first and add a warning.

### The pipeline

```
load preset
  └─ template → new Document (styles, numbering, page setup, headers/footers)
for each included item in binder order (depth-first):
  ├─ open sections/<id>.docx            (error → warning, skip; compile continues)
  ├─ rename its bookmarks:  name → name, or <id>_name if the name is already taken
  │    and rewrite REF/PAGEREF/NOTEREF fields inside the same section to match
  ├─ collect its sources into the union (by tag)
  ├─ adopt into the compiled document    (template styles win)
  ├─ unlink its sections (as insert.textFromFile does)
  ├─ optional heading from the item's title
  └─ separator (section break / page break / none) per preset
resolve cross-section references (below)
insert TOC if the preset asks and the template has none
update fields: TOC, captions/SEQ, table of figures, index, cross-references, bibliography
export to the preset's format, or open in the editor (compile.preview)
```

Compile never changes a section file. It builds a new `Document` in memory, so it can't damage
the project. A failure partway leaves nothing written, because `save_path_with` already writes
safely.

### Cross-references between sections

This is the hardest part, and every project-based writing tool has to solve it.
- In Word, a cross-reference is a `REF` field pointing at a bookmark, and both must be in one
  file. If a reference in chapter 3 points to a figure in chapter 2, no section file can hold
  both ends.
- Proposed: a **project-wide reference name**.
  - `insert.crossReference`, when a project is open, also lists captions and headings in other
    sections, read from an index that compile and the binder keep.
  - Picking one inserts a normal `REF` field whose bookmark name carries a project-scoped prefix:
    `_wcp_<itemid>_<name>`. The target section gets a bookmark of the same name.
  - Opened alone, the field shows its last result, which is ordinary Word behaviour for a missing
    bookmark. Its cached text stays readable.
  - In the compiled document, both ends exist and the field updates normally.
- Compile's bookmark renaming leaves `_wcp_` names alone, because they are already unique per
  project.
- Compile reports every `REF` whose bookmark doesn't exist in the compiled document as a
  warning naming the section.

### Numbering across sections

- **Captions (`SEQ` fields) and heading numbers** are recalculated over the whole compiled
  document by the field update. Figure 3.2 in the compiled thesis is correct even though the
  section file alone says Figure 1.
- **Footnotes and endnotes** are renumbered by `adopt` already. Restarts per chapter come from
  the template's section properties.
- **Page numbering** (roman for front matter, arabic from chapter 1) comes from the preset: the
  front-matter folder's separator starts a section with roman numbering, and the first body
  folder restarts at 1. Both are existing section properties (Format Page Numbers).

### Citations

- **WordCraft citations** (`CITATION` fields plus `Document::sources`) work after the sources
  union described above. `references.bibliography` then renders one bibliography for the whole
  compiled document.
- **Zotero citations** are self-contained: each field carries its CSL JSON
  (`ADDIN ZOTERO_ITEM CSL_CITATION {…}`, `crates/zotero/src/fields.rs`), so they survive
  `adopt` unchanged.
  - Their numbering and the bibliography, however, are produced by the Zotero desktop app
    (`ui.zotero.refresh`), which runs only in the app with Zotero open.
  - `compile.preview` in the app can run a Zotero refresh on the compiled document before export.
    Headless compile (CLI, MCP) keeps each citation's last rendered text and warns that the
    bibliography wasn't refreshed.
  - Numeric styles (IEEE, Vancouver) therefore need the in-app path. Author–date styles look
    right either way, apart from bibliography ordering.

### Figures

Compile is the place to deal with floating pictures moving when text changes (discussed in the
issue that led to this document):
- **`binder.add` and the section template default to inline figures:** an inline picture in its
  own paragraph, with a caption and Keep with Next.
- **Compile can check for floating pictures** anchored to paragraphs and list them as warnings
  ("Figure in s0012 floats; it may move when text above it changes"). Converting them is left to
  the user.
- Lock Anchor (`wp:anchor/@locked`) isn't modelled today. The DOCX writer always writes
  `locked="0"` (`crates/docx/src/write/story/drawing.rs:259`). Supporting it is a separate,
  independent fix and should land first.

## Snapshots and targets

- `snapshot.take` copies the section's saved bytes to `snapshots/<id>/<UTC time>.docx`, with a
  small `.toml` beside it for the label and word count. Times go through `chrono`.
  - Optional automatic snapshot before `binder.split`, `binder.merge` and `snapshot.restore`.
  - Restoring takes a snapshot of the current state first, so a restore can be undone.
- Word counts reuse `review.wordCount`'s counting on each section. The binder caches counts by
  file modification time, so a large project doesn't open every section to show totals.
- Session target: words written since the project was opened, counted as the change in the
  total. Deleting text can make it negative; it is shown as is.

## User interface (L6)

- **Binder sidebar** on the left, in place of the Navigation Pane when a project is open (the
  Navigation Pane stays available as a toggle). It shows the tree, the word count against the
  target, and status colours. Dragging an item runs `binder.move`.
- **Corkboard:** a grid of cards for a folder's children, showing title, synopsis, label and
  status. Dragging a card reorders it. The cards are drawn in code (`theme::Tokens` colours, no
  bitmap assets).
- **Inspector** on the right for the selected item: synopsis, notes, label, status, target and
  snapshots.
- **Compile dialog:** pick a preset, edit its options, then Preview or Export.
- **Project tab** in the ribbon for these commands. Its catalog entries are WordCraft's own,
  outside Word parity, so they don't affect `cargo xtask parity`.

UI work follows the house rule of looking at the result: `ui_shot` screenshots of each view
before merging.

## Web (wasm)

- The web build has no direct file system. A project can be opened there as a `.zip` of the
  `.wcproj` folder (the zip crate is already in the lock file), edited in memory, and downloaded
  again as a zip, with compile producing a download.
- This is Milestone 4. Milestones 1–3 must still **build** for wasm (`cargo xtask wasm`), with
  the file-system commands returning a clear "not available in the browser" error.

## Testing

- **Unit:**
  - `project.toml` round-trips, unknown keys are kept, every cap is enforced, and recovery mode
    triggers on a broken file.
  - Bookmark renaming, and the sources union with conflicting tags.
- **Compile fixtures (synthetic, made in tests):**
  - Three sections with figures, a cross-reference between sections, footnotes in two sections,
    one `CITATION` per section and a TOC.
  - Assert: figure numbers run 1–N, the cross-reference resolves to the right number, footnotes
    run 1–N, there is one bibliography with every source, and the TOC lists every heading.
- **Hostile:** run every new command with junk params; use a project folder whose
  `project.toml` points at `../../etc/passwd`, nests 10,000 levels deep or names 100,000 items.
- **Round-trip:** compile to DOCX, reopen and compare the document model. Compile to PDF and
  check the page count and the text on each page.
- **Performance:** a generated 300-page, 40-section project with 100 pictures, measured for
  compile time and peak memory. Gap 20 in [gaps.md](gaps.md) notes that large documents are
  unmeasured, and this project would measure them.

No file produced by Word is committed. Fixtures are built in test code.

## Milestones

Estimates are rough, for one person working on it most days, and assume the engine behaves as
described here.

| # | Scope | Estimate |
|---|---|---|
| 0 | Lock Anchor support (separate fix, see Figures) | 1–2 days |
| 1 | `crates/manuscript`: project format, binder commands, snapshots, CLI `project` subcommands, tests. No UI | 1–1.5 weeks |
| 2 | Compile: pipeline, bookmark renaming, sources union, cross-section references, presets, warnings, fixtures | 1.5–2 weeks |
| 3 | UI: binder sidebar, inspector, compile dialog, Project tab, word-count targets | 1.5–2 weeks |
| 4 | Corkboard, Scrivenings-style view of several sections in one scroll, web zip projects, import-and-split from a long `.docx` with headings | 2–3 weeks |

The first useful release is milestones 1–3: write in sections, reorder them, compile a
correctly numbered PDF or DOCX.

## Risks

| Risk | Mitigation |
|---|---|
| Cross-section references confuse users when a section is opened alone | The field keeps its last result; the binder marks sections with unresolved references; compile warnings name them |
| `adopt` misses another document part that collides when joining many files (beyond bookmarks and sources) | The compile fixture covers every kind the doc model has: content controls, permissions, comments, revisions, text boxes, charts. Each collision found gets a test in `crates/doc` |
| Long compile times on large theses | Measure early (Testing › Performance); compile is pure, so later sections can be read on worker threads (not on wasm) |
| Zotero numbering in headless compile | Documented limitation; in-app preview refreshes |
| Two copies of WordCraft editing one project | Advisory `.wcproj-lock`; the second opens read-only with a message |
| Scope creep into a different product | Non-goals above; each milestone ships on its own |

## Open questions for maintainers

1. **Mode or separate app?** A project mode inside WordCraft (a Project tab, a binder that
   replaces the Navigation Pane) or a separate binary under `apps/` that reuses the crates? This
   design works for either. The commands are the same, and only the shell differs.
2. **Is this in scope for upstream WordCraft,** whose roadmap is Word parity, or should it live
   in a fork or a sibling Craft repository?
3. **Section storage:** `.docx` (chosen here, for interoperability) or the internal JSON model
   (faster to load and cleaner in git, but nobody else can open it)?
4. **Name and extension:** `.wcproj` is a placeholder.

## Revision history

| Date | Change |
|---|---|
| 2026-11-07 | First draft |
