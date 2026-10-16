# WordCraft Project Report

*Status: 2026-10-13 · branch `fix/open-issues-and-linux-deb-only` · version 0.6.2. Written from the
sources (no guessing from filenames); every claim cites a path. An earlier draft of this file
circulated with an unverified "never-crash violations" section — that section was wrong (the cited
`panic!`s are test-only) and is corrected in §6 below.*

## 1. Overview

WordCraft (`https://github.com/storytold/wordcraft`) is a clean-room, open-source, Rust-native word
processor aiming at Microsoft Word parity — ribbon, styles, tables, track changes, references, mail
merge — plus agent control (every action is a command). Target users: writers and professionals who
want a Word-like workflow without proprietary software, and AI agents driving documents over CLI /
MCP / control channel. Maturity: **alpha** (`README.md`: ~88% of ribbon features have commands,
~60% of the way to replacing Word for general work). 271 GitHub issues were open at the time of
writing (all filed 2026-10-08…13, none labelled).

## 2. Tech stack

- **Language**: Rust, edition 2024, `rust-version = "1.90"` (`Cargo.toml` workspace root).
- **UI**: `egui`/`eframe` 0.36 over `wgpu` (`apps/wordcraft`, `crates/ui-egui`). Web front end via
  eframe + `Trunk` (`apps/wordcraft-web`, `Trunk.toml`).
- **Rendering/export**: `vello_cpu` 0.2 (raster), `krilla` 0.8 (PDF), `image` 0.25.
- **Text**: `skrifa` 0.47 + `harfrust` 0.13 (shaping), `wuff` 0.2.9 (WOFF), `unicode-bidi`,
  `unicode-segmentation`, `unicode-linebreak`, `unicode-script`.
- **Formats**: `quick-xml` 0.41 + `zip` 3 + `cfb` 0.15 (OOXML / legacy `.doc`), `miniz_oxide`.
- **Infra**: `serde`/`serde_json`, `thiserror`, `rayon`, `regex`, `kurbo`; clipboard `arboard` 3.6
  (+ `wl-clipboard-rs`, XDG portals), dialogs `rfd`, printing via git dep `craft-print`.
- **Toolchain**: Cargo workspace (`crates/*`, `apps/*`, `xtask`), `cargo xtask ci` gate,
  GitHub Actions (thin wrappers over `storytold/craft-common-actions@main`), `nfpm` for the .deb.

## 3. Architecture

| Directory | Purpose |
|---|---|
| `apps/wordcraft` (`src/main.rs`) | Native desktop binary (eframe/wgpu); clipboard, file dialogs, control server, printing, logging |
| `apps/wordcraft-cli` (`src/main.rs`) | Headless `convert/info/text/inspect/render/run/commands/parity/mcp/zotero` |
| `apps/wordcraft-web` (`src/main.rs`, `web.rs`) | Browser build (WebGPU + WebGL2 fallback), `trunk build` → `dist/web` |
| `crates/geom` (L0) | Units (pt/twip/EMU/in/cm), rects, measurement parsing |
| `crates/doc` (L1) | Document model (stories/paras/runs/tables/styles/fields/notes/comments/revisions) + edit primitives |
| `crates/fonts` (L1) | Font DB (bundled OFL + system), metrics/outlines/shaping, Word substitution (`word.rs`) |
| `crates/proof` (L3) | Spelling/grammar/hyphenation (EN list, PL Hunspell) |
| `crates/layout` (L2) | Line breaking, pagination, tables, headers/footers, hit testing |
| `crates/docx` (L2) | OOXML read/write (ECMA-376); bounded reader (`package.rs: Package::open_limited`) |
| `crates/docbin` (L2) | Word 97–2003 `.doc` reader ([MS-DOC] over `cfb`) |
| `crates/formats` (L2) | TXT/MD/HTML/RTF/ODT/LaTeX via intermediate `model::Flow` |
| `crates/metafile` | WMF/EMF → neutral vector model |
| `crates/render` (L3) | `vello_cpu` rasteriser, PNG/JPEG export |
| `crates/pdf` (L3) | `krilla` PDF (subset fonts, selectable text, links/outline) |
| `crates/engine` (L4) | `Session`, undo, **command registry** (`src/cmd/*.rs`), feature catalog, I/O dispatch |
| `crates/mcp` (L5) | MCP server (headless or bridged to the app) |
| `crates/control-key` | Control-port key file location/reading |
| `crates/zotero` | Zotero word-processor wire protocol vs engine `Session` |
| `crates/ui-egui` (L6) | Ribbon front end (swappable); `WordApp::run` = user path, `WordApp::execute` = programmatic path |
| `packaging/linux` | `package.sh` (builds + stages), `nfpm.yaml` (deb metadata), `.desktop`/MIME/AppStream, `postinst.sh` |
| `packaging/web`, `apps/wordcraft-web` | Static-site build (dev artifact; not a release target) |
| `xtask` | `ci` (fmt, clippy `-D warnings`, tests, assets, layers, wasm), `parity`, `layers`, `assets`, `version`, `stats`, `ico`, `corpus` |
| `docs/` | Parity checklists, architecture, roadmap, protocols, and the Phase reports in §6 of `CHANGES_SUMMARY.md` |
| `assets/` | `app-icon/` (original SVG + generated PNGs), OFL fonts, hyphenation/spelling data; every file has an `ATTRIBUTION.md` row (enforced) |
| `contributors/` | Generated `contributors.json` + self-chosen `people.toml`, baked by `crates/ui-egui/build.rs` |

