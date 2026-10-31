# Typing parity with Word

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-29 · **Change:** trivial (Relative font-size commands preserve mixed sizes in the selection (#4903); existing button steps and one-point shortcuts are unchanged; readiness estimates unchanged); previously trivial (highlight at a caret leaves the word alone, #4086); previously trivial (Arabic Backspace removes one combining mark and keeps its base letter (#4757); previously trivial: Thai WTT 2.0 input-sequence rules: Reject drops an invalid combining mark silently; Replace swaps same-class marks in-place; DeleteThenAppend reorders [cons, vowel, tone]+ำ to canonical [cons, tone, ำ]; rules skipped when selection non-collapsed or Track Changes on (#4399); previously trivial: `*Bold* and _italic_ as real formatting` in AutoFormat As You Type; previously trivial: highlight target includes selected paragraph marks, #4085; previously trivial: full-target highlight check beyond 10,000 runs, #4085; previously trivial: queued IME replacements are rejected before mutation after their marked composition loses its body/equation target; previously trivial: the transactional equation delete proves its commit with the engine revision; previously trivial: a refused byte-counted equation deletion drops only the immediately adjacent stale replacement, so a later direct commit still applies; previously trivial: a tracked own-insertion shifts the rest of a two-sided IME deletion; previously trivial: AutoCorrect/AutoFormat classification compares the actual paragraph; previously trivial: a replaced body starts a new input-method context and interrupts the composition; a quote the engine leaves as typed is the input method's own change; equation `DeleteSurrounding` erases counted characters; previously trivial: AutoCorrect finds the word after a no-break space or an ideographic space; previously trivial: a command that changes nothing is not an edit: no undo step, no unsaved changes, no relayout, and the redo stack survives it; previously trivial: while an input method composes, the caret and its candidate window sit at the input method's cursor inside the composition; previously trivial: Ctrl+Tab types a tab in a table cell, #3137; previously trivial: List AutoFormat triggers in front of an already-written paragraph, not only a fresh empty one, checked in Word for Windows, #3352; previously trivial: AltGr types its characters without opening KeyTips on Linux, #2388; previously trivial: Delete on whole selected cells leaves one empty paragraph in each, with the cell's last paragraph's formatting; nested tables go too, #2802; previously trivial: Bullets and Numbering toggle off style-inherited lists at their effective level, including basedOn chains and numbered ListParagraph styles; other paragraphs and list definitions stay unchanged (related to #3459); previously trivial: a text box that keeps the keyboard no longer resets the input method every frame, #2986; previously trivial: a slash opening a word travels whole to the next line, #2963; previously trivial: typing over a selection takes the first selected character's formatting, also when the selection was made right to left, #2879; previously trivial: Turkish and Azerbaijani case mapping in AutoCorrect, #1923; previously trivial: smart quotes in Hebrew pinned: logical order, ” in acronyms as in Word, #1670; previously trivial: Backspace on a selected table, rows or columns deletes them, Delete clears them, #1844; previously trivial: typed text takes its direction from its own letters, #1543; previously trivial: an input method's request to erase text around the caret is honoured, so web typing in Cyrillic and other non-Latin text no longer repeats, #1480; previously trivial: `-->` AutoCorrect replacement, #2223; previously trivial: a heading-list preset keeps Heading styles and makes other paragraphs Heading 1, as in Word, #1793; previously trivial: numbered headings: a heading-linked Multilevel List numbers new headings by their style; numbering keeps a heading a heading, #1793; previously trivial: reversing an automatic capital adds an exception, #1857; previously trivial: smart quotes open after guillemets and low quotes and nest; previously trivial: Enter on an empty list item numbered by its paragraph style moves it up a level, #2143; previously trivial: Backspace after a section break and Delete before it remove the break, also on the last paragraph, #2127; previously trivial: a table inserted in right-to-left text is right to left, so Tab moves left, #2634; previously trivial: Shift+Delete cuts the selection on every platform, #1669; previously trivial: Select All then Delete removes a table the document starts with, #1198; previously trivial: Alt+numpad character codes, #961; previously trivial: Down and Up from a line start beside a table go into it, #1007; previously trivial: sample-text formulas on Enter, #1067; previously trivial: Track Changes: Accept All after a tracked delete from a paragraph end keeps the next paragraph; Reject All after typing into an empty paragraph keeps it (#864); previously minor: Correct TWo INitial CApitals, cAPS LOCK and table-cell capitals, #544; previously trivial: complex-script caret and Backspace, #604; Move Paragraph Up/Down, Promote/Demote, Go Back and Alt+X keys, #343; the body IME deletion maps signed placeholder growth/shrink, keeps control markers outside the range and scopes its undo join to the matching replacement) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

What Word does when you type, observed black-box (same keystrokes typed into Word and
WordCraft, results read back paragraph by paragraph) and pinned by tests in
`crates/engine/src/tests/typing.rs`. When typing behaviour changes, change it here and in the
tests together. Line breaking and pagination are in [`layout-parity.md`](layout-parity.md); the
overall picture is in [`target-app-parity.md`](target-app-parity.md).

## Highlight shortcut (#4085)

On Windows/Linux, Ctrl+Alt+H toggles the last chosen highlighter colour (yellow in a
new session). If every character in the formatting target already has that colour,
it applies explicit No Color; otherwise it applies the chosen colour to the target.
Choosing another colour applies it, and choosing No Color makes that the current choice.
Explicit `format.highlight {"color": ...}` remains an idempotent setter; omitting `color`
uses the toggle. Ribbon and mini-toolbar colour buttons remain setters, not toggles.
The highlight decision checks the complete target, including mixed colours after 10,000 runs;
formatting-state queries retain their existing sampling limit. Fully selected paragraph marks
(including whole cells and empty paragraphs) participate in that decision; partial text
columns ignore marks outside the selected range. Palette state is outside document undo; the formatting itself is undoable.
Highlighting formats the selection only: with a caret, even inside a word, it sets the
colour for what is typed next and leaves the word alone, as in Word (#4086; Bold and other
character formats still take the word at the caret).

Regressions: `highlight_shortcut_toggles_the_last_colour` and
`highlight_toggle_handles_mixed_inherited_and_pending_formatting`. Synthetic command/key
qualification, with additional large-selection, non-mutating preview, whole-cell mark,
empty-paragraph mark and partial-column controls. Native Word comparison has not been
performed for this change.

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

Bullets and Numbering decide whether to toggle off from the selected paragraphs' effective numbering, including numbering inherited from a paragraph style or its `basedOn` chain. An explicit `numId=0` override remains unlisted until a list is applied. The effective list level and any level override determine the list kind, including an explicitly requested gallery kind. Toggling preserves custom styles, including a ListParagraph style that supplies numbering, list definitions and paragraphs outside the selection. A directly applied list still clears the automatic ListParagraph style. Original command and DOCX round-trip tests cover this narrow case related to #3459; it is not a new Word UI measurement. Tests: `toggle_style_inherited_list_off_preserves_other_paragraphs_and_round_trips`, `explicit_list_off_override_is_not_toggled_off_again`, `list_toggle_uses_effective_level_and_override`, `list_paragraph_style_survives_inherited_toggle_off`, `direct_list_toggle_still_clears_the_automatic_list_paragraph_style`.

| Keys | Word's result | Test |
|---|---|---|
| `* item` Enter Enter `text` | the second Enter ends the list; `text` is a **Normal** paragraph (no list indent left behind) | `enter_twice_after_a_list_item_ends_the_list`, `the_paragraph_after_a_list_is_normal_text` |
| … Enter ×3, ×4 | each further Enter adds a blank Normal paragraph (never swallowed) | `more_enters_after_a_list_make_blank_paragraphs` |
| Enter on an empty **nested** item | moves it up one level; only a top-level empty item leaves the list. The same when the list comes from the paragraph style (List Number 2 in a Word file, #2143) | `enter_on_an_empty_nested_item_moves_it_up_a_level_first`, `enter_on_an_empty_item_numbered_by_its_style_moves_it_up_a_level` |
| `* ` Enter (nothing typed) | list cancelled, Normal paragraph | `enter_right_after_starting_a_list_cancels_it` |
| Tab / Shift+Tab at the start of an item | demote / promote | `shift_tab_and_tab_change_list_levels` |
| Tab with a whole paragraph (or several paragraphs) selected | indents the paragraph(s) by one default tab stop; the text stays. A partial selection inside one paragraph is replaced by a tab character | `tab_with_a_whole_paragraph_selected_indents_it`, `tab_with_a_partial_selection_still_inserts_a_tab` |
| Backspace at the start of an item | 1st: number removed, indent kept; 2nd: Normal paragraph — **not** joined to the one above | `backspace_at_the_start_of_an_item_removes_the_number_then_the_indent` |
| Enter mid-item / at item start | splits the item / inserts an empty item above | `enter_in_the_middle_or_at_the_start_of_an_item` |
| Shift+Enter in an item | line break inside the same item | `shift_enter_stays_inside_the_list_item` |
| Paste: Keep Text Only `A`, empty line, `B` into an empty item | three paragraphs `A`, empty, `B`: pasted paragraph breaks are text, not Enter (they never end the list) | `plain_text_paste_keeps_an_empty_paragraph_in_a_list` |
| `* ` `- ` `1. ` `1) ` `a. ` `a) ` `A. ` `i. ` `I. ` + text | bullet, dash bullet, 1. 1) a. a) A. i. I. lists | `list_autoformat_triggers` |
| `- ` typed at the start of an already-written paragraph (checked in Word for Windows, 2026-10-21) | still becomes a list item; the rest of the paragraph's text is kept after it, not only when the trigger is the whole paragraph | `list_autoformat_triggers_in_front_of_existing_text` |
| `> ` `-> ` `=> ` + text | bulleted list with an arrow bullet (➢ ➔ ⇨; Word takes them from a symbol font, we use the Unicode arrows) ¹ | `arrow_bullets` |
| `1. ` after other text | a new list restarting at 1 (never continues an earlier list) | `typing_1_dot_starts_numbering_again` |
| Heading 1 `One`, Heading 2 `Two`, Heading 1 `Three`, the caret in `Three`, Multilevel List › the "1 / 1.1 / 1.1.1" heading list, then at the end Enter, Heading 2, `Four` Enter `Body` | One = 1, Two = 1.1, Three = 2, styles kept; the new Heading 2 is numbered 2.1 by itself (the heading styles carry the list); `Body` (Normal) stays unnumbered | `a_new_heading_after_a_heading_linked_list_is_numbered` |
| Numbering on a Heading 1 | "1. One", still Heading 1 | `numbering_a_heading_keeps_the_heading_style` |
| The "1 / 1.1 / 1.1.1" heading list with a caret or a selection | paragraphs already in a Heading style keep their style and level; every other paragraph style (Normal, Quote, Title…) becomes Heading 1. One H1 / Body / Two H2 → 1, Body H1 "2", Two H2 "2.1"; Two H2 + Three H3 with no Heading 1 above → 1.1, 1.1.1; caret in a Heading 2 → stays Heading 2 "1.1"; Normal alone → Heading 1 "1" (checked in Word for Mac, 2026-10-16) | `a_heading_preset_keeps_headings_and_makes_other_paragraphs_heading_1`, `a_heading_preset_on_a_lone_normal_paragraph_makes_it_heading_1` |
| Undo after a heading list | one step: paragraph styles and the heading styles' numbers come back | `undo_takes_a_heading_preset_back_in_one_step` |
| A paragraph whose numbering was switched off, then Heading 2 from a heading-linked list | numbered (1.1) by the style | `applying_a_numbered_heading_style_numbers_the_paragraph` |
| `1. ` typed in a Heading 1 (also `a) ` in a Heading 2, `1. ` in a Subtitle) | numbered, the style kept; only body text becomes List Paragraph (checked in Word for Mac, 2026-10-15) | `typing_1_dot_in_a_heading_keeps_the_heading_style` |
| `---` or `+--+--+` Enter in a heading numbered by a heading-linked list, at the top of the document or below another paragraph | stays text, with no border or table, and the heading keeps its number, as in a paragraph numbered directly (WordCraft's rule; not compared with Word) | `border_and_table_lines_stay_text_in_a_style_numbered_heading` |
| ⌘Z right after an AutoFormat | undoes only the AutoFormat (`* ` comes back as text) | `undo_right_after_autoformat_undoes_only_the_autoformat` |

## Undo

| Keys | Word's result | Test |
|---|---|---|
| type, select some of it, type over it | two undo steps: the first ⌘Z brings the selected text back (still selected), the second removes the typing. Uninterrupted typing stays one step | `typing_then_selecting_then_typing_are_two_undo_steps` |
| ⌘B with nothing selected, ⌘Z, type | the text isn't bold (Undo takes the waiting caret formatting back too); ⌘Y brings it back | `undoing_bold_at_the_caret_turns_bold_typing_off_again` |
| ⌫ at the start of the document, or typing nothing | nothing happens: no undo step is added, the document is not marked edited, and the redo stack survives ³ | `backspace_at_the_start_of_the_document_is_not_an_edit`, `a_command_that_changes_nothing_is_not_an_edit`, `a_no_op_command_keeps_the_redo_stack` |
| ⌫ right after a floating shape's anchor at the paragraph start (In Front of Text, Square…) | joins the paragraph above and keeps the shape; the anchor is not a character to delete (checked in Word for Windows, #4592) | `backspace_after_a_floating_shape_anchor_joins_paragraphs_and_keeps_the_shape` |

³ Not re-observed side by side in Word for this change: Word's undo list does not grow when a
key changes nothing, and the document is not marked edited. The gap was found by the read-only
audit, which measured WordCraft's own history.

## Paste

| Keys | Word's result | Test |
|---|---|---|
| Paste (⌘V) of plain text with several lines from another program, in the middle of a right-to-left paragraph | pasted as Keep Text Only pastes it: every line becomes a paragraph formatted like the paragraph it lands in (direction, style, alignment, indents, list), and so does the text after the caret. One ⌘Z takes the whole paste out ² | `plain_text_paste_gives_every_line_the_formatting_of_its_paragraph` |
| … with empty lines, or a line break at the end | each empty line is an empty paragraph with that formatting too, in a list an empty item. The text takes the formatting before the caret (bold there makes it bold) ² | `plain_text_paste_keeps_empty_lines_as_paragraphs`, `plain_text_paste_keeps_an_empty_paragraph_in_a_list` |

² From Microsoft's description of the paste options: Word offers no options for text without
formatting, and Keep Text Only gives text the style of the paragraph where it is pasted and the
formatting of the text before the cursor. Not yet compared side by side in Word.

## Track Changes

| Keys | Word's result | Test |
|---|---|---|
| Enter (tracking on) | the new paragraph mark is a tracked insertion; Reject All joins the paragraphs again, Accept All keeps them | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::reject_all_removes_a_tracked_paragraph_break` |
| Paste of several paragraphs (tracking on) | every paragraph mark the paste adds is a tracked insertion; Reject All restores the one paragraph, Accept All keeps the pasted ones | `reject_all_takes_out_the_paragraph_breaks_a_tracked_paste_inserted` |
| Backspace at a paragraph start / Delete at a paragraph end (tracking on) | the paragraph mark is marked deleted (still shown); the caret moves before it (Backspace) or past it (Delete). Accept joins the paragraphs, Reject keeps them. A mark you inserted yourself is just removed | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::tracked_backspace_and_delete_mark_a_paragraph_break_deleted` |
| Delete a range from the end of one paragraph into a later one (tracking on) | the paragraph marks and text in the range are tracked deletions. Accept All removes exactly those, so the paragraph after the range stays a paragraph of its own; Reject All restores everything. Typing into an empty paragraph and rejecting leaves that paragraph | `tests::accept_all_after_a_tracked_delete_from_a_paragraph_end_keeps_the_next_paragraph`, `tests::reject_all_after_typing_into_an_empty_paragraph_keeps_the_paragraph` |
| Backspace just after a section break / Delete just before it | the break goes and the two paragraphs join; the text before it takes the following section's page setup (Word's documented behaviour: a break holds the setup of the section before it); headers and footers the following section showed by linking to the previous one stay. Delete at the end of the last paragraph removes a break on its mark too, with no paragraph to join (with Track Changes on it still does nothing there). Undo brings the break back. Breaks read from a .docx and breaks inserted here alike (#2127) | `backspace_and_delete_remove_a_section_break`, `delete_removes_a_section_break_on_the_last_paragraph` |
| Select All, then Delete or Backspace | the whole document goes, tables included, also when it starts with a table; one empty paragraph is left | `select_all_then_delete_removes_tables` |
| Backspace / Delete with a whole table selected (Select Table, or every cell dragged over) | Backspace removes the table, like Delete Table (one undo step; with tracking on its text is marked deleted and the structure stays); the caret goes to the paragraph after it. Delete clears the cells and keeps the table (each cell as below). A nested table goes on its own and leaves the cell's paragraph; a table that was the only block leaves one empty paragraph | `backspace_with_the_whole_table_selected_removes_the_table`, `delete_with_the_whole_table_selected_clears_the_cells_and_keeps_the_table`, `backspace_on_a_selected_nested_table_removes_only_it`, `backspace_on_a_selected_table_that_is_the_only_block_leaves_an_empty_paragraph`, `tracked_backspace_on_a_selected_table_marks_its_text_deleted_like_delete_table` |
| Backspace / Delete with a table and the paragraph before or after it selected | both remove the table, whatever end the selection starts in; the text around joins. Any table the selection covers whole goes, at any nesting level | `delete_or_backspace_with_the_table_and_the_paragraph_after_selected_removes_the_table`, `delete_or_backspace_with_the_paragraph_before_and_the_table_selected_removes_the_table` |
| Backspace / Delete with whole rows or columns selected | Backspace deletes the rows or columns (Delete Rows / Delete Columns); Delete clears their cells. One selected cell is just that cell, also in a one-row or one-column table: Backspace clears it. A cleared cell is left with one empty paragraph, however many it had, with the formatting of its last paragraph (whose mark ends the cell); a table nested in it goes too (checked against Word 2019 for Windows, #2802). With Track Changes on, Delete still marks the cells' text deleted as before. **Differs:** in a 1×1 table, Select Cell then Backspace removes the table (WordCraft has no end-of-row mark to tell the cell from the whole table; the end-of-row caret position is still missing, #1844) | `backspace_deletes_selected_rows_and_delete_clears_them`, `backspace_deletes_selected_columns_and_delete_clears_them`, `backspace_on_one_selected_cell_of_a_one_row_or_one_column_table_clears_it`, `delete_on_whole_cells_leaves_one_empty_paragraph_in_each`; `delete_range` edge cases: `edit::tests::a_range_ending_at_the_end_of_a_cell_that_isnt_the_last_keeps_the_table` and five more (other tables, nested tables, story ends, headers and footnotes, content controls, nesting past the paragraph walk) |
| Table Layout › Delete Table / Rows / Columns / Cells (tracking on) | like Word deleting a selected table: the cells' text is marked deleted and the table structure stays (WordCraft has no tracked row or column deletion, `w:trPr/w:del`, so rows and columns keep their cells too, where Word's Accept removes the rows); text you inserted yourself is just removed; Reject All restores the text. A table, row or column with no text at all (empty, or its deletion already accepted) is removed as with tracking off; one whose text is all already marked deleted is left alone with a message (accept or reject first). Delete Columns leaves a merged cell that also spans a kept column alone (it narrows, keeping its text). A content control locked against deletion, or content locked against editing, refuses the delete with tracking on or off | `tests::revision_checks::tracked_table_delete_marks_the_text_deleted_and_reject_all_restores_it`, `tracked_row_column_and_cell_deletes_mark_their_text_deleted`, `tracked_table_delete_over_tracked_insertions_rejects_back_to_the_original`, `table_deletes_refuse_a_control_locked_against_deletion`, `tracked_delete_table_removes_a_table_with_no_live_text`, `delete_columns_leaves_a_merged_cell_that_keeps_a_column_alone`, `tracked_row_and_column_deletes_over_tracked_changes_reject_back_to_the_original` |
| Change Case (Shift+F3, Home › Aa) (tracking on) | Word shows no insertion or deletion markup (checked with Formatting markup set to none, so Word may record it as a hidden formatting revision). **Differs:** WordCraft records the changed letters as a tracked deletion of the old and insertion of the new, since case isn't a run property a formatting revision could hold; Reject All restores the old case, Accept All keeps the new. Your own tracked insertion is rewritten in place; already-deleted text is left alone and doesn't count (with tracking off too), so changing case again (or Shift+F3) works from the visible text. Refused in content locked against editing | `tests::revision_checks::tracked_change_case_rejects_back_to_the_original_case`, `tracked_change_case_of_own_insertion_stays_an_insertion`, `tracked_change_case_twice_reads_only_the_live_text`, `tracked_change_case_handles_sharp_s_and_mixed_formatting`, `change_case_refuses_content_locked_against_editing` |
| Unlink Fields (Ctrl+Shift+F9) (tracking on) | not tracked, like Word: the field's result stays as ordinary text. Refused in content locked against editing | `tests::revision_checks::tracked_unlink_keeps_the_result_as_plain_text`, `unlink_refuses_a_field_in_content_locked_against_editing` |

> **Three-column preset update:** 2026-10-15 · `blankThree` headers/footers start the caret in the first of three table cells (#2197). `text.tab` reaches the next cell and `text.insert` edits it through the existing command path. `crates/engine/tests/it/three_column_headers.rs` checks cell navigation, independent formatting, preset/text undo/redo and repeated DOCX save/reopen.

## Editing keys

Word's documented keyboard behaviour (Microsoft's shortcut list, feature names only); the
Windows/Linux keys are given first, macOS second.

| Keys | Word's result | Test |
|---|---|---|
| select text right to left (from its end), type | the new text takes the first selected character's formatting (font, size…), as with a selection made left to right; not the text's before it (#2879) | `typing_over_backward_selection_keeps_its_font` |
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
| AltGr (Right Alt on Polish, German, Czech… layouts) | AltGr+letter types the layout's character (AltGr+S → ś, AltGr+L → ł) and a bare AltGr tap does nothing; neither opens KeyTips. On Linux AltGr is the Right Alt key with Level 3 Shift, not Alt, so its modifiers event no longer reads as Alt let go (#2388); on Windows it is Ctrl+Alt. Right Alt on a layout without AltGr (US) is Alt and opens KeyTips like Left Alt | ui-egui `altgr_types_its_characters_without_keytips` |
| Shift+Right (⇧→) at the end of a cell's text | the whole cell is selected first (its end-of-cell mark); the next Shift+Right selects the next cell with it. In an empty cell the first press selects the cell | `shift_right_at_the_end_of_a_cell_selects_the_cell_first` |

## AutoCorrect and AutoFormat as you type

The `-->` replacement produces `→` when completed with a space (#2223), including
at paragraph start without creating a list. It respects AutoCorrect preferences, user
entries and undo. The existing `-> ` paragraph-start arrow bullet is unchanged.

Applied when a word is finished by a space, punctuation **or Enter** (`enter_finishes_a_word_for_autocorrect`).
The word starts after the last white-space character before it, also when that is a no-break space or an
ideographic space (`autocorrect_finds_the_word_after_a_wide_space`):

- First letter of a sentence (paragraph or list item start, after `. ! ?`) is capitalised; not
  after abbreviations (`e.g.`, `Dr.`, …) or initials (`J.`), and not for mixed-case words
  (`iPhone`) or single-letter labels (`a.`). `teh`→`the`, `i`→`I`, `(c)`→`©`, `(tm)`→`™`,
  `wait...`→`wait…`, smart quotes (`autocorrect_as_you_type`); a quote opens after a space,
  an opening bracket, a dash, `«` `‹` `„` `‚`, or the other kind of opening quote, so nested
  quotes curl correctly (`smart_quotes_nested_and_opening_delimiters`).
  Smart quotes in Hebrew (and other right-to-left text) are chosen from the characters around
  the quote in logical order, whatever the display direction: `"` after a space or at the start
  is “ (U+201C) even though it shows on the right of the word, after a letter ” (U+201D); in an
  acronym (`צה"ל`) it becomes ” as in Word for Mac 16.113.4, not gershayim (״); `'` after a letter
  (`ג'ירפה`) is ’ as after any letter, with no geresh conversion (not yet checked in Word; `smart_quotes_in_hebrew_use_logical_order`, #1670).
- Add a period with double-space (File › Options › Proofing › AutoCorrect Options › AutoFormat As
  You Type, `tools.autocorrect {"doubleSpacePeriod"}`; **off by default**, as macOS's system switch
  is a per-user choice): a second Space after a letter, a digit or a closing bracket or quote turns
  the first into a period (`Hello  world`→`Hello. world`). Other punctuation before the spaces,
  a lone space, the start of a paragraph, a tab, a selection typed over and a third space are left
  as typed; Undo puts the two spaces back (`double_space_adds_a_period_when_asked`, #2848). Not
  observed in Word; Word for Mac follows the macOS setting.
- Correct TWo INitial CApitals: a finished word of three or more letters with two capitals and
  the rest lowercase loses its second capital (`WOrd`→`Word`, `TEh`→`The`); not the exceptions
  (`IDs`, `CDs`, `PCs`, `TVs`, … plus the user's, `tools.autocorrect {"addInitialCapsException"}`)
  (`two_initial_capitals_are_corrected`).
- Reversing an automatic capital makes an exception (Word's "Automatically add words to list",
  observed on the Mac, #1857): Undo of the sentence capital's own step adds the word before it
  to the First Letter list (`there.`; only words ending in a full stop, the one form seen in
  Word), and so does Backspacing over the capitalised word and typing it again; the word is then
  not capitalised after `there.` any more. Any other Undo (of later typing, of a paste) adds
  nothing, and the capital is forgotten once another Undo runs, a new document is opened, or the
  undo history is cleared. Each list has its own switch
  (`tools.autocorrect {"autoAddFirstLetter", "autoAddInitialCaps"}`, on by default); words over
  255 characters or with spaces are never added. Undoing a replacement (`teh`→`the`) adds
  nothing: Word doesn't either. Undoing a TWo INitial CApitals fix adds the word to the INitial
  CAps list (as in Word, checked on the Mac: `TWo ` typed again isn't corrected; same switch rule as First Letter). The first word of a
  paragraph has no word before it, so nothing is added there; Word also left the retyped first
  word ("hello") lowercase in that paragraph, which WordCraft doesn't (paragraph-level
  suppression, only for the paragraph where the Undo happened: Word capitalises the first word
  of a new paragraph again; a separate follow-up). Word adds nothing in bulleted-list paragraphs; not covered
  (`undoing_a_sentence_capital_adds_a_first_letter_exception`,
  `backspacing_a_sentence_capital_and_retyping_adds_a_first_letter_exception`,
  `automatic_exceptions_can_be_turned_off`, `undoing_a_replacement_adds_no_exception`,
  `undoing_an_initial_caps_fix_adds_an_initial_caps_exception`,
  `undoing_other_edits_after_a_capital_adds_no_exception`,
  `a_full_undo_history_does_not_match_later_undos`, `a_new_document_forgets_the_capital`,
  `an_overlong_word_is_not_added_as_an_exception`).
- Correct accidental usage of cAPS LOCK: `wORD`→`Word` (three or more letters; not `iOS`,
  `mRNA`, …). Word also switches Caps Lock off; we only correct the word
  (`accidental_caps_lock_is_corrected`).
- Capitalize first letter of table cells: the first word typed in a cell is capitalised like a
  sentence start, under its own option (`first_letter_of_a_table_cell_is_capitalised`).
- Each fix is one undo step right after it, and each has its own switch (`twoInitialCaps`,
  `capsLock`, `capCells` on `tools.autocorrect`; on by default)
  (`capitalisation_fixes_undo_and_turn_off`).
- `1/2 1/4 3/4` → `½ ¼ ¾`; `1st 22nd 13th` get superscript suffixes (`ordinals_become_superscript`).
- With `*Bold* and _italic_ as real formatting` on (`tools.autocorrect {"boldItalic": true}`;
  off by default), `*word*` becomes bold `word` and `_word_` italic `word`, without the marks,
  when the word is finished ¹. The rest of this item is WordCraft's own rule: the marks may
  enclose several words; WordCraft converts a span only when the opening mark starts a word and
  the text inside holds a letter or digit and does not start or end with a space, so `2*3*`,
  `file_name_` and `*?!*` stay as typed, and so do marks around a picture, field or content
  control. AutoCorrect then goes on with the word without its marks, so `*teh* ` becomes bold
  `The `, and it corrects the first word of a span of several words as it is typed, so
  `*teh end* ` becomes bold `The end `. Undo right after brings the marks
  back (after Enter, the Enter stays). With Track Changes on, WordCraft converts only marks the
  current author typed as tracked insertions, and it converts nothing while formatting is
  limited to the document's styles (`bold_and_italic_marks_become_formatting`,
  `bold_and_italic_marks_stay_as_typed_unless_they_enclose_words`,
  `autocorrect_goes_on_after_bold_and_italic_marks`).
- `word -- word` → en dash, `word - word` → en dash, `word--word` → em dash (`dashes`).
- Web addresses become links; following text isn't linked (`web_addresses_become_links`).
- A hyperlink covers only its own text: typing at either end of it, or after Enter at its end,
  isn't linked; typing inside it is. Linking a whole paragraph doesn't link its mark, so the
  paragraphs Enter makes from it aren't links (#413; `a_link_covers_only_its_own_text`,
  `typing_at_either_end_of_a_link_is_not_linked`).
- A field covers only its own result: text typed or pasted with the caret just after a field
  (a Zotero citation at the end of a paragraph, the caret put there with a click) goes after
  the field, not into its result (#898; `typing_after_a_citation_goes_outside_its_field`).
- With Track Changes on, the border-line and table AutoFormats below mark the characters
  deleted, so Reject All brings them back; characters the same author inserted are removed.
  The border is a tracked formatting change, at the top of a story too
  (`a_tracked_border_line_is_a_deletion_and_a_formatting_change`); the table is inserted
  untracked, as Insert › Table inserts one
  (`a_tracked_table_line_keeps_its_characters_as_a_deletion`). A line holding a tracked
  deletion stays as it is, so Enter after the deleted characters is a plain Enter. This is
  WordCraft's own rule.
- Enter after `---` `===` `***` `___` `~~~` `###` → a bottom border on the paragraph above
  (single, double, dotted, thick, wave, triple) (`border_line_autoformat`). ⌘Z right after brings the
  characters back and removes the border; the Enter stays (one undo step) ¹
  (`undo_right_after_a_border_line_brings_the_characters_back`).
- Built-in Heading styles (`tools.autocorrect {"headingStyles": true}`, off by default): Enter
  twice after a line of five or fewer words that does not end with punctuation makes the line
  Heading 1; one or two tabs before it make it Heading 2 or 3, and the tabs are removed ¹
  (`two_enters_after_a_short_line_make_it_a_heading`). The rest is WordCraft's own rule: the
  line is a Normal paragraph of the main text (not in a list, a table, a header or a text box)
  and ends in a letter or digit; the caret stays in the empty paragraph after the heading; ⌘Z
  right after brings back the tabs and the Normal style, and the second Enter stays
  (`undo_right_after_a_heading_autoformat_keeps_the_enter`); with Track Changes on, the tabs are
  marked deleted (or removed, when the same author inserted them) and the style change is a
  tracked formatting change, and a line holding a tracked deletion stays as it is
  (`a_tracked_heading_autoformat_can_be_rejected`); a heading style locked by formatting
  restrictions is not applied (`a_locked_heading_style_is_not_applied_as_you_type`).
- `+---+------+` Enter (plus signs around runs of hyphens, the whole paragraph) → a one-row
  table across the text width, each column as wide as its share of the hyphens; the caret goes
  into the first cell. ⌘Z right after brings the characters back. More than 63 columns stay
  text ¹ (`plus_and_hyphen_line_becomes_a_table`).
- Set left- and first-indent with tabs and backspaces (`tools.autocorrect {"tabIndent": true}`,
  off by default): Tab before the first line of a paragraph with text indents its first line by
  one default tab stop, and Tab before any other line, after a line break or where the text
  wraps, indents the whole paragraph. A second Tab before an indented or hanging first line
  indents the whole paragraph. Backspace at the start takes the first-line indent back, then the
  paragraph's own left indent one tab stop at a time. An empty paragraph and Tab inside a line
  still take a tab character. While formatting is limited to the document's styles, Tab and
  Backspace work as with the option off ¹ (`tab_and_backspace_at_a_paragraph_start_set_its_indents`,
  `tab_before_a_later_line_of_a_paragraph_indents_the_whole_paragraph`).
- Smileys `:)` `:-)` `:(` `:-(` `:|` → ☺ ☹ 😐, also straight after a word (`ok:)` → `ok☺`) ¹
  (`smileys_are_replaced_inside_words_too`).

- Turkish, Azerbaijani and Crimean Tatar text (the word's language, else the editing language)
  maps `i` → `İ` and `ı` → `I` when a sentence or day is capitalised, and `I` → `ı` when
  TWo INitial CApitals / cAPS LOCK lowercase a letter (`istanbul` → `İstanbul`, `İNgilizce` →
  `İngilizce`, `ılık` → `Ilık`; observed in Word for Mac with Turkish proofing). Lowercasing `İ`
  gives the single letter `i` in every language, never `i` + combining dot. Other languages keep
  the Unicode default (`istanbul` → `Istanbul` in English)
  (`autocorrect_capitalisation_follows_turkic_case_mapping`); Format > Change Case follows the
  same Turkic mapping from each run's language, while other languages keep the Unicode default
  (`change_case_follows_turkic_case_mapping`, #3479).
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
## Spelling as you type (#1552)

Word draws no red squiggle under the word being typed. The squiggle comes when the word ends
(space, punctuation, Enter, Tab) or when the caret leaves it any other way (arrow keys, Home,
End, a click, a selection). Only the word being typed is exempt: clicking into a misspelled word
that is already there does not hide its squiggle, and typing into it makes it the word being
typed again. F7 and the Spelling pane report the word all the while. Grammar marks are not held
back.

| Keys | Result | Test |
|---|---|---|
| `The helo` | no squiggle under `helo`; `review.issues` and F7 (`review.spelling`) still list it | `the_word_being_typed_is_not_marked_until_it_ends` |
| `The helo` then space, `.`, Enter or Tab | `helo` underlined | same |
| `The helo` then Left, Home, Ctrl+Left, Ctrl+Home or Select All; End after that | `helo` underlined, and still underlined when the caret comes back | `moving_the_caret_out_of_the_typed_word_marks_it` |
| `The helo` in one paragraph, a click into another, a click back, `o` typed into it | underlined on leaving; still underlined on return ¹; typing into it hides the mark again until the next space ¹ | `a_click_elsewhere_marks_the_typed_word_and_a_click_back_keeps_the_mark` |
| `The helo ` then Backspace | the mark goes while the word is edited again ¹ | `backspace_into_a_finished_word_edits_it_again` |
| `I typed teh ` (AutoCorrect); a phrase by one `text.insert`; Undo | AutoCorrect's result is checked as usual; of inserted text only the word the caret ends in is exempt; Undo moves the caret, so what is left is marked | `autocorrect_and_inserted_text_are_marked_as_usual` |
| `It doesn'` (smart quotes make it `doesn’`); `The nonn-` then `exist` | no squiggle until the word ends: the apostrophe and the hyphen stay part of the word, as in the spelling check ¹ | `apostrophes_and_hyphens_stay_part_of_the_word_being_typed` |
| A click at the end of `helo`, then Insert, Delete with nothing after the caret, or Shift+Tab | `helo` stays underlined: only an edit to the text makes it the word being typed ¹ | `text_commands_that_edit_nothing_leave_an_existing_word_marked` |

¹ Follows the rule above; not yet keystroke-compared with Word.

The exemption is a layout input (`LayoutOptions::typing_word`, filtered in
`crates/layout/src/para.rs`). The Session records the word after a `text.*` command that changes
the caret's paragraph, and drops it once the caret is anywhere else or the word changed under it
(`crates/engine/src/session.rs`), so a caret move without an edit lays only that paragraph out
again.

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
- A table inserted in a right-to-left paragraph is a right-to-left table (its first cell at the
  right), as in Word, so Tab moves to the cell on the left (the next one in reading order) and
  Shift+Tab back; in a left-to-right table Tab still moves right (#2634,
  `tab_in_a_table_inserted_in_right_to_left_text_moves_left`).
- Ctrl+Tab (⌥Tab on macOS) types a tab character in a table cell instead of moving to the next
  cell (#3137, `ctrl_tab_inserts_a_tab_inside_a_table_cell`).
- Text typed or inserted (links, fields, symbols, pasted text with merged formatting) takes its
  direction from its own letters, not from the run around the caret, keeping only that run's
  bidi language: Latin typed into Hebrew from a reopened file is saved as left-to-right text, and
  the saved file marks the right-to-left pieces `w:rtl` itself (#1543,
  `latin_typed_into_reopened_hebrew_is_saved_left_to_right`). Known display difference: spaces,
  digits and punctuation typed into reopened Hebrew text draw in the Latin font, not the
  complex-script font, until the file is saved and reopened.

## Layout

- No line break right after a slash (`and/or`, URLs) unless the word can't fit a line
  (`layout::tests::no_line_break_right_after_a_slash`). A slash that opens a word after a
  space (`Kazakh /Russian` captions) travels with it: the break is before the slash, so
  `/заполняется` moves to the next line whole and is never split between its letters (#2963,
  `layout::tests::a_word_after_a_slash_wraps_whole`, also in a fixed table cell and not after a no-break space).

## Complex scripts

- Arabic Backspace removes the last stored nonspacing mark, including harakat and Quranic marks,
  without deleting its base letter (#4757). Multiple marks are removed in stored order, with no
  normalization or decomposition. Left/Right and forward Delete keep whole-cluster behavior;
  Latin accents, emoji and the existing Brahmic policy retain their behavior. Original synthetic
  command regressions cover deletion, replacement, Undo/Redo and controls
  (`arabic_backspace_removes_one_mark_and_keeps_the_base`,
  `arabic_backspace_uses_stored_mark_order_but_delete_keeps_the_cluster`). Native keyboard and
  Microsoft Word behavior have not been observed for this change.

- Myanmar, Khmer and the Indic scripts are typed in memory order (consonant, medials, then
  vowel signs such as ေ, which is drawn before its consonant); the caret steps over whole
  syllables (Myanmar vowel signs and stacked consonants never split off), Backspace removes one
  character (vowel sign, medial, stacked consonant), Delete the whole syllable — the platforms'
  standard behaviour, not yet compared against Word (#604,
  `myanmar_typing_steps_by_syllable_and_backspaces_by_sign`,
  `indic_and_khmer_backspace_removes_one_character`).
- Thai combining marks (vowels, tone marks, diacritics) follow the WTT 2.0 input-sequence rules
  within the current Thai Character Block (TCB). Three outcomes, applied only when the selection
  is collapsed and Track Changes is off:
  - **Reject** — an invalid mark for the current TCB state (e.g. a tone mark typed with no
    preceding consonant): the character is silently dropped; the document is unchanged
    (`thai_tcb_reject_drops_invalid_mark`).
  - **Replace** — a new mark of the same positional class replaces the existing one in-place
    (e.g. sara_uu replaces sara_u, a second tone mark replaces the first); the caret stays after
    the slot (`thai_tcb_replace_swaps_same_class_vowel`).
  - **DeleteThenAppend** — sara_am typed after [cons, vowel, tone]: the intervening vowel is
    deleted first, then ำ is appended at the adjusted position, yielding the canonical
    [cons, tone, ำ] order (`thai_tcb_delete_then_append_reorders_vowel_tone_am`).
  A non-collapsed selection follows the normal type-over-selection path. The document always
  stores canonical Unicode codepoints; PUA short-tail forms (U+F700, U+F70F) are render-only.

## Input methods

- An input method can ask to erase characters before and after the caret (`DeleteSurrounding`)
  and send replacement text. The web build's hidden text field does this whenever its contents
  change other than by adding to the end: Backspace and corrections from phone keyboards, and
  the Chrome caret jump below. The 0.37 desktop bridge can also deliver character- or
  byte-counted surrounding deletions from the input method. WordCraft erases those characters as Backspace erases a selection, stopping at the
  paragraph's ends, then types the new text. The new text keeps the erased text's formatting
  and never overtypes what follows. Erasing and retyping stay in the open typing undo step, and
  typing after them joins it too (#1480,
  `keys::tests::delete_surrounding_from_the_web_text_agent_does_not_repeat_text`,
  `delete_surrounding_erases_characters_on_both_sides_of_the_caret`,
  `delete_surrounding_stays_inside_the_paragraph`, `delete_surrounding_joins_the_typing_undo_step`,
  `delete_surrounding_keeps_the_replaced_texts_formatting`,
  `delete_surrounding_text_does_not_overtype`, `delete_surrounding_rewrite_ends_at_a_key_press`).
- After AutoCorrect or AutoFormat changes the text as you type, the web build tells its hidden
  text field to start afresh. Its later counts then can't reach past the change
  (`automatic_change_asks_the_input_method_to_start_afresh`). The reset itself happens in eframe
  and hasn't been checked in a browser.

- On the desktop, a text box that keeps the keyboard while its dialog is open (Search, Go To's
  page box, the Icons search, a new caption label) asks for focus only while it lacks it. egui
  resets the input method on every focus request, and on X11 a reset recreates the XIM input
  context, so asking every frame reset it every frame and could leave typing dead until a
  restart (#2986, `dialogs::tests::boxes_that_keep_focus_do_not_interrupt_ime_every_frame`).
- While an input method composes (Japanese, Chinese…), its marked text is drawn from the caret,
  and the caret and the candidate window sit at the input method's cursor inside it: the end of
  the range it reports, or the end of the marked text when it reports none. The range is clamped
  to the text (`canvas::tests::the_caret_follows_the_input_method_cursor_in_the_composition`).
- A composition belongs to the text it was started on. When a command replaces the body under
  the same document id — Replace Document Text, a version restore, Compare, Undo/Redo — or opens
  a new document, the body's replacement epoch advances, the platform clears the composition, and
  any marked text is dropped. Replacing the text with an identical one still starts a new
  context (`canvas::tests::ime_generation_changes_on_body_replacement_with_the_same_text`,
  `ime_generation_changes_on_undo_and_version_restore`,
  `keys::tests::observe_ime_drops_a_preedit_without_focus`). Undo/Redo also gives the equation
  being edited a fresh session token, so its context follows the restored tree. The marked text
  is never painted without focus.
- If replacing the body or changing the edited equation invalidates nonempty marked text,
  the next canvas input batch rejects that composition's queued surrounding deletions and one
  replacement commit before any engine mutation. An associated `Text` fallback is rejected too.
  A fresh nonempty preedit or ordinary typing ends that batch's suppression; without an old
  marked composition, direct commits remain eligible. A later commit after the rejected one,
  or in a later canvas batch, is also fresh input. New regression checks cover identical-body
  replacement, both deletion units, equation exit before or within the batch, unchanged text
  and Undo history, and fresh input (`keys::tests::ime_interrupted_body_preedit_drops_queued_replacement_before_mutation`,
  `ime_interrupted_equation_preedit_drops_queued_body_replacement`,
  `ime_interrupted_preedit_does_not_suppress_fresh_input`). Their managed validation is pending;
  this does not claim native input-method acceptance or identify untagged events across frames.
- An accepted IME commit earns an `InputMethod` receipt only when the engine inserted exactly the
  committed text. A single quote the engine leaves as typed (smart quotes off) is the input
  method's own change; a smart quote or AutoCorrect replacement is `Other`
  (`ime_apostrophe_with_smart_quotes_off_is_an_input_method_change`,
  `ime_apostrophe_with_smart_quotes_on_stays_other`,
  `ime_literal_commit_after_autoformat_is_input_method`). A `DeleteSurrounding` whose two sides
  cannot both be applied is rolled back whole and publishes no receipt
  (`a_refused_second_delete_surrounding_rolls_the_request_back`); every such refusal also
  interrupts the composition and clears the painted pre-edit and both replacement joins because
  the platform's hidden text already changed
  (`a_refused_body_delete_clears_preedit_and_joins`). Under Track Changes, deleting text this
  author inserted shortens the stored paragraph, so the following range and caret are shifted by
  the bytes actually removed (`tracked_delete_surrounding_shifts_past_its_own_insertion`,
  `tracked_own_insertion_then_a_refused_side_rolls_the_whole_request_back`). Emptying a content
  control restores its placeholder, which may make the paragraph longer; a body deletion maps the
  following range and caret by the signed stored-length change, and refuses the whole transaction
  when growth cannot be proven to come from a placeholder restored at or before the erased range
  (`delete_surrounding_maps_after_text_and_caret_past_a_growing_placeholder`,
  `delete_surrounding_maps_a_shrinking_placeholder_and_after_text`,
  `delete_surrounding_maps_a_placeholder_restored_by_the_after_side`). Control markers are
  zero-length visible objects, so a range start maps after the start marker and after the end
  marker: an ordinary IME deletion never breaks a control
  (`ime_text::tests::control_markers_are_not_included_in_delete_ranges`). The replacement's undo
  join is scoped to the next `text.insert` at the selection the deletion left; an intervening
  command owns its own step (`body_delete_join_does_not_leak_to_an_intervening_command`).
- Inside an equation, `DeleteSurrounding` erases the counted characters with the equation's own
  transactional command (at most 64 units a side), refusing the whole request if any step cannot
  delete — including a step that would only move the caret into another argument
  (`equation_delete_surrounding_refuses_a_mixed_caret_only_step`); the delete and the following
  commit are one undo group. Byte counts have no reversible mapping into the
  structured tree, so the whole request is refused, the composition is interrupted and the
  replacement the platform sends for that rejected deletion — the immediately adjacent `Commit`,
  or an adjacent `Text` + `Commit` pair — is dropped instead of being inserted beside the
  untouched equation (the pre-edit never lives in the model). Any later `Text`/`Commit` in the
  same raw-input batch is fresh input and applies, as is a later direct commit without a pre-edit;
  no persistent tombstone can swallow it. An over-cap request clears the painted pre-edit and any
  earlier deletion's replacement join before refusing. The equation's surroundings stay explicitly
  unavailable (`ime_equation_delete_is_one_input_method_change`,
  `ime_equation_delete_and_commit_are_one_undo_step`,
  `ime_equation_byte_delete_drops_only_its_immediate_replacement`,
  `over_cap_equation_delete_clears_the_painted_preedit`,
  `over_cap_equation_delete_clears_the_previous_replacement_join`). The engine's checked revision
  on the unavailable stamp proves a successful content change; a refusal, an automatic rewrite or
  an exhausted revision publishes no receipt (`ime_transformed_equation_commits_are_other`,
  `exhausted_ime_revisions_never_certify_equation_or_wide_body_edits`).
- Pre-edit and commit sizes are bounded (the visible window at 4000 bytes, a pre-edit at the same
  cap, a commit at 64 KiB): larger hostile events are ignored rather than painted or inserted
  (`oversized_ime_events_are_ignored`).

## Known gaps

- Web (#1480): Chrome has been seen moving the caret of eframe's hidden text field to its
  start after the first non-Latin character. Later letters then go in before what was typed
  since the field was last cleared, so `hello` then `мир` gives `риhelloм`. The text is no
  longer repeated, but the cause of the caret jump hasn't been found and isn't worked around.
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

## Relative font-size changes

Increase/Decrease Font Size and the one-point shortcuts transform each selected
run from its own resolved size, including paragraph and character style inheritance
(#4903). The existing button steps are unchanged: 20 pt and 10 pt become 22 pt and
10.5 pt with Increase Font Size; Grow Font 1 Point makes them 21 pt and 11 pt.
Both commands continue to set normal and complex-script sizes explicitly. Whole-body
selections visit the selected paragraph paths once, without repeated prefix scans.

Partial selections leave surrounding text and paragraph marks alone; fully selected
paragraphs include their marks. Column pieces, selected cells, text-box frames and
the existing word-at-caret/pending-typing targets retain their behavior. Each
command is one Undo step and still obeys editing protection and the 1–1638 pt
limits. Original-fixture engine and Linux CLI/headless UI save/reopen checks cover
this correction; native Windows interaction and Microsoft Word remain untested.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-29 | trivial | Relative font-size commands preserve mixed sizes in the selection (#4903); existing button steps and one-point shortcuts are unchanged; readiness estimates unchanged |
| 2026-10-28 | trivial | Highlighting at a caret leaves the word alone and sets the colour for what is typed next (#4086). |
| 2026-10-28 | trivial | Arabic Backspace removes one combining mark and keeps its base letter (#4757); stored mark order, caret/forward-Delete and other-script behavior retained; original synthetic command/history/protection/save-reopen checks, no native Word comparison; readiness estimates unchanged |
| 2026-10-26 | trivial | Thai WTT 2.0 input-sequence rules: Reject drops an invalid combining mark silently; Replace swaps same-class marks in-place; DeleteThenAppend reorders [cons, vowel, tone]+ำ to canonical [cons, tone, ำ]; rules skipped when selection non-collapsed or Track Changes on (#4399). |
| 2026-10-25 | trivial | Highlight decisions include fully selected paragraph marks in whole cells and ordinary ranges, including empty paragraphs; partial text columns exclude unselected marks (#4085). |
| 2026-10-25 | trivial | Highlight toggle checks the full target beyond 10,000 runs without collecting resolved formatting; mixed and inherited tails and Undo/Redo covered (#4085). |
| 2026-10-24 | trivial | Input methods: reject queued old-composition replacements before mutation after body/equation target replacement (new regression validation pending); equation `DeleteSurrounding` is one transactional command whose checked revision proves a receipt; a caret-only step in a mixed request refuses the whole deletion, and an over-cap request clears the painted pre-edit and the previous replacement join; a refused byte-counted equation deletion drops only the immediately adjacent stale replacement, never a later direct commit; a tracked own-insertion shifts the following range; a body deletion maps signed placeholder growth/shrink, keeps control markers outside the range and scopes its replacement undo join to the matching `text.insert`; refusals clear the painted pre-edit and joins; AutoCorrect/AutoFormat classification compares the paragraph as typed; equation replacement interrupts the composition |
| 2026-10-24 | trivial | AutoCorrect: a word typed after a no-break space or an ideographic space is corrected as after a plain space |
| 2026-10-24 | trivial | Highlight shortcut toggles the last chosen colour off when the whole selection has it; explicit colour commands remain setters (#4085). |
| 2026-10-24 | trivial | AutoFormat As You Type: `*Bold* and _italic_ as real formatting` turns `*word*` bold and `_word_` italic without the marks; off by default |
| 2026-10-23 | trivial | Input methods: a replaced body (Replace Document Text, version restore, Compare, Undo/Redo) starts a new context and interrupts the composition; smart quotes off keeps an IME quote an `InputMethod` change; equation `DeleteSurrounding` erases counted characters; bounded pre-edit/commit sizes; a partly refused two-sided deletion is rolled back |
| 2026-10-22 | trivial | Input methods: while composing, the caret and the candidate window follow the input method's cursor inside the marked text |
| 2026-10-21 | trivial | Bullets and Numbering toggle off style-inherited lists at their effective level, including basedOn chains and numbered ListParagraph styles; other paragraphs and list definitions stay unchanged (related to #3459) |
| 2026-10-21 | trivial | List AutoFormat (`- `, `1. `, …) triggers at the start of an already-written paragraph, not only a fresh empty one; checked in Word for Windows (#3352) |
| 2026-10-19 | trivial | Input methods: boxes that keep the keyboard ask for focus only while they lack it, so the input method isn't reset every frame (#2986) |
| 2026-10-19 | trivial | Delete on whole selected cells (rows, columns, the table, a cell) leaves one empty paragraph in each, with the cell's last paragraph's formatting; nested tables go too (#2802) |
| 2026-10-19 | trivial | Ctrl+Tab (⌥Tab on macOS) types a tab character in a table cell (#3137) |
| 2026-10-16 | trivial | AltGr on Linux (Wayland) types ś, ł, ż… instead of opening KeyTips: the modifiers event without Alt that follows the Right Alt key no longer reads as Alt let go, and a bare AltGr tap doesn't open KeyTips (#2388) |
| 2026-10-20 | trivial | AutoFormat As You Type: optional "Add a period with double-space" (off by default), undoable (#2848) |
| 2026-10-15 | trivial | Three-column header/footer preset: first-cell caret, Tab navigation, independent formatting and undo/redo regression coverage (#2197) |
| 2026-10-18 | trivial | Typing over a selection takes the first selected character's formatting, also when the selection was made right to left (#2879) |
| 2026-10-17 | trivial | A table inserted in a right-to-left paragraph is a right-to-left table, so Tab moves to the cell on the left (#2634) |
| 2026-10-15 | trivial | Backspace after a section break and Delete before it remove the break, also one on the last paragraph's mark; the text takes the following section's setup and keeps the headers it showed (#2127) |
| 2026-10-15 | trivial | Enter on an empty nested list item whose numbering comes from its paragraph style (List Number 2) moves it up a level instead of adding another item (#2143) |
| 2026-10-18 | trivial | Smart quotes open after `«` `‹` `„` `‚` and after the other kind of opening quote, so nested quotes curl correctly |
| 2026-10-15 | minor | AutoCorrect: undoing the capital's own step (or backspacing it and retyping) adds an exception; "Automatically add words to list" switches (#1857) |
| 2026-10-16 | trivial | #1793: a heading-list preset keeps paragraphs already in a Heading style at their level and makes other paragraphs Heading 1 (rechecked in Word for Mac); undo test |
| 2026-10-16 | trivial | #1793: a heading-linked Multilevel List preset makes every selected paragraph its first level (Heading 1), checked in Word for Mac; `1. ` in a heading checked in Word; a single Heading 2 becoming Heading 1 is unchecked |
| 2026-10-15 | trivial | Numbered headings (#1793): a heading-linked Multilevel List numbers headings through their styles, so a new Heading 2 after the list is numbered by itself; Numbering and `1. ` keep a heading a heading; applying a numbered heading style numbers the paragraph |
| 2026-10-15 | trivial | Add the missing `-->` → `→` AutoCorrect entry (#2223), with typing, undo and preference regression coverage; existing arrow bullets unchanged |
| 2026-10-15 | trivial | Input methods (#1480 review): the rewritten text keeps its formatting, doesn't overtype, keeps the typing undo group; AutoCorrect/AutoFormat reset the web text field; desktop doesn't receive `DeleteSurrounding` |
| 2026-10-15 | trivial | Input methods: `DeleteSurrounding` erases around the caret, so web typing in non-Latin text no longer repeats; Chrome's caret jump in the hidden field is listed as a known gap (#1480) |
| 2026-10-15 | minor | Backspace with a whole table, whole rows or whole columns selected deletes them; Delete clears them; a selection covering a table and the text beside it removes the table (#1844) |
| 2026-10-16 | trivial | Smart quotes in Hebrew follow logical order; ” (not gershayim) in acronyms, as Word does (#1670) |
| 2026-10-15 | trivial | Paste: plain text from another program pastes as Keep Text Only does. Every line takes the formatting of the paragraph it lands in (a right-to-left paragraph stays right-to-left), and empty lines stay empty paragraphs |
| 2026-10-15 | trivial | Spelling as you type: the word being typed, apostrophes and hyphens included, is not marked until the caret leaves it; F7 and the Spelling pane still report it (#1552) |
| 2026-10-14 | trivial | Shift+Delete cuts the selection, and does nothing without one, on macOS and Linux too, as in Word for Windows (#1669); Shift+Backspace already erased (#707) |
| 2026-10-13 | trivial | Select All then Delete or Backspace removes a table the document starts with (#1198) |
| 2026-10-12 | trivial | Editing keys: Down and Up from the start of a line beside a table go into it instead of skipping it (#1007) |
| 2026-10-12 | trivial | Sample-text formulas on Enter (#1067), exact matching, bounded parameters, original English and one-step undo |
| 2026-10-13 | minor | Alt+numpad decimal character codes: the ANSI code page with a leading 0, the OEM page without (the system locale's on Windows: 1250–1254, 437/737/850/852/857/866), Unicode above 255 (#961) |
| 2026-10-13 | trivial | Fields: text typed or pasted just after a field (a Zotero citation) goes outside it (#898) |
| 2026-10-15 | trivial | AutoCorrect: Turkish/Azerbaijani case mapping (i↔İ, ı↔I), no combining dot from İ (#1923) |
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
| 2026-10-10 | trivial | Status line, revision history and cross-links added |
| 2026-10-10 | minor | Track Changes rows: tracked Enter, Backspace and Delete on paragraph marks (#244) |
| 2026-10-10 | major | First version: lists, AutoCorrect and AutoFormat as you type, observed against Word (#204) |
| 2026-10-15 | trivial | Text direction: typed and inserted text takes its direction from its own letters, not the caret's run; spaces typed into reopened Hebrew draw in the Latin font until reopened (#1543) |

| 2026-10-27 | trivial | Backspace after a floating shape's anchor at a paragraph start joins paragraphs and keeps the shape (#4592) |