# Zotero

WordCraft works with the [Zotero](https://www.zotero.org) desktop app the way Word does: Zotero's
own citation dialog, all of its citation styles, automatic bibliography and refresh. Nothing to
install in Zotero; it only has to be running. The **Zotero** tab in the ribbon has the buttons:
Add/Edit Citation, Add Note, Add/Edit Bibliography, Refresh, Document Preferences and Unlink
Citations (each runs one of the commands below).

## Commands

| Command | What it does |
|---|---|
| `ui.zotero.addEditCitation` | Add a citation at the caret, or edit the one the caret is in |
| `ui.zotero.addEditBibliography` | Add the bibliography at the caret, or edit it |
| `ui.zotero.refresh` | Update every citation and the bibliography |
| `ui.zotero.setDocPrefs` | Citation style, language, footnotes or in-text |
| `ui.zotero.removeCodes` | Unlink citations: keep the text, drop Zotero's codes |
| `ui.zotero.addNote` | Insert a Zotero note |
| `ui.zotero.status` | `{busy, waitingForAnswer, last}` |
| `ui.zotero.answer` | `{"button": n}`: answer a question Zotero is asking (OK/Yes 1, No/Cancel 0; with Yes/No/Cancel: Yes 2, No 1, Cancel 0) |

They run in the app (and through the control channel and MCP bridged to the app). A command
returns at once with `{"started": …}`; Zotero then works in its own window and WordCraft answers
it in the background. All changes from one command are one undo step.

From a terminal, against a file:

```
wordcraft-cli zotero refresh paper.docx --save paper.docx [--trace]
```

Switching between an in-text style and a note style in Document Preferences moves the citations:
into new footnotes (or endnotes) at the same place, and back into the text where the note marks
were; a note that held only the citation is removed.

## Word compatibility

Citations are stored as Word's Zotero plugin stores them — fields with the code
`ADDIN ZOTERO_ITEM CSL_CITATION {…}`, the bibliography as `ADDIN ZOTERO_BIBL … CSL_BIBLIOGRAPHY`,
document preferences in the custom properties `ZOTERO_PREF_1…n` — so one document can be edited
in Word and WordCraft in turn, with Zotero working in both. (Zotero talks to WordCraft over the
protocol it uses for LibreOffice and calls the field type "Reference Mark" there; WordCraft keeps
Word's "Field" in the file.)

## Limits

- Zotero's "Bookmarks" field type isn't supported; keep the default.
- Zotero's document transfer (Export/Import Document) isn't needed and isn't supported.
- The web version can't reach Zotero (browsers can't open the socket).

## How it works

`crates/zotero` implements Zotero's published word-processor wire protocol (port 23116):
`client` runs a session, `Bridge` answers Zotero's `Document_*` / `Field_*` calls against the
session. The app (`crates/ui-egui/src/zotero.rs`) runs the socket on a thread and answers on the
UI thread.
