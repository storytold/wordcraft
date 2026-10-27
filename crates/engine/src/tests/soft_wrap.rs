//! The caret at a soft line wrap. Where a line wraps after text (Chinese, a word broken because
//! it is wider than the line), the end of that line and the start of the next are the same
//! position in the text; the caret can still be put at either, with End, Shift+End, the arrow
//! keys, Up/Down and a click, in a paragraph and in a table cell, whatever the alignment. A line
//! that wraps at a space keeps its end before the space.

use serde_json::json;
use wordcraft_doc::Pos;
use wordcraft_layout::Placed;
use wordcraft_layout::para::{ClKind, LineEnd};

use crate::{Selection, Session};

/// Run command `id` with params `v`; a failing command fails the test.
fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Chinese, a word too long for the line, Chinese with Latin letters and digits, and words
/// wrapping at spaces.
const TEXTS: [(&str, &str); 4] = [
    ("chinese", "春风拂过山岗，溪水绕着村庄静静流淌。"),
    ("long word", "Lanternsdriftingoverquietharbours"),
    ("mixed", "春风ABC拂过123山岗，溪水xyz绕着村庄。"),
    ("spaces", "Quiet lanterns drift along the harbour at dusk. "),
];

const ALIGNS: [&str; 4] = ["para.alignLeft", "para.alignCenter", "para.alignRight", "para.justify"];

/// A narrow page with one paragraph of `text` (repeated so it wraps over several lines) aligned
/// by `align`, in a one-cell table when `cell`; the caret is at its start.
fn session(text: &str, align: &str, cell: bool) -> Session {
    let mut d = wordcraft_doc::Document::new();
    d.last_section.page_w = 200.0;
    d.last_section.margin_left = 20.0;
    d.last_section.margin_right = 20.0;
    let mut s = Session::new(d);
    if cell {
        run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    }
    run(&mut s, "text.insert", json!({"text": text.repeat(4)}));
    run(&mut s, align, json!({}));
    let start = Pos { off: 0, ..s.sel.focus.clone() };
    s.sel = Selection::caret(start);
    s
}

/// One line of the caret's paragraph: its byte range, the start of its last character and where
/// to click on it (page, the x of that character's right half, and the line's middle).
#[derive(Clone, Debug)]
struct WrapLine {
    start: usize,
    stop: usize,
    last: usize,
    wraps_at_space: bool,
    click: (usize, f32, f32),
}

/// The lines of the paragraph at `at`, and its length.
fn lines(s: &mut Session, at: &Pos) -> (Vec<WrapLine>, usize) {
    let l = s.layout();
    let mut out = Vec::new();
    let mut len = 0;
    for (pi, page) in l.pages.iter().enumerate() {
        for it in &page.items {
            let Placed::Lines { story, path, para, l0, l1, x, y, .. } = it else { continue };
            if *story != at.story || *path != at.path {
                continue;
            }
            len = para.text_len;
            let first = &para.lines[*l0];
            for li in *l0..*l1 {
                let line = &para.lines[li];
                let last = &para.clusters[line.c1 - 1];
                let (x0, x1) = (para.x_of(li, last.start).unwrap(), para.x_of(li, line.stop).unwrap());
                let mid = y + (line.top - first.top) + line.height / 2.0;
                out.push(WrapLine {
                    start: line.start,
                    stop: line.stop,
                    last: last.start,
                    wraps_at_space: line.end == LineEnd::Wrap && last.kind == ClKind::Space,
                    click: (pi, x + x0 + (x1 - x0) * 0.75, mid),
                });
            }
        }
    }
    (out, len)
}

/// The position at byte `off` of the caret's paragraph.
fn at(s: &Session, off: usize) -> Pos {
    Pos { off, ..s.sel.focus.clone() }
}

/// Put the caret at byte `off` of its paragraph, with no goal column left from an earlier move.
fn put(s: &mut Session, off: usize) {
    s.sel = Selection::caret(at(s, off));
    s.goal_x = None;
}

