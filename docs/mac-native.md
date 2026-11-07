# Mac-native fork

This fork adds an AppKit menu bar to WordCraft's existing Rust editor. Development
is on `mac-native`; `main` follows the upstream project.

The starting implementation is [Piyush Goel's upstream PR #4303](https://github.com/storytold/wordcraft/pull/4303),
at `a2f1240f257c730e86262a58c227d81f3b848dbb`, adapted to upstream
`729cd0a5`. Its menu definitions, recent-document list, localization, window/help
registration, and viewport input routing are retained. This fork adds native key
equivalents, Settings, focused-field Undo/Redo, and modal-dialog protection.

## Menus and shortcuts

WordCraft, File, Edit, View, Go, Insert, Format, Tools, Table, Window, and Help are
real system menus. File includes recent documents. Window exposes Minimize,
Zoom, full screen, and Bring All to Front; it is registered with AppKit for the
system's window features. Help is registered for macOS menu search.

| Action | Shortcut |
| --- | --- |
| Settings | ⌘, |
| New / Open / Save / Close / Print | ⌘N / ⌘O / ⌘S / ⌘W / ⌘P |
| Save As | ⇧⌘S |
| Undo / Redo | ⌘Z / ⇧⌘Z |
| Cut / Copy / Paste / Select All | ⌘X / ⌘C / ⌘V / ⌘A |
| Find | ⌘F |
| Bold / Italic / Underline | ⌘B / ⌘I / ⌘U |
| Hide / Minimize / Quit | ⌘H / ⌘M / ⌘Q |

Document shortcuts follow Customize Keyboard, including removal and reassignment
of Command combinations. Standard Mac application and text-editing shortcuts
are reserved. Other commands retain the editor's existing keyboard handling.
Undo and clipboard actions target the focused field or document. Quit goes through
the unsaved-document checks, and document actions cannot replace a modal dialog.

The editor, ribbon, and dialogs still use egui. This is a native menu integration,
not an AppKit rewrite of the entire application. Some menu-only labels fall back
to English. AppKit Services integration is registered; full document Services
support is not added by this change.

## Local build

Install Rust 1.95 or newer. Optionally check out `storytold/craft-i18n` at the
revision in `craft-i18n.rev`, and set `CRAFT_I18N_DIR` to that checkout to embed
translations and proofing data. Then run:

```sh
bash packaging/macos/build-local.sh
```

The app is `dist/WordCraft Mac.app`, compiled for the current Mac and ad-hoc
signed for local use. It has a distinct bundle identifier but uses WordCraft's
existing preferences and recovery directory. It is not a notarized distribution.
Set `WORDCRAFT_LOCAL_PROFILE=release` for an optimized release build.

## Validation

```sh
cargo test -p wordcraft
cargo test -p wordcraft-ui-egui --lib route_input_reaches_the_active_window
cargo xtask ci
```

The main-thread `mac_menu_appkit` test inspects the installed AppKit menus and
key equivalents, exercises dirty-document Quit and Cancel, checks modal menu
enablement, and sends native Select All/Cut actions to a focused egui text field.
The viewport regression checks that a menu paste reaches a separate document
window without changing the first document.

### Check status

The full `cargo xtask ci` check currently stops at an existing Rust 1.95 Clippy
error in `crates/engine/src/cmd/insert/items.rs:301` (`nonminimal_bool`). That
unchanged drop-cap expression is outside this fork's menu changes. The remaining
full-suite steps are therefore not claimed as passing; focused native and
viewport tests and the separate compatibility checks are recorded for this fork.

Verified locally on Apple silicon with Rust 1.95:

- Desktop tests: 66 passed, one ignored; four CLI integration tests passed.
- AppKit menu integration: passed, including focused-field Undo/Redo.
- Window tests: all 16 passed, including menu input in a separate window.
- Clippy for both changed crates (`--all-targets --no-deps -D warnings`): passed.
- Formatting, asset attribution, and dependency layering: passed.
- WebAssembly compatibility: all 15 checked crates passed (existing warnings remain).
- Local `.app` build, signature verification, property-list validation, and `--version`: passed.
