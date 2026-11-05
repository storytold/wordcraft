# CSL citation styles

WordCraft's References ▸ Citations & Bibliography ▸ Style combo lists its twelve built-in
citation styles (APA, MLA, Chicago…) and, below a separator, the CSL styles you added. Any
independent style from the [Citation Style Language](https://docs.citationstyles.org) — the
XML format Zotero and Mendeley styles are written in, a public spec — can be added: **Add
style from file…** picks a `.csl` file, installs it and makes it the document's style.
Warnings (a part of the file WordCraft's renderer doesn't draw on, a non-en-US locale) show
in the status line; the style still loads. A *dependent* style (one that only links to a
parent style and carries no citation layout of its own) is refused — install its independent
parent instead.

## Where the files live

Added styles are copied into the styles folder and loaded again at startup:

| System | Folder |
|---|---|
| Linux/BSD | `~/.config/wordcraft/styles/` (`$XDG_CONFIG_HOME/wordcraft/styles/`) |
| macOS | `~/Library/Application Support/WordCraft/styles/` |
| Windows | `%APPDATA%\WordCraft\styles\` |

You can also drop a `.csl` file there yourself and restart WordCraft. Files that fail to
parse are skipped (the problem is reported in the status line), and removing a file removes
the style at the next start. The `references.cslStyles` commands do the same live:
`{}` lists the installed styles, `{"add": path}` installs a file, `{"remove": id}` removes
one; the web build has no styles folder, so adding and removing are errors there.

## How a document records its style

A document using a CSL style records `csl:<id>` — the style's own `<info><id>` (the file's
stem when the file has none) — as its citation style, so it round-trips through `.docx`
untouched. Choose it back by id (`references.citationStyle {"style": "csl:<id>"}`) or by
title. If the style isn't installed, updating citations says so and changes nothing, rather
than silently falling back to another style: re-add the file (References ▸ Style ▸ Add style
from file…) and the document renders again.

## Supported subset

`crates/csl` parses and renders the parts of CSL 1.0.2 that citations and bibliographies
draw on: text, numbers, labels, dates, names (with et-al, abbreviations and substitution),
groups, `choose` conditionals, macros, affixes, delimiters, `strip-periods`, plural forms,
first/subsequent/ibid positions, and bibliography sorting (including citation-number and
the name/date/title keys, ascending or descending). The locale terms are the spec's en-US
set; other locales load with a warning and render with en-US terms. Constructs that would
silently change output — citation collapsing and disambiguation, formatting attributes
(font style/weight/variant, text case), note-class styles, non-en-US locale terms and date
formats, page-range formats — load with a warning. A warning does not prevent unsuitable
output: a note-class style, say, still renders its citations in text. Inspect the rendered
citations and bibliography before trusting an installed style.

## See also

- [Zotero](zotero.md): the Zotero desktop app's own citation dialog and all of its styles,
  no file installing needed.
- [MCP server](mcp.md): the `references.*` commands over MCP.
