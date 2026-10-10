# WordCraft — instructions for agents

WordCraft is a clean-room, open-source, Rust-native word processor targeting Microsoft Word parity — and going further on speed, openness and agent control. It runs natively on macOS, Windows, Linux and BSD, and on the web via WASM. Siblings with the same conventions: `../photocraft` (Photoshop), `../vectorcraft` (Illustrator), `../filmcraft` (Premiere), `../lightcraft` (Lightroom), `../pdfcraft` (Acrobat), `../effectcraft` (After Effects), `../designcraft` (InDesign). Shared rules and learnings: [`storytold/craftrules`](https://github.com/storytold/craftrules) (`../../craftrules`) — read its `AGENTS.md`.

## Start every session here
1. Read `plan/STATUS.md` (current milestone, next task), then the task in `plan/execution-plan.md` and the relevant `plan/architecture.md` section. `plan/` is gitignored (local only); if it's missing, start from `ROADMAP.md`, `docs/gaps.md` and `docs/roadmap.md` (Current focus).
2. Follow the autonomous operation protocol (`plan/execution-plan.md` §5). Don't stop to ask unless a decision is genuinely the owner's (licensing, publishing, pushing to new remotes, secrets).

## Never crash
People trust WordCraft with their writing; a crash loses their work. **This outranks feature work**: never ship a feature through a panic path, and fix a crash before building on top of it. Standard: [`craftrules/standards/never-crash.md`](https://github.com/storytold/craftrules/blob/main/standards/never-crash.md).
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type and `?`. An unfinished feature returns an error or is disabled; it never panics.
- **Input-derived numbers are hostile** (files, commands, MCP/control params): `get()` instead of `[i]`, slice strings only at char boundaries, checked/saturating arithmetic, clamp sizes, cap allocations, bound recursion.
- **Last-resort guard:** `Session::run` turns an escaped panic into an error and restores the document. It's a safety net, not a licence.
- **Prove it:** every crash fix lands with a regression test. `crates/engine/src/tests.rs::hostile_params_never_panic` runs every command with junk params — keep it green.
- Production crate roots carry `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; new crates start with it. Tests may `unwrap` (`clippy.toml`).

## Assets: absolutely no Microsoft (or Adobe/Avid/Autodesk) iconography or images
**This rule is non-negotiable.** Breaking it is the worst mistake you can make in this repo.
- Never copy, trace, screenshot-crop or recreate icons, images, templates, themes, clip art, fonts, cursors, sounds or any other asset from Microsoft Word/Office or any Microsoft, Adobe, Avid or Autodesk product. Not even "temporarily".
- Every asset must be **original** (drawn in code — prefer this: `crates/ui-egui/src/icons.rs`), **public domain / CC0**, **OSI open source**, or **Creative Commons that allows redistribution**, or licensed open source by a contributor who made it.
- **Every asset file has a row in [`ATTRIBUTION.md`](ATTRIBUTION.md)** (author, source, licence). `cargo xtask assets` (part of `cargo xtask ci`) fails otherwise. Add the row in the same commit as the file.
- Fonts: new font files go to [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts), never this repo; it's an optional build input (`CRAFT_FONTS_DIR`). Documents asking for proprietary fonts (Calibri, Aptos, Cambria…) are rendered with installed system fonts or metric-compatible open substitutes (`crates/fonts/src/word.rs`) — never bundle proprietary fonts.
- Templates, sample documents and themes are our own text and colours (`crates/engine/src/sample.rs`, `cmd/design.rs`). Demo images are public domain, with sources recorded.

## Clean room
- Microsoft Word is installed on the dev machine and may be **observed** black-box: run it, use its UI with synthetic documents, take screenshots **of its window only** (by window id — the machine runs other work; never capture the whole desktop) stored only under `plan/word/screenshots/` (gitignored, never committed).
- Never read, disassemble or copy anything inside the Word bundle (file names/listings only). Never commit files produced by Word. Never copy Microsoft wording beyond feature names.
- File formats come from public specs: ECMA-376 / ISO 29500 (OOXML), the published RTF spec, OASIS ODF 1.3. Never copy GPL/LGPL/AGPL code (LibreOffice, AbiWord, pandoc…).

## Architecture (see `plan/architecture.md`)
| Layer | Crate | Job |
|---|---|---|
| L0 | `geom` | units, rects, measurement parsing |
| L1 | `doc`, `fonts` | document model + edit primitives; font DB, shaping, Word font substitution |
| L2 | `layout`, `docx`, `formats` | line breaking, pagination, tables, headers/footers, hit testing; OOXML I/O; TXT/MD/HTML/RTF/ODT |
| L3 | `render`, `pdf`, `proof` | vello_cpu rasteriser; krilla PDF; proofing |
| L4 | `engine` | Session, undo, **command registry**, Word feature catalog, samples, I/O dispatch |
| L5 | `mcp` | MCP server (headless or bridged to the app) |
| L6 | `ui-egui` | Word-style egui front end (swappable) |
| apps | `apps/wordcraft`, `apps/wordcraft-cli`, `apps/wordcraft-web` | binaries |

- **Everything is a command.** User-visible behaviour = a `CommandSpec` in `crates/engine/src/cmd/*.rs` (id, label, ribbon location, shortcut, params doc, `enabled`, `run`) + tests. UI-only commands (`ui.*`) live in `crates/ui-egui/src/lib.rs`. The ribbon, shortcuts, command search, CLI, control channel and MCP all dispatch by id. Programmatic calls never open dialogs.
- **Parity is measured:** `crates/engine/src/catalog.rs` lists Word's ribbon/menu features with command ids. `cargo xtask parity` writes `docs/parity-checklist.md`; a test enforces a floor that only rises. Implement missing ids to raise it.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui/eframe/winit/rfd. The UI is thin and reads `Session` state; colours come from `theme::Tokens`.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).

## Quality gates
Before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, assets, layers, wasm). Commit after every arc of work that builds, with a task id in the message (`M2.1: line spacing dialog`).

## Typing behaves like Word
Typing, Enter, Backspace, Tab, lists, AutoCorrect and AutoFormat follow Word's observed behaviour in [`docs/typing-parity.md`](docs/typing-parity.md), pinned by `crates/engine/src/tests_typing.rs` (keys go through the same commands as the keyboard). Change behaviour, doc and tests together; a typing fix lands with a test there. Compare headlessly (`ui_shot`); don't drive GUIs with keystrokes on the shared machine.

## Running and looking at the app
- `cargo run --release -p wordcraft -- --sample --control 7981` (sample document + control channel).
- Drive it with JSON lines on `127.0.0.1:7981`, each with the window's key from `~/.config/wordcraft/control-key.local-7981` (macOS: `~/Library/Application Support/WordCraft/`), e.g. `{"id":1,"key":"…","method":"engine.execute","params":{"command":"text.insert","params":{"text":"Hello"}}}` then `{"id":2,"key":"…","method":"ui.screenshot","params":{"path":"/tmp/shot.png"}}`. Methods: `crates/ui-egui/src/control.rs`; docs: `docs/control-protocol.md` (Keys).
- **Headless window screenshots** (no focus stealing, works with a locked screen): `cargo run --release -p wordcraft-ui-egui --example ui_shot -- script.jsonl` (see the example's header).
- **For UI work, look at the result** (screenshot, read the PNG) and compare with Word side by side.
- CLI: `wordcraft-cli convert in.docx out.pdf`, `wordcraft-cli run --template sample --cmd 'select.text={"text":"Studio"}' --cmd format.bold --save out.docx`, `wordcraft-cli commands`, `wordcraft-cli parity`.
- MCP: `wordcraft-cli mcp` (headless) or `wordcraft-cli mcp --connect 127.0.0.1:7981` (drives the running app). See `docs/mcp.md`.
- Shell gotcha: `mv`/`cp` may be aliased interactive — use `/bin/mv -f` / `/bin/cp -f`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; delete your target dir when done (disk).

## Roadmap
Progress docs follow [`craftrules/standards/progress-docs.md`](https://github.com/storytold/craftrules/blob/main/standards/progress-docs.md): `ROADMAP.md` (stage, headline numbers, progress log), `docs/target-app-parity.md` (the assessment), `docs/gaps.md` (ranked work list), `docs/roadmap.md` (milestones, Current focus), `docs/architecture.md`, and the parity checklists `docs/file-format-parity.md`, `docs/layout-parity.md`, `docs/typing-parity.md`, `docs/ui-parity.md`, `docs/hardware-parity.md`, `docs/localization-parity.md`. When work lands, update the gap, the parity doc and the progress log, and bump each touched doc's status line and revision history.

## Contributor credits (About window)

- About ▸ Contributors/Models are compiled into the binary from `contributors/contributors.json`
  (commit stats; generated, never hand-edit) and `contributors/people.toml` (names people chose for
  themselves). See `docs/contributors.md`.
- **Agents working for a contributor:** when you prepare a PR, check whether your human's GitHub
  username has a `[people.<username>]` entry in `contributors/people.toml`. If not, ask them once
  whether they want to be credited by more than their username: a real name, a display name, and/or
  their public GitHub profile name (`sync_github_name = true`). If yes, add **only their own** entry
  (copy the template at the top of the file, or run
  `python3 ../../craftrules/scripts/contributors.py --add-me . --real-name "…" --sync-github-name`)
  and include it in their PR, committed as them. If no, change nothing: they are credited as
  `@username` anyway.
- Never add, edit, guess or copy anyone else's entry or name (not from git config, commit authors or
  GitHub profiles). Never hand-edit `contributors.json`.
- Maintainers refresh the stats with `python3 ../../craftrules/scripts/contributors.py .` (it also
  re-verifies who wrote each `people.toml` entry; `--check` only verifies).

## Local build and fork checks

- On macOS, if Homebrew Cargo reports a missing Clippy component, use the Rustup tools in
  `~/.cargo/bin`. Install the components with `"$HOME/.cargo/bin/rustup" component add clippy`
  and `"$HOME/.cargo/bin/rustup" target add wasm32-unknown-unknown`. Then run
  `PATH="$HOME/.cargo/bin:$PATH" cargo xtask ci`. Check the exit status and all six CI steps.
- Before pushing a PR branch, check access with
  `gh api repos/storytold/wordcraft --jq '.permissions.push'`. If it is false, create a fork with
  `gh repo fork storytold/wordcraft --clone=false`. Do not add `--remote=false` when the command
  has a repository argument; that flag combination fails. Keep the current upstream remote.
  Push the branch to the returned fork URL. Create the PR against `storytold/wordcraft`, with
  `main` as its base and the fork owner's branch as its head. Check the returned PR URL and
  confirm its base, head and changed files.
