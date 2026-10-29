# KDE Plasma / Linux

Updated 2026-10-29. The implementation supports two distinct window bindings:

- Native Wayland: KDE `org_kde_kwin_appmenu_manager` associates the D-Bus menu with
  the existing surface. The bundled winit patch exposes a safe cloned proxy;
  winit retains the socket, surface ownership and event dispatch.
- X11/XWayland: KDE window properties plus `com.canonical.AppMenu.Registrar`.

Both use the same `vendor/craft-desktop` transport. `ui-egui::native_menu` derives
commands from the registry and interactive UI entry points; `WordApp::run` remains
the handler. Clicks are revalidated against the current document, modal state and
availability. Lazy submenu state updates use property signals and per-parent layout
signals. The ribbon stays visible and retains upstream's narrow-window overflow menu.

The exporter currently binds only the original native window. Additional document windows,
including another view of the same document, retain their local controls. While a child is
active, the root menu is cleared and its queued actions are rejected. Per-window Global Menu
binding remains pending; this migration does not claim full multiwindow Plasma acceptance.

## Build and launch

```sh
cargo build -p wordcraft
WORDCRAFT_WAYLAND=1 cargo run -p wordcraft -- --sample
```

The existing default display policy still prefers XWayland when available. The environment
override above selects native Wayland, which avoids XWayland fractional-scale resampling.
The current pinned winit fork includes native Wayland file-drop support; that path has not
been verified live in this reconciliation. File dialogs remain available on both backends.

Add Plasma's **Global Menu** widget to the panel if needed. No GTK module or Qt
conversion is required. Absence of that widget, registrar, session bus or compositor
protocol must not remove local commands. `WORDCRAFT_NO_GLOBAL_MENU=1` or
`KDE_NO_GLOBAL_MENU=1` disables the exporter. On other operating systems the adapter
is not compiled. No compositor configuration is edited by the application.

The root Cargo manifest's winit patch and both vendor directories are required in
source distributions. See `vendor/winit/CRAFT_PATCH.md` for the one-method patch,
upstream checksum and licence. Upstream winit contains its own platform unsafe code;
Craft's new accessor and integration crate contain no unsafe code.

## Appearance and scaling

System mode now reads the standardized portal color-scheme (0/1/2) and accent-color,
updates while idle (two-second polling, bounded calls on a worker), recovers after
portal restart, and stops the worker when its handle is dropped. Explicit Light/Dark
retain WordCraft's palettes. Portal accent adaptation protects text contrast; this
is not a full Qt/Breeze widget skin. Linux window decorations remain compositor-owned.

Native DPI remains separate from Interface Size. For reproducible offscreen checks,
the `ui_shot` script's first JSON line accepts `width`, `height`,
`pixels_per_point` (1, 1.25, 1.5, 1.75, 2) and `integrated_titlebar:false` (Linux SSD).
These are app-test settings, not changes to the desktop scale.

## Reproducible validation

Follow-up validation (2026-10-30): incorporated upstream
`165d5725dc25b4a7887d8025c3b1fe506ccc3dcc` (recipient label sheets), preserving both
upstream and KDE progress entries. Exact `cargo xtask ci` again passed all eight stages,
with 4,611 passing tests and four ignored cases across 72 test summaries; required pinned
fonts/translations and the matching WASM sysroot were used. Log:
`/tmp/wordcraft-kde-ci-165d5725.log`. This follow-up changes mailings engine code/tests and
progress documents, not the UI, native host or toolkit. The earlier native smoke remains
evidence for those unchanged integration sources; it is not a new GUI run of the rebuilt binary.

Current reconciliation (2026-10-29): exact `cargo xtask ci` passed all eight stages
(formatting, workspace Clippy, tests, i18n report, doctests, assets, layers and WASM),
using required pinned fonts and translations. The log contains 4,609 passing tests
and four ignored cases across its test summaries. The focused native-menu, root-window,
accent and i18n checks also passed; these overlap the full suite.

The first full run exposed a host-font-dependent upstream pagination fixture. It now
uses the existing bundled Source Sans 3 fixture helper and verifies the exact font bytes
and 12 pt size; all original geometry assertions remain. All 559 layout tests passed on
the full rerun. This is test portability work, not a change to document layout behavior.

A fresh isolated native Wayland smoke toggled Ruler from true to false through the exported
DBusMenu action, confirmed `view.ruler` through the control API and checked the exported
checkmark. The window screenshot was inspected; the real `plasmashell` process requested
`GetLayout`. Evidence: workspace artifact `native-smoke-wordcraft-r4k_n68m` and
`/tmp/wordcraft-kde-full-ci-rerun.log`. This validates one root-window action and transport,
not child-window export or the full clipboard, portal, dragging and physical-DPI matrix.

```sh
cargo fmt --all -- --check
cargo check -p wordcraft
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p wordcraft-ui-egui native_menu
cargo test -p wordcraft-ui-egui desktop_
cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal
dbus-run-session -- cargo test --manifest-path vendor/craft-desktop/Cargo.toml --features linux,portal appmenu_dbus -- --ignored
```

Use fresh temporary XDG_CONFIG_HOME and WORDCRAFT_PROFILE_DIR directories for synthetic GUI tests;
do not overwrite a real document or clipboard. Check both native Wayland and XWayland:

1. Focus the editor. Confirm textual top-level sections in the Plasma panel, and
   that focus on another app switches the global menu.
2. Toggle Bold through the panel; check the document and checkmark, then Undo/Redo.
   Open a contextual table menu and verify its removal after leaving the table.
3. Open a modal dialog, replace/close the document and reject delayed menu events.
   Close a dirty document and verify the normal save confirmation.
4. Test with no Global Menu widget/exporter disabled and after panel restart.
5. In System mode change the desktop preference and accent; then pin Light/Dark
   and verify they no longer track it. Check popups and dialogs too.
6. Resize at 100/125/150/175/200%; scroll the tabs/groups at narrow widths. Move
   between different-DPI monitors; check text sharpness, fullscreen, SSD buttons,
   drag/resize, system menu and shadows. This requires real monitor/compositor QA.
7. Open/save/cancel a synthetic document through the KDE portal file picker.

Automated protocol/layout tests do not certify visible Plasma rendering, real monitor
transitions, file chooser interaction or sandbox packaging.

## Recorded verification (2026-10-21)

Desktop check/build, workspace Clippy, formatting and asset/layer checks passed.
The UI suite passed 367 tests (2 ignored); four desktop tests also passed.
The shared adapter's private-session-bus tests were run separately.
In a live Plasma session, native Wayland menu reads, Bold state updates, About modal
blocking, dirty-document Close and disabled-export fallback were checked. XWayland
registration and Bold activation were also checked. Ten light/dark headless screenshots
at 100/125/150/175/200% were inspected, with Ribbon arrow/wheel regression coverage.
These observations do not establish full CI, WASM, Flatpak or physical mixed-DPI acceptance.