Data flow: files → `engine::io` → `doc::Document`; every action is a `CommandSpec` through
`Session::run` (undoable, hostile-param fuzzed); `layout` produces positioned display lists;
the UI paints them (`render`); save/export goes back through `engine::io`. UI state lives in
`WordApp`/`Session`; prefs in `ui.json`; logs beside prefs. IPC: JSON-lines control channel
(`ui-egui/control.rs`, key file per `control-key`), MCP over stdio, CLI as subprocess.

## 4. Build and packaging system (full multi-platform build scope)

- `packaging/linux/package.sh`: `cargo build --release --locked -p wordcraft -p wordcraft-cli`,
  stages an FHS tree (`/usr/bin`, `.desktop`, MIME, metainfo, hicolor icons, docs, man pages, copyright,
  changelog), validates with `desktop-file-validate`/`appstreamcli` when present. Supports building
  AppImage, deb, rpm, and tar.gz for `x86_64`, `aarch64`, and `riscv64` cross-targets.
- `packaging/linux/flatpak/`: Flatpak manifest (`ai.storyteller.wordcraft.yml`) and bundling script.
- `packaging/macos/`: `package.sh` builds universal `WordCraft.app` bundle and DMG with layout and background;
  `import-cert.sh` imports Developer ID signing certificates; `entitlements.plist` specifies macOS hardened runtime.
- `packaging/windows/`: `package.ps1` builds MSI installers (WiX `wordcraft.wxs`) and portable zips for x64,
  arm64, and x86; `sign.ps1` handles Azure Trusted Signing.
- CI (`.github/workflows/`): `release.yml` builds multi-platform release artifacts across macOS (universal DMG/CLI),
  Windows (MSI + zip for x64/x86/arm64), Linux (AppImage, deb, rpm, tar.gz for x86_64/aarch64, riscv64 tarball, Flatpak),
  FreeBSD (`freebsd.yml`), and WebAssembly zip. `printing.yml` matrix tests Ubuntu, Windows, and macOS.
  `windows-arm64.yml` and `freebsd.yml` provide specialized platform test runners.
- Release process (`docs/releasing.md`): bump via `cargo xtask version set`, merge to `release`,
  CI drafts the GitHub Release with `SHA256SUMS.txt`. All platform targets are fully enabled and supported.

## 5. Testing

- `cargo xtask ci` = `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings`
  + `cargo test --workspace` + `xtask assets` + `xtask layers` + wasm check. Focused runs:
  `cargo test -p <crate>`, `cargo test -p wordcraft -p wordcraft-ui-egui`, headless UI shots via
  `wordcraft-ui-egui --example ui_shot`.
- Coverage: ~800+ unit/integration tests (`crates/*/src/tests*.rs`, `crates/{docx,engine,mcp,fonts,ui-egui}/tests/`),
  typing-parity tests (`engine/tests_typing.rs`), hostile-input property tests incl.
  `hostile_params_never_panic` (every command with junk params), format round-trips + fuzz
  (`formats/tests.rs`), print stability tests. This work added 10 regression tests (table
  selection, clipboard HTML, unsafe-link ODT/LaTeX, txt labels, `document.text` final-shown,
  CLI validation/help, save-changes guard).
