# Typing parity with Word

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-17 · **Change:** minor (German text: smart quotes „…“ / «…» and German abbreviations and ordinals that don't end a sentence, #735; previously trivial: Shift+Delete cuts the selection on every platform, #1669; previously trivial: Select All then Delete removes a table the document starts with, #1198; previously trivial: Alt+numpad character codes, #961; previously trivial: Down and Up from a line start beside a table go into it, #1007; previously trivial: sample-text formulas on Enter, #1067; previously trivial: Track Changes: Accept All after a tracked delete from a paragraph end keeps the next paragraph; Reject All after typing into an empty paragraph keeps it (#864); previously minor: Correct TWo INitial CApitals, cAPS LOCK and table-cell capitals, #544; previously trivial: complex-script caret and Backspace, #604; Move Paragraph Up/Down, Promote/Demote, Go Back and Alt+X keys, #343) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

What Word does when you type, observed black-box (same keystrokes typed into Word and
WordCraft, results read back paragraph by paragraph) and pinned by tests in
`crates/engine/src/tests_typing.rs`. When typing behaviour changes, change it here and in the
tests together. Line breaking and pagination are in [`layout-parity.md`](layout-parity.md); the
overall picture is in [`target-app-parity.md`](target-app-parity.md).

## Sample text (#1067)

Enter at the end of a paragraph containing exactly `=rand()` or `=lorem()` replaces
the formula with five paragraphs of three sentences and leaves the caret in a following
empty paragraph. `=rand(p)` / `=lorem(p)` set the paragraph count; `(p,s)` sets both
counts. WordCraft accepts integers from 1 to 200 for each count; if either parameter
is invalid or outside that range, both revert to 5/3. Other text, a selection or Enter
inside the formula follows ordinary Enter behaviour. The English text is original to
WordCraft; lorem ipsum uses traditional pseudo-Latin. This implements the issue's
requested defaults; the exact numerical limits have not been compared in Word.

The replacement and its paragraph breaks are one undo step. Typing and paragraph
formatting use the existing editing primitives. Tests: `sample_text_formulas_expand_on_enter`,
`sample_text_invalid_parameters_use_defaults`, `sample_text_requires_an_exact_paragraph_and_caret_at_end`,
`sample_text_preserves_typing_and_paragraph_formatting`, `sample_text_replacement_tracks_changes`.

## Lists

| Keys | Word's result | Test |
|---|---|---|
| `* item` Enter Enter `text` | the second Enter ends the list; `text` is a **Normal** paragraph (no list indent left behind) | `enter_twice_after_a_list_item_ends_the_list`, `the_paragraph_after_a_list_is_normal_text` |
| … Enter ×3, ×4 | each further Enter adds a blank Normal paragraph (never swallowed) | `more_enters_after_a_list_make_blank_paragraphs` |
| Enter on an empty **nested** item | moves it up one level; only a top-level empty item leaves the list | `enter_on_an_empty_nested_item_moves_it_up_a_level_first` |
| `* ` Enter (nothing typed) | list cancelled, Normal paragraph | `enter_right_after_starting_a_list_cancels_it` |
| Tab / Shift+Tab at the start of an item | demote / promote | `shift_tab_and_tab_change_list_levels` |
| Backspace at the start of an item | 1st: number removed, indent kept; 2nd: Normal paragraph — **not** joined to the one above | `backspace_at_the_start_of_an_item_removes_the_number_then_the_indent` |
| Enter mid-item / at item start | splits the item / inserts an empty item above | `enter_in_the_middle_or_at_the_start_of_an_item` |
| Shift+Enter in an item | line break inside the same item | `shift_enter_stays_inside_the_list_item` |
| Paste: Keep Text Only `A`, empty line, `B` into an empty item | three paragraphs `A`, empty, `B`: pasted paragraph breaks are text, not Enter (they never end the list) | `plain_text_paste_keeps_an_empty_paragraph_in_a_list` |
| `* ` `- ` `1. ` `1) ` `a. ` `a) ` `A. ` `i. ` `I. ` + text | bullet, dash bullet, 1. 1) a. a) A. i. I. lists | `list_autoformat_triggers` |
| `> ` `-> ` `=> ` + text | bulleted list with an arrow bullet (➢ ➔ ⇨; Word takes them from a symbol font, we use the Unicode arrows) ¹ | `arrow_bullets` |
| `1. ` after other text | a new list restarting at 1 (never continues an earlier list) | `typing_1_dot_starts_numbering_again` |
| ⌘Z right after an AutoFormat | undoes only the AutoFormat (`* ` comes back as text) | `undo_right_after_autoformat_undoes_only_the_autoformat` |

