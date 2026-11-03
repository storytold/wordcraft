# WordCraft: egui 0.37 migration plan

**Status:** current-main reconciliation through `b4da59f0c38b506c36629b1d812ad7c0519fc238` passed all eight mandatory CI stages on 2026-10-25: 4,270 passing test executions, zero failures and 4 ignored (`egui037-wordcraft-main-refresh-ci-20261025-0603`). This includes main's encrypted-DOCX integrity requirement and selected-paragraph-mark highlight behavior. The dependency pins and lockfile are unchanged. A fresh review verified that the merge preserves the IME documentation and upstream fixes. Earlier counts and source snapshots below are historical.

Read-only comment balloons now consume the explicit `AlignedGalley` result from craft-ui at
`25da1314d443171b4adeaad2cd9454b273f35c6f`: height comes from the requested reading area,
and painting retains the existing reading origin and card clip while preserving overwide
right-to-left rows through an explicit offset. Added geometry/ink and actual card/body/reply
regressions use bundled Latin fonts and require exact embedded Arabic/Hebrew faces in the
canonical `CRAFT_FONTS_REQUIRED=1` lane. Optional-input builds still exercise Latin geometry.
The earlier 616-file migration/current-main PDF overlay and its CI run remain historical
baseline evidence. The current full CI qualifies the API/dependency followup, preview fix,
phonetic labels and preserved notice/packaging source repair.
Native GUI/IME/capture/save-reopen traces, package notice verification, paired performance,
platform/installer execution and app publication remain separate gates. This records local
work, not publication or complete platform acceptance. Historical notes below remain history.
**Scope:** the WordCraft checkout at
`/home/codex/work/CraftApps/egui037-migration-20261022/wordcraft-implementation`.
**Baseline source:** `3a4770ef8ce037d2650c93e2d5764536549357c7` ("Merge pull request #3286 from
matstrange/fix/protection-inspect-tracking"), workspace version 0.10.0, edition 2024,
`rust-version = "1.95"`, resolver 3. Baseline `Cargo.lock` SHA-256
`be3d0675270364a44b7011d1d80a6a255d5823c1a52088347faa18fa9cebfd88`; baseline `Cargo.toml`
SHA-256 `7838ccd3744441a54b9e8024121e9007e64b54294a6e0b442fe758edc45c184a`.

Baseline dependency identities in that lockfile:

| Source | Baseline identity |
|---|---|
| egui family | registry `egui`/`eframe`/`egui-winit`/`egui-wgpu`/`egui_glow`/`egui_kittest`/`epaint`/`emath`/`ecolor` = 0.36.2; `kittest` = 0.4.0 |
| winit | registry 0.30.13 |
| accesskit | registry `accesskit` 0.24.1, `accesskit_winit` 0.32.2, `accesskit_unix` 0.21.1 |
| craft-libs | `craft-ui` + `craft-print` at `d27646a5f7c8b4fc85d9dd3213b8c88b58e0088d`; `craft-pdf` at `c3487320b2d1de657b860b3c8abb44e1f2b32bd8` |
| fonts | `CRAFT_FONTS_REF` `b076417b5880bd4154bd22f00695b1d28c6bea83` (CI), plus the bundled Inter/SourceSans/JetBrains Mono assets |
| toolchain | workspace MSRV 1.95; the planning host used rustc/cargo 1.99.0, which is not an acceptance toolchain by itself |

**Target and current local pins:** egui family 0.37.0 at
`6d518fc799681571963842f95b55a6b68431b15a`, winit at
`a517b3846ee8457d84ee4975110bfda6797c44dc`, and craft-ui at
`25da1314d443171b4adeaad2cd9454b273f35c6f`. The regenerated lock records these exact
revisions and passed the dependency-source audit. The current-main shared PDF, printing and
locale update uses craft-pdf, krilla, craft-print and craft-i18n at
`17744430be327d0257496a6a31c8d38fe20a6756`. These are immutable canonical Git URL pins.
The predecessor passed full CI; the latest main integration is undergoing its own full CI.
The translation catalog is pinned
separately to the reviewed `9205108cfdced5177706264038848b68b2422d2b`; it is not a Cargo dependency.
Publication observations and
authority remain separate from this local status. Stock 0.37 is a measurement control, not the production target.
Earlier baseline and research revisions below remain historical references.

This plan follows the suite contract in
`craft-libs/docs/egui-037-suite-rollout.md` and the planning contract. It records only
WordCraft-specific source, behavior and evidence. The shared rollout plan remains the authority
for suite-wide publication order and the shared performance protocol.

## 1. What is and is not changing

WordCraft already uses the 0.36 `App::logic` / `App::ui` split and an existing root `Ui`.
`App::logic` is already called before every UI pass, including repeated passes; `App::ui`
already receives the root `Ui`. The 0.37 migration must preserve that architecture rather than
rewriting it. The real work is:

1. making the dependency graph coherent and immutable;
2. repairing the concrete 0.37 API breaks;
3. removing application-side text processing that the fork now performs correctly;
4. preserving one-shot lifecycle work across repeated passes and logic-only frames;
5. validating that the page, chrome, accessibility, input, clipboard and save/reopen behavior
   remain what users expect.

No engine, command id, document format, undo semantic or persistence format is intentionally
changed by this migration. The app remains `Session`-driven; egui remains a thin L6 front end.

## 2. Repository inventory

### 2.1 Workspace and package targets

`Cargo.toml` declares one workspace, `members = ["crates/*", "apps/*", "xtask"]`, with no nested
workspace and no vendor directory. `cargo metadata --no-deps` lists these packages:

- application: `wordcraft` (`apps/wordcraft`, native eframe app), `wordcraft-web`
  (`apps/wordcraft-web`, wasm eframe app), `wordcraft-cli` (no egui dependency).
- UI: `wordcraft-ui-egui` (`crates/ui-egui`, the only direct egui/eframe consumer in the
  workspace). It also depends on `craft-ui` for print-page controls and on `egui_kittest` as a
  dev-dependency.
- lower layers and stand-alone crates: `wordcraft-{geom,doc,fonts,layout,docx,docbin,formats,
  render,pdf,proof,engine,mcp,zotero,control-key,metafile}`. None may acquire an egui dependency;
  `cargo xtask layers` enforces this (`xtask/src/layers.rs`, `UI_CRATES`).
- `xtask` itself has no egui dependency.

Direct egui-family declarations are only:

| Manifest | Declaration | Notes |
|---|---|---|
| root `Cargo.toml` | `egui = "0.36"`, `egui_extras = { version = "0.36", default-features = false }`, `eframe = { version = "0.36", default-features = false }`, `egui_kittest = "0.36"` | `egui_extras` is currently unused by source; remove it or pin it deliberately. |
| `crates/ui-egui/Cargo.toml` | `egui = { workspace = true }`; dev: `egui_kittest = { workspace = true, features = ["wgpu", "snapshot"] }` | `egui` is not given `default_fonts` here; production gets it from the app roots' eframe feature, while the UI test root must declare egui's font features itself because kittest's own dev-dependencies do not propagate. |
| `apps/wordcraft/Cargo.toml` | `eframe` with `default_fonts`, `wgpu`, `accesskit`, `wayland`, `x11`, `links`; `egui`; Linux/BSD `winit = "0.30"` with `x11` | `eframe` is the native renderer; S1 adds `clipboard` to preserve 0.36’s integration-forced OS text clipboard. |
| `apps/wordcraft-web/Cargo.toml` | `egui`; wasm `eframe` with `default_fonts`, `wgpu`, `accesskit` | web runner and browser download/inbox shell. |

`craft-libs` is pinned in two places: `craft-ui` and `craft-print` at
`d27646a5f7c8b4fc85d9dd3213b8c88b58e0088d`, and `craft-pdf` at
`c3487320b2d1de657b860b3c8abb44e1f2b32bd8`. `craft-ui` and `craft-print` share public print
types; `craft-pdf` does not share those types. Do not churn `craft-pdf` just to make revision
strings look uniform.

### 2.2 Entry points and examples

- Native: `apps/wordcraft/src/main.rs`. `impl eframe::App for App` at `main.rs:44` implements
  `logic`, `raw_input_hook`, `ui` and `on_exit` (`main.rs:45-120`). `NativeOptions` and the
  winit/platform setup are at `main.rs:376-420`; Windows wgpu backend/probe code is
  `apps/wordcraft/src/graphics.rs:26-180`.
- Web: `apps/wordcraft-web/src/web.rs`. `WebShell` implements `eframe::App` at `web.rs:117`;
  `logic` drains browser drops/messages and calls `WordApp::logic`, `ui` calls `WordApp::ui`;
  `WebOptions`/WebGPU-WebGL2 selection is at `web.rs:35-65`.
- UI library: `WordApp::logic` at `crates/ui-egui/src/lib.rs:2087`, `WordApp::ui` at
  `lib.rs:2353`, `raw_input_hook` at `lib.rs:2325`.
- Headless screenshot example: `crates/ui-egui/examples/ui_shot.rs` (1440x900 at 2x by default,
  `wgpu` harness, `--sample`, control JSON lines).
- Performance probe: `crates/ui-egui/examples/perf_probe.rs` (open, typing, wheel/trackpad
  scrolling, zoom, ribbon tabs, resize, accept-all; returns medians, p95 and max, not p99).
- Large-table benchmark: `crates/ui-egui/src/tests/perf.rs` (`big_table_scroll_bench`, ignored,
  2000x8 synthetic table by default, `WORDCRAFT_BENCH_DOCX` override).
- Control protocol: `crates/ui-egui/src/control.rs`, documented in `docs/control-protocol.md`.
  `ui.screenshot`/`ui.render` reply shapes, request ids, timeouts and paths are public contracts.

### 2.3 Platform configuration

Relevant cfgs are all in the UI/app boundary, not in lower layers:

- `cfg(target_os = "linux")`: `zbus = "=5.19.0"` in `crates/ui-egui/Cargo.toml`;
  `desktop_theme.rs` reads the desktop portal. `system_clipboard.rs` and
  `apps/wordcraft/src/clipboard.rs` use `wl-clipboard-rs` for the **regular** Wayland
  clipboard (formats/text/pictures); they do not read or write PRIMARY
  (`ClipboardType::Regular` only, `clipboard.rs:135-160`). Middle-click on the canvas is
  WordCraft's autoscroll, not a PRIMARY paste (`autoscroll.rs:1-10`).
- `cfg(all(unix, not(target_os = "macos")))`: app `winit` X11; XWayland selection is in
  `main.rs:404-413`.
- `cfg(target_os = "macos")`: `fmv-macos-events`, `objc2-app-kit`, `dispatch2`; termination
  workaround `schedule_macos_terminate` at `main.rs:121-160`; AppKit pasteboard additions in
  `apps/wordcraft/src/clipboard.rs`.
- `cfg(target_os = "windows")`: `open`, `windows` 0.62, `clipboard-win`; wgpu DX12/FXC and
  backend probe in `apps/wordcraft/src/graphics.rs`.
- `cfg(target_arch = "wasm32")`: `js-sys`, `web-sys`, browser inbox, `font_store.rs`, browser
  downloads; `sys-locale` with `js`.

No code uses child viewports (`show_viewport_deferred`/`show_viewport_immediate`), `craft-ui`
docking, or `craft-window`. The app's "windows" are `egui::Window` areas inside the single root
viewport. A viewport-focused migration is therefore not triggered by this app; the one root
viewport still matters for screenshots, DPI and wgpu surface limits.

## 3. Dependency target and publication constraints

### 3.1 Exact coherent target

All egui-family crates must resolve from one immutable fork revision, not merely from
`version = "0.37.0"`. The target family is:

| Crate | Target | Source |
|---|---|---|
| `egui`, `eframe`, `egui-winit`, `egui-wgpu`, `egui_glow`, `egui_kittest`, `egui_extras`, `epaint`, `emath`, `ecolor`, `epaint_default_fonts`, `egui_system_font_provider` | 0.37.0 | the approved, published, reachable `storytold/egui` P8 release containing the complete series: P2b async W15P routing, P7 with held-key release pairing, P7b/E21b, P7c's selected #8763/#8748 fixes and justified performance fixes; the exact SHA is assigned only after review/publication. Research input: aiongg `a4f9a17f64a6bae61350fb85804c9df0d9865916` (`ai-0.37` in an `emilk/egui` clone). |
| `winit` | 0.30.13 | the completed, approved, published, reachable `storytold/winit` revision containing W1-W15 minus W5, plus W17-W27 including W20 as W5’s replacement including the W27 lifecycle revision, and W15P PRIMARY; exact SHA assigned after review/publication. Research input: aiongg `8b35388bd719e8cc86a95971ba0bc026c9a8f058` (`ai-lang`). |
| `accesskit` / `accesskit_winit` | 0.25.x / 0.34.x | registry, resolved by the lockfile |
| `kittest` | 0.5.0 | registry, via `egui_kittest` 0.37 |
| `craft-ui`, `craft-print` | the published craft-libs revision containing CL-1 and the CL-2F frozen helper containing the reviewed PR 199 port on current main | exact git `rev`, not a branch; it must move in the same atomic dependency stage as the egui/winit pins |
| `craft-pdf` | existing `c3487320...` unless a separate reviewed reason appears | exact git `rev` |

Required prerequisites, all in the approved revisions above:

- **E4/P2b + W15P:** the complete shared contract in `egui` plan §6 E4/P2b and winit plan
  §5.3.1-5.3.2. Claim selection synchronously on a fresh **pointer or keyboard** triggering
  serial for the focused seat, with origin and epoch; only serving/reading is async, using the
  total-deadline (2 s), byte (16 MiB), worker (4 readers, 2 writers, queue 8) bounds. A live
  per-owner completion waker routes only to the still-current request; canceled/stale results
  are discarded. WordCraft's application clipboard stays regular-only; built-in `TextEdit`
  PRIMARY is tested only where the feature graph enables it.
- **E21b/P7b + winit W27 lifecycle:** the complete shared contract in `egui` plan §6 E21b and
  winit plan §5.2.3/§5.2.2, not a partial restatement. It includes the owner/context generation
  on both Available and Unavailable, distinct explicitly-sent empty Available payloads, cache
  clearing plus a committed disable/enable transition on owner/unavailable changes, the latest
  completed pass (including `None`) as authoritative, and current-epoch serial-mismatch
  handling that applies incoming edits once while deferring outgoing state. WordCraft's two
  canvas producers need real model adapters.
- **P7c selected UI-regression fixes:** the fork owns the selected #8763 click-away selection
  fix and #8748 hidden-parent/child fix, with pointer and keyboard regressions. #8764 remains
  only a proposal input. WordCraft has no child viewports, so #8748 is a dependency-series
  gate rather than app behavior, but no app workaround substitutes for P7c.

The root `[patch.crates-io]` must contain every crate whose crates.io identity would otherwise
pull a second egui/winit. A dependency's own `[patch]` table is ignored when it is a dependency,
so the wordcraft root must repeat the patches. At minimum: `egui`, `eframe`, `egui-winit`,
`egui-wgpu`, `egui_glow`, `egui_kittest`, `egui_extras`, `epaint`, `emath`, `ecolor`,
`epaint_default_fonts`, `egui_system_font_provider`, and `winit`. Keep the existing
`krilla` patch only because `craft-pdf` requires it; it is unrelated to this migration.
The published manifest must not contain any absolute path patch, branch, `path = "/home/..."`,
or machine-local checkout.

Use exact requirements in the workspace (`=0.37.0`, `=0.30.13`) so that removing a patch cannot
silently float to a different registry release. The resulting `Cargo.lock` must contain exactly
one `egui`, `epaint`, `emath`, `ecolor`, `eframe`, `egui-winit`, `egui-wgpu` and `winit` package,
all with the intended `git+...#<sha>` source and no `path` source.

### 3.2 Existing shared work

- **Current shared-library integration base:** `3832c47ef68a0e6214f2c1e85f3526b6adcb686e`,
  the reconciled merge base that carries the reviewed 0.37 API port of `craft-ui`. It is the
  base for this migration; the exact reachable immutable publication pin remains a
  central-publication prerequisite and is not guessed here. The historical PR 199 head
  `b68de8c5f4f1c060904d211a7b1320aa0f7bce78` and audit merge
  `ec8840af8c5fda695cf195cdd05e5ae078415c07` are evidence only and must not be merged as
  trees: they omit current `craft-platform` and revert security, settings, docking and printer
  fixes.
- **craft-libs PR 199** (historical). Its API port — `WidgetType` to `accesskit::Role`,
  `UiBuilder::id`/`Ui::id` to `scope_id`, `Id::new` to `Id::unique`,
  `egui_extras`/`egui_kittest` 0.37, and the missing-glyph docking-fixture change — is already
  contained in the `3832c47…` base. The historical API-port hunks from
  `8614e4a04fae516fba16c22b31ff1008cb6be10b` and the relevant README changes from `1764bca`
  on the earlier `49190884278f16c9a9f5823d7698a8ffb5deb062` main describe how that base was
  assembled; they are no longer the base to apply. Preserve intervening fixes and provenance,
  and re-run the actual gates; neither PR 199 nor historical checks qualify the resulting
  tree. WordCraft pins the published CL-1 plus CL-2F revision in the same atomic dependency
  stage as the egui/winit pins (section 9), never after a 0.37-only first step.
- **craft-libs #192 / print types** already landed on the path to PR 199: `craft-ui::printing::Printer`
  gains `label`; `craft-print::Printer` gains `description`. WordCraft must adapt when it moves
  its `craft-ui`/`craft-print` pin:
  - `apps/wordcraft/src/print.rs:8` builds `printing::Printer` from
    `craft_print::printers()`; add `label: printer.description`.
  - `crates/ui-egui/src/lib.rs:6038` (test) constructs `Printer { name, default }`; add
    `label: String::new()` or `..Default::default()`.
  - `crates/ui-egui/src/printing.rs` continues to re-export the shared types and labels.
- **craft-libs #198** is a language-support proposal/issue, not a merged dependency. Its
  requirements that matter to WordCraft are already represented in fork patches E5/E12/E13/E16/E17/
  E19/E21: logical text at OS/accessibility boundaries, no double bidi, IME replacement by bytes,
  and Latin shortcut keys on non-Latin layouts. Do not import the proposal's locale/formatting
  roadmap or any prohibited font asset into this migration.

### 3.3 Fork patch provenance and decisions

The fork is a superset of 0.37.0 plus the suite patches documented in its `PATCHES.md`. The
completed P8 revision retains the whole series; do not delete unrelated patches to reduce the
diff. WordCraft consumes only that published P8 revision, never the aiongg candidate or an
intermediate stage. The WordCraft-relevant patches are:

| Patch | Relevance to WordCraft | Decision |
|---|---|---|
| E1 eframe viewport hooks | No child viewports today | Retain; do not adopt. |
| E2+E3 egui-wgpu surface fit/error scopes | Protects large windows and device setup | Retain; test large/high-DPI window and device-error paths. |
| E4 egui-winit Wayland clipboard | WordCraft reads the regular clipboard through arboard/wl-clipboard-rs; E4 changes egui-winit's own path | Required fork prerequisite with the complete P2b/W15P contract (synchronous pointer-or-keyboard claim, bounded async transfer); test the app's regular clipboard separately and do not assume E4 replaces WordCraft's HTML/RTF/MathML path. Do not add app-level PRIMARY paste. |
| E5/E12/E16 bidi shaping, direction, caret affinity | Replaces WordCraft's manual `bidi_display`/`visual_row` workarounds | Required. Remove double processing; use logical text and fork layout. |
| E6 IME focus request | Text fields and canvas IME | Retain; test preedit/focus. |
| E7/E19 Latin keys on non-Latin layouts | App shortcuts use `Event::Key.key`; fork supplies Latin layout characters | Required for shortcut parity; test Russian/Arabic/Hebrew layouts. |
| E8 TextEdit glyph overshoot | Dialogs and custom combo fields | Retain; visual test combining marks/Thai/Vietnamese. |
| E9 RTL UI mirroring | WordCraft has no RTL interface catalog yet | Retain as capability; do not enable a half-mirrored UI without a locale. |
| E10 web named keys/language | Browser shortcuts and hidden input | Required for web; test browser workflow. |
| E11 vello single-threaded glyph rasterization | WordCraft's page renderer is separate; epaint UI glyphs still use it | Retain; no app change unless `vello_cpu` feature use is introduced. |
| E13 grapheme editing | Fork TextEdit/caret behavior; app's document editing is engine-owned | Retain; ensure no duplicate grapheme handling; test emoji/combining. |
| E14 fallback baseline | CJK/Indic UI fallbacks | Retain; test CJK/Indic interface text. |
| E15 line breaker | egui UI text (ribbon, dialogs); document layout is app-owned | Retain; do not move document layout to epaint. |
| E17 language on Context/Ui | Accessibility language and widget text | Adopt `Context::set_language(Some(lang.code()))`; optionally tag document/canvas content with the editing language. |
| E18 eframe UI language | Could replace app-side `Frame` language plumbing | Evaluate; not required. |
| E20 widget text language | Dialog/ribbon text layout | Retain; test CJK/Thai line breaking. |
| E21 `ImeEvent::DeleteSurroundingBytes` | Desktop macOS/Wayland IME replacement | Required. Add a byte-range path alongside the existing char-count web path. |
| E21b/P7b outgoing IME surrounding text | Canvas/Wayland surrounding-text support; the candidate has incoming deletion only | Required fork prerequisite under the complete E21b/W27 contract (owner generation, empty-Available, serial/epoch rules). TextEdit producer and W27 adapter; both WordCraft canvas producers need real model/preedit/UTF-8 adapters. |

Winit provenance: the final pin must be the completed, approved, published `storytold/winit`
revision containing W1-W15 minus W5, plus W17-W27 including W20 as W5’s replacement including the W27 lifecycle revision, and the
W15P PRIMARY path. The inspected `8b35388` `ai-lang` commit is an aiongg research input, not a production
pin. WordCraft's existing issue references (#787, #721 and related IME/DPI fixes) depend on
W1-W15; do not compare only against upstream 0.30.13 and conclude the patches are new. The patch
table records the removal condition for each. No wordcraft vendor directory exists, so no vendor
deletion is part of this plan.

### 3.4 Pin and lock checks

Proposed checks, to run through the managed `task.py` workflow after the plans land:

1. `cargo metadata --locked --format-version 1` over the root and with
   `--filter-platform wasm32-unknown-unknown` (`cargo metadata` has no `--target`); parse
   package `source` and `version` and fail on any `path` source,
   branch source, unknown git URL, duplicate egui/winit, or a `craft-ui`/`craft-print` pair from
   different craft-libs revisions.
2. `cargo tree -d --workspace` and `cargo tree -i egui`, `cargo tree -i winit`; no duplicate
   family member.
3. `cargo fetch --locked` from a clean clone, then a second `cargo metadata --locked`; no
   network-only branch resolution.
4. A small `xtask` dependency gate (proposed name `cargo xtask deps`) with tests for: missing
   root patch, two egui versions, old `craft-ui`, moving branch, absolute path, and a git SHA
   that is not reachable from the approved fork ref. This turns the manual checks into a
   regression.
5. Feature-identity checks, not just version metadata: `cargo tree -e features --target
   wasm32-unknown-unknown -p wordcraft-web` and the native `-p wordcraft` tree must show the
   expected `egui/default_fonts`, `egui-winit/clipboard` (required on native, with arboard resolved),
   `accesskit`, `wgpu` and no `system_fonts` resolution for the app root. Record the exact
   feature sets in the lock/evidence manifest.
6. Publication reachability: a clean clone must fetch the approved `storytold` revisions and
   pass `--locked` without local overrides. S1 cannot start while the only available trees are
   the aiongg research inputs; do not substitute a candidate SHA for a published pin.
7. Release/packaging check after the source migration: signed native builds and the web package
   must use the same lock identity; no local overrides.

## 4. Concrete 0.37 API and behavior refactors

The historical compiler audit at
`/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/{fork,stock}/wordcraft.errors`
lists 42 diagnostics, identical for stock and fork. They are clues, not current acceptance and
not an exhaustive list. The concrete fixes are:

### 4.1 `Event::MouseWheel` gains `source`

`egui::Event::MouseWheel` now has `source: MouseWheelSource` (0.37 `data/input/event.rs`).
Affected constructors and patterns:

- `crates/ui-egui/src/control.rs:247` (`ui.wheel` synthetic event): add
  `source: egui::MouseWheelSource::Unknown`.
- `crates/ui-egui/src/canvas.rs:642` (pattern), `canvas.rs:3821` (test constructor).
- `crates/ui-egui/src/scroll.rs:180`, `:278`, `:329`, `:418`; `tests/perf.rs:39`;
  `examples/perf_probe.rs:106`; `mini_toolbar.rs:779`; `dialogs_keyboard.rs:430`; and the
  test constructors in `lib.rs`
  (`:3772`, `:3817`, `:3924`, `:4131`, `:4277`, `:4332`, `:4397`, `:4522`, `:4821`, `:5018`,
  `:6094`).
- Add `..` to patterns where the source is not used. For tests, use `Wheel` or `Trackpad` to
  match the behavior under test; for the control channel use `Unknown`.
- Preserve `CanvasScroll` behavior: WordCraft disables egui's canvas wheel handling
  (`ScrollArea::scroll_source { mouse_wheel: false }`) and computes its own delta. Do not start
  feeding the same momentum twice. The app's `SYNTH_MOMENTUM` policy remains Linux/BSD-only;
  macOS/Windows/Web already send their own momentum. Use `source` only for diagnostics/future
  differentiation, not to change current deltas without a measured test.

### 4.2 `ScrollArea::State::offset` is private

Use the 0.37 accessors and builder:

- `crates/ui-egui/src/canvas.rs:1650`: `app.canvas.scroll_offset = out.state.clamped_offset();`
  (the old public offset was the clamped value; `unclamped_offset()` is for painting/rubber-band
  only).
- `crates/ui-egui/src/widgets.rs:1158`: `shown.state.clamped_offset().y`.
- `crates/ui-egui/src/scroll.rs:440` (test helper): `seen = (out.state.clamped_offset().y, zoom);`.
- `crates/ui-egui/src/ribbon.rs:543` and `:560`: read `scroll.state.clamped_offset().x`.
  Setting `state.offset.x` is no longer possible. Store a one-frame pending horizontal offset
  (in `ui.data_mut` keyed by the scroll area id, or a small `RibbonScroll` field) and build the
  next frame with `egui::ScrollArea::horizontal().horizontal_scroll_offset(x)`. Clear the pending
  value after it is consumed. This preserves the edge-chevron behavior without touching private
  state.
- `crates/ui-egui/src/canvas.rs:1173-1205` already uses `.scroll_offset(after)`; keep that
  builder path. The scroll maximum remains the app's own `content_size - inner_rect.size()`.

Verify the sweep is exhaustive: the only `state.offset` reads/writes in `crates/ui-egui/src`
are `canvas.rs:1650`, `widgets.rs:1158`, `scroll.rs:440`, `ribbon.rs:543/560`, plus the builder
path at `canvas.rs:1173-1205`; no other direct field access remains. Regression: wheel notch and
trackpad tests assert the same offsets and page indices after the change; ribbon left/right
chevrons move exactly one step and clamp at both ends.

### 4.3 `WidgetType` becomes `accesskit::Role`

The app uses `egui::WidgetInfo::labeled/selected(WidgetType::...)` in 34 places. Replace with
`egui::Role` using the mapping egui itself documents (and craft-libs PR 199 uses):

| Old | New | Files |
|---|---|---|
| `Button` | `Role::Button` | `backstage.rs:79,127,234,524`; `widgets.rs:294,333,336,402,510,523,934,1256,1289`; `print_page.rs:290`; `format_shape.rs:439`; `previews.rs:231`; `chrome.rs:333,392,416,492,551,698`; `window_close.rs:60`; `ribbon.rs:355,467,696,2271` (Full Screen) |
| `SelectableLabel` (selected) | `Role::Button` with `selected` | `options.rs:150`; `previews.rs:530`; `chrome.rs:671`; `ribbon.rs:404` |
| `TextEdit` | `Role::TextInput` | `widgets.rs:878` |
| `Image` | `Role::Image` | `print_page.rs:278` |
| `Other` | `Role::Splitter` for the ruler drag grip, not `Unknown` | `table_ruler.rs:60` |

The `table_ruler` grip is a draggable column/row boundary; `Role::Splitter` is an input role and
matches its behavior. If a test insists on the old generic role, review whether that test was
asserting implementation rather than semantics; do not hide a real control under `Unknown`.
`Role::Unknown` is not an input in `egui::accessibility::is_input`, so `unnamed_widgets()` cannot
detect a missing/Unknown node. Add an explicit `get_by_role(Role::Splitter, ...)` assertion for
the grip; a mutation that removes its name must fail `unnamed_widgets()`, and a mutation that
changes the role to `Unknown` must fail the explicit role query.

`egui::accesskit::Role` remains available for tests; `egui::Role` is the re-export. Existing
`get_by_role_and_label(Role::Button, ...)` tests remain valid.

### 4.4 Screenshot command becomes a callback

0.37 removes `Event::Screenshot` and `UserData`; `ViewportCommand::Screenshot` carries a
`ScreenshotCallback`, and `Context::request_screenshot` is the convenience API. Current code:

- `crates/ui-egui/src/lib.rs:2577` `issue_screenshots` sends
  `ViewportCommand::Screenshot(UserData::new(token))`.
- `crates/ui-egui/src/lib.rs:2593` `collect_screenshots` scans `egui::Event::Screenshot` and
  returns immediately when `pending_shots` is empty.
- `crates/ui-egui/src/control.rs:312` `save_screenshot(&ColorImage, path)` encodes/writes.

Required design. WordCraft is one of the eleven `craft-ui` capture consumers and adopts the
shared helper only after CL-2F freezes it; it does not take the provisional CL-2 revision used
by the DesignCraft/GridCraft pilots. The callback contract is:

- Keep request admission, paths, reply schema, settling delay and the 8 s deadline in
  `WordApp`. Each request record owns a strong completion slot and waker; the
  `ScreenshotCallback` holds only a `Weak` reference to the slot and receives pixels only. It
  must not capture `WordApp`, a `Context`, a file path, or an unbounded `mpsc::Sender`. The
  callback stores the image, validates it, releases the lock, and only then invokes the live
  slot's app-owned waker. It never calls `Context::request_repaint` itself and never wakes
  while holding the lock; never strongly capture a context from a callback queued in that same
  context.
- `Context::request_screenshot` itself schedules a repaint: it calls `send_viewport_cmd` ->
  `send_viewport_cmd_to` -> `request_repaint_of`. Only callback *arrival* is documented not to
  request a repaint. Do not claim the request is repaint-free; keep the existing
  `request_repaint_after(16 ms)` while a request is pending and wake explicitly after
  completion.
- Admit by count **and** bytes. Before issuing, estimate `width * height * 4` from the current
  viewport logical content and pixels-per-point with checked arithmetic, then reserve its
  working set (raw RGBA + worst-case PNG + base64, a 10/3 factor) in an in-flight budget that
  also counts completed-but-undrained slots. Use the shared defaults: at most 4 outstanding
  requests, 128 MiB reserved per image and 256 MiB aggregate; reject excess admission without
  evicting earlier requests. A 4K viewport at 2x is about 7680x4320x4 ~= 127 MiB raw and is
  therefore rejected by the working-set limit rather than silently exceeding the encode peak.
  Revalidate the checked dimensions and geometry epoch immediately before issuance.
- Bind the image to the issuance-time UI geometry: logical content/screen bounds, size,
  pixels-per-point, zoom and surface epoch. On completion, validate dimensions/byte length
  against the reservation and that epoch. Any geometry/DPI/zoom change before save, **even a
  change back**, cancels and retries within the deadline or returns an explicit stale-capture
  error; never combine old pixels with current geometry. With an unchanged epoch, derive the
  pixel scale from the returned image dimensions and verify aspect/rounding.
- `collect_screenshots` drains the slots unconditionally in `logic`, matches the request
  record, calls `control::save_screenshot(&image, path.as_deref())`, sends exactly one reply,
  and releases the slot and reservation. A timeout/cancel/drop removes the record and releases
  the slot; a late callback then finds a dead weak reference and retains nothing, writes no
  file and sends no second reply. Retained image bytes must return to zero after drain,
  timeout, cancel and shutdown.
- Hidden windows: the screenshot command itself forces a full hidden `App::logic` + `App::ui`
  pass (`ActionRequested::wants_paint()`), while callback arrival does not repaint. A request
  queued in `logic` but issued only in `ui`, including every remaining settling frame, must
  therefore send `ViewportCommand::RequestPaintWhileHidden` from the logic side to reach
  issuance; a wake/repaint alone may run hidden logic only. Keep the timeout contract and test
  both the forced pass and the queued-not-issued settling path.
- The app has one root viewport. If child viewports are added later, route requests by
  `ViewportId` and generation; do not reuse an old token for a new document or viewport.
- `ui_shot` uses `Harness::render`, not the viewport callback; keep it as a separate headless
  path. Do not make `ui.render` depend on the native callback.
- The suite has fourteen capture consumers. The eleven existing `craft-ui` consumers require
  CL-1 and CL-2F; only the DesignCraft/GridCraft pilots use the reviewed CL-2 provisional
  revision, and their adapters are reviewed before CL-2F freezes the API. DeckCraft, FigCraft
  and ArtCraft Native keep contract-compatible local adapters; `ai-egui` has no capture path.
  Do not add `craft-ui` solely for capture; if DeckCraft/FigCraft later adopt it, WordCraft's
  adapter stays unchanged.

Tests: four-request/128 MiB/256 MiB admission, two outstanding requests with distinct
tokens, out-of-order completion, callback once and no self-wake, late callback after
timeout/cancel (no file/second reply), forced hidden pass, queued-not-issued
`RequestPaintWhileHidden`, resize/DPI/zoom change and change-back, and retained-bytes-zero
after drain/shutdown.

### 4.5 `Id::new` and `Ui::id` deprecations

0.37 deprecates `Id::new` in favor of `Id::unique` and renames `Ui::id` to `Ui::scope_id`
(`Ui::id` remains deprecated). Exact symbol-filtered counts in `crates/ui-egui/src` at this
baseline are 108 egui `Id::new` calls (89 `egui::Id::new` and 19 bare `Id::new`) and 37
`ui.id()` calls; `FontId::new` (3) and `LayerId::new` (3) are unrelated and must not be swept.
The mechanical count is a review aid, not a replacement trigger. Replace with same-hash
equivalents:

- `egui::Id::new(x)` -> `egui::Id::unique(x)`; hashes are intentionally unchanged for existing
  ids, so widget state, focus and temp data keys stay stable.
- `ui.id()` -> `ui.scope_id()`; the value is unchanged.
- `UiBuilder::new().id(x)` -> `.scope_id(x)` if any is introduced (none currently in app code).
- Do not replace `Id::unique` with `IdSalt` for globally stored state; `IdSalt` is for child
  scopes only. Preserve the distinction between stable scope ids and `ui.unique_id()`.

The mechanical sweep is large but must be reviewed by symbol, not regex alone: `ui.id()` on a
`Ui` is not the same as `TextureHandle::id()`, `CommandSpec::id()`, `Document::id()` or other
domain ids. The replacement applies only to egui `Ui`.

### 4.6 Panels take `AsIdSalt`

`Panel::left/right/top/bottom` in 0.37 take `impl AsIdSalt`, not `impl Into<Id>`; the panel id
is now `parent_scope.with(IdSalt)`. Existing string literals in `chrome.rs`, `panes.rs`,
`backstage.rs` and `ribbon.rs` compile, but their effective ids change. WordCraft does not persist
panel sizes in `ui.json` or enable egui `persistence`, so the only consequence is fresh in-process
panel state after upgrade. Verify:

- panel order remains title bar -> ribbon -> status -> side panes -> central canvas in
  `WordApp::ui` (`lib.rs:2353-2410`);
- `Panel::top("title_bar")`, `Panel::top("ribbon")`, `Panel::bottom("status")`, the side panes in
  `panes.rs:20-105`, and the `CentralPanel` reserve and clip correctly at 760x480, 1440x900 and
  2x DPI;
- no panel is shown on a non-root `Ui` where 0.37 would change order. `backstage.rs:40-160`
  shows a left panel and then a central panel on the passed root `Ui`; keep that contract.
- Do not use `Context::root_ui` inside `WordApp::ui`: 0.37 explicitly panics when the root `Ui`
  is already borrowed (`Context::root_ui` docs, upstream #8535). Pass the existing `Ui`.

### 4.7 `default_fonts` is no longer a default feature

0.37 removes `default_fonts` from `egui`/`epaint` default features; `eframe` still enables it.
`egui_kittest`'s normal `[dependencies] egui` uses `default-features = false`; only kittest's
own `[dev-dependencies]` enable `default_fonts`/`monochrome_emoji_fonts`, and a dependency's
dev-dependencies do not propagate. Neither kittest's manifest nor its dev-dependency graph
supplies fonts to WordCraft's test root. WordCraft's `theme::font_definitions_language` starts from
`FontDefinitions::default()`, inserts `Inter` at index 0, pushes `SourceSans` at the end, then
appends CJK/system faces (`theme.rs:536-620`). With `default_fonts` the default proportional
family is `["Ubuntu-Light", "egui-icons"]`; with `monochrome_emoji_fonts` it is
`["Ubuntu-Light", "egui-icons", "NotoEmoji-Regular", "emoji-icon-font"]`.

The actual production order is therefore:
`Inter, Ubuntu-Light, egui-icons, SourceSans, <CJK/system faces>`.
Explicitly enable `default_fonts` and `monochrome_emoji_fonts` on the UI test root's egui
dev dependency for the controlled emoji fixture; its expected order is:
`Inter, Ubuntu-Light, egui-icons, NotoEmoji-Regular, emoji-icon-font, SourceSans,
<CJK/system faces>`.
Do not assert the earlier proposed `Inter, SourceSans, Ubuntu-Light, egui-icons` order; it is
wrong and would force an unintended fallback reorder.

Concrete fix:

- Keep eframe's `default_fonts` feature in both app manifests.
- Make `wordcraft-ui-egui` test features explicit with `egui` dev-dependency features
  `default_fonts` and `monochrome_emoji_fonts`; these are egui features, not kittest features.
  Do not rely on kittest's private dev-dependencies: without this explicit declaration the
  test root is font-empty apart from WordCraft's own assets. Do not enable
  `monochrome_emoji_fonts` in production just to make the test order match.
- Add a font coverage test that asserts the exact prefix/position for the feature set under
  test: `Inter` at 0, `Ubuntu-Light` and `egui-icons` in the default fallback block,
  `SourceSans` immediately after that block, then CJK/system faces. Assert `has_glyphs` for the
  interface catalog's punctuation/symbol strings, `icons.rs` labels, Arabic/Hebrew, CJK,
  Thai/Vietnamese combining marks and only emoji the chosen UI font actually covers.
- Choose the emoji source explicitly: interface emoji coverage is the bundled monochrome
  `egui-icons` set (plus the test-only `NotoEmoji-Regular`/`emoji-icon-font` fallbacks from
  `monochrome_emoji_fonts`). The production UI family has no color emoji font; do not assert
  color/ZWJ emoji in the interface test. For document/canvas emoji, use the pinned test-only
  Noto Color Emoji v2.047 fixture from egui plan 10.6 (`39ee3c58...`, OFL-1.1) or record a
  native platform color-font result separately. Do not bundle a proprietary emoji font.
- Keep `system_fonts` off; it would add a system provider and startup discovery cost (upstream
  #8512), change coverage and complicate reproducibility.

The strict accessible-name sweep and this font/glyph work are part of S1, not deferred to the later screenshot stage:
`egui_kittest` 0.37's `check_accessibility` defaults to true and panics on unnamed visible
inputs after `Harness::run`, so S1 cannot leave `cargo xtask ci` green without them.

### 4.8 kittest 0.37 stricter defaults

`egui_kittest` 0.37 fails tests when an input widget has no accessible name and panics on a glyph
no installed font can draw. WordCraft has many custom-drawn controls and many `TextEdit`s whose
adjacent labels are not attached. The migration must not disable these checks globally.

- First run the UI suite with defaults; enumerate `Harness::unnamed_widgets()`.
- Attach names with `Response::labelled_by(label.id)` where a visible label exists, or
  `Response::accessible_name(...)` for icon-only controls. Prefer visible-label association over
  duplicating translated strings. `Ui::label_inputs_by`/`Ui::name_inputs` are available in 0.37
  for groups.
- Audit the dialog modules: `dialogs_controls.rs`, `dialogs_review.rs`, `dialogs_layout.rs`,
  `dialogs_insert.rs`, `dialogs_para.rs`, `dialogs_lists.rs`, `dialogs_table_style.rs`,
  `options.rs`, `backstage.rs`, `colors.rs`, `print_page.rs`, and the combo field in
  `widgets.rs:842-878`. A `TextEdit` with a `hint_text` passes the check, but a visible label is
  better semantics; do not add hints merely to satisfy the test.
- Keep `Role::Button`/`Role::TextInput` accurate; a named widget with the wrong role is still a
  screen-reader regression. The a11y tree in `a11y.rs` (Document/paragraph/table/text-run) must
  keep its current logical text; the fork's bidi changes must not leak visual-order strings into
  AccessKit.
- This is API compatibility, not behavior, so the name sweep and font coverage are part of S1.
  The suite has 15 `h.run()` call sites and 153 harness constructions; most tests use
  `step`/`run_steps`, where the default check does not fire. Add explicit
  `h.check_accessibility()`/`unnamed_widgets()` assertions where the UI settles, and add explicit
  role queries for controls such as the ruler grip. `Role::Unknown` is not an input and cannot
  detect a missing node.

### 4.9 `craft-ui`/`craft-print` print type change

Already described in 3.2. Add a test that discovery maps CUPS `description` to `label`, and that
`shown_names` distinguishes two printers with the same label. Keep the app's command ids and
print settings schema unchanged.

### 4.10 Text: remove double bidi and handle byte IME replacement

This is the highest-risk app-specific change. WordCraft manually reorders right-to-left text for
egui 0.36:

- `crates/ui-egui/src/canvas.rs:2132-2185` `has_rtl`/`bidi_display`/`visual_row`, used by
  `backstage.rs:529-530` for recent file names.
- `crates/ui-egui/src/canvas.rs:2190-2245` `balloon_text` wraps and visually reorders comment
  text itself, then right-aligns RTL rows.
- `crates/ui-egui/src/keys.rs:352-390` handles `ImeEvent::Preedit/Commit/DeleteSurrounding`
  (char counts) and `keys.rs:626-680` implements `delete_surrounding` and `type_text`.
- `apps/wordcraft/src/clipboard.rs` and `crates/ui-egui/src/a11y.rs` use logical text at the
  OS/accessibility boundary; they must stay logical.

With the fork's E5/E12/E16, egui shapes RTL runs and places rows in visual order. Keeping
`bidi_display` would reverse the text a second time. Refactor:

- Replace `bidi_display(name/path)` calls in `backstage.rs:529-530` with the original logical
  strings. Set `LayoutJob`/widget direction only where a specific element needs an explicit base
  direction; for ordinary labels, let `TextDirection::Auto` choose from the first strong
  character.
- Replace the manual wrapping/reordering in `balloon_text` with an egui `LayoutJob` over logical
  text, `TextDirection::Auto`, the existing font/color and `TextWrapping::max_width = width`.
  Do not use `.trim()` on each visual row; let epaint handle trailing whitespace and line breaks.
  If a right-aligned RTL paragraph needs an explicit `halign`, use the fork's
  `halign_min_is_start`/`TextDirection` controls, not pre-reversed strings.
- Keep `has_rtl` only if a non-layout decision needs it; otherwise remove the dead helper and its
  tests. Replace the old `visual_row` tests with tests that assert screen x positions of the first
  and last Arabic/Hebrew words (fork behavior), not the old reordered strings.
- Keep document layout (`crates/layout`) untouched: it has its own bidi/line-break logic and is
  not egui text. This migration only changes egui UI text and comment balloons.
- Add `Context::set_language(Some(lang.code().to_owned()))` in `WordApp::logic` when the effective
  interface language changes, so AccessKit exposes the UI language. Optionally tag the canvas
  document `Ui`/AccessKit node with the document editing language via `UiBuilder::language`; do
  not tag every text run with a guessed language.

IME byte replacement:

- The fork adds `ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes }` on macOS and
  Wayland, sent immediately before the `Commit`. The existing char-count
  `DeleteSurrounding` remains for web/legacy paths.
- Add a match arm next to `keys.rs:384` that:
  1. returns early if a selection exists or an equation is open, as the char path does;
  2. computes `start = off.saturating_sub(before_bytes)` and
     `end = off.saturating_add(after_bytes)` and verifies both are UTF-8 boundaries in the
     paragraph text; if not, refuse the event rather than slice at a non-boundary;
  3. returns immediately when `start == end` (the existing char path does, `keys.rs:656`);
     otherwise selects `start..end`, joins the current typing group, runs `text.backspace` (or
     the same delete command used by the engine), records `ime_join_text` for the following
     commit, and restores the caret if the command is refused;
  4. does not erase paragraph marks/list labels beyond the paragraph.
- Keep one undo step for delete+commit. Extend the existing `#1480` tests and add:
  emoji modifier/ZWJ replacement, combining Vietnamese accent replacement, a letter after an
  emoji sequence, a deletion-only batch, a `0/0` and clamped-empty byte range, and a commit
  with empty text. Test that outgoing
  surrounding text (document text after the operation) is updated, not merely the event mapping.

**E21b/P7b outgoing surrounding text (shared fork prerequisite).** The candidate egui-winit
never calls `WindowExtWayland::set_ime_surrounding_text`, and `egui::IMEOutput` has no logical
text (`data/output.rs:100-114`). E21's incoming delete/commit is not full surrounding-text
support. The complete contract is owned by `egui` plan §6 E21b and winit plan §5.2.3/§5.2.2;
WordCraft depends on it and does not restate a weaker subset. It provides:
an owner/context generation present for both Available and Unavailable; a valid empty document
as Available, explicitly sent as `set_surrounding_text("", 0, 0)` plus commit, distinct from
Unavailable; a bounded transient payload (logical committed text, UTF-8 byte cursor/anchor,
preedit excluded, at most 4,000 bytes, explicit unavailable when the selection cannot fit) with
no full-document allocation for the new payload; W27 cache clearing plus a committed
disable/enable boundary on owner/support transition; the latest completed pass's descriptor,
including authoritative `None`, as the only published value; and current-epoch `done` serial
mismatch applying incoming edits once while deferring outgoing state until a matching serial,
with obsolete epochs invalidated.

Both WordCraft canvas producers (`crates/ui-egui/src/canvas.rs:1556-1561` math/equation caret
and `:1633-1638` text caret) need real model adapters, not `None`: derive the bounded logical
paragraph/equation excerpt and byte cursor/anchor from `session.sel`/the equation editor,
convert scalar/grapheme positions deliberately, exclude `ime_preedit`/`ime_preedit_range`,
and never send display-reordered bidi text. The adapter updates on commit/delete/selection/
focus changes and clears on focus loss, password/no-context and widget switch. Existing
TextEdit buffer cloning is baseline behavior, not part of the new payload allocation.

Tests: outgoing text after delete+commit and deletion-only batches; preedit exclusion and caret
movement; supplementary emoji/combining/RTL logical order; long selection/window limits;
empty→non-empty and unavailable/password→available; late `Done` and current-epoch serial
mismatch vs stale epoch; same-window field switch and two windows; a final pass with `None`;
focus loss/reset/password/no-context clearing; and a native Wayland text-input trace on a real
compositor. A kittest event test is not native IME proof.

Do not make the document engine depend on egui's grapheme implementation. The engine's own
grapheme/backspace rules remain the source of truth for document editing.

### 4.11 Accessibility roles, names and selection

- Keep the custom AccessKit document tree in `a11y.rs` (Document, Paragraph, Heading, List,
  ListItem, ListMarker, Table/Row/Cell, TextRun, Image, Link). The fork's bidi must not change
  the logical `TextRun` strings; `set_text_direction` remains based on the layout's `line.rtl`.
- Ensure every new `Role` assignment has a name. The kittest default check will catch unnamed
  input roles that `is_input` recognizes (`Button`, `TextInput`, `Splitter`, `ComboBox`, ...).
- `refocus_after_chrome_click` (`lib.rs:2471-2520`) relies on `ctx.memory().focused()` and
  `ctx.cumulative_pass_nr()`. 0.37's `TextEdit` can lose its selection before a clicked widget
  runs. P7c owns the selected #8763 and #8748 fixes; #8764 is an open, unmerged proposal input,
  not the dependency. This is a blocking fork prerequisite, not accepted behavior. The
  approved P8 revision must contain the central fixes with pointer and keyboard tests proving
  the intended range reaches the adjacent action in the same pass.
- `Role::Splitter` on the ruler grip must not make a passive ruler tick an input; only the grip
  gets the role.

### 4.12 GPU, surface and texture behavior

WordCraft uses wgpu 30 on native and WebGPU/WebGL2 on web; `graphics.rs` configures Windows
DX12/FXC and the Intel-Mac indirect-validation skip. wgpu and winit versions do not change, so
those APIs should compile; revalidate anyway under the new eframe/egui-wgpu:

- `apps/wordcraft/src/graphics.rs:26-120` still reaches `WgpuSetup::CreateNew` and
  `InstanceDescriptor`; confirm with `cargo check` on Windows.
- E2/E3 surface fit fixes should remove a class of large-window configure errors. Add a window
  size/DPI test near the surface limit if hardware permits; otherwise record the unresolved
  hardware gate.
- Page textures use `TextureOptions::LINEAR` (`canvas.rs:1261-1332`, `print_page.rs:128`,
  `previews.rs`). Do not change filtering/wrap; the upstream #8736 shader cost is about
  predictable filtering, not the app's sampling choice. If a renderer option is changed, test
  the actual page pixels and color emoji, not just compile.
- Screenshot callback memory: `Arc<ColorImage>` may be large at 2x/4K. The count/byte
  admission, in-flight reservation and completed-slot accounting are part of the design.

### 4.13 Clipboard and PRIMARY

The app's clipboard is host-owned (`apps/wordcraft/src/clipboard.rs`, `system_clipboard.rs`) and
supports text, HTML, RTF, MathML, pictures and format names. It uses `wl-clipboard-rs`
`ClipboardType::Regular` only; there is no application PRIMARY read/write, and middle-click on
the canvas is autoscroll, not paste. S1 enables native `eframe/clipboard` to preserve the
0.36 integration-forced OS text clipboard and the `Event::Paste` paths used by canvas and
dialog TextEdits. This does not add PRIMARY to the app’s canvas or change rich-format
ownership. Do not route `Event::PasteImage` into a new app-level paste path. Verify:

- copy/cut/paste text and formatting through the app's commands, not only egui `TextEdit`;
- regular Wayland clipboard behavior with `wl-clipboard-rs` and `arboard` after E4/W15, while
  W15P/P2b PRIMARY remains a fork-level prerequisite and is not added to the app's own paste
  path;
- native `egui-winit/clipboard` is enabled and built-in TextEdit PRIMARY follows W15P;
- paste text from an independent process into the canvas and a dialog/search TextEdit,
  then copy back to the other process; an in-process fallback must fail this regression;
- no double clipboard owner or last-writer race on X11/Wayland when both egui and the app write;
- macOS/Windows native pasteboard formats (HTML, MathML, RTF) unchanged;
- browser paste picture path in `apps/wordcraft-web/src/web.rs:160-220` still receives events;
- a regression that middle-click on the canvas starts autoscroll and never pastes PRIMARY.

## 5. Lifecycle, root `Ui`, hidden windows and repeated passes

### 5.1 What eframe does

In 0.37, `eframe` calls `App::logic` and `App::ui` inside `Context::run_ui` for every pass on
native and web (`crates/eframe/src/native/epi_integration.rs:349-365` and
`crates/eframe/src/web/app_runner.rs` in the target source). `Context::run_ui` may run the
closure more than once when a pass calls `request_discard` (default `max_passes = 2`).
`new_input.take()` gives events/dropped files to the first pass only. A hidden window normally
goes through `update_logic_only` -> `Context::run_logic`, which updates only viewport/focus
state and leaves events/time/layout from the previous UI pass. However a pending
`ViewportCommand::Screenshot` or `RequestPaintWhileHidden` makes eframe run a full hidden pass
(`is_visible = viewport.info.visible().unwrap_or(true) ||
actions_requested.any(ActionRequested::wants_paint)`), so `ViewportInfo::visible()` is not a
phase indicator. After a normally completed `run_ui`, `current_pass_index()` is zero
(`end_pass` takes the output); do not use a stale nonzero index as evidence of hidden work.
This is the exact contract the WordCraft code must be audited against.

### 5.2 `logic` is currently mostly idempotent; fix the exceptions

`WordApp::logic` (`lib.rs:2087-2238`) drains channels (`drain_control`, `zotero::poll`,
`read_aloud::poll`, `poll_file_dialog`, `drain_inbox`, `drain_pasted`), ticks recovery/autosave,
issues/collects screenshots, and handles dropped files. Most drains are inherently once-only
because they `take`/`recv`. The audit must prove, not assume:

- **Dropped files are stale in hidden/forced logic passes.** `lib.rs:2208-2230` reads
  `ctx.input(|i| i.raw.dropped_files.clone())` without taking. A hidden `run_logic` frame or a
  forced hidden full pass can see the last shown frame's drops and open/insert them again.
  The browser shell already takes the vector in `apps/wordcraft-web/src/web.rs:119`; native
  `WordApp::logic` still clones it. Change `WordApp::logic` to
  `ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files))` and handle the drained vector.
  Apply the same rule to any future input-derived queue read in `logic`; never poll a
  prior pass's event twice.
- **SmartArt Tab** uses `ctx.input_mut(|i| i.consume_key(...))` (`lib.rs:2106-2118`); consuming
  makes a repeated logic pass harmless. Keep that pattern.
- **Synthetic input** (`raw_input_hook`, `lib.rs:2325`) runs before each frame, not once per UI
  pass, and can run on a hidden `update_logic_only` tick. Keep at most one unapplied synthetic
  gesture step pending until a full `WordApp::ui` pass consumes it; repeated hidden logic ticks
  must not inject the next step, and a press/release pair stays two ordered events, never
  collapsed. When queued synthetic input needs a UI pass while hidden, request paint through
  the same `RequestPaintWhileHidden` path as 4.4.
- **Screenshots** replace event collection with the request-owned completion slots in 4.4; a
  repeated pass must not issue the same request twice. `queued_shots` and the request records
  (with their slots/deadlines) remain the one-shot state.
- **Autosave** sets `last_autosave` before running `file.save` (`lib.rs:2312-2325`); a second
  logic pass in the same frame sees the new timestamp. Keep that ordering; add a test with a
  forced double pass that counts save calls.
- **Recovery/read-aloud/Zotero** mutate state flags (`recovery_seen`, `read_aloud_at`,
  `zotero` channel state). Add a forced double-pass test that asserts one recovery tick, one
  status transition and one Zotero answer per event.
- **`report_dirty`** is idempotent; do not move it out of `logic`/`ui`.
- **Undo transactions** are engine-owned. Do not open/close a transaction in `logic` on every
  pass; only commands do. A repeated pass must not add an undo step. The IME delete+commit path
  explicitly joins the typing group once.

### 5.3 Repeated `ui` passes

The app has one existing `request_discard` in `widgets.rs:985` (font list sizing). The
`WordApp::ui` path performs command side effects through widget clicks and `keys.rs`; events
exist only in the first pass, so the normal multi-pass layout rerun should not duplicate a click
or keystroke. This must be demonstrated:

- add a test-only wrapper that requests a discard once in a `WordApp::ui` pass (or drive the font
  list to a new row count) and assert one command execution, one undo step, one autosave and one
  screenshot request;
- keep `logic` in the same test to prove it does not replay stale input;
- if a duplicate is found, fix it with a narrowly scoped one-shot action queue or by deferring
  only the affected layout action. Do not introduce a generic "compatibility layer" or move all
  commands into `logic` wholesale.

### 5.4 Root `Ui` and panel ordering

`WordApp::ui` receives `&mut egui::Ui` and must continue to use it directly:

- `chrome::title_bar(self, ui)`, `ribbon::show(self, ui)`, `chrome::status_bar(self, ui)`,
  `panes::show(self, ui)`, then `CentralPanel::default().show(ui, |ui| canvas::show(self, ui))`
  (`lib.rs:2360-2410`).
- `backstage::show` uses `Panel::left` + `CentralPanel` on the same root; no recursive
  `root_ui`.
- Dialogs/windows use `egui::Window`/`Area` on the context; they are not viewports. Keep their
  `Id`/layer/order semantics.
- Test `Context::run_ui(RawInput, |ui| { app.logic(ui.ctx()); app.ui(ui) })` at 760x480 and
  1440x900, 1x/2x, both themes; assert the central canvas rect equals the root area after
  panel reservations, page rects are unchanged, and clicks at page coordinates still hit the
  same document positions.

### 5.5 Logic-only and forced hidden-pass checks

Use `Context::run_logic(&RawInput { viewports, focused, ..Default::default() }, |ctx| app.logic(ctx))`
(the 0.37 API) only for the background/timer contract; it is not a model of a screenshot-driven
hidden pass. Check:

- a pending autosave still fires on its timer without a UI pass;
- a pending print/Zotero/read-aloud result still drains;
- stale events from the previous shown frame are not consumed twice;
- a pending screenshot may force a full hidden `App::logic` + `App::ui` pass; assert exactly one
  request/reply and one command/undo/autosave, and test that no duplicate side effects occur.
  Do not call that path logic-only, and do not infer the phase from `visible()`;
- a queued capture issued only in `ui` (including settling frames) or pending synthetic input
  on a hidden window sends `RequestPaintWhileHidden`; test that it reaches issuance, that a
  plain wake may run logic only, and measure its repaint cost;
- after a completed multi-pass `run_ui`, assert `current_pass_index() == 0`; do not claim a
  stale nonzero index;
- fonts are not initialized in a logic-only test and then assumed laid out; `Context::run_logic`
  performs no pass or layout, so call `run_ui` for layout assertions. (#8535 documents only the
  recursive `root_ui` panic, not font initialization.)
- repeated hidden logic ticks do not advance the synthetic gesture queue; a full UI pass
  consumes exactly one pending step, and a queued capture/synthetic step on a hidden window
  uses `RequestPaintWhileHidden` to reach issuance.

## 6. Behavior contracts and regression assertions

### 6.1 Commands, undo and persistence

The engine and command registry are not migrated. Tests must still prove the UI dispatch path:

- `engine.execute` and keyboard shortcuts run the same command id for: text insert, bold,
  undo/redo, save/saveAs/open, find, print settings.
- One typing run or IME replacement is one undo step; undo restores text and formatting;
  redo reapplies.
- Save/reopen: create a synthetic document, type text, apply bold, insert a comment/bookmark,
  save as a temp `.docx`, open it in a fresh `Session`, assert text, formatting, comments and
  selection/page hints that the product expects. Do not assert egui widget ids survive reopen.
- Autosave writes the same file and does not clear undo; failure shows the existing message and
  disables autosave for that file.
- `file.close` with unsaved changes still refuses unless `force`/Save, through control and UI.

### 6.2 UI geometry and screenshots

Fixtures: `wordcraft_engine::sample::sample_document()`, a large synthetic table
(`tests::perf::big_table_doc`), and a durable evidence `.docx` with Latin/Arabic/Hebrew/CJK/
Vietnamese/emoji/comments/headers. Fixtures and fonts are licensed and pinned: use
`CRAFT_FONTS_DIR` at `b076417b5880bd4154bd22f00695b1d28c6bea83` for the app's CI text faces,
the central craft-fonts fixtures at `2481a8a324b986a45954e90045ed1f8f2efaaf3c` for
Arabic/Thai/CJK/Japanese coverage, the pinned Noto Sans Hebrew variable fixture
(`7ef36a2c...`, OFL-1.1) for Hebrew, and the test-only Noto Color Emoji v2.047
(`39ee3c58...`, OFL-1.1) for document emoji. Record every fixture file SHA256, source and
licence in the evidence manifest; do not redistribute proprietary system fonts. Run at 1x/2x,
light/dark, 1440x900 and 760x480.

Assertions:

- `ui.inspect`/`document.inspect` JSON: page count, page rects, canvas rect, caret screen point.
  Page/canvas geometry for the same fixture and DPI must be unchanged; chrome heights may change
  only after manual review of the 0.37 style deltas.
- `ui_shot` images: the page area (app-rendered textures) must match the 0.36 baseline exactly at
  the same DPI; chrome comparisons use a documented per-region threshold. Proposed starting
  policy: a candidate passes only if no page-area pixel differs and chrome changed-pixel fraction
  is <=0.5% with max channel delta <=2 after review; any larger change needs a named cause and a
  reviewed replacement image, never a bulk refresh. The exact threshold is frozen in the baseline
  report before candidate measurement, as the shared protocol requires.
- Ribbon fit: at widths 760, 920, 1200, 1440 and with English/German/Chinese/Japanese, the same
  groups compact/expand at the same widths, or a reviewed delta is recorded. `ribbon_fit` uses
  measured text widths (`ribbon_fit.rs`), so font changes can alter layout even when the code is
  unchanged.
- Keyboard focus after a chrome click still returns to the canvas (`refocus_after_chrome_click`)
  and a dialog/field keeps focus while open.
- Accessibility snapshots: `h.unnamed_widgets()` empty; button/input names; document role tree;
  Arabic text logical order in AccessKit; selected/disabled states for ribbon toggles.

### 6.3 Accessibility and keyboard

- Kittest `AccessKit` tree: Document node named after the file, paragraph/heading/list/table/
  image/link roles, selected text ranges, caret position. Assert text content is logical, not
  visually reversed.
- Every input widget has a name; `Role::TextInput` fields keep their current values and labels.
- Tab/Shift+Tab, arrows, Enter/Space, Escape and F10 context menu still trigger the same
  commands as before. A synthetic RawInput test can prove the E7/E19 translation contract for a
  non-Latin layout, but kittest carries no OS keyboard layout: real Russian/Arabic/Hebrew
  shortcut behavior and typing must be observed on native X11/Wayland/macOS. A green kittest
  run is not native layout evidence.
- RTL/CJK/Thai/Vietnamese/combining-mark text is not clipped or tofu; `MissingGlyphPolicy::Panic`
  is the default in kittest tests; deliberate tofu tests opt out explicitly.

### 6.4 IME, bidi and graphemes

- Preedit/commit/cancel: one edit and one undo step; candidate window rect follows the caret.
- `DeleteSurroundingBytes`: exact byte replacement on macOS/Wayland; char-count path remains for
  web; both refuse non-boundary ranges without panicking; deletion-only and empty-commit batches
  work. Inside an equation a byte range has no reliable tree mapping: the request is refused,
  the composition interrupted and only the replacement immediately paired with the rejected
  deletion dropped — the next `Commit`, or an adjacent `Text` + `Commit` pair. Any later
  text/commit in the same raw-input batch is fresh input and must apply, as is a later direct
  commit without a pre-edit (no persistent tombstone). An over-cap equation deletion likewise
  clears the painted pre-edit and any earlier deletion's replacement join before it interrupts.
  The backend's composition interrupt fences protocol-late stale input; an untagged synthetic
  event in a later frame cannot be distinguished from fresh input. These are synthetic contract
  tests; native macOS/Wayland IME traces are separate.
- E21b outgoing surrounding text: after delete+commit and deletion-only batches, after
  focus/password transitions and with two windows, the payload carries the logical committed
  text and byte cursor/anchor with preedit excluded. A kittest event test is not native IME
  evidence.
- Bidi: Arabic/Hebrew comment balloon reads from the right, a Latin word inside an Arabic
  paragraph keeps its left-to-right order, brackets mirror correctly, and no double reversal
  occurs in Backstage file names. Test screen x positions of first/last words, not just string
  equality.
- Grapheme editing: emoji ZWJ/modifiers, regional indicators, combining marks and Indic sequences
  use the engine's cluster rules; caret/hit-test/selection do not split UTF-8.

### 6.5 Clipboard and native behavior

- Native Linux X11 and Wayland: copy formatted text, paste into another app, read
  HTML/RTF/picture formats; no double data-device failure.
- Middle-click on the canvas starts autoscroll and never pastes PRIMARY. PRIMARY is tested only
  in built-in `TextEdit` where the app's feature graph actually enables egui clipboard; do not
  add app-level PRIMARY paste.
- Windows: paste HTML/RTF/MathML and pictures; save/open a document through a real file picker;
  close/quit behavior unchanged.
- macOS (if evidence available): pasteboard formats, IME, native termination; otherwise mark
  unresolved rather than claiming pass.
- Browser: WebGPU and WebGL2 fallback, paste pictures, download/save, localStorage prefs,
  keyboard shortcuts in an iframe, resize; actual browser run, not just wasm compile.

### 6.6 Deliberate mutation checks

Before accepting the suite, introduce and then remove each defect in a scratch branch; each must
fail the named assertion:

| Deliberate defect | Must fail |
|---|---|
| Reuse a screenshot token/path for two requests | screenshot correlation test, one reply per request |
| Deliver a late callback after timeout/cancel | no file, no second reply, retained bytes return to zero |
| Drop the ruler grip's accessible name | `unnamed_widgets()` fails |
| Change the ruler grip to `Role::Unknown` | explicit `get_by_role(Role::Splitter, ...)` fails |
| Remove `default_fonts` from the test build | glyph coverage test or kittest missing-glyph panic |
| Reintroduce `bidi_display` on fork text | RTL balloon/file-name screen-position test (double reversal) |
| Ignore `DeleteSurroundingBytes` | IME replacement/undo test |
| Make `ScrollArea` offset unclamped for page tracking | wheel/ribbon offset test |
| Force a duplicate logic pass | one-shot autosave/insert/screenshot counter test |
| Replay a dropped file from a prior pass in a forced hidden pass | stale-drop test counts one open/insert |
| Make middle-click paste PRIMARY | canvas autoscroll regression fails |
| Change page texture filtering | page-area pixel comparison |

## 7. Validation matrix and managed execution

No command below was run for this planning document. Every new host build/test and owned-VM
command must use `/home/codex/work/maintenance/task.py run`, then `status`, and `finish` after
all checks and evidence are complete, with a unique `--task`, a fresh `--job-id`, exact
`--source`, durable `--evidence`, and the allocator's target. Do not run Cargo directly outside
the allocated target; do not reuse another worker's target/job.

### 7.1 Host gates (Linux)

Proposed `task.py run` command bodies (the parent supplies task/job/evidence):

- `cargo metadata --locked --format-version 1`
- `cargo check --workspace --all-targets --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test -p wordcraft-ui-egui --locked`
- `cargo test -p wordcraft-engine tests::typing --locked`
- `cargo test -p wordcraft-ui-egui --release big_table_scroll_bench -- --ignored --nocapture`
- `cargo run --release -p wordcraft-ui-egui --example perf_probe -- <fixture.docx>`
- `cargo run --release -p wordcraft-ui-egui --example ui_shot -- <script.jsonl>`
- `cargo check -p wordcraft-web --target wasm32-unknown-unknown --locked`
- `cargo build --release -p wordcraft --locked`
- `cargo xtask ci` (fmt, clippy, tests, assets, layers, wasm)
- `cargo xtask parity`, `cargo xtask assets`, `cargo xtask layers`
- `cargo xtask deps` (proposed new gate) and its negative tests

`cargo xtask wasm` does not include `apps/wordcraft-web`; the separate
`printing.yml` browser check (`cargo check -p wordcraft-web --target wasm32-unknown-unknown
--locked`) is mandatory. A real browser run uses `packaging/web/package.sh` (trunk) or a local
`dist/web` server, with WebGPU and `?webgl` fallback, and the control/inbox workflows.

### 7.2 Native Linux

Run the release app with `--sample --control 7981` and a temporary settings dir
(`WORDCRAFT_NO_PREFS` for agent runs where appropriate). Use `ui.screenshot`, `ui.inspect`,
`ui.click`, `ui.key`, `ui.text`, `ui.wheel`, save/open. Observe X11 and a real Wayland
compositor (record compositor identity); use `WORDCRAFT_WAYLAND=1` where needed. Check regular
clipboard, middle-click autoscroll (no PRIMARY paste), file drop, native IME and
DeleteSurroundingBytes/outgoing surrounding text, real Russian/Arabic/Hebrew keyboard layouts,
mixed DPI, window resize, a forced hidden screenshot pass, screenshot timeout/cancel and page
geometry. `ui.key`/kittest injection is not a substitute for OS layout or IME evidence.

### 7.3 Windows

Use a uniquely named `craft-windows` clone, unused SSH/VNC ports, its own source checkout and
target. Build with `cargo build --release -p wordcraft --locked` (or the repo's documented
build/test commands), launch through `Launch-CraftDesktop.ps1`/`guest-exec.py`, and perform a
visible typed/edited/saved/reopened workflow plus print settings. Observe DX12/FXC startup and
the screenshot path. Do not claim macOS/Metal or physical audio from Windows software rendering.

### 7.4 Web

Build the wasm app, run it in a browser over `http://localhost` or HTTPS, exercise WebGPU and the
`?webgl` fallback, type/save/download/reopen a document, paste a picture, use the control
protocol if available, resize the iframe, and inspect console/panic output. A `cargo check`
alone is not browser acceptance.

### 7.5 macOS, FreeBSD, other platforms

- macOS: required for macOS IME/pasteboard/Metal/native termination claims. If no owned macOS
  evidence is available, mark those rows **unresolved/blocking**, not passed. The server's
  Linux host cannot substitute.
- FreeBSD: keep the existing nightly/FreeBSD workflow; run after migration and record the
  result. Do not infer from Linux.
- Physical GPU: the host has an i915 render device but no inherited display session; that is
  availability, not a qualified Vulkan/OpenGL/native compositor result. Record software-rendered
  and real-GPU results separately when both exist.

## 8. Performance protocol

Use the shared protocol in `craft-libs/docs/egui-037-suite-rollout.md` and make WordCraft-specific
measurements. Floors: at least 10 warmups and 30 timed steady-operation iterations, at least 20
fresh processes for startup, and at least 1,000 frames for any frame distribution with p99.
Licensed pinned fixtures apply; an app may tighten floors, never loosen them.

- **S0/P0 instrumentation, before any production edit:** extend `perf_probe`/the bench harness to
  emit raw per-frame samples with `n`, p50/p95/p99 and the full sample list. Raise typing, wheel
  and trackpad workloads to at least 1,000 frames each; do not use `big_table_scroll_bench`'s
  10-idle/40-scroll-frame median/max as tail evidence. Add a measurement-only native process
  harness (environment/flag gated, no behavior change) that records process start, first UI
  pass, each pass begin/end, winit redraw/AboutToWait timestamps where exposed, and screenshot
  readback; samples `/proc/<pid>/stat` and `VmHWM` for 30 s idle; counts repaints/wakeups via
  `Context::set_repaint_observer`; and writes a CSV/JSON trace. Run at least 20 fresh processes
  for cold start. Report harness `WordApp::logic + ui` CPU time separately from presented-frame
  latency/interval; never substitute one for the other. If a presented-frame timestamp cannot
  be observed without changing the renderer, record that gap and use the screenshot-readback
  timestamp only as a clearly labelled proxy. Preserve the 0.36.2 release
  executable and SHA256 with a saved fixture for rollback rehearsal.
- Every baseline report and every investigated regression includes an equivalent stock-0.37
  control alongside 0.36 and the fork. Fork-only app code need not compile unchanged on stock:
  use the same operation in a pinned minimal adapter/harness and record all three source/lock
  identities. Unavailable equivalence is an attribution limitation, not a reason to omit the
  control.
- Baseline and candidate: same host, compiler (stable/1.95+), `--release`, `--locked`, features,
  `CRAFT_FONTS_DIR` ref, fixtures, OS, CPU/GPU model, wgpu backend/adapter/driver and host load, 1440x900, 1x and 2x DPI,
  light/dark. Record full SHA and `Cargo.lock` hash for each. Use separate allocated targets and
  alternate run order; no concurrent builds or noisy workloads.
- Workloads: cold start to first frame (>=20 fresh processes), open sample/large fixture, idle
  30 s (CPU, repaints, wakeups), typing in a long paragraph and a table (>=1,000 frame samples
  per condition, repeating the input as needed), wheel and trackpad scroll (>=1,000 frames),
  zoom in/out, ribbon tab switches, resize sweep, screenshot latency (>=30 requests, report n),
  save/reopen latency, large-table scroll (`big_table_scroll_bench` with an extended sample
  count), peak RSS, texture upload/free counts and bytes, allocation counters where available,
  and font-atlas churn.
- Use the licensed pinned fixture/font set from section 6.2 for both baseline and candidate;
  record file hashes, licences, DPI, theme and renderer identity.
- Report raw samples plus median/p95/p99 for frame distributions, median/p95 for discrete
  operations with n, paired baseline/candidate ratios, and peak memory. Label harness CPU time
  separately from presented-frame latency. Do not label a p99 from 20-50 samples as a tail
  estimate; use at least 1,000 frames for frame distributions.
- Preliminary gates, frozen before candidate measurement: investigate a repeatable >5% median,
  >10% p95 or >5% peak-RSS regression, any new idle busy-loop/repaint, or a lost absolute
  budget. For sub-millisecond changes require the relative increase also to exceed the 0.05 ms
  noise floor or corroborating allocation/instruction counts. Reject noisy comparisons as
  inconclusive and rerun; never widen a threshold or tolerance after seeing a failure; any
  exception needs a documented, pre-measurement tradeoff and review.
- Specifically reproduce/measure:
  - upstream #8731: plain-text tessellation cost (UI chrome and dialogs); compare 0.36.2,
    stock 0.37 and fork. If confirmed, review a row-level fast path, not a semantic change.
  - upstream #8736: DX12 FXC pipeline creation and startup on Windows; separate renderer
    construction from first frame.
  - upstream #8512: system-font discovery startup if `system_fonts` is ever considered; this
    plan keeps it off.
- Performance evidence must never be compile duration, a changed golden image, or an
  unpaired run. Preserve baseline/candidate executables or hashes outside disposable targets.

## 9. Reviewable PR stages, dependency DAG and rollback

All stages are local until the user gives fresh direct approval for each publication. Each stage
must be independently reviewable and leave `main` buildable. No stage pushes/publishes by itself.

```
S0/P0 baseline harness + instrumentation (no production behavior)
  └─ S1 atomic dependency + required text/capture adapters + craft-libs CL-1/CL-2F (one PR)
       ├─ S2 adversarial screenshot/capture tests + visual/role review
       ├─ S3 extended native IME/bidi/grapheme tests and optional language policy
       ├─ S4 lifecycle/multi-pass/forced-hidden-pass/synthetic-input fixes + tests
       └─ S5 native/web/Windows validation + performance evidence (after S2-S4)
            └─ S6 documentation/progress updates + final adversarial review
```

- **S0/P0:** add the baseline capture scripts/fixtures, instrumentation and evidence layout;
  record 0.36.2 baseline hashes, executable SHA256, pre-existing failures and the saved rollback
  fixture. No dependency or production behavior change. Rehearse old-behavior replay with the
  saved 0.36.2 executable and fixture before S1.
- **S1:** one atomic dependency PR, after the approved published P8 and completed winit
  revisions exist: pin `=0.37.0`/`=0.30.13`, root patches, bump `egui_kittest`, and move
  `craft-ui`/`craft-print` to the published revision containing CL-1 and CL-2F **in this same PR** (the
  `d27646a` `craft-ui` in `crates/ui-egui/Cargo.toml:12` still requires egui 0.36 and cannot
  compile against the 0.37 `Ui` passed at `crates/ui-egui/src/printing.rs:35`). Include
  `Id::unique`, `scope_id`,
  `Role` (all 34 sites), `MouseWheel.source` (including `examples/perf_probe.rs:106`),
  `ScrollArea` accessors (including `scroll.rs:440`), `ScreenshotCallback`,
  native `eframe/clipboard`, `default_fonts`/glyph policy, accessible-name/role sweep
  and the print `label` adaptation.
  Include CL-2F capture admission/drain/timeout/basic replies, remove duplicate
  `bidi_display`/balloon visual reordering, handle E21 byte deletion and supply both
  canvas E21b producers. These are required at the first fork pin; do not defer them.
  The approved revisions contain P2b async W15P, P7 held-key pairing, P7b/E21b and P7c's
  selected #8763/#8748 fixes; stock 0.37 or an intermediate stage is never pinned. Include the
  minimal compile fixes, feature-identity checks and `cargo metadata` gate. This PR is not
  split into "bump then fix"; the tree builds and `cargo xtask ci` passes at the end. Never pin
  the aiongg research SHAs.
- **S2:** screenshot callback behavior: request-owned strong slots with weak callbacks, the
  shared 4-request/128 MiB/256 MiB admission, geometry/DPI/zoom epoch cancel/retry,
  `RequestPaintWhileHidden` for queued hidden requests, timeout/cancel/late-result tests, forced
  hidden pass, visual review and explicit role assertions. Adopt `craft_ui::capture` only after
  CL-2F freezes the API in S1; S2 expands stress tests. WordCraft needs CL-1 and CL-2F, while only the DesignCraft/GridCraft
  pilots use the reviewed CL-2 provisional revision. If CL-2F is not available, wait; do not
  publish a provisional adapter.
- **S3:** extend native grapheme/IME/bidi coverage and optional language policy on Context.
  Required removal of double bidi, E21 deletion and both E21b adapters already land in S1.
  Rollback removes only these later policy/test additions, never restores duplicate reorder.
- **S4:** lifecycle: stale dropped-file fix, forced multi-pass and forced hidden-pass tests,
  logic-only tests, synthetic one-step gesture retention, autosave/screenshot/undo one-shot
  tests. Can proceed in parallel with S3 after S1.
- **S5:** evidence: Linux X11/Wayland, Windows clone, browser, optional FreeBSD; performance
  paired report including the stock-0.37 control; screenshots/geometry/accessibility/mutation
  checks. No production edits except fixes found by the evidence, which go back into the owning
  stage.
- **S6:** update `ROADMAP.md` progress log, `docs/gaps.md`/`docs/roadmap.md`/parity docs only for
  behavior actually changed, and record the migration's status/revision history. No agent/
  model/tool signatures in public text; contributor credit remains username-only.

Rollback: restore the previous coherent set, not just one version string: root `Cargo.toml`
egui family declarations, root `[patch.crates-io]`, `Cargo.lock`, `craft-ui`/`craft-print` rev,
and the source adaptations in one revert. Keep the old lockfile and baseline branch reachable.
Preserve the actual 0.36.2 release executable and SHA256 outside the disposable build target,
plus a saved `.docx` fixture; before S1 and after each stage, restore the prior coherent
manifests/lockfile/source set, run that executable on the saved fixture, and verify the old
open/edit/save/reopen behavior. Restoring only an old lockfile is not a rollback rehearsal.
No app persistence format changes are planned; panel ids are in-memory, preferences are unchanged.
If required text/IME compatibility cannot be proven, block S1 or restore its full coherent
predecessor set. Never restore 0.36 manual visual reordering while retaining the fork.
S3 alone may be reverted only because it contains later policy/tests, not required adapters.

## 10. Open questions and blocking unknowns

1. **craft-libs PR 199 is open/conflicting at inspection.** Its head `b68de8c` and historical
   `ec8840a` tree must not be merged; port only the `8614e4a` API hunks and `1764bca` README
   changes onto current `4919088`, preserving intervening fixes. The reviewed and published
   CL-1 `craft-ui`/`craft-print` revision is a prerequisite for the WordCraft pin and must land
   in the same atomic dependency stage (section 9).
2. **Fork publication/reachability.** Final pins must be the completed P8 `storytold/egui`
   and completed `storytold/winit` revisions: approved, reachable and immutable. Their SHAs do
   not exist at planning time and must not be invented; the aiongg candidate commits and stock
   0.37 are research/measurement inputs only. No branch pins and no intermediate production
   stage in the release manifest.
3. **P7c selection/UI-regression fixes.** P7c owns the selected #8763 and #8748 fixes;
   #8764 is an open, unmerged proposal input. The approved P8 revision must contain the central
   fixes and their pointer/keyboard regressions; a WordCraft workaround is not an accepted
   substitute.
4. **E21b/P7b and E4/P2b+W15P prerequisites.** The approved revisions must satisfy the complete
   shared contracts in `egui` plan §6 E21b/E4 and winit plan §5.2/§5.3, including the W27
   lifecycle/serial rules, the W15P pointer-or-keyboard synchronous claim and bounded async
   transfer. WordCraft's two canvas producers need real model/preedit/UTF-8 adapters.
5. **Capacity and platform evidence.** Host build reservations, a Windows clone, a Wayland
   compositor, a browser with WebGPU/WebGL2, and macOS hardware are separate gates. Missing
   evidence remains an explicit blocker, not a pass.
6. **Upstream #8731/#8736 measurements** are not yet reproduced on WordCraft. They may require a
   reviewed fork patch or a documented tradeoff; do not accept a clamp-only texture change.
7. **Strict names/glyph sweep.** The exact list of unnamed inputs is unknown until the suite
   runs. Fixing names and glyph coverage is part of S1, required rather than disabling the
   check; most tests use `step`/`run_steps`, so explicit accessibility assertions are required.
8. **Bidi visual deltas.** Removing `bidi_display` should fix double reversal, but it changes
   exact text placement in Backstage and comment balloons. Review screenshots at 1x/2x and both
   themes; do not bulk-refresh goldens.
9. **IME byte ranges and native behavior.** macOS and Wayland text-input behavior cannot be
   qualified on this Linux host. The byte path must refuse non-boundaries and be tested in a
   native environment; a passing harness test is not native IME proof. The same applies to
   native keyboard layouts and the screenshot-driven hidden pass.
10. **S0 instrumentation gap.** Raw frame samples, >=1,000-frame tails, cold start, idle CPU/
    wakeups and RSS are not measurable with the current tools and must be added before
    production edits. Harness CPU time is not presented-frame latency.
11. **Font/emoji policy.** The actual family order is `Inter, Ubuntu-Light, egui-icons,
    SourceSans, ...` (with `monochrome_emoji_fonts` explicitly enabled on WordCraft's test
    root, not inherited from kittest), not the earlier proposed order. The UI has no color
    emoji; document emoji use a pinned test fixture or a recorded native platform result.
12. **Craft-libs #198** remains a proposal, not a dependency. Do not silently include its
    locale/formatting scope or font assets.
13. **`craft-print`/`craft-ui` #192** changes public print fields; confirm no other WordCraft
    caller constructs these structs without the new field.
14. **Panel ids and any persisted egui state.** The app does not enable egui persistence now;
    if that changes, old panel ids will not load. Record this as an explicit non-goal or add a
    migration key deliberately.

## 11. References

Immutable or primary sources used for the API and behavior statements above:

- egui 0.37.0 release commit:
  <https://github.com/emilk/egui/tree/841a328ba2dd8f6996beb7189ebb7fc00e668a2a>
- `ScreenshotCallback` and `Context::request_screenshot`:
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/screenshot_callback.rs>
  and
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/context.rs>
- `Event::MouseWheel` source and 0.37 input events:
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/input/event.rs>
- `Ui::scope_id` / `Ui::id`, `Id::unique` / `Id::new`:
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/ui.rs>
  and
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/id.rs>
- `Context::root_ui` panic semantics (upstream #8535):
  <https://github.com/emilk/egui/pull/8535>
- 0.37 release notes, including `default_fonts`, `Role`, screenshot and panel-id changes:
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/CHANGELOG.md>
- `egui_kittest` 0.37 accessibility/missing-glyph changes:
  <https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui_kittest/CHANGELOG.md>
- Candidate aiongg egui research input (not a production pin):
  `a4f9a17f64a6bae61350fb85804c9df0d9865916`, `ai-0.37`; the inspected checkout's origin is
  `emilk/egui`, and no `storytold/egui` `craft/0.37` ref contained it at inspection.
- Candidate aiongg winit research input (not a production pin):
  `8b35388bd719e8cc86a95971ba0bc026c9a8f058`, `ai-lang` in `storytold/winit`.
- Final patch inventories will be linked from the approved published `storytold` revisions
  after publication; do not use the candidate SHAs as immutable production links.
- craft-libs PR 199: <https://github.com/storytold/craft-libs/pull/199>
- craft-libs language-support issue 198: <https://github.com/storytold/craft-libs/issues/198>
- craft-libs current main used for the port:
  `49190884278f16c9a9f5823d7698a8ffb5deb062`.
- Shared suite rollout/performance contract:
  `craft-libs/docs/egui-037-suite-rollout.md` in the assigned suite checkout.
- Complete shared E21b/P2b and winit W15P/W27 contracts: `egui/docs/egui-037-migration-plan.md`
  §6 (E4/P2b, E21b) and `winit/docs/egui-037-migration-plan.md` §5.2-5.3 in the planning
  workspace.
- Pinned font fixtures (craft-fonts `2481a8a...`, Noto Color Emoji v2.047):
  <https://github.com/storytold/craft-fonts/blob/2481a8a324b986a45954e90045ed1f8f2efaaf3c/fonts/manifest.txt>
  and <https://github.com/googlefonts/noto-emoji/blob/22e564626297b4df0a40570ad81d6c05cc7c38bd/fonts/NotoColorEmoji.ttf>.
- Upstream reports used as risk hypotheses, not reproduced WordCraft results:
  <https://github.com/emilk/egui/issues/8731>,
  <https://github.com/emilk/egui/issues/8736>,
  <https://github.com/emilk/egui/issues/8763>,
  <https://github.com/emilk/egui/pull/8764>,
  <https://github.com/emilk/egui/pull/8748>,
  <https://github.com/emilk/egui/pull/8512>,
  <https://github.com/emilk/egui/pull/8657>,
  <https://github.com/emilk/egui/pull/8577>.
- Historical compiler diagnostics used only as clues:
  `/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/fork/wordcraft.errors`.

## 12. Revision history

- 2026-10-23: initial plan. Inventory and source anchors from the assigned checkout; dependency
  target, API refactors, lifecycle/root-`Ui` audit, behavior/evidence matrix, performance
  protocol, staged PR DAG, rollback and blocking unknowns. No tests/builds run; no publication.
- 2026-10-23: revision 2 after independent review `plan-reviews/wordcraft-v1.md`. Corrected the
  aiongg/reachability identity and PR 199 stale-tree rule; added E21b/P7b and W15P/E4/P2b as
  fork prerequisites; replaced the screenshot channel with request-owned weak slots, count/byte
  admission, in-flight budget, resize revalidation and post-unlock wakeup; documented the
  screenshot-forced hidden pass and that requesting a screenshot schedules a repaint while
  callback arrival does not; moved strict names/glyph work into S1; corrected font order and
  emoji source; added `scroll.rs:440`; added exact Id counts, feature checks, P0 instrumentation,
  native-vs-kittest boundaries, explicit role assertions, middle-click autoscroll preservation,
  #8763/#8764 selection fix and executable rollback rehearsal. No tests/builds run.
- 2026-10-23: revision 3 after independent review `plan-reviews/wordcraft-v2.md`. Made S1 one
  atomic dependency stage including craft-libs CL-1, replacing the S1/S2 cycle; made P8 and the
  completed winit the only production pins; corrected the E21b/P2b/W15P contracts to the full
  shared plans; adopted the 4/128 MiB/256 MiB capture limits and geometry-epoch rule; added
  `RequestPaintWhileHidden` for queued hidden captures and the synthetic one-step gesture rule;
  fixed `cargo metadata --filter-platform`, the 34th role site and the `perf_probe` wheel site;
  added stock-0.37 controls, warmup/iteration floors and the RSS/noise thresholds; corrected
  P7c/#8764 provenance, font prose and the #8535 citation. No tests/builds run.
- 2026-10-24: revision 4 after the IME repair review. Bounded the refused byte-counted equation
  replacement to its raw-input batch (a later direct commit applies; no persistent tombstone),
  and moved the simulated IME revision exhaustion behind the engine `test-support` feature that
  only the UI's dev-dependencies enable. No tests/builds run.


## Local implementation update — 2026-10-23

The user clarified that reviewed plans remain local and implementation proceeds before publication. The earlier requirement to publish/land this plan first is superseded; pushes and PR publication still require explicit approval.

Canonical `storytold/wordcraft` `main` now resolves to `00ccee52354d9dfc48cb21a943b55f258fc76df0`. The owned checkout was fast-forwarded to this revision. Earlier baseline SHAs and line anchors above are retained as historical review evidence; use this new revision and its exact lockfile for current-main integration and paired measurements. The original refs remain reachable.

Preserve the separate 64-entry engine UI-intent queue, newest Paste Special deduplication, save/export/print view preservation, linear-time unknown settings, finite font preview sizes and Format Painter cursor. Screenshot admission budgets are a separate queue.

The complete static delta review and exact old/new object identities are retained in the task evidence: `canonical-diffs-refresh-20261023.json` and `overnight-consumer-review-20261023.md`. This amendment claims no new build, native GUI or performance acceptance.

## Local implementation note — 2026-10-23

Implemented in the owned worktree at canonical `00ccee52` without changing the workspace lockfile,
committing, running Cargo, or publishing:

- Coherent 0.37 API port: `egui::Role` (including `Role::TextInput`, `Role::Image` and
  `Role::Splitter` for the table-ruler grip), `Id::unique`, `Ui::scope_id`,
  `MouseWheelSource`, `ScrollArea::clamped_offset`/`horizontal_scroll_offset`, and
  `TextEdit::align`.
- Screenshot capture now uses `craft_ui::capture::CaptureSlot` with a weak callback. `WordApp`
  owns request identity, an 8 s monotonic deadline (`web_time::Instant` on native and in the
  browser), count (4), per-image (128 MiB) and aggregate (256 MiB) admission, retry accounting,
  an immutable admitted document identity and a monotonic geometry epoch. Geometry is captured
  from the host `RawInput` in the raw-input hook, including `minimized`/`occluded` and a Wayland
  `inner_rect: None` fallback to the same frame's `screen_rect`; it is never read from stale
  `InputState` geometry in a hidden logic tick. A minimized/occluded window and a document
  switch cancel the request with an explicit error instead of retrying or rebinding it. The
  deadline is checked before any successful image is encoded or written; stale geometry retries
  only within the budget, while malformed, oversized and out-of-order completions do not.
- The engine's 64-entry UI-intent queue remains separate from the four screenshot requests.
  Save/export/print view preservation, Paste Special deduplication, settings loading, font
  preview bounds and the Format Painter cursor are unchanged.
- `logic` owns screenshot deadlines/results and asks for `RequestPaintWhileHidden` while a
  capture or synthetic step is pending. `ui` issues the one callback command and counts only
  first-pass layout settling. Native and web dropped-file handling moved to the first real UI
  pass so hidden `run_logic` cannot replay stale RawInput.
- Synthetic input is offered once per host frame, survives hidden ticks, and is removed only
  after a final non-discarded UI pass. Multi-pass and real-RawInput accumulation regressions are
  in `crates/ui-egui/src/lib.rs`.
- Removed manual visual bidi reordering from Backstage and comment balloons. Comment balloons
  now use `LayoutJob` with `TextDirection::Auto` and start alignment; the screen-position RTL
  regression remains the oracle.
- Added `ImeEvent::DeleteSurroundingBytes` handling with UTF-8 boundary checks, selection
  replacement, one typing undo group and deletion/empty-commit/emoji/combining-mark tests.
- Declared `default_fonts` + `monochrome_emoji_fonts` on the UI test root and `clipboard` on
  the native eframe feature list. The exact test font order is pinned in `theme.rs`.

### Fresh-review repairs (2026-10-23)

The first 0.37 port was reviewed before compilation. These repairs are in the worktree:

- Deadlines now use `web_time::Instant` on both native and browser, not `Date.now()` with a
  backward clamp. Screenshot deadline, settling, autosave and animation share the same
  monotonic clock.
- Capture geometry is taken from the host `RawInput` in `raw_input_hook`, never from stale
  `InputState` geometry. The snapshot includes `minimized`, `occluded` and the Wayland
  `inner_rect: None` fallback to that frame's `screen_rect`. A minimized/occluded surface is
  rejected at admission and cancelled at issue/completion.
- A screenshot request stores an immutable admitted document id. Issuing or draining after a
  document switch cancels the request with one explicit error; it is never rebound and retries
  only for a same-document geometry change. Malformed/oversized completions are not retried.
- Admission reserves the encode working set (raw RGBA + worst-case PNG + base64), revalidates
  it at issuance and checks the actual PNG/base64 size before replying. Completion requires the
  exact expected physical dimensions; every reply carries the internal request token.
- Dropped files and the SmartArt Tab action run before the font warm-up return, so a drop in the
  first two UI passes is not lost. Synthetic input waits for those frames and for a final
  non-discarded UI pass.
- Chart-data cells use `Align2::RIGHT_TOP` (the deprecated `horizontal_align(Align::Max)`
  changed x only); the regression checks the painted text's top/right position. The rename
  field is tied to its label and the chrome/rename screens are checked for unnamed inputs.
- `unicode-bidi` is no longer a `ui-egui` dependency. Workspace manifests now name the 0.37
  egui family and `web-time`; `Cargo.lock` and the `craft-ui`/`craft-print` revision containing
  `capture::CaptureSlot` and `Printer::label` remain parent integration gates.

### E21b outgoing surrounding text (wired in this worktree)

The reviewed E21b outgoing API is wired through the canvas observation path:

1. `keys::observe_ime` emits one `IMEOutput` per focused viewport pass with an immutable
   `ImeTarget` owner, context generation, purpose, canvas rectangle, cursor rectangle,
   `should_interrupt_composition`, `change_cause` and `surrounding_text`.
2. The body producer uses `Document::ime_surrounding` and `VisibleParagraph`: tracked deletions
   are hidden, inline objects contribute their plain text, offsets are logical UTF-8 bytes (never
   display-reordered bidi), and the window is bounded by `MAX_IME_TEXT_BYTES`. It is reported as
   `CommittedText`, with an unavailable revision when the selection spans blocks or cannot map.
3. The body owner is `(document id, story, body replacement epoch)`; an equation uses its stable
   editing-session token. Undo/Redo and whole-body replacements advance the epoch/session so a
   stale composition cannot be reused.
4. An equation or an unmappable/over-wide body selection is an explicit `Unavailable`, carrying
   the engine revision that proves a committed edit; no truncated text is fabricated.
5. `publish_ime_receipt` accepts a successful IME edit only when the classified outcome and the
   committed/unavailable stamps prove the exact transition; mixed availability requires the
   unavailable side's checked revision, and exhausted revisions fail closed.

Still outstanding for publication: native Wayland/focus-loss traces, an independent review of
this repair, final immutable fork/winit pins, and the parent's post-repair full gate rerun.
Kittest events are not native IME evidence.

PR 787 is a historical patch reference only. This repository has no vendored-winit tree and
none was added; only the fetched PR metadata was inspected, and no vendored winit files or
security fixes were copied or overridden.

All build, test, lockfile, formatting, native GUI and performance gates remain for the parent’s
managed `task.py` workflow.

### Local behavioral qualification, 2026-10-23

The pinned craft-fonts corpus enables the strict UI suite without disabling missing-glyph checks. An intermediate run had 816 passed, 36 failed and 2 ignored; the later parent-managed run recorded 941 engine tests passing and 898 UI tests passing, with one doc IME fixture failure and three content-control placeholder undo failures whose source repairs are in the working tree. Those runs are pre-repair evidence only: the post-repair rerun remains required, and this document is not acceptance evidence.

Read-only comment balloons now use `craft_ui::text::align_galley_rows` to place native-shaped rows against the card's paragraph start edge. This preserves the fork's logical text and mirrored glyphs. The existing Arabic test was updated to inspect atlas bitmap orientation because `Glyph::chr` now remains logical rather than describing a manually reordered display string. It also verifies an inherited-neutral paragraph and an LTR paragraph in the same balloon. The targeted RTL test passes.

Pointer-driven caret changes explicitly interrupt IME after 0.37 stopped treating an already-focused `request_focus` as an interruption. The canvas regression passes while Space/Enter remain non-interrupting. The Ctrl-wheel anchor regression exposed a ScrollArea stale-bounds/previous-momentum defect; its fork repair has independent pre-fix failing controls and is under qualification. Native GUI traces for the wired E21b outgoing adapters, immutable publication pins and paired application performance remain outstanding.


## Latest local qualification — 2026-10-24

The candidate starts at `14001aee651090932fe6a6e23898f38b819f8c61` and includes the three-file
content overlay from observed main `c524027f34be2d1322b23064f80662d3638cf880` (root manifest,
Cargo.lock and shared-PDF documentation). The canonical lock SHA-256 is
`7f8878d742ad7ceab8d38dfcf878808bfd82c449088c7563c740e20372840865`. Independent reviews cover
the migration, stale-composition ownership, grapheme-safe edit/wrap boundaries, deterministic
pagination fixtures, normal production font features and current-main PDF overlay.

The final full CI run passed fmt, strict clippy, tests, i18n-report, doctests, assets, layers and
WASM (16 crates). Its main test stage reports 4,106 passed, zero failed, four ignored and one
filtered test across 27 result summaries. Engine: 941 passed; layout: 503 passed; UI: 904 passed,
two ignored and one filtered. The separate i18n-report stage runs that filtered report test:
one passed, 4,110 filtered. Those filtered counts are selections over the same test binaries,
not additional unique tests. The doctest stage selected zero doctests. Earlier narrower runs
are not added to these totals. Full logs, source hashes and review bindings are retained in the
local task evidence under `final-landing/wordcraft-local-commit`.

A separate normal-dependency production font probe passed for the visible table-style `⊞`
marker without external/system fonts or dev-dependency font unification. That probe preceded
the PDF-only overlay; font/runtime inputs remain unchanged. It establishes the bounded glyph
claim, not a full typography or platform result. Native input/GUI/capture/save/reopen traces,
paired app performance and memory/noise qualification, platform and installer execution and
explicit approval before each push/tag/new PR remain outstanding. No publication is claimed.

Revision 5: replace stale pending local checks with the actual reviewed-source full-CI pass,
current immutable pins and bounded remaining acceptance gates; preserve earlier evidence/history.


### Bundled-font notice completion — pending package verification

Canonical egui0.37 default_fonts embeds `EGUI_ICONS` (the small emoji-icon-font subset), and
monochrome_emoji_fonts embeds its parent emoji-icon-font face. WordCraft starts from
`FontDefinitions::default()` in `theme.rs`. NOTICE now carries the exact upstream MIT text,
including Copyright (c) 2014 John Slegers, and ATTRIBUTION identifies both dependency assets.
The PDF notice names the actual pinned 17744430 revision. Shared Linux/web/CLI staging, macOS app
resources, Windows portable ZIP and unconditional MSI notice components now include NOTICE
and ATTRIBUTION independently of optional craft-fonts. The existing optional OFL rules remain.
This documentation/packaging follow-up changes no runtime/font/manifests/lock bytes; its asset,
staging and fresh review gates are pending. Final packages and platform execution remain
unqualified. This scoped repair is separate from the six new craft-fonts faces and does not
claim a complete historical bundled-font audit. Exact upstream license bytes are preserved in
the local commit evidence.