/// Run `check` on every text, alignment and placement (in a table cell or not), with the
/// paragraph's lines and length; each paragraph wraps over at least three lines.
fn each_case(check: impl Fn(&str, &mut Session, &[WrapLine], usize)) {
    for (name, text) in TEXTS {
        for align in ALIGNS {
            for cell in [false, true] {
                let mut s = session(text, align, cell);
                let p = s.sel.focus.clone();
                let (lines, len) = lines(&mut s, &p);
                let case = format!("{name}, {align}, cell {cell}");
                assert!(lines.len() > 2, "{case}: the paragraph wraps ({lines:?})");
                check(&case, &mut s, &lines, len);
            }
        }
    }
}

/// End goes after the line's last character, and Home from there back to the line's start: the
/// caret is on that line, not at the start of the next one (the same position in the text).
/// Shift+End selects the whole line. Before a space the line wraps at, the end stays where it was.
#[test]
fn end_goes_after_the_last_character_of_a_wrapped_line() {
    each_case(|case, s, lines, len| {
        for (i, line) in lines.iter().enumerate() {
            let last_line = i + 1 == lines.len();
            let end = if last_line {
                len
            } else if line.wraps_at_space {
                line.last
            } else {
                line.stop
            };
            put(s, line.start);
            run(s, "caret.end", json!({}));
            assert_eq!(s.sel.focus.off, end, "{case}: End on line {i} {line:?}");
            run(s, "caret.home", json!({}));
            assert_eq!(s.sel.focus.off, line.start, "{case}: Home after End on line {i}");
            run(s, "caret.end", json!({"extend": true}));
            assert_eq!(s.sel.ordered(), (at(s, line.start), at(s, end)), "{case}: Shift+End on line {i}");
        }
    });
}

/// Right from before a wrapped line's last character goes after it, at that line's end; the next
/// Right goes past the next line's first character. Left from there comes back to the start of
/// that line, and the next Left goes before the last character of the line above. Right over the
/// space a line wraps at still goes to the start of the next line.
#[test]
fn arrows_step_over_a_soft_wrap_one_character_at_a_time() {
    each_case(|case, s, lines, _| {
        for (i, pair) in lines.windows(2).enumerate() {
            let (line, next) = (&pair[0], &pair[1]);
            if line.wraps_at_space {
                put(s, line.last);
                run(s, "caret.right", json!({}));
                run(s, "caret.home", json!({}));
                assert_eq!(s.sel.focus.off, next.start, "{case}: Right over the space at the end of line {i} goes to the next line");
                continue;
            }
            put(s, line.last);
            run(s, "caret.right", json!({}));
            assert_eq!(s.sel.focus.off, line.stop, "{case}: Right onto the end of line {i}");
            run(s, "caret.home", json!({}));
            assert_eq!(s.sel.focus.off, line.start, "{case}: the caret was on line {i}, not the next");
            put(s, line.last);
            run(s, "caret.right", json!({}));
            run(s, "caret.right", json!({}));
            let after_first = s.sel.focus.off;
            assert!(after_first > next.start && after_first <= next.stop, "{case}: second Right goes into line {}", i + 1);
            run(s, "caret.left", json!({}));
            assert_eq!(s.sel.focus.off, next.start, "{case}: Left to the start of line {}", i + 1);
            run(s, "caret.end", json!({}));
            assert!(s.sel.focus.off > next.start, "{case}: the caret was at the start of line {}, not the end of line {i}", i + 1);
            put(s, after_first);
            run(s, "caret.left", json!({}));
            run(s, "caret.left", json!({}));
            assert_eq!(s.sel.focus.off, line.last, "{case}: second Left goes before the last character of line {i}");
            put(s, line.last);
            run(s, "caret.right", json!({"extend": true}));
            assert_eq!(s.sel.ordered(), (at(s, line.last), at(s, line.stop)), "{case}: Shift+Right selects the last character of line {i}");
        }
    });
}

