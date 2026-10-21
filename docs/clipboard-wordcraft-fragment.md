# Clipboard: WordCraft Fragment

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** new (copy and paste between WordCraft windows keep the formatting)

Each WordCraft window is a process of its own, and a copy's rich fragment lived only in the window
it was made in. Pasting into another window went through the system clipboard's HTML, which loses
what HTML can't say: paragraph and character styles, a document's own Normal and defaults, list
definitions, spacing and indents that come from styles, text box stories. The HTML was also
exported in the built-in styles, so even the font of the source's Normal was lost.

Copy now also puts the copy on the system clipboard as a **WordCraft Fragment**, so another
WordCraft window pastes it as it was:

| Platform | Written | Read |
|---|---|---|
| Windows | registered clipboard format `WordCraft Fragment` (UTF-8 JSON), beside arboard's text and HTML | yes |
| Wayland | not yet: arboard's offer can't take another type (a data-control offer of our own can, see the RTF work) | `application/x-wordcraft-fragment` |
| X11, macOS | not yet | — |

Without it another window pastes the HTML as before.

## Payload

JSON, at most 64 MB (`native_clip::MAX_NATIVE_BYTES`):

```json
{"format": "wordcraft-fragment", "version": 1, "text": "…", "doc": { …Document… }, "media": {"image1.png": "<base64>"}}
```

`doc` is a `wordcraft_doc::Document` whose body is the copied blocks and whose parts are the text box
stories they show, with the source document's styles, numbering and revisions. `media` holds only
the pictures the copy refers to. Reading it is lenient and bounded like opening a `.json` document:
size-capped, unknown fields ignored, runs repaired (`Document::normalize_runs`).

## Paste

`edit.paste`, `edit.pasteMerge` and `edit.pasteSpecial` take it as the `wordcraft` param; the desktop
front end fills it from the clipboard (`Services::clipboard_native`).

1. **This window's copy** (the payload's hash is the one Copy made here): the session's own fragment
   goes in, as before.
2. **Another window's copy**: the styles the destination lacks come with it (a list a style gives is
   renumbered with it), its lists and pictures are adopted, and, as Word's *Keep Source Formatting*
   does, wherever a style it uses means something else here (another Normal font, Heading size,
   default spacing) the source's look is set as direct formatting on the paragraphs, paragraph marks
   and runs that don't set it themselves. Styles that mean the same in both documents stay clean.
3. A payload whose text is not the clipboard's text is stale (another program copied since) and is
   ignored; so is anything that doesn't read as a payload. The HTML, RTF or text is pasted then.

WordCraft's own copy (either case) keeps its paragraph formatting as Word's Keep Source Formatting
does: a pasted paragraph whose mark came with the copy is formatted as it was — the first one too,
though it joins the paragraph the paste starts in (it used to keep that paragraph's formatting
whenever it had any, so a DOCX's spacing before and after and line spacing were lost) — and so is
a whole paragraph (copied from its start to its end, as Word's smart paragraph selection takes its
mark) pasted into an empty one. A piece of a paragraph pasted into text takes that paragraph's.

Merge Formatting and Keep Text Only work on it as on any other paste.

## Other programs

Copy's HTML is now exported with the source document's styles, lists and pictures, so Word,
LibreOffice and browsers see a copy's Normal font and style sizes as they are in WordCraft. HTML and
RTF exports (the clipboard's HTML, Save as HTML or RTF) also carry each paragraph's spacing before
and after, line spacing and indents as they resolve in its document: HTML `margin-top`/`-bottom`,
`line-height` (with `mso-line-height-rule` for exact and at-least spacing), `margin-left`/`-right`
and `text-indent`; RTF `\sb \sa \sl\slmult \li \ri \fi`. They used to be left out (RTF wrote fixed
values per kind of paragraph).

Tests: `crates/engine/src/cmd/native_clip.rs` (round trip, keep source look, own copy, stale and
hostile payloads, pictures) and `crates/ui-egui/src/system_clipboard.rs`
(`a_copy_pastes_with_its_formatting_in_another_window`).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | new | WordCraft Fragment on the Windows clipboard; Paste keeps another window's formatting; own copies keep paragraph spacing; HTML and RTF carry spacing, line spacing and indents |
