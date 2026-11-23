# wordcraft-audit

Audit a `.docx` file for hidden information before sharing. Reads the raw OPC
package (ZIP) bytes — never the document model — and reports what a recipient
of the file could learn about its authors.

## What it detects

| ID | Severity | What |
|---|---|---|
| `author-pii/revision` | warn | `w:author`/`w:initials` on tracked-change elements: `w:ins`/`w:del`, moves, table revisions, custom XML markers, and tracked property changes (`w:rPrChange`, `w:pPrChange`, `w:sectPrChange`, `w:tblPrChange`, `w:tblPrExChange`, `w:trPrChange`, `w:tcPrChange`, `w:numberingChange`). Never `w:tblGridChange` or `*RangeEnd` markers (no `w:author` in their types). Scanned in every story part: `word/document.xml`, `word/header*.xml`, `word/footer*.xml`, `word/footnotes.xml`, `word/endnotes.xml` |
| `author-pii/comment` | warn | `w:author`/`w:initials`/`w:date` on `w:comment` |
| `author-pii/people` | warn | `w15:author` on `w15:person` in `word/people.xml` (the comment-author roster) |
| `author-pii/core-properties` | warn | `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml`; identity-named properties (`Author`, `Creator`, `Owner`, …) in `docProps/custom.xml` |
| `author-pii/extended-properties` | info | `Company` / `Manager` in `docProps/app.xml` |
| `hidden-text` | warn | `w:vanish` run properties (text that renders hidden) |
| `active-content/vba` | warn | VBA macro project: by relationship type, or a `word/vbaProject.bin` part |
| `active-content/ole` | warn | embedded object: by relationship type, or parts under `word/embeddings/` |
| `package/orphan-part` | info | ZIP entries no relationship targets |

Finding IDs are the crate's public vocabulary: they are stable and must not be
renamed (add a new variant instead).

Matching is by namespace URI, not by prefix, so producer-chosen prefixes do
not evade the scan. `w:vanish` counts as hidden unless `w:val` is explicitly
false-ish (ECMA-376 §17.18.67 `ST_OnOff`).

The audit is visibility only: it never modifies the file, and active content
is reported, never blocked. Sanitizing is a separate, future step.

## Report shape

`wordcraft-cli audit <file>` prints:

```json
{
  "file": "contract.docx",
  "findings": [
    {
      "id": "author-pii/revision",
      "title": "author name on tracked change",
      "severity": "warn",
      "location": "word/document.xml <w:ins>",
      "detail": "author=\"Quill Ramsey\" initials=\"QR\""
    }
  ],
  "count": 1,
  "scanned": ["_rels/.rels", "docProps/core.xml", "word/document.xml"],
  "not_scanned": ["word/theme/theme1.xml"]
}
```

The report always lists the parts it `scanned` and the parts it did not,
so a zero-finding result shows exactly what "clean" covered. Values copied
out of the file (author names, part names) have control characters
replaced with U+FFFD, so printing a report cannot spoof a terminal.

Exit codes: `0` = no findings, `1` = findings present (CI-gate usable),
`2` = the audit could not run (unreadable file, unparsable package).

## Spec references

- ECMA-376 Part 1: tracked-change elements (§17.13.5; `w:tblGridChange`
  excluded — its type defines no `w:author`), `w:comment` (§17.13.4.2),
  `w:vanish` (§17.3.2.42), `ST_OnOff` (§17.18.67)
- ECMA-376 Part 2: core properties (§12.3.1), extended properties (§12.3.2),
  custom properties (§12.3.3), OPC relationships (§12.2.4), part names are
  case-insensitive (§9.1.1)

## Limits

- Story parts are scanned by their standard part names; a story part at a
  non-standard path is not parsed, but it is never invisible — it appears
  in the report's `not_scanned` list, as does every other unparsed part.
- A part that cannot be parsed is an error, never a silent pass (fail closed).
  Malformed XML (mismatched or unclosed tags) fails the audit.
- Hostile-input guards: at most 100,000 ZIP entries, 256 MiB per part,
  256 MiB total decompressed bytes across all scanned parts,
  2,000,000 XML elements per part.
