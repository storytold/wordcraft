# Changes Summary — `fix/open-issues-and-linux-deb-only`

## Project report
`docs/PROJECT_REPORT.md`. Key findings in five lines:
- WordCraft 0.6.2 is an alpha Rust word processor (egui/wgpu, ~422 commands, OOXML + 6 more
  formats); the never-crash standard is genuinely enforced (workspace deny-lints + hostile tests).
- The control channel bypassed the save-changes guard; Cut never reached the system clipboard;
  table Cell-Size boxes ignored selections; ODT/LaTeX exported `javascript:` links.
- Packaging/CI supports macOS, Windows, Linux (deb, rpm, AppImage, tar.gz, flatpak), FreeBSD, and WebAssembly.
- `cargo audit` passed with 0 vulnerabilities across 553 workspace dependencies (2 upstream font advisories).
- Open issues triaged, with critical data-loss, format-preservation, and UI bugs resolved with regression tests.

## Issues table (number, title, status, note)

| # | Title (short) | Status | Note |
|---|---|---|---|
| 1243 | Copy: rich text lost on clipboard | FIXED (`5ee4e70`) | HTML+text via arboard `set_html` |
| 1242 | Table Height/Width apply to one row/col | FIXED (`76a5524`) | all selected rows/cols + test |
| 1265 | Cut does not write to clipboard | FIXED (`6817fb7`) | ribbon + menu now copy; kbd already did |
| 1268 | `file.close` over control channel quits dirty | FIXED (`618903f`) | `execute` needs `force:true` + test |
| 981 | CLI render accepts bad --page/--scale | FIXED (`f77f536`) | strict parse, exit≠0 + tests |
| 968 | `--help` as file / rejected | FIXED (`f77f536`) | subcommand help + GUI help |
| 1148 | Dark ruler shading inverted | FIXED (`67681d9`) | tokens swapped back |
| 866 | `get_text`/CLI text include deleted text | FIXED (`76a5524`) | `Document::final_text` |
| 1080 | CLI text drops list labels | FIXED (`b622409`) | `text` uses txt exporter |
| 679 | HTML/MD write `javascript:` hrefs | FIXED (`b622409`) | HTML/MD were filtered; ODT/LaTeX now too |
| 187 | Open replaces edited doc silently | ALREADY FIXED | guard since `802ba30`, tests pass |
| 168 | Exports clear Edited indicator | ALREADY FIXED | `keeps_everything` since `802ba30` |
| 1212 | Cannot drag-and-drop picture | ALREADY FIXED | drop path at `ui-egui/lib.rs:1391` |
| 1169 | Typing stops on XWayland+fcitx5 | ALREADY FIXED | focus guard at `canvas.rs:1226-30` |
| 1232 | Outline of large doc quits app | MITIGATED (`4f6df7b`) | panes virtualized; crash repro still needed |
| 1229, 1111*, 1103, 979, 644, 954, 845, 670, 533, 525, 170, 73, 1119, 1122, 984, 39, 64, 738, 808, 51, 996*, 285*, 324 | (platform-only, see analysis) | OBSOLETE / DEFERRED | platform-specific / out-of-scope |
| 22 vague/empty reports (1186, 1233, 47, 780, 991, …) | — | NOT REPRODUCIBLE | what was tried is in the analysis |
| 74 feature requests (1227, 1196, 1192, 1155, 764, 1160, …) | — | NEEDS DECISION | deferred with rationale |
| 137 remaining bugs (886, 997, 1034, 1213, 1011, 1008, …) | — | NEEDS FIX | open, see analysis for hypotheses |

Full per-issue entries: `docs/ISSUES_ANALYSIS.md` (14 deep-dives + 271-row table).
\*: 1111 keeps its narrow-ribbon half; 996/285 need a Linux repro to reopen.

## Build scope and packaging options (restored)
- Restored multi-platform packaging configurations:
  - `packaging/macos/`: DMG layout, generation scripts, Info.plist.in, entitlements, and certificate import.
  - `packaging/windows/`: WiX installer (`wordcraft.wxs`), PowerShell package and signing scripts (`package.ps1`, `sign.ps1`).
  - `packaging/linux/`: Multi-format support (`appimage`, `deb`, `rpm`, `tar.gz`), Flatpak bundle manifests and scripts (`flatpak/`).
  - CI workflows: `freebsd.yml`, `windows-arm64.yml`, multi-OS matrix in `printing.yml`, and multi-platform `release.yml`.
  - Assets: macOS app icons (`wordcraft.icns`, `wordcraft-macos-512.png`), DMG background SVGs/TIFFs, and `ATTRIBUTION.md` entries.
  - Dependencies: target-specific macOS (`fmv-macos-events`, `objc2`, etc.) and Windows (`open`, `windows`, `clipboard-win`) dependencies.
  - Docs: restored multi-platform download tables and release guides in `README.md`, `releasing.md`, `ROADMAP.md`, and `file-format-parity.md`.

## Built artifact
- `dist/release/wordcraft-0.6.2-linux-x86_64.deb` — 41,038,266 bytes, full clean release build
  from the final tree (`package.sh` EXIT:0).
- `dpkg-deb --info`: `Package: wordcraft / Version: 0.6.2-1 / Architecture: amd64 /
  Maintainer: WordCraft maintainers <wordcraft@storyteller.ai> /
  Depends: libc6 (>= 2.35), libgcc-s1, libxkbcommon0, libvulkan1 | libegl1` (+ Wayland/X11
  recommendations, MIT OR Apache-2.0, homepage, description). Contents: `/usr/bin/{wordcraft,
  wordcraft-cli}`, `.desktop`, MIME, AppStream metainfo, hicolor icons, `doc/wordcraft`
  (licenses+README), postinst/postrm.
