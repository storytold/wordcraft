# Georgian interface

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-24 · **Change:** trivial (the catalog is in storytold/craft-i18n, `locales/ka/wordcraft.tsv`; previously trivial: checks follow the shared interface-language tests; previously minor: review scope of the newer rows; previously minor: Georgian interface notes and terminology) · **Target:** WordCraft 0.5.0

Choose **File › Options › Interface language › ქართული**, or follow a Georgian system display
language (`ka`, `ka-GE`, `ka_GE.UTF-8`). The catalog code is `ka`; `ui.language` with
`{"value":"ka"}` selects it explicitly, and `WORDCRAFT_LANGUAGE=ka-GE` does the same for a headless
screenshot or a launch while the preference is `auto`. The choice is saved with the other interface
settings. Document text, command ids, CLI/MCP/control responses and file paths do not change.

craft-i18n's `locales/ka/wordcraft.tsv` (storytold/craft-i18n) has 2,788 entries: every key of the other catalogs (built-in style
names, equation gallery labels, paper sizes, watermarks, greetings and citation style names
included) and everything the interface audit finds. It is written from the meaning of the English
labels; no proprietary translation resources were consulted. A native speaker reviewed the 2,610
entries of 2026-10-13 string by string against one terminology glossary shared by the Craft apps;
the rows added since were written the same way, and only their doubtful terms were reviewed. Product, format and technology names
(WordCraft, PDF, Markdown, MCP, citation styles such as APA and IEEE) stay in Latin letters, and
`OK` stays `OK`. Strings added later fall back to English until they are translated.

## Georgian script

- **No case.** Mkhedruli has no capital letters in running text. Unicode maps it to Mtavruli
  (U+1C90–U+1CBF), but Mtavruli is only for all-caps text, so the catalog has no Mtavruli anywhere,
  not even for UPPERCASE in Change Case.
- **Count-neutral numbers.** A Georgian noun stays singular after a numeral, so counts are written as `noun: n` (`სიტყვა: 21`, `მიმღები: {count}`), which
  reads right for every number.
- **Fonts.** None of the bundled interface faces (Inter, JetBrains Mono) has Georgian. The
  interface draws it with craft-fonts' Noto Sans Georgian (`Geor`), which release builds embed
  (the pinned `craft-fonts-ref` includes it) and the web build keeps.
- **Numbers and dates.** The interface uses a decimal comma in its own labels (`სტრიქონთა დაშორება
  1,5`). Lengths, paper and document numbers follow the system region (File › Options › Advanced ›
  Region), not the interface language; there are no Georgian date formats of its own yet.

## Conventions

- Commands are verbal nouns (masdar): შენახვა, გახსნა, წაშლა, not imperatives. Messages and hints
  address the user politely in the plural (`აირჩიეთ`, `დაწკაპეთ`).
- Ribbon labels keep their `\n` break at a natural word boundary; labels that open a dialog keep
  their `…`. Placeholders are never inflected: they go after a colon or in parentheses
  (`Undo {action}` → `დაბრუნება: {action}`).
- Georgian quotation marks („…“).

## Terminology

| Source | Georgian |
|---|---|
| File / Home / Insert / Draw / Design | ფაილი / მთავარი / ჩასმა / ხატვა / დიზაინი |
| Layout / References / Mailings / Review / View | განლაგება / ცნობარები / დაგზავნა / განხილვა / ხედი |
| New / Open / Save / Save As / Print | ახალი / გახსნა / შენახვა / შენახვა როგორც / დაბეჭდვა |
| Undo / Redo / Repeat | დაბრუნება / ხელახლა შესრულება / გამეორება |
| Cut / Copy / Paste / Paste Special | ამოჭრა / კოპირება / ჩასმა / სპეციალური ჩასმა |
| Find / Search / Replace / Select / Mark | ძებნა / ძიება / ჩანაცვლება / მონიშვნა / მარკირება |
| Clipboard | გაცვლის ბუფერი |
| Font / Paragraph / Styles | შრიფტი / აბზაცი / სტილები |
| Bold / Italic / Underline | მუქი / დახრილი / ხაზგასმული |
| Change Case / UPPERCASE / All Caps | რეგისტრის შეცვლა / მთავრული / მთავრული |
| Line spacing / Paragraph spacing | სტრიქონთა დაშორება / აბზაცთა ინტერვალი |
| Justify | ორმხრივი სწორება |
| Bullets / Numbering / Indent | ბურთულები / დანომვრა / შეწევა |
| Tabs (tab stops) / Tab (ribbon) | ტაბულაცია (ტაბულაციის გაჩერებები) / ჩანართი |
| Borders / Shading | ჩარჩოები / ფონი |
| Table / Row / Column / Cell | ცხრილი / სტრიქონი / სვეტი / უჯრა |
| Picture / Shape / Chart / Text Box | სურათი / ფიგურა / დიაგრამა / ტექსტური ბლოკი |
| Header / Footer | ზედა კოლონტიტული / ქვედა კოლონტიტული |
| Page Break / Section Break / Line Break | გვერდის გამყოფი / სექციის გამყოფი / სტრიქონის წყვეტა |
| Margins / Orientation / Portrait / Landscape | ველები / ორიენტაცია / ვერტიკალური / ჰორიზონტალური |
| Watermark / Equation / Symbol | ჭვირნიშანი / განტოლება / სიმბოლო |
| Table of Contents / Index | სარჩევი / ინდექსი |
| Footnote / Endnote | სქოლიო / დოკუმენტის სქოლიო |
| Citation / Bibliography / Caption | ციტატა / ბიბლიოგრაფია / წარწერა |
| Comment / Track Changes / Tracking | კომენტარი / ცვლილებების ჩაწერა / თვალის დევნება |
| Accept / Reject | მიღება / უარყოფა |
| Spelling & Grammar / Thesaurus / Word Count | მართლწერა და გრამატიკა / თეზაურუსი / სიტყვების რაოდენობა |
| Mail Merge / Envelopes / Labels | წერილების შერწყმა / კონვერტები / იარლიყები |
| Draft / Outline / Read Mode / Zoom | მონახაზი / სტრუქტურა / კითხვის რეჟიმი / მასშტაბი |
| Options / Reset / Preset | პარამეტრები / ჩამოყრა / პრესეტი |
| Page {page} of {pages} | გვერდი {page} / {pages} |

Please follow this table when you add or change Georgian strings; the shared Craft-apps glossary
decides any term not listed here.

## Maintenance

Add or fix rows in craft-i18n's `locales/ka/wordcraft.tsv` (`<TAB>English source<TAB>Georgian`, the
English exactly as the code passes it to `tl!`; its `docs/integration.md`). They show here once
`craft-i18n.rev` moves to a commit that has them. A test in `crates/ui-egui/src/i18n/tests.rs`
checks locale selection and the saved preference; craft-i18n's checker validates the rows, and
`cargo xtask i18n-report` lists any interface text Georgian lacks.

Georgian spelling, grammar and hyphenation are not included: this is an interface translation, not
Georgian proofing.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-24 | trivial | The catalog is in storytold/craft-i18n (`locales/ka/wordcraft.tsv`, 2,788 entries) |
| 2026-10-13 | minor | Georgian interface notes: choosing it, coverage (2,610 entries), script notes, conventions, terminology and checks |
