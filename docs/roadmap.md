# WordCraft roadmap: milestones and what's next

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (milestones moved here from ROADMAP.md; Current focus set from the re-measured gaps) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

Forward-looking plan. The summary is [`ROADMAP.md`](../ROADMAP.md); the ranked work list is
[`gaps.md`](gaps.md); the numbers are in [`target-app-parity.md`](target-app-parity.md). Hours are
Opus 5.5 agent wall-clock.

## Current focus (toward beta, ~150–230 h)

1. **Merge or close the ~45 open PRs** before starting overlapping work: many close catalog gaps
   (Paste Special #235, Clipboard pane #264, Advanced Find #234, Column Selection #237, Style
   Inspector #236, Manage Styles #270, Group #267, custom table styles #256), fix field bugs
   (Hebrew RTL #20, IME fixes #155–#164, touchpad scrolling #252) or add languages (de, nb,
   nn, sr, he). Owner review needed.
2. **DOCX real-world corpus** (gap #1, 30–50 h): collect openly licensed real-world .docx files,
   open/render/round-trip them, compare with Word locally, fix what breaks.
3. **Preserve what we can't render** (gap #2, 10–15 h): charts, SmartArt, OLE, ink, group shapes
   survive a round trip and show their fallback pictures.
4. **Stability sweep** (gap #3, 20–30 h): the open crash, hang and write-failure issues, plus an
   autosave/recovery soak test.
5. **Pagination fidelity** (gap #4, 30–50 h): column balancing, footnote continuation, compat
   options, a local page-by-page comparison harness; an owner decision on an Aptos-metric font.
6. **Formatting revisions and content controls** (gap #5, 12–18 h), **native printing** (gap #6,
   6–10 h), **dialog depth** (gap #7, 20–30 h).

## Milestones

| # | Milestone | State | Remaining |
|---|---|---|---|
| M0 | Skeleton and vertical slice (model, layout, render, engine, Word-style UI, CLI, MCP, web) | done | — |
| M1 | DOCX I/O | done (first version); real-world corpus not started; charts/SmartArt/OLE dropped | 45–75 h |
| M2 | Home tab | done except Paste Special, Clipboard pane, Style Inspector, Manage Styles, Asian typography (PRs open for four) | 5–10 h |
| M3 | Insert tab | mostly done; equations done (#191); charts, SmartArt, icons, 3D, screenshot, online media missing | 50–80 h |
| M4 | Layout and Design tabs | done except column balancing, group | 6–10 h |
| M5 | Tables | done (row splitting, floating tables, style formatting); Draw Table, custom styles (#256) missing | 8–12 h |
| M6 | References | first version done (TOC fields, cross-references, Zotero); footnote continuation, 8 more bibliography styles missing | 15–25 h |
| M7 | Review | done (balloons, paragraph-mark revisions); formatting revisions, move tracking, multilingual proofing missing | 35–60 h |
| M8 | View | mostly done; window commands missing | 3–5 h |
| M9 | Mailings | done (first version); Excel/Outlook data sources and email merge missing | 6–10 h |
| M10 | File/Backstage | mostly done; native printing, Options depth missing | 12–20 h |
| M11 | Draw and objects | text wrap, text boxes, canvas handles done; ink, rotation, grouping, contour wrap missing | 35–55 h |
| M12 | Formats breadth (PDF, ODT, RTF, HTML, MD, TXT, LaTeX, `.doc` import) | done (first versions); `.doc` write, encryption, minor formats missing | 60–100 h |
| M13 | Performance budgets | on track (1.4 ms relayout); large real-world documents unmeasured | 10–20 h |
| M14 | 1.0 polish, packaging, signing | releases v0.1.0–v0.4.0 published (signed macOS, Windows MSI x64/x86/arm64, Linux, FreeBSD, web) | — |
| M15 | Localization (12 key languages, RTL interface, proofing languages) | 7 UI languages, English proofing | 80–130 h |
| M16 | Right-to-left and East Asian typography | RTL paragraphs done (#207) | 30–45 h |

## Next after beta

Charts and SmartArt editing, the Draw tab, `.doc` writing, PDF Reflow, the remaining interface
languages, screen-reader access, ecosystem decisions (VBA, add-ins, cloud), an in-app assistant.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | Created: milestones moved from ROADMAP.md with remaining hours; Current focus from gaps.md; M15–M16 added |
