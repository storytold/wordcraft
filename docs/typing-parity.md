# Typing parity with Word

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (Move Paragraph Up/Down, Promote/Demote, Go Back and Alt+X keys, #343) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

What Word does when you type, observed black-box (same keystrokes typed into Word and
WordCraft, results read back paragraph by paragraph) and pinned by tests in
`crates/engine/src/tests_typing.rs`. When typing behaviour changes, change it here and in the
tests together. Line breaking and pagination are in [`layout-parity.md`](layout-parity.md); the
overall picture is in [`target-app-parity.md`](target-app-parity.md).

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
| `* ` `- ` `1. ` `1) ` `a. ` `a) ` `A. ` `i. ` `I. ` + text | bullet, dash bullet, 1. 1) a. a) A. i. I. lists | `list_autoformat_triggers` |
| `1. ` after other text | a new list restarting at 1 (never continues an earlier list) | `typing_1_dot_starts_numbering_again` |
| ⌘Z right after an AutoFormat | undoes only the AutoFormat (`* ` comes back as text) | `undo_right_after_autoformat_undoes_only_the_autoformat` |

## Track Changes

| Keys | Word's result | Test |
|---|---|---|
| Enter (tracking on) | the new paragraph mark is a tracked insertion; Reject All joins the paragraphs again, Accept All keeps them | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::reject_all_removes_a_tracked_paragraph_break` |
| Backspace at a paragraph start / Delete at a paragraph end (tracking on) | the paragraph mark is marked deleted (still shown); the caret moves before it (Backspace) or past it (Delete). Accept joins the paragraphs, Reject keeps them. A mark you inserted yourself is just removed | `tracked_enter_and_backspace_track_the_paragraph_mark`, `tests::tracked_backspace_and_delete_mark_a_paragraph_break_deleted` |

## Editing keys

Word's documented keyboard behaviour (Microsoft's shortcut list, feature names only); the
Windows/Linux keys are given first, macOS second.

| Keys | Word's result | Test |
|---|---|---|
| Alt+Shift+Up / Down (⌃⇧↑ / ⌃⇧↓) | the paragraphs the selection touches swap with the paragraph above / below, keeping style and list membership; the selection moves with them; one undo step. A selection ending at the start of a paragraph doesn't take it along. Refused at the top or bottom of the story or cell, next to a table and across a section break | `alt_shift_down_then_up_moves_a_paragraph_and_back_and_undoes` |
| Alt+Shift+Left / Right (⌃⇧← / ⌃⇧→) | Promote / Demote: a heading goes up or down one level (Heading 1 and Heading 9 are the ends); promoted body text becomes a heading at the level of the heading before it; a list item changes level like Shift+Tab / Tab | `alt_shift_left_and_right_promote_and_demote_headings_and_list_items` |
| Shift+F5 | Go Back: the caret cycles through the last four places edited, most recent first | `shift_f5_goes_back_through_the_last_edits` |
| Alt+X (no macOS default) | the hex code before the caret (up to six digits, `U+` optional) or the selected code becomes its character; otherwise the character before the caret becomes its code (`é` ↔ `00E9`); one undo step | `alt_x_toggles_between_a_character_and_its_code` |

## AutoCorrect and AutoFormat as you type

Applied when a word is finished by a space, punctuation **or Enter** (`enter_finishes_a_word_for_autocorrect`):

- First letter of a sentence (paragraph or list item start, after `. ! ?`) is capitalised; not
  after abbreviations (`e.g.`, `Dr.`, …) or initials (`J.`), and not for mixed-case words
  (`iPhone`) or single-letter labels (`a.`). `teh`→`the`, `i`→`I`, `(c)`→`©`, `(tm)`→`™`,
  `wait...`→`wait…`, smart quotes (`autocorrect_as_you_type`).
- `1/2 1/4 3/4` → `½ ¼ ¾`; `1st 22nd 13th` get superscript suffixes (`ordinals_become_superscript`).
- `word -- word` → en dash, `word - word` → en dash, `word--word` → em dash (`dashes`).
- Web addresses become links; following text isn't linked (`web_addresses_become_links`).
- Enter after `---` `===` `***` `___` `~~~` `###` → a bottom border on the paragraph above
  (single, double, dotted, thick, wave, triple) (`border_line_autoformat`).

## Layout

- No line break right after a slash (`and/or`, URLs) unless the word can't fit a line
  (`layout::tests::no_line_break_right_after_a_slash`).

## Known gaps

- Word's body font Aptos isn't installed outside Office; our substitute (Carlito, else Helvetica
  Neue) is wider, so lines wrap earlier than in Word. Needs a metric-matched open substitute.
- `:)` inside a word (`ok:)`) isn't replaced (Word replaces it).
- Moving paragraphs with Track Changes on isn't recorded as a tracked move; whole table rows
  don't move yet (a selection across cells is refused). Go Back's places aren't mapped through
  later edits (they are clamped to the document).
- Not yet compared: `+---+---+` Enter (table AutoFormat), `> ` arrow bullets, undo after a
  border line, Correct TWo INitial CApitals.

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
| 2026-10-10 | minor | Editing keys: Move Paragraph Up/Down, Promote/Demote, Go Back (Shift+F5), Alt+X (#343) |
| 2026-10-10 | trivial | Status line, revision history and cross-links added (progress-docs standard) |
| 2026-10-10 | minor | Track Changes rows: tracked Enter, Backspace and Delete on paragraph marks (#244) |
| 2026-10-10 | major | First version: lists, AutoCorrect and AutoFormat as you type, observed against Word (#204) |