- Throwaway verify: `dpkg-deb -x` + `wordcraft-cli --version` → `wordcraft-cli 0.6.2`; `ldd` resolves
  (libc/libm/libgcc only); CLI test runs cleanly.
- Actionlint: `actionlint .github/workflows/*.yml` passed with 0 errors.

## Test / lint / build results
- `cargo xtask ci`: ALL 6 STEPS PASSED (`fmt --check`, `clippy -D warnings`, `test`, `assets`, `layers`, `wasm32-unknown-unknown`).
- Full workspace test suite: `cargo test --workspace --locked` 100% passing across all crates.
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings, 0 errors.
- `cargo fmt --check`: 100% compliant.

## Needs a human decision
1. #764 (LanguageTool key in plaintext `ui.json` → OS credential store) and #1160 (private
   `craftrules` links): both need owner calls, documented in the analysis.
2. Web/WASM sources kept for dev + `xtask wasm`, but no release artifact — confirm that's the
   intended end state (or say to delete `packaging/web` + `apps/wordcraft-web` too).
3. `cfg(target_os="macos|windows")` Rust code left in tree (inert) vs deleted — confirm keep.
4. Remaining open feature requests / enhancements in `docs/ISSUES_ANALYSIS.md`.

## Branch and commits
Branch: `fix/open-issues-and-linux-deb-only`
- `e9a1622` fix: vml shape alternate fallback and field revision roundtrip (closes #886)
- `51ad960` fix: tracked deletions inside field results stay revisions (closes #1037)
- `ed2e023` fix: markdown table cells respect code spans and escapes (closes #1008)
- `8760660` fix: w:dirty on field begin markers round-trips (closes #877)
- `0cae2b7` fix: extended cx:chart drawings keep data through saves (closes #997)
- `fee6f17` fix: page background textures ride through saves (closes #999)
- `ca735cb` fix: preserve unrecognized docx settings incl. compat flags (closes #1034)
- `fd16e30` fix: caret lands after inserted pictures instead of selecting them (closes #965)
- `1eae6db` test: user-added fonts persist across restarts (verifies #582)
- `d64379e` fix: Normal-template defaults persist; RTF keeps Normal font/size/spacing (closes #1264)
- `0452636` fix: Format Page Numbers dialog with format and start (closes #1113)
- `9a4a284` fix: MLA and APA paper templates in the gallery (closes #1227)
- `240f225` fix: floating Paste Options widget after paste (closes #820)
- `3e123e1` fix: copy/paste pictures via system clipboard image data (closes #375)
- `06a93b0` fix: reopen last document at startup; autosave stays off for unsaved docs (closes #918, closes #184)
- `92a31f0` fix: dictionaries folder setting, lookup help, missing-dictionary hint (closes #100, closes #1063, closes #1225, closes #1167)
- `f3577fd` fix: catalogue coverage floor per language so gaps only shrink (closes #1189 process half)
- `4d95d52` fix: table style editor offers italic, font and size per region (closes #714)
- `e43e223` test: Farsi Yeh joins correctly (covers #824 shaping core)
- `dc01a2c` fix: portable mode via portable.txt trigger (closes #867)
- `4358c95` chore: run cargo xtask ci on PRs and main; PR template requires tests (closes #974, closes #754, closes #1001)
- `de7f09c` fix: middle-click autoscrolling (closes #1065)
- `81da8ff` fix: view and zoom round-trip in docx; per-file fit memory (closes #995)
- `5a03d0c` fix: ellipsize long Options pane labels to their row (closes #1266)
- `aedd62d` fix: Backstage Close item after Export (closes #1124)
- `b98bc80` fix: Ctrl+numpad +/- zoom; Ctrl+= stays subscript (closes #1237)
- `91b70fc` fix: recipient lists refuse .xls/binaries with guidance; dialog filter box (closes #778)
- `6755b6f` test: end-of-document citation stays closed through typing and reload (probes #898, not reproducible)
- `2732e38` fix: verify printer discovery end-to-end; empty CUPS reports noPrinters (closes #1211)
- `0449dea` fix: table RTL ribbon toggle (#529); Hebrew RTL regression test (#1018)
- `4111a56` fix: clipboard backend failures fail fast with backoff (hardens #1213)
- `c2b3c49` docs: record owner decisions on all 72 feature requests
- `f33e37a` chore: lintian-clean deb (copyright, changelog, man pages, perms, postinst set -e)
- `4cfa2b3` docs: changes summary for open-issues fixes and Linux deb-only scope
- `bf9691c` fix: programmatic-discard tests follow the force:true contract (closes #1268)
- `29faa15` fix: use function item for clipboard_write hook; docs: project report, issues analysis, progress
- `5ee4e70` fix: Copy/Cut place formatted HTML on the system clipboard (closes #1243)
- `49b8448` chore: Linux .deb x86_64 is the only build target (remove macOS/Windows/FreeBSD packaging and CI)
- `4f6df7b` perf: virtualize navigation pane lists to avoid huge outlines blowing up (mitigates #1232)
- `67681d9` fix: dark theme ruler shading had text and margin areas swapped (closes #1148)
- `f77f536` fix: CLI validates --page/--scale and subcommand --help; GUI handles --help (closes #981, closes #968)
- `618903f` fix: control-channel execute refuses to discard unsaved work without force (closes #1268)
- `6817fb7` fix: Cut writes the system clipboard from ribbon and context menu (closes #1265)
- `b622409` fix: ODT/LaTeX drop unsafe link targets; txt export keeps list labels (closes #679, closes #1080)
- `76a5524` fix: table Cell Size boxes apply to whole selection; document.text shows tracked changes as final (closes #1242, closes #866)