/// Down from the end of a wrapped line goes to the next line (not the one after it), and Up comes
/// back to the end of the line it started from (before the space, for a line that wraps at one).
#[test]
fn up_and_down_from_the_end_of_a_wrapped_line_keep_to_the_next_line() {
    each_case(|case, s, lines, _| {
        for (i, pair) in lines.windows(2).enumerate() {
            let (line, next) = (&pair[0], &pair[1]);
            if line.wraps_at_space {
                put(s, line.start);
                run(s, "caret.end", json!({}));
                run(s, "caret.down", json!({}));
                let off = s.sel.focus.off;
                assert!(off >= next.start && off <= next.stop, "{case}: Down from the end of line {i} lands on line {} ({off})", i + 1);
                run(s, "caret.up", json!({}));
                assert_eq!(s.sel.focus.off, line.last, "{case}: Up comes back to before the space ending line {i}");
                continue;
            }
            put(s, line.start);
            run(s, "caret.end", json!({}));
            run(s, "caret.down", json!({}));
            let off = s.sel.focus.off;
            assert!(
                off > next.start && off <= next.stop.max(next.start + 1),
                "{case}: Down from the end of line {i} lands on line {} ({off})",
                i + 1
            );
            run(s, "caret.up", json!({}));
            assert_eq!(s.sel.focus.off, line.stop, "{case}: Up comes back to the end of line {i}");
            run(s, "caret.home", json!({}));
            assert_eq!(s.sel.focus.off, line.start, "{case}: after Up the caret is on line {i}");
        }
    });
}

/// A click on the right half of a wrapped line's last character puts the caret after it, at that
/// line's end; a click on the space a line wraps at puts it before the space.
#[test]
fn a_click_after_the_last_character_of_a_wrapped_line_stays_on_that_line() {
    each_case(|case, s, lines, _| {
        for (i, line) in lines.iter().enumerate().take(lines.len() - 1) {
            if line.wraps_at_space {
                let (page, x, y) = line.click;
                run(s, "caret.set", json!({"page": page, "x": x, "y": y}));
                assert_eq!(s.sel.focus.off, line.last, "{case}: click on the space ending line {i}");
                continue;
            }
            let (page, x, y) = line.click;
            run(s, "caret.set", json!({"page": page, "x": x, "y": y}));
            assert_eq!(s.sel.focus.off, line.stop, "{case}: click after the last character of line {i}");
            run(s, "caret.home", json!({}));
            assert_eq!(s.sel.focus.off, line.start, "{case}: the click put the caret on line {i}");
        }
    });
}

/// A double-click on the right half of a wrapped line's last character selects that character's
/// word or sentence, as one on its left half does, not the next line's.
#[test]
fn a_double_click_after_the_last_character_selects_its_word() {
    each_case(|case, s, lines, _| {
        for (i, line) in lines.iter().enumerate().take(lines.len() - 1) {
            for unit in ["select.word", "select.sentence"] {
                put(s, line.last);
                run(s, unit, json!({}));
                let want = s.sel.ordered();
                let (page, x, y) = line.click;
                run(s, "caret.set", json!({"page": page, "x": x, "y": y}));
                run(s, unit, json!({}));
                assert_eq!(s.sel.ordered(), want, "{case}: {unit} after the last character of line {i}");
            }
        }
    });
}

/// After Shift+End, Right (or Escape to the selection's end) leaves the caret at the line's end.
#[test]
fn collapsing_a_selection_to_a_line_end_keeps_the_caret_there() {
    each_case(|case, s, lines, _| {
        for (i, line) in lines.iter().enumerate().take(lines.len() - 1) {
            if line.wraps_at_space {
                continue;
            }
            for collapse in [("caret.right", json!({})), ("select.collapse", json!({"end": true}))] {
                put(s, line.start);
                run(s, "caret.end", json!({"extend": true}));
                run(s, collapse.0, collapse.1);
                assert_eq!(s.sel.focus.off, line.stop, "{case}: {} after Shift+End on line {i}", collapse.0);
                run(s, "caret.home", json!({}));
                assert_eq!(s.sel.focus.off, line.start, "{case}: {} left the caret on line {i}", collapse.0);
            }
        }
    });
}