## Undo

| Keys | Word's result | Test |
|---|---|---|
| type, select some of it, type over it | two undo steps: the first ⌘Z brings the selected text back (still selected), the second removes the typing. Uninterrupted typing stays one step | `typing_then_selecting_then_typing_are_two_undo_steps` |
| ⌘B with nothing selected, ⌘Z, type | the text isn't bold (Undo takes the waiting caret formatting back too); ⌘Y brings it back | `undoing_bold_at_the_caret_turns_bold_typing_off_again` |

## Track Changes

| Keys | Word's result | Test |
|---|---|---|
| Enter (tracking on) | the new paragraph mark is a tracked insertion; Reject All joins the paragraphs again, Accept All keeps them | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::reject_all_removes_a_tracked_paragraph_break` |
| Paste of several paragraphs (tracking on) | every paragraph mark the paste adds is a tracked insertion; Reject All restores the one paragraph, Accept All keeps the pasted ones | `reject_all_takes_out_the_paragraph_breaks_a_tracked_paste_inserted` |
| Backspace at a paragraph start / Delete at a paragraph end (tracking on) | the paragraph mark is marked deleted (still shown); the caret moves before it (Backspace) or past it (Delete). Accept joins the paragraphs, Reject keeps them. A mark you inserted yourself is just removed | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::tracked_backspace_and_delete_mark_a_paragraph_break_deleted` |
| Delete a range from the end of one paragraph into a later one (tracking on) | the paragraph marks and text in the range are tracked deletions. Accept All removes exactly those, so the paragraph after the range stays a paragraph of its own; Reject All restores everything. Typing into an empty paragraph and rejecting leaves that paragraph | `tests::accept_all_after_a_tracked_delete_from_a_paragraph_end_keeps_the_next_paragraph`, `tests::reject_all_after_typing_into_an_empty_paragraph_keeps_the_paragraph` |
| Select All, then Delete or Backspace | the whole document goes, tables included, also when it starts with a table; one empty paragraph is left | `select_all_then_delete_removes_tables` |
| Table Layout › Delete Table / Rows / Columns / Cells (tracking on) | like Word deleting a selected table: the cells' text is marked deleted and the table structure stays (WordCraft has no tracked row or column deletion, `w:trPr/w:del`, so rows and columns keep their cells too, where Word's Accept removes the rows); text you inserted yourself is just removed; Reject All restores the text. A table, row or column with no text at all (empty, or its deletion already accepted) is removed as with tracking off; one whose text is all already marked deleted is left alone with a message (accept or reject first). Delete Columns leaves a merged cell that also spans a kept column alone (it narrows, keeping its text). A content control locked against deletion, or content locked against editing, refuses the delete with tracking on or off | `tests_revision_checks::tracked_table_delete_marks_the_text_deleted_and_reject_all_restores_it`, `tracked_row_column_and_cell_deletes_mark_their_text_deleted`, `tracked_table_delete_over_tracked_insertions_rejects_back_to_the_original`, `table_deletes_refuse_a_control_locked_against_deletion`, `tracked_delete_table_removes_a_table_with_no_live_text`, `delete_columns_leaves_a_merged_cell_that_keeps_a_column_alone`, `tracked_row_and_column_deletes_over_tracked_changes_reject_back_to_the_original` |
| Change Case (Shift+F3, Home › Aa) (tracking on) | Word shows no insertion or deletion markup (checked with Formatting markup set to none, so Word may record it as a hidden formatting revision). **Differs:** WordCraft records the changed letters as a tracked deletion of the old and insertion of the new, since case isn't a run property a formatting revision could hold; Reject All restores the old case, Accept All keeps the new. Your own tracked insertion is rewritten in place; already-deleted text is left alone and doesn't count (with tracking off too), so changing case again (or Shift+F3) works from the visible text. Refused in content locked against editing | `tests_revision_checks::tracked_change_case_rejects_back_to_the_original_case`, `tracked_change_case_of_own_insertion_stays_an_insertion`, `tracked_change_case_twice_reads_only_the_live_text`, `tracked_change_case_handles_sharp_s_and_mixed_formatting`, `change_case_refuses_content_locked_against_editing` |
| Unlink Fields (Ctrl+Shift+F9) (tracking on) | not tracked, like Word: the field's result stays as ordinary text. Refused in content locked against editing | `tests_revision_checks::tracked_unlink_keeps_the_result_as_plain_text`, `unlink_refuses_a_field_in_content_locked_against_editing` |

