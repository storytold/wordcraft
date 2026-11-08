# Native macOS menus

WordCraft installs an AppKit menu bar with File, Edit, View, Go, Insert, Format,
Tools, Table, Window, and Help menus. Commands use the existing editor dispatch,
including document dialogs and unsaved-change checks. Open Recent follows the
editor's recent documents. Window and Help are registered with AppKit.

The implementation builds on [#4303](https://github.com/storytold/wordcraft/pull/4303)
by @piyushgoel2808, at `a2f1240f257c730e86262a58c227d81f3b848dbb`.
That contribution supplies the menu definitions, localization, recent files,
guarded Quit, and viewport input bridge. This version adds native key equivalents,
focused-field Undo/Redo, Settings, and modal-dialog guards.

| Action | Default native shortcut |
| --- | --- |
| Settings | ⌘, |
| New / Open / Save / Close / Print | ⌘N / ⌘O / ⌘S / ⌘W / ⌘P |
| Save As | ⇧⌘S |
| Undo / Redo | ⌘Z / ⇧⌘Z |
| Cut / Copy / Paste / Select All | ⌘X / ⌘C / ⌘V / ⌘A |
| Find | ⌘F |
| Bold / Italic / Underline | ⌘B / ⌘I / ⌘U |
| Hide / Minimize / Quit | ⌘H / ⌘M / ⌘Q |

The listed document shortcuts follow Customize Keyboard's Command combinations.
Standard Mac application and text-editing shortcuts are reserved: in particular,
⌘H hides the app and ⌘M minimizes the active window, taking precedence over
Word-style bindings for those keys. Other shortcuts remain in the editor.

Clipboard actions and Undo/Redo target the focused text field or document.
Document actions are disabled while a modal dialog is open, so Settings or Open
cannot replace a Save Changes prompt. Quit checks all unsaved documents through
the existing close cascade. Native menu actions target the active document window.
Bold, Italic, and Underline are disabled while a text field owns the keyboard,
including fields in other document windows. Before routing new input, pending
events for a different or closed window are discarded so they cannot reach
another document.

The editor and dialogs still use egui. Some menu-only strings fall back to English.
The Services submenu is registered, but full document Services support is outside
this change. Checkmarks for view/format toggles are not added here.

## Tests

`cargo test -p wordcraft` includes menu registry/parameter checks and the
main-thread `mac_menu_appkit` integration test. The latter reads actual AppKit
menus and key equivalents, exercises Quit and Cancel on a dirty document, checks
modal enablement, and invokes native Select All, Cut, Undo, and Redo against a
focused egui field while checking that the document behind it is unchanged. It
also checks that queued Bold, Italic, and Underline respect field focus and that
formatting works again after focus leaves the field.

`cargo test -p wordcraft-ui-egui --lib windows::tests` includes a regression that
routes menu input to a separate document window without changing the first one,
plus regressions for undrained input across window switches and closing a window.