/// The caret at a line's end is that only while the text and the selection stay as they were:
/// after an edit (even one undone) or another selection, the same position is the next line's start.
#[test]
fn an_edit_or_another_selection_drops_the_line_end() {
    each_case(|case, s, lines, _| {
        let (line, next) = (&lines[0], &lines[1]);
        if line.wraps_at_space {
            return;
        }
        put(s, line.start);
        run(s, "caret.end", json!({}));
        let end = s.sel.focus.clone();
        s.sel = Selection::caret(at(s, 0));
        run(s, "select.collapse", json!({}));
        s.sel = Selection::caret(end.clone());
        run(s, "caret.home", json!({}));
        assert_eq!(s.sel.focus.off, next.start, "{case}: back at the same position after another selection, the caret starts the next line");
        put(s, line.start);
        run(s, "caret.end", json!({}));
        run(s, "text.insert", json!({"text": "x"}));
        run(s, "edit.undo", json!({}));
        assert_eq!(s.sel.focus, end, "{case}");
        run(s, "caret.home", json!({}));
        assert_eq!(s.sel.focus.off, next.start, "{case}: after an edit the caret starts the next line");
    });
}

/// A column (block) selection from the end of one wrapped line to the end of another takes those
/// lines and the ones between, not the line after each.
#[test]
fn a_column_selection_between_line_ends_takes_those_lines() {
    each_case(|case, s, lines, _| {
        if lines[..3].iter().any(|l| l.wraps_at_space) {
            return;
        }
        let ((fp, fx, fy), (tp, tx, ty)) = (lines[0].click, lines[2].click);
        run(s, "select.column", json!({"from": {"page": fp, "x": fx, "y": fy}, "to": {"page": tp, "x": tx, "y": ty}}));
        let segs = s.column_segments().unwrap_or_default().to_vec();
        assert_eq!(segs.len(), 3, "{case}: {segs:?}");
        for (seg, line) in segs.iter().zip(lines) {
            assert!(seg.0.off >= line.start && seg.1.off <= line.stop, "{case}: {seg:?} on {line:?}");
        }
    });
}

/// Column mode started at the end of a wrapped line (Ctrl+Shift+F8 there) keeps that line when
/// the arrow keys grow the block: the corner it started at stays at the end of its line.
#[test]
fn column_mode_started_at_a_line_end_keeps_that_line() {
    each_case(|case, s, lines, _| {
        if lines[..2].iter().any(|l| l.wraps_at_space) {
            return;
        }
        put(s, lines[0].start);
        run(s, "caret.end", json!({}));
        run(s, "select.column", json!({"on": true}));
        run(s, "caret.down", json!({}));
        let segs = s.column_segments().unwrap_or_default().to_vec();
        assert_eq!(segs.len(), 2, "{case}: {segs:?}");
        for (seg, line) in segs.iter().zip(lines) {
            assert!(seg.0.off >= line.start && seg.1.off <= line.stop, "{case}: {seg:?} on {line:?}");
        }
    });
}

/// Right in a column one character wide (each line wraps after its only character) goes from the
/// end of each line to the end of the next, one character at a time.
#[test]
fn right_in_a_column_one_character_wide_goes_from_line_end_to_line_end() {
    for cell in [false, true] {
        let mut d = wordcraft_doc::Document::new();
        d.last_section.page_w = 60.0;
        d.last_section.margin_left = 20.0;
        d.last_section.margin_right = 20.0;
        let mut s = Session::new(d);
        if cell {
            run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
        }
        run(&mut s, "text.insert", json!({"text": "春风拂过山岗溪水绕着村庄"}));
        let p = Pos { off: 0, ..s.sel.focus.clone() };
        let (lines, _) = lines(&mut s, &p);
        assert!(lines.len() > 4 && lines.iter().all(|l| !l.wraps_at_space), "cell {cell}: {lines:?}");
        put(&mut s, 0);
        run(&mut s, "caret.end", json!({}));
        for (j, line) in lines.iter().enumerate().take(lines.len() - 1).skip(1) {
            run(&mut s, "caret.right", json!({}));
            assert_eq!(s.sel.focus.off, line.stop, "cell {cell}: Right to the end of line {j}");
            run(&mut s, "caret.home", json!({}));
            assert_eq!(s.sel.focus.off, line.start, "cell {cell}: the caret was on line {j}");
            run(&mut s, "caret.end", json!({}));
        }
    }
}
