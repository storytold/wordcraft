# Arabic interface

> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (Arabic interface catalog, Arabic interface font fallback) · **Target:** WordCraft main (2026-10-15)

Choose **File › Options › Interface language › العربية**, or follow an Arabic system display
language (`ar`, `ar-SA`, `ar-EG`, `ar_SA.UTF-8` and the other regions). The catalog code is `ar`;
`ui.language` with `{"value":"ar"}` selects it explicitly, and `WORDCRAFT_LANGUAGE=ar-SA` does the
same for a headless screenshot or a launch while the preference is `auto`. Document text, command
ids, CLI/MCP/control responses and file paths do not change.

`crates/ui-egui/src/i18n/ar.tsv` has 2,616 entries: the union of every key in every other catalog
(built-in style names, equation gallery labels, paper sizes, watermarks and citation styles
included). It is written from the meaning of the English labels in ordinary Arabic computing
vocabulary; no other product's Arabic interface, string table or terminology database was
consulted, and no other catalog was used as a source. Placeholders, `\n` breaks in ribbon labels
and dialog ellipses are kept (checked by `bundled_catalogs_are_well_formed`), and strings added
later fall back to English until they are translated.

The draft was made with Claude Code. **It still needs a native speaker to approve the terminology
and review the running app** before this line can say so.

## Interface font

The interface's Latin faces (Inter, Source Sans 3) have no Arabic letters, so without a fallback
every Arabic label, and «العربية» in the language list, drew as boxes. `theme::font_definitions_language`
now appends, after the CJK faces:

- the craft-fonts Arabic face (`wordcraft_fonts::ui_arabic_fonts`) when the build has one, else
- an installed Arabic font (`wordcraft_fonts::system_arabic_ui_font`): Segoe UI, Tahoma or Arial
  on Windows; SF Arabic, Geeza Pro or Arial on macOS; Noto Sans Arabic, Noto Naskh Arabic or
  DejaVu Sans elsewhere. The file is chosen once for the catalog's Arabic letters, like the system
  CJK fallback (#241, #486).

## Word order

egui 0.36 joins letters within a word but lays a line's words out left to right (no bidi), so
`الحفظ التلقائي` drew as `التلقائي الحفظ`. `i18n::bidi` (ported from PhotoCraft's `bidi_text`)
puts each translation into display order with the Unicode Bidirectional Algorithm, paragraph
direction right to left: words reversed, Latin names and numbers kept in order, punctuation and
ellipses moved to the left end, brackets mirrored, `\n` lines kept.

- `tr`/`t`/`tl!` return the display form for a right-to-left language, built once per catalog.
  `tr_logical` and `has` keep the reading order for text that is compared or stored.
- `fmt` fills a right-to-left template in display order: its own words are reordered and every
  value moves as one piece, so numbers, paths and labels already in display order stay intact
  (`الصفحة {page} من {pages}` → `10 من 2 الصفحة`, read from the right).
- `location` reverses the segments and points the separator left (`الخط ‹ الرئيسية`).
- Command search matches word by word, so it still finds Arabic labels in display order.

Trade-off: the accessibility tree gets the display-order labels too, so a screen reader may read
the words of a multi-word Arabic label in reverse. Fixing that needs the mirrored, bidi-aware
layout below rather than reordered strings.

## Conventions

- Modern Standard Arabic, neutral across regions. Commands are verbal nouns (`حفظ`, `إدراج جدول`,
  `تحديد الكل`); messages address the user directly in the imperative (`احفظ المستند…`).
- Western digits (0–9) in fixed labels (`تباعد أسطر 1.5`, `المستوى 3`), so they match the
  numbers the app fills in through placeholders. Decimal point kept as in the English source.
- Arabic comma `،` in lists (`A، B، C، …`) and guillemets «…» for quoted names
  (`البحث عن «{text}» في المستند`).
- Ribbon buttons with a `\n` break keep the break, split where the Arabic phrase breaks naturally.
- Names stay in Latin letters: WordCraft, Zotero, Mendeley, EndNote, SmartArt, WordArt, Discord,
  the Craft theme, file formats, paper sizes (Letter, Legal, Executive …), citation styles (APA,
  MLA, Chicago …), key names (Backspace, Tab, Page Up …) and the watermark texts that are inserted
  into the document as written (DRAFT, CONFIDENTIAL, DO NOT COPY, SAMPLE, ASAP, URGENT).
- No plural forms in the catalog format: counts use the singular after the number
  (`{n} سطر`, `{words} كلمة`), which reads acceptably for most counts; full Arabic plural rules
  (one, two, 3–10, 11+) need `@plural` support in WordCraft's catalog parser first.

## Terminology to review

| English | Arabic | Note |
|---|---|---|
| Document | مستند | |
| Paragraph / Section | فقرة / قسم | |
| Style / Theme | نمط / سمة | |
| Font | خط | |
| Ribbon / Tab (ribbon) | الشريط / علامة تبويب | |
| Home (tab) | الرئيسية | |
| Insert / Draw / Design / Layout | إدراج / الرسم / التصميم / التخطيط | |
| References / Mailings / Review / View | المراجع / المراسلات / مراجعة / عرض | |
| Select / Choose | تحديد / اختر | |
| Highlight / Mark | تمييز / تعليم | kept apart so they never mix |
| Track Changes / Markup | تتبع التغييرات / العلامات | |
| Comment / Reply / Resolve | تعليق / رد / حل | |
| Header / Footer | الرأس / التذييل | |
| Footnote / Endnote | حاشية سفلية / حاشية ختامية | |
| Table of Contents / Index | جدول المحتويات / الفهرس | |
| Caption / Cross-reference | تسمية توضيحية / إسناد ترافقي | |
| Citation / Bibliography / Source | استشهاد / قائمة المراجع / مصدر | |
| Bookmark / Hyperlink / Field | إشارة مرجعية / ارتباط تشعبي / حقل | |
| Mail Merge / Recipient | دمج المراسلات / مستلم | |
| Content control | عنصر تحكم محتوى | |
| Text box / Shape / Picture | مربع نص / شكل / صورة | |
| Indent / Hanging indent | مسافة بادئة / مسافة بادئة معلّقة | |
| Line spacing / Spacing | تباعد الأسطر / التباعد | |
| Margins / Orientation / Portrait / Landscape | الهوامش / الاتجاه / عمودي / أفقي | |
| Break (page, section, column) | فاصل | |
| Tab stop / Leader | علامة جدولة / الأحرف السابقة | |
| Bold / Italic / Underline / Strikethrough | غامق / مائل / تسطير / يتوسطه خط | |
| Superscript / Subscript / Small caps | مرتفع / منخفض / أحرف كبيرة صغيرة | |
| Proofing / Spelling / Grammar / Thesaurus | التدقيق / الإملاء / القواعد / المعجم | |
| Hyphenation | تقسيم الكلمات | |
| Drop cap / Watermark | حرف استهلالي / علامة مائية | |
| Outline (view) / Outline (shape border) | المخطط التفصيلي / حد خارجي | two different English senses |
| Pane / Dialog | جزء / نافذة | |
| Undo / Redo | تراجع / إعادة | |
| Overtype | الكتابة الفوقية | |
| Caret | مؤشر الكتابة | |
| Equation / Matrix / Integral / Summation | معادلة / مصفوفة / تكامل / مجموع | |

## Known gaps

- **Text the code assembles itself** (`format!` of two translated parts, user file names inside
  a label) is not reordered; only catalog lookups and `fmt` are.
- **Interface mirroring.** The ribbon, panes, dialogs and rulers stay left to right (estimate in
  [target-app-parity › Localization](target-app-parity.md): 15–25 h). The open Hebrew PR (#20)
  covers the same work.
- **Arabic proofing.** No Arabic spelling or grammar; new documents keep the default language.
- **Kashida justification** in documents is still missing (Right-to-left and complex scripts row).
- The catalog is AI-assisted and awaits native-speaker review in the running app.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | minor | Arabic catalog (2,616 entries), registry row, Arabic interface font fallback, labels in display order (`i18n::bidi`) |