- Gaps: no `cargo audit` integration (tool unavailable here); UI tests need a display or `ui_shot`;
  system-clipboard *writes* and print spooling can't be covered headlessly; many open issues lack
  repro tests (see `ISSUES_ANALYSIS.md`); web/E2E coverage is thin.

## 6. Code quality findings (all verified at the cited lines)

- **Never-crash holds.** Workspace roots carry `#![deny(clippy::unwrap_used, …)]`
  (e.g. `crates/docx/src/lib.rs:8`, `crates/engine/src/lib.rs:7`); `clippy.toml` allows
  unwrap/panic/indexing **only in tests**. The `panic!`s previously flagged in
  `crates/docx/src/read/math.rs:330ff` and `diagram.rs:577ff` sit inside `#[cfg(test)] mod tests`
  (modules open at `math.rs:313`, `diagram.rs:550`); the `unwrap()`s in `docx/src/package.rs:353`
  etc. are likewise test-only. `cargo clippy --workspace --all-targets -- -D warnings` is green.
  Hostile input is bounded (`Package::open_limited`, capped allocations, `*_limited` readers).
- **Fixed by this work**: clipboard writes missing on Cut paths (`ribbon.rs`, `canvas.rs` — #1265);
  control-channel `execute` bypassed the save-changes guard (`ui-egui/lib.rs:execute` — #1268);
  `table.columnWidth`/`rowHeight` ignored multi-selections (`engine/cmd/table.rs` — #1242);
  ODT/LaTeX exported `javascript:` targets (`formats/odt.rs:224`, `latex.rs:321` — #679 rest);
  CLI silently defaulted bad `--page`/`--scale` (`apps/wordcraft-cli/main.rs` — #981); dark-ruler
  tokens swapped (`ui-egui/theme.rs` — #1148).
- **Remaining risks**: deceased-platform `cfg` code left in tree (audited, inert); `Session::run`'s panic-guard is a last resort, not a
  licence; clipboard *reads* on Wayland depend on arboard/XDG portal (#1213 open).
  (Note: high-severity silent model drop bugs #886 DrawingML shapes, #997 cx:chart, #1034 settings,
  #999 backgrounds, and #1037 field deletions have all been fixed with round-trip regression tests.)
- **Security**: `javascript:`/`vbscript:`/`data:` hrefs now filtered in HTML/MD (pre-existing,
  `formats/tests.rs`) **and** ODT/LaTeX (new); LanguageTool API key in plaintext `ui.json` (#764,
  open, needs decision); encrypted `.doc` unsupported (#1079, documented limitation).
- **Performance**: caret glide tuning (#1208), full-page CPU re-render per edit (#1109),
  large-document responsiveness (#554) — all open, none regressed here.
- **Accessibility/i18n**: save-changes dialog a11y (#1015), KeyTips exposure (#1012) open; 10
  catalogues ~397 keys behind (#1189); translation mechanism (#336) open.
- **Docs**: parity docs updated for the deb-only scope (`releasing.md`, README, roadmap,
  file-format table, control protocol); `ROADMAP.md` history left untouched deliberately.

## 7. Dependency audit

`cargo audit` was run on `Cargo.lock` (553 dependencies scanned across the workspace).
Results: **0 vulnerabilities / CVEs found**.
Advisories: 2 unmaintained crate warnings from upstream font parsing libraries:
1. `rustybuzz` 0.20.1 (`RUSTSEC-2026-0206`: unmaintained)
2. `ttf-parser` 0.25.1 (`RUSTSEC-2026-0192`: unmaintained)

Workspace pins are current-generation (`egui`/`eframe` 0.36, `skrifa` 0.47, `kurbo` 0.13,
`quick-xml` 0.41, `zip` 3, `arboard` 3.6, `winit` 0.30). macOS-only `fmv-macos-events`/`objc2*`
and Windows-only `windows`/`clipboard-win` dependencies were removed from the lock.

## 8. Prioritized recommendations

**High**: decide #764 (credential store) before more key-bearing integrations; reproduce #1232's
outline crash under `ui_shot` and prove the virtualization fix.
**Medium**: wire `cargo-audit` / `cargo-deny` directly into `xtask ci`; Wayland clipboard robustness (#1213);
backfill repro tests for remaining open bugs; Hunspell dictionaries UX (#1225/#1063/#1167).
**Low**: caret-glide tuning (#1208), ribbon overflow polish (#1228/#897/#1110), template gallery
(#1227), Docker images (#1196), KeyTips for table ribbons (#1183).
