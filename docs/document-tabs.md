# Optional document tabs (desktop)

WordCraft now has a multi-document engine and separate native windows. This feature offers an **optional** browser-like tab strip instead of separate windows, without duplicating the document model. Enable it in File → Options → General. It is off by default.

- A **+** creates a document; **×** closes the selected document through the existing Save/Don't Save/Cancel prompt.
- Click a tab to activate its document. Drag one tab over another to reorder them.
- Ctrl+Tab / Ctrl+Shift+Tab (Cmd on macOS) switches documents.
- Each document keeps its own undo history, selection, zoom, and recovery managed by WordCraft's existing engine.
- Optionally restore previously saved files on startup. Unsaved files without paths are not silently restored.
- To disable tab mode, first close all but one document. To switch window mode, close any additional native windows.
- The browser build keeps its existing one-window behavior; this setting is desktop-only.

This branch intentionally does not re-add the old in-repository pt-BR TSV file: translations are managed separately by the upstream project.

## Validation still required

- cargo fmt --all --check
- cargo test -p wordcraft-ui-egui document_tabs
- Desktop smoke tests on Windows/macOS/Linux, including save prompts, multiple files, startup restore, and switching window mode