## Editing keys

Word's documented keyboard behaviour (Microsoft's shortcut list, feature names only); the
Windows/Linux keys are given first, macOS second.

| Keys | Word's result | Test |
|---|---|---|
| Down / Up (↓ / ↑) from a line beside a table | into the table's nearest row: the cell under the caret, or the nearest one when the caret is left or right of the cells' text (the start of the line above a table is left of the first cell's text); never past the table to the paragraph beyond it | `down_and_up_from_a_line_start_beside_a_table_go_into_it` |
| Alt+Shift+Up / Down (⌃⇧↑ / ⌃⇧↓) | the paragraphs the selection touches swap with the paragraph above / below, keeping style and list membership; the selection moves with them; one undo step. A selection ending at the start of a paragraph doesn't take it along. Refused at the top or bottom of the story or cell, next to a table and across a section break | `alt_shift_down_then_up_moves_a_paragraph_and_back_and_undoes` |
| Alt+Shift+Left / Right (⌃⇧← / ⌃⇧→) | Promote / Demote: a heading goes up or down one level (Heading 1 and Heading 9 are the ends); promoted body text becomes a heading at the level of the heading before it; a list item changes level like Shift+Tab / Tab | `alt_shift_left_and_right_promote_and_demote_headings_and_list_items` |
| Outline view: Alt+Shift+Up / Down, Left / Right | as above, but a collapsed heading takes what is hidden under it along: moving passes whole shown paragraphs with their hidden content; promote/demote shifts the hidden subheadings too | `outline_collapse_hides_descendants_and_expand_restores` |
| Outline view: Alt+Shift+Plus / Minus, Alt+Shift+1–9, Alt+Shift+A | Expand / Collapse the heading (one level at a time); Show Level 1–9; show all levels. The caret never stays in hidden text: it moves on to the next shown paragraph | `outline_show_level_filters_paragraphs` |
| Ctrl+Left / Right (⌥← / ⌥→); Home / End (⌘← / ⌘→) | one word left / right; the start / end of the line. On macOS ⌘← / ⌘→ go to the line's ends, as in every Mac text field, and ⌥ moves by word (#954); Shift extends the selection | `word_default_shortcuts_resolve` |
| Shift+F5 | Go Back: the caret cycles through the last four places edited, most recent first | `shift_f5_goes_back_through_the_last_edits` |
| Alt+X (no macOS default) | the hex code before the caret (up to six digits, `U+` optional) or the selected code becomes its character; otherwise the character before the caret becomes its code (`é` ↔ `00E9`); one undo step | `alt_x_toggles_between_a_character_and_its_code` |
| Shift+Backspace / Shift+Delete | Shift+Backspace erases like Backspace (#707). Shift+Delete cuts the selection to the clipboard and does nothing without one (Word for Windows, observed 2026-10-14). Windows delivers Shift+Delete as a Cut; macOS and Linux deliver the key, which did nothing (#1669) | `shift_delete_cuts_the_selection_and_shift_backspace_erases` |
| Alt+numpad code (Windows and Linux; not macOS) | hold Alt, type a decimal code, release Alt: the character appears at the caret as typed text (one undo step). With a leading 0 the code is the ANSI code page — Windows-1252 by default (`0233` → é, `0128` → €); without one, 1–255 is the OEM code page — 437 by default and in US English (`233` → Θ, `1` → ☺, `7` → •), 850 for Western European locales and English outside the US (`155` → ø; `233` is Ú there, per the CP850 mapping file — é is `130` in both). On Windows the system locale picks both pages (`text.typeCode` `ansi`/`oem`): Central European 1250/852 (Polish `0185` and `165` → ą), Cyrillic 1251/866, Greek 1253/737, Turkish 1254/857, and for codes up to 255 the character Windows composes itself replaces ours (it knows every code page, double-byte ones too); Linux keeps 1252 and 437/850. Above 255, with or without the 0, the Unicode character with that decimal value (`8364` → €). Control characters, 0, unassigned bytes and values past Unicode type nothing; auto-repeat of a held digit adds nothing, and switching windows drops a half-typed code. Word counts only keypad digits; egui reports keypad and top-row digits alike, so top-row digits with Alt alone compose codes too. An Alt code never opens KeyTips | `alt_numpad_codes_map_like_word_on_windows`, `alt_numpad_code_types_one_character`, ui-egui `alt_numpad_code_types_its_character_without_keytips` |
| Shift+Right (⇧→) at the end of a cell's text | the whole cell is selected first (its end-of-cell mark); the next Shift+Right selects the next cell with it. In an empty cell the first press selects the cell | `shift_right_at_the_end_of_a_cell_selects_the_cell_first` |

## AutoCorrect and AutoFormat as you type

Applied when a word is finished by a space, punctuation **or Enter** (`enter_finishes_a_word_for_autocorrect`):

- First letter of a sentence (paragraph or list item start, after `. ! ?`) is capitalised; not
  after abbreviations (`e.g.`, `Dr.`, …) or initials (`J.`), and not for mixed-case words
  (`iPhone`) or single-letter labels (`a.`). `teh`→`the`, `i`→`I`, `(c)`→`©`, `(tm)`→`™`,
  `wait...`→`wait…`, smart quotes (`autocorrect_as_you_type`).
- German text (the run's language, else its style's, the document default or the editing
  language, as for the replacement list, #735): smart quotes are „…“ and ‚…‘, in Swiss and
  Liechtenstein German «…» and ‹…›; a single quote after a letter or digit is the apostrophe ’
  (`geht’s`) unless it closes an open ‚. German abbreviations (`z.B.`, `bzw.`, `vgl.`, `ggf.`,
  `usw.`, …) and ordinal numbers (`3.`) don't end a sentence. Word itself was not observed for
  this; the quotes are the Duden's and the Swiss conventions
  (`smart_quotes_follow_the_texts_language`, `german_abbreviations_and_ordinals_dont_end_sentences`).
- Correct TWo INitial CApitals: a finished word of three or more letters with two capitals and
  the rest lowercase loses its second capital (`WOrd`→`Word`, `TEh`→`The`); not the exceptions
  (`IDs`, `CDs`, `PCs`, `TVs`, … plus the user's, `tools.autocorrect {"addInitialCapsException"}`)
  (`two_initial_capitals_are_corrected`).
- Correct accidental usage of cAPS LOCK: `wORD`→`Word` (three or more letters; not `iOS`,
  `mRNA`, …). Word also switches Caps Lock off; we only correct the word
  (`accidental_caps_lock_is_corrected`).
- Capitalize first letter of table cells: the first word typed in a cell is capitalised like a
  sentence start, under its own option (`first_letter_of_a_table_cell_is_capitalised`).
- Each fix is one undo step right after it, and each has its own switch (`twoInitialCaps`,
  `capsLock`, `capCells` on `tools.autocorrect`; on by default)
  (`capitalisation_fixes_undo_and_turn_off`).
- `1/2 1/4 3/4` → `½ ¼ ¾`; `1st 22nd 13th` get superscript suffixes (`ordinals_become_superscript`).
- `word -- word` → en dash, `word - word` → en dash, `word--word` → em dash (`dashes`).
- Web addresses become links; following text isn't linked (`web_addresses_become_links`).
- A hyperlink covers only its own text: typing at either end of it, or after Enter at its end,
  isn't linked; typing inside it is. Linking a whole paragraph doesn't link its mark, so the
  paragraphs Enter makes from it aren't links (#413; `a_link_covers_only_its_own_text`,
  `typing_at_either_end_of_a_link_is_not_linked`).
- A field covers only its own result: text typed or pasted with the caret just after a field
  (a Zotero citation at the end of a paragraph, the caret put there with a click) goes after
  the field, not into its result (#898; `typing_after_a_citation_goes_outside_its_field`).
- Enter after `---` `===` `***` `___` `~~~` `###` → a bottom border on the paragraph above
  (single, double, dotted, thick, wave, triple) (`border_line_autoformat`). ⌘Z right after brings the
  characters back and removes the border; the Enter stays (one undo step) ¹
  (`undo_right_after_a_border_line_brings_the_characters_back`).
- `+---+------+` Enter (plus signs around runs of hyphens, the whole paragraph) → a one-row
  table across the text width, each column as wide as its share of the hyphens; the caret goes
  into the first cell. ⌘Z right after brings the characters back. More than 63 columns stay
  text ¹ (`plus_and_hyphen_line_becomes_a_table`).
- Smileys `:)` `:-)` `:(` `:-(` `:|` → ☺ ☹ 😐, also straight after a word (`ok:)` → `ok☺`) ¹
  (`smileys_are_replaced_inside_words_too`).

- Names of days are capitalised (`monday` → `Monday`). AutoCorrect Options (#412,
  `tools.autocorrect`, saved per user) switch each of these off: replacements, sentence and
  table-cell capitals, days, smart quotes, fractions, ordinals, dashes, links, bulleted and
  numbered lists, border lines; user entries are added, replaced or deleted (built-in ones too),
  and "don't capitalize after" exceptions extend the abbreviation list. A replacement of several
  words takes a sentence capital on its first (`autocorrect_options_change_what_typing_does`).
- The built-in replacement list is English: its words (`teh`, `i`, `dont`…) are replaced only
  in English text (the run's `w:lang`, then its style, then the document default; text with no
  language at all is in the editing language, so with a Polish interface a `.docx` without
  `w:lang` is Polish). English is decided as proofing decides it (`ProofLang`: `en`, `eng`).
  Polish `Ala i kot` keeps its `i` ("and"); symbols (`(c)`, `->`, `1/2`) and the user's own
  entries apply in every language (#835; `autocorrect_replacement_list_follows_the_texts_language`).
  Croatian, Bosnian and Serbian `i` is kept only when the text is tagged with that language: those
  interfaces don't set an editing language yet. Per-language lists for other languages aren't
  there yet (#735).

¹ From Word's documented AutoFormat and AutoCorrect options; not yet keystroke-compared with
Word.
## Editing options (File › Options › Advanced)

| Keys / setting | Result | Test |
|---|---|---|
| Typing over a selection | replaces it (Word's default); with "Typing replaces selected text" off, the text goes in front of the selection, which stays | `typing_over_a_selection_follows_the_replace_selection_option` |
| Overtype on, typing | each character replaces the one after the caret, never the paragraph mark or an object; Insert switches overtype only with the Insert-key option on (off by default, as in Word) | `overtype_replaces_the_characters_after_the_caret` |

## Text direction

- Typing Arabic, Hebrew or another right-to-left script as the first letters of a paragraph
  whose direction nothing sets (not the paragraph, its style or the document defaults) makes it
  a right-to-left paragraph, so the caret starts at the right margin. A paragraph that already
  has letters keeps its direction, and so do opened documents: a paragraph without `w:bidi` stays
  left-to-right whatever its text (`typing_rtl_letters_into_an_empty_paragraph_makes_it_rtl`).
  Word takes the direction from the keyboard language instead; not yet compared side by side.

## Layout

- No line break right after a slash (`and/or`, URLs) unless the word can't fit a line
  (`layout::tests::no_line_break_right_after_a_slash`).

## Complex scripts

- Myanmar, Khmer and the Indic scripts are typed in memory order (consonant, medials, then
  vowel signs such as ေ, which is drawn before its consonant); the caret steps over whole
  syllables (Myanmar vowel signs and stacked consonants never split off), Backspace removes one
  character (vowel sign, medial, stacked consonant), Delete the whole syllable — the platforms'
  standard behaviour, not yet compared against Word (#604,
  `myanmar_typing_steps_by_syllable_and_backspaces_by_sign`,
  `indic_and_khmer_backspace_removes_one_character`).

## Known gaps

- Word's body font Aptos isn't installed outside Office; our substitute (Carlito, else Helvetica
  Neue) is wider, so lines wrap earlier than in Word. Needs a metric-matched open substitute.
- Moving paragraphs with Track Changes on isn't recorded as a tracked move; whole table rows
  don't move yet (a selection across cells is refused). Go Back's places aren't mapped through
  later edits (they are clamped to the document).
- Not yet compared: keystroke-by-keystroke comparison of Correct TWo INitial CApitals, cAPS LOCK
  and table-cell capitals (implemented from Word's AutoCorrect options; Word's exception list
  not compared); the rows marked ¹ above.

## How to compare

- WordCraft: the headless `ui_shot` example (no window, no focus) with `UI_SHOT_FULL=1` prints
  the whole `document.inspect`; drive it with `ui.text` / `ui.key` lines so keys take the real
  input path.
- Word: the machine is shared. Prefer AppleScript that doesn't take focus (open, read
  paragraphs, export PDF). Keystroke runs steal focus and race the person at the keyboard: keep
  them short, read the document back to verify, and ask first. Screenshots of Word's window only,
  under `plan/word/screenshots/`.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | minor | German text: smart quotes „…“ and ‚…‘ (Swiss «…» and ‹…›), apostrophe ’ after a letter; German abbreviations and ordinal numbers don't end a sentence (#735) |
| 2026-10-14 | trivial | Shift+Delete cuts the selection, and does nothing without one, on macOS and Linux too, as in Word for Windows (#1669); Shift+Backspace already erased (#707) |
| 2026-10-13 | trivial | Select All then Delete or Backspace removes a table the document starts with (#1198) |
| 2026-10-12 | trivial | Editing keys: Down and Up from the start of a line beside a table go into it instead of skipping it (#1007) |
| 2026-10-12 | trivial | Sample-text formulas on Enter (#1067), exact matching, bounded parameters, original English and one-step undo |
| 2026-10-13 | minor | Alt+numpad decimal character codes: the ANSI code page with a leading 0, the OEM page without (the system locale's on Windows: 1250–1254, 437/737/850/852/857/866), Unicode above 255 (#961) |
| 2026-10-13 | trivial | Fields: text typed or pasted just after a field (a Zotero citation) goes outside it (#898) |
| 2026-10-14 | trivial | Track Changes: a table delete over text already marked deleted changes nothing, so Reject All still restores it (#1329 review 2) |
| 2026-10-14 | trivial | Track Changes: empty tables are removed under tracking; Delete Columns leaves merged cells that keep a column; repeated Change Case reads only live text (#1329 review) |
| 2026-10-14 | trivial | Track Changes: table deletes mark the cells' text deleted and keep the table; Change Case is a tracked delete+insert (differs from Word's hidden formatting revision); Unlink Fields stays untracked; all three refuse locked content controls (#1329) |
| 2026-10-12 | trivial | Track Changes: Accept All after a tracked delete from a paragraph end keeps the next paragraph; Reject All after typing into an empty paragraph keeps it (#864) |
| 2026-10-12 | trivial | macOS: ⌘← / ⌘→ go to the line's start and end instead of moving by word (#954) |
| 2026-10-12 | minor | AutoCorrect: Correct TWo INitial CApitals, accidental cAPS LOCK, capitalize first letter of table cells (#544) |
| 2026-10-12 | trivial | Editing keys: Shift+Right at the end of a cell's text selects the cell first, then the next cell with it (#971) |
| 2026-10-11 | minor | Text direction: right-to-left letters typed into an empty paragraph make it right-to-left; existing paragraphs keep theirs (#713) |
| 2026-10-12 | trivial | AutoCorrect: untagged text follows the editing language; English decided as proofing does (#835 review) |
| 2026-10-11 | trivial | Grammar proofing (not AutoCorrect) no longer suggests capitalising a sentence after `...` or `…` (`proof::tests::ellipses_do_not_start_sentences`) |
| 2026-10-12 | minor | AutoCorrect: the English replacement list applies only to English (or untagged) text; Polish `i` stays (#835) |
| 2026-10-11 | minor | AutoFormat tables from `+---+`, arrow bullets, smileys inside words, undo after a border line (#546) |
| 2026-10-11 | trivial | Outline view keys: expand/collapse, Show Level, moving and promoting with collapsed content (#493) |
| 2026-10-11 | minor | Editing options: typing replaces selection, overtype and the Insert key (#485) |
| 2026-10-11 | minor | Pasted paragraph breaks: tracked under Track Changes (#417), kept in a list by Keep Text Only (#422) |
| 2026-10-11 | minor | Complex scripts: caret by syllable, Backspace by character (#604) |
| 2026-10-11 | minor | Undo rows: a selection ends the typing group (#419); Undo of Bold at the caret restores unbolded typing (#424) |
| 2026-10-11 | minor | Names of days capitalised; AutoCorrect Options (user entries, deleted built-ins, exceptions, every AutoCorrect/AutoFormat switch) change typing (#412) |
| 2026-10-11 | minor | Editing keys: Move Paragraph Up/Down, Promote/Demote, Go Back (Shift+F5), Alt+X (#343) |
| 2026-10-11 | minor | Hyperlinks: text typed at either end of a link or after Enter stays outside it (#413) |
| 2026-10-10 | trivial | Status line, revision history and cross-links added (progress-docs standard) |
| 2026-10-10 | minor | Track Changes rows: tracked Enter, Backspace and Delete on paragraph marks (#244) |
| 2026-10-10 | major | First version: lists, AutoCorrect and AutoFormat as you type, observed against Word (#204) |
