# 1. The Normal template is a .dotx file

Date: 2026-10-27 · Status: Accepted

## Context
Set As Default (Design tab, Font, Paragraph, Page Setup, Modify Style › "New documents based on this template") must make new blank documents start from the user's formatting, across restarts. What it saves is not one value: document defaults, whole styles with the styles and lists they refer to, theme fonts and colours, and page setup.

Earlier attempts (#1822, #4256, #4449) stored a hand-picked subset as JSON in `ui.json`. Each needed its own sanitiser, which drifted from the document validators (numbering references not copied, section checks disagreeing), and each left parts out.

## Decision
The Normal template is a WordCraft-written `Normal.dotx` in the settings folder (base64 under its own `localStorage` key on the web). It is read with the same docx reader and validated by the same code as any opened document; Set As Default re-reads it, changes only what was set, and writes it atomically — only once the command has succeeded and passed Restrict Editing's post-check, so a refused or failed command leaves both the document and the template as they were. A missing, damaged or oversized file means built-in defaults and a status message, never a crash. Text language is not stored there; Proofing Language › Set As Default owns it.

## Consequences
- One store, one validator: anything a document can hold, the template can hold, with no second schema to migrate.
- Matches Word's Normal.dotm model, so "this document only / all new documents" behaves as users expect.
- A docx parse at each blank-document creation; it is small, and capped.
- `ui.json` defaults from unmerged PRs are never read; nothing to migrate.
