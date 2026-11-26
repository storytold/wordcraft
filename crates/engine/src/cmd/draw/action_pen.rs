//! Draw › Pens › Action Pen: the natural gestures a stroke makes on the text, as observed in
//! Word ("Edit your document with natural gestures"): a loop selects the words it encloses; a
//! line, diagonal or squiggle through text deletes the characters it crosses; a curve touching
//! two words joins them; a vertical line inside a word splits it; a caret between words puts the
//! insertion point there; a backwards L moves the text after the point to a new line.
//!
//! The recogniser is geometry only: pure functions over the stroke's page points and a
//! [`PageText`] (each line's characters on the page, built from the layout by [`page_text`]).
//! It never touches the document. Unrecognised or too-small strokes stay ink, which the caller
//! draws with `draw.stroke`. Every point list and coordinate is bounded before any work, so a
//! hostile command can't make recognition expensive or panic.
//!
//! Nothing here needs a model or the network: when a caret gesture has put the insertion point
//! down, the words are typed through the ordinary `text.insert` path (or the recogniser seam's
//! `text` parameter), which is what the app's keyboard and IME already use.

use serde_json::{Value, json};
use wordcraft_doc::{Path, Pos, StoryRef};
use wordcraft_geom::Rect;
use wordcraft_layout::DocLayout;

use crate::cmd::{delete_selection, pos_json, sel_result, split_para, type_text};
use crate::{CmdError, CmdResult, Selection, Session, p};

/// Most points a gesture is classified from (a longer stroke is sampled evenly).
pub const MAX_GESTURE_POINTS: usize = 2_048;
/// Most characters one page's text map keeps, and one line's, so recognition stays bounded.
const MAX_CELLS: usize = 4_000;
const MAX_ROW_CELLS: usize = 600;
/// Largest stroke coordinate kept, points.
const MAX_COORD: f32 = 1.0e5;
/// Smallest stroke (its longer side, points) that can be a gesture at all: a tap or tick stays ink.
const MIN_GESTURE: f32 = 8.0;
/// Smallest loop's longer side, points.
const MIN_LOOP: f32 = 16.0;
/// Longest a stroke may be compared with the chord between its ends before it is not a line.
const LINE_RATIO: f32 = 1.12;
/// Points either side of a stroke's vertical extent whose lines still join its map: a stroke on
/// a line's edge, or just above its text, meets that line.
const BAND_MARGIN: f32 = 8.0;

/// A gesture recognised in a stroke.
#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    /// Select the words the loop encloses.
    Select { a: Pos, b: Pos },
    /// Delete the characters the stroke crosses.
    Delete { a: Pos, b: Pos },
    /// Join the words at the stroke's two ends (the gap between them goes).
    Join { a: Pos, b: Pos },
    /// Split the word the vertical line is drawn in (a space goes in at `at`).
    Split { at: Pos },
    /// Put the insertion point between the words at the caret's vertex.
    Insert { at: Pos },
    /// Split the paragraph at `at`: the text after it starts a new line.
    NewLine { at: Pos },
}

impl Gesture {
    /// Its name, for command results and messages.
    pub fn name(&self) -> &'static str {
        match self {
            Gesture::Select { .. } => "select",
            Gesture::Delete { .. } => "delete",
            Gesture::Join { .. } => "join",
            Gesture::Split { .. } => "split",
            Gesture::Insert { .. } => "insert",
            Gesture::NewLine { .. } => "newLine",
        }
    }

    /// The range it selects, deletes or joins (a highlight while the stroke is drawn).
    pub fn range(&self) -> Option<(&Pos, &Pos)> {
        match self {
            Gesture::Select { a, b } | Gesture::Delete { a, b } | Gesture::Join { a, b } => Some((a, b)),
            _ => None,
        }
    }

    /// The point it acts on, when it doesn't cover a range.
    pub fn point(&self) -> Option<&Pos> {
        match self {
            Gesture::Split { at } | Gesture::Insert { at } | Gesture::NewLine { at } => Some(at),
            _ => None,
        }
    }
}

/// One character drawn on the page.
#[derive(Clone, Debug)]
pub struct CharBox {
    /// Where its byte range starts in the paragraph.
    pub pos: Pos,
    /// Where its byte range ends in the paragraph.
    pub end: usize,
    /// Its box on the page, points.
    pub rect: Rect,
    /// The character at `pos` (the object character for a picture, field or equation).
    pub ch: char,
}

impl CharBox {
    fn is_word(&self) -> bool {
        is_word_char(self.ch)
    }
}

/// One laid-out line of text on the page.
#[derive(Clone, Debug)]
pub struct RowBox {
    pub story: StoryRef,
    pub path: Path,
    /// The line's text band on the page, points.
    pub top: f32,
    pub bottom: f32,
    /// Its characters, in the paragraph's order.
    pub cells: Vec<CharBox>,
}

impl RowBox {
    fn start_off(&self) -> Option<usize> {
        self.cells.first().map(|c| c.pos.off)
    }
    fn end_off(&self) -> Option<usize> {
        self.cells.last().map(|c| c.end)
    }

    /// The character whose range holds `off` (the one after it at a boundary).
    fn at(&self, off: usize) -> Option<&CharBox> {
        self.cells.iter().find(|c| c.end > off)
    }

    /// The character that starts before `off` (the one holding it, or the one just before a
    /// boundary).
    fn before(&self, off: usize) -> Option<&CharBox> {
        self.cells.iter().rev().find(|c| c.pos.off < off)
    }

    /// The ranges of word characters on this line (the indexes of each run in `cells`), each
    /// with its box: a circle through part of a run selects the characters it covers.
    fn runs(&self) -> Vec<(usize, usize, Rect)> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(c) = self.cells.get(i) {
            if !c.is_word() {
                i += 1;
                continue;
            }
            let start = i;
            let mut end = i;
            while self.cells.get(end).is_some_and(CharBox::is_word) {
                end += 1;
            }
            if let (Some(first), Some(last)) = (self.cells.get(start), self.cells.get(end.saturating_sub(1))) {
                out.push((start, end, first.rect.union(&last.rect)));
            }
            i = end;
        }
        out
    }

    /// The words of this line: runs of word characters, each with its byte range and box.
    fn words(&self) -> Vec<(usize, usize, Rect)> {
        self.runs()
            .into_iter()
            .filter_map(|(a, b, r)| {
                let first = self.cells.get(a)?;
                let last = self.cells.get(b.saturating_sub(1))?;
                Some((first.pos.off, last.end, r))
            })
            .collect()
    }

    /// Byte range of the word holding (or touching) `off`.
    fn word_at(&self, off: usize) -> Option<(usize, usize)> {
        self.words().into_iter().find(|(a, b, _)| off >= *a && off <= *b).map(|(a, b, _)| (a, b))
    }

    /// The word nearest page x `x` within `tol` points, with its range.
    fn word_near(&self, x: f32, tol: f32) -> Option<(usize, usize)> {
        self.words().into_iter().find(|(_, _, r)| x >= r.x - tol && x <= r.right() + tol).map(|(a, b, _)| (a, b))
    }

    /// The position under page x `x`: the nearest caret edge of any of its characters.
    fn pos_at(&self, x: f32) -> Option<Pos> {
        let mut best: Option<(Pos, f32)> = None;
        for c in &self.cells {
            for (off, cx) in [(c.pos.off, c.rect.x), (c.end, c.rect.right())] {
                let d = (cx - x).abs();
                if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
                    best = Some((Pos { off, ..c.pos.clone() }, d));
                }
            }
        }
        best.map(|(p, _)| p)
    }
}

/// The text a page shows, for gesture recognition: each line's characters and where they are.
#[derive(Clone, Debug)]
pub struct PageText {
    pub page: usize,
    /// The page's size, points: stroke coordinates are clamped to it.
    pub w: f32,
    pub h: f32,
    pub rows: Vec<RowBox>,
}

impl PageText {
    /// The position nearest page point (x, y): the line whose band is nearest, then its nearest
    /// caret edge.
    pub fn hit(&self, x: f32, y: f32) -> Option<Pos> {
        let row = self.rows.iter().filter(|r| !r.cells.is_empty()).min_by(|a, b| band_distance(y, a).total_cmp(&band_distance(y, b)))?;
        row.pos_at(x)
    }

    /// The line holding `pos` (the last line of its paragraph at or past its end).
    fn row_at(&self, pos: &Pos) -> Option<&RowBox> {
        let mut last: Option<&RowBox> = None;
        for row in self.rows.iter().filter(|r| r.story == pos.story && r.path == pos.path) {
            let (Some(a), Some(b)) = (row.start_off(), row.end_off()) else { continue };
            if pos.off >= a && pos.off <= b {
                return Some(row);
            }
            last = Some(row);
        }
        last
    }

    /// Whether `pos` sits between words: at, before or after a space or a line's edge.
    fn at_gap(&self, pos: &Pos) -> bool {
        let Some(row) = self.row_at(pos) else { return false };
        let after_ws = row.at(pos.off).is_none_or(|c| !c.is_word());
        let before_ws = row.before(pos.off).is_none_or(|c| !c.is_word());
        after_ws || before_ws
    }

    /// Whether `pos` is inside a word: a word character on either side of it.
    fn inside_word(&self, pos: &Pos) -> bool {
        let Some(row) = self.row_at(pos) else { return false };
        let after = row.at(pos.off).is_some_and(CharBox::is_word);
        let before = row.before(pos.off).is_some_and(CharBox::is_word);
        after && before
    }

    /// The word touching page point (x, y), with its range.
    fn word_at_point(&self, point: [f32; 2]) -> Option<(Pos, Pos)> {
        let pos = self.hit(point[0], point[1])?;
        let row = self.row_at(&pos)?;
        let range = row
            .word_at(pos.off)
            .or_else(|| row.at(pos.off).filter(|c| c.is_word()).and_then(|c| row.word_at(c.pos.off)))
            .or_else(|| row.before(pos.off).filter(|c| c.is_word()).and_then(|c| row.word_at(c.pos.off)))
            .or_else(|| row.word_near(point[0], 8.0))?;
        Some((Pos { off: range.0, ..pos.clone() }, Pos { off: range.1, ..pos }))
    }

    /// `pos` moved past whitespace to the next word (or the line's end): a caret or backwards L
    /// drawn in the gap between words inserts before the next word, not before its space.
    fn snap_gap(&self, pos: &Pos) -> Pos {
        let Some(row) = self.row_at(pos) else { return pos.clone() };
        let mut off = pos.off;
        while let Some(c) = row.cells.iter().find(|c| c.pos.off <= off && off < c.end) {
            if !c.ch.is_whitespace() {
                break;
            }
            off = c.end;
        }
        Pos { off, ..pos.clone() }
    }
}

fn band_distance(y: f32, row: &RowBox) -> f32 {
    if y < row.top {
        row.top - y
    } else if y > row.bottom {
        y - row.bottom
    } else {
        0.0
    }
}

/// A word character: letters and digits, and the marks that keep a word together.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '\'' | '’' | '-')
}

/// Page points a gesture is recognised from: non-finite entries dropped, coordinates clamped to
/// the page, runs of identical points and tiny steps dropped, and the count capped at
/// [`MAX_GESTURE_POINTS`] by even sampling.
fn clean(points: &[[f32; 2]], w: f32, h: f32) -> Vec<[f32; 2]> {
    let (pw, ph) = (w.clamp(1.0, MAX_COORD), h.clamp(1.0, MAX_COORD));
    let mut v: Vec<[f32; 2]> = Vec::new();
    for p in points.iter().take(MAX_GESTURE_POINTS.saturating_mul(8)) {
        let [x, y] = *p;
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let q = [x.clamp(0.0, pw), y.clamp(0.0, ph)];
        let close = v.last().is_some_and(|l| (l[0] - q[0]).abs() < 0.05 && (l[1] - q[1]).abs() < 0.05);
        if !close {
            v.push(q);
        }
    }
    if v.len() > MAX_GESTURE_POINTS {
        let step = v.len() as f32 / MAX_GESTURE_POINTS as f32;
        let mut out = Vec::with_capacity(MAX_GESTURE_POINTS);
        let mut i = 0.0f32;
        while out.len() < MAX_GESTURE_POINTS {
            let Some(p) = v.get((i as usize).min(v.len().saturating_sub(1))) else { break };
            out.push(*p);
            i += step.max(1.0);
        }
        v = out;
    }
    v
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Length of the path through `p`.
fn path_len(p: &[[f32; 2]]) -> f32 {
    p.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).map(|(a, b)| dist(*a, *b)).sum()
}

/// Bounds of `p`: left, top, width, height.
fn bbox(p: &[[f32; 2]]) -> [f32; 4] {
    let Some(first) = p.first() else { return [0.0; 4] };
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
    for [x, y] in p {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// Twice the signed area the closed polygon through `p` would have; 0 for a straight line.
fn shoelace(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    let mut twice = 0.0;
    for i in 0..n {
        let (Some(a), Some(b)) = (p.get(i), p.get((i + n - 1) % n)) else { continue };
        twice += a[0] * b[1] - b[0] * a[1];
    }
    twice
}

/// Whether a stroke goes around and comes back: its ends meet, it is longer than its bounds, and
/// it encloses much of those bounds (a doubled-back line doesn't).
fn loop_shape(p: &[[f32; 2]]) -> bool {
    let (Some(first), Some(last)) = (p.first(), p.last()) else { return false };
    let [_, _, w, h] = bbox(p);
    let size = w.max(h);
    if size < MIN_LOOP {
        return false;
    }
    let perim = path_len(p);
    let gap = dist(*first, *last);
    if gap > 0.25 * perim || perim < 2.0 * size {
        return false;
    }
    shoelace(p).abs() >= 0.18 * w * h
}

/// Ramer–Douglas–Peucker with an iterative stack (no recursion to bound).
fn simplify(p: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let n = p.len();
    if n < 3 {
        return p.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(f) = keep.first_mut() {
        *f = true;
    }
    if let Some(l) = keep.last_mut() {
        *l = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (Some(pa), Some(pb)) = (p.get(a), p.get(b)) else { continue };
        let mut best = 0.0f32;
        let mut at = None;
        for i in (a + 1)..b {
            let Some(q) = p.get(i) else { break };
            let d = seg_distance(*pa, *pb, *q);
            if d > best {
                best = d;
                at = Some(i);
            }
        }
        if let Some(i) = at.filter(|_| best > eps)
            && let Some(k) = keep.get_mut(i)
            && !*k
        {
            *k = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    p.iter().zip(&keep).filter(|(_, k)| **k).map(|(q, _)| *q).collect()
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_distance(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return dist(a, p);
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    dist([a[0] + dx * t, a[1] + dy * t], p)
}

/// Whether the segment `a`–`b` passes through `r` (Liang–Barsky).
fn seg_hits_rect(a: [f32; 2], b: [f32; 2], r: &Rect) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a[0] - r.x), (dx, r.right() - a[0]), (-dy, a[1] - r.y), (dy, r.bottom() - a[1])] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return false;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return false;
                }
                t1 = t1.min(t);
            }
        }
    }
    t0 < t1
}

/// Whether page point (x, y) is inside the closed loop `poly` (even–odd rule).
fn inside(poly: &[[f32; 2]], x: f32, y: f32) -> bool {
    let n = poly.len();
    let mut odd = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + n - 1) % n)) else { continue };
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

/// The words a closed loop selects: a word whose whole box is inside the loop is selected; a
/// word only partly inside (a loop through it, or text without spaces between words) has the
/// characters its box covers selected instead. Returns the first and last position, in document
/// order.
fn enclosed(p: &[[f32; 2]], text: &PageText) -> Option<(Pos, Pos)> {
    let mut lo: Option<Pos> = None;
    let mut hi: Option<Pos> = None;
    let mut add = |a: Pos, b: Pos| {
        if lo.as_ref().is_none_or(|l| a < *l) {
            lo = Some(a);
        }
        if hi.as_ref().is_none_or(|h| b > *h) {
            hi = Some(b);
        }
    };
    for row in &text.rows {
        for (start, end, r) in row.runs() {
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let wrapped = inside(p, cx, cy) && inside(p, r.x, cy) && inside(p, r.right(), cy);
            let cells = row.cells.get(start..end).unwrap_or_default();
            if wrapped {
                let (Some(first), Some(last)) = (cells.first(), cells.last()) else { continue };
                add(first.pos.clone(), Pos { off: last.end, ..first.pos.clone() });
                continue;
            }
            for c in cells {
                let (cx, cy) = (c.rect.x + c.rect.w / 2.0, c.rect.y + c.rect.h / 2.0);
                if inside(p, cx, cy) && inside(p, c.rect.x, cy) && inside(p, c.rect.right(), cy) {
                    add(c.pos.clone(), Pos { off: c.end, ..c.pos.clone() });
                }
            }
        }
    }
    let (a, b) = {
        drop(add);
        (lo?, hi?)
    };
    (a < b).then_some((a, b))
}

/// The vertex of a caret (`^`) drawn between words, if the stroke is one.
fn caret_vertex(p: &[[f32; 2]]) -> Option<[f32; 2]> {
    let [_, _, w, h] = bbox(p);
    let size = w.max(h);
    if w < MIN_GESTURE || h < 0.25 * size {
        return None;
    }
    let s = simplify(p, (0.08 * size).max(0.5));
    let [a, v, b] = s.as_slice() else { return None };
    let (la, lb) = (dist(*a, *v), dist(*b, *v));
    if la < MIN_GESTURE || lb < MIN_GESTURE {
        return None;
    }
    let ratio = la / lb;
    if !(0.5..=2.0).contains(&ratio) {
        return None;
    }
    // The vertex sits below both arms (the caret opens upward).
    if v[1] < a[1] + 0.35 * la.min(lb) || v[1] < b[1] + 0.35 * la.min(lb) {
        return None;
    }
    let (u, t) = ([a[0] - v[0], a[1] - v[1]], [b[0] - v[0], b[1] - v[1]]);
    let cos = (u[0] * t[0] + u[1] * t[1]) / (la * lb);
    if !(-0.5..=0.94).contains(&cos) {
        return None;
    }
    (path_len(p) <= 1.35 * (la + lb)).then_some(*v)
}

/// A backwards L (down, then left): the point at the top of its vertical arm, where the text
/// after it should start a new line. `None` when the stroke isn't one.
fn backwards_l(p: &[[f32; 2]]) -> Option<[f32; 2]> {
    let [_, _, w, h] = bbox(p);
    let size = w.max(h);
    if size < 12.0 {
        return None;
    }
    let s = simplify(p, (0.08 * size).max(0.5));
    if s.len() != 3 {
        return None;
    }
    let (Some(top), Some(corner), Some(side)) = (s.first(), s.get(1), s.get(2)) else { return None };
    // Normalise to the corner being the lowest point, with one arm from above and the other
    // going left.
    let (top, corner, side) = if corner[1] >= top[1] && corner[1] >= side[1] {
        (*top, *corner, *side)
    } else if top[1] >= corner[1] && top[1] >= side[1] {
        (*corner, *top, *side)
    } else {
        (*top, *side, *corner)
    };
    let (lv, lh) = (dist(top, corner), dist(corner, side));
    if lv < 10.0 || lh < 10.0 {
        return None;
    }
    let vertical = (top[0] - corner[0]).abs() <= 0.3 * lv && corner[1] >= top[1] + 0.6 * lv;
    let left = (side[1] - corner[1]).abs() <= 0.3 * lh && side[0] <= corner[0] - 0.6 * lh;
    let ratio = lv / lh;
    (vertical && left && (0.35..=2.8).contains(&ratio) && path_len(p) <= 1.35 * (lv + lh)).then_some(top)
}

/// Whether the stroke is a single vertical line (so short it couldn't be a backwards L).
fn vertical(p: &[[f32; 2]]) -> bool {
    let [_, _, w, h] = bbox(p);
    if h < 10.0 || w > 0.28 * h {
        return false;
    }
    let (Some(first), Some(last)) = (p.first(), p.last()) else { return false };
    let mid = [(first[0] + last[0]) / 2.0, (first[1] + last[1]) / 2.0];
    let drift = p.iter().map(|q| (q[0] - mid[0]).abs()).fold(0.0, f32::max);
    let (dx, dy) = ((last[0] - first[0]).abs(), (last[1] - first[1]).abs());
    drift <= 0.18 * h + 1.0 && dy >= 0.85 * h && dx <= 0.3 * h && path_len(p) <= 1.35 * h
}

/// A split: the position in the middle of a vertical stroke drawn inside one word.
fn split_point(p: &[[f32; 2]], text: &PageText) -> Option<Pos> {
    let (Some(first), Some(last)) = (p.first(), p.last()) else { return None };
    let [_, y0, _, h] = bbox(p);
    let pos = text.hit((first[0] + last[0]) / 2.0, y0 + h / 2.0)?;
    if !text.inside_word(&pos) {
        return None;
    }
    let row = text.row_at(&pos)?;
    // The line is about as tall as the stroke, not a stray mark somewhere else.
    if h < 0.4 * (row.bottom - row.top) || h > 3.0 * (row.bottom - row.top).max(1.0) {
        return None;
    }
    Some(pos)
}

/// Whether a stroke is a curve (longer than the chord between its ends) rather than a line.
fn curved(p: &[[f32; 2]]) -> bool {
    let (Some(first), Some(last)) = (p.first(), p.last()) else { return false };
    let chord = dist(*first, *last);
    if chord < 12.0 {
        return false;
    }
    let len = path_len(p);
    len >= LINE_RATIO * chord && len <= 6.0 * chord
}

/// A join: the gap between the two words the curve's ends touch.
fn join_gap(p: &[[f32; 2]], text: &PageText) -> Option<(Pos, Pos)> {
    let (Some(first), Some(last)) = (p.first(), p.last()) else { return None };
    let (w0, w1) = (text.word_at_point(*first)?, text.word_at_point(*last)?);
    if w0.0.path != w1.0.path || w0.0.off == w1.0.off {
        return None;
    }
    let (a, b) = if w0.1.off <= w1.0.off { (w0.1, w1.0) } else { (w1.1, w0.0) };
    if a.off >= b.off {
        return None;
    }
    let row = text.row_at(&a)?;
    let between: Vec<&CharBox> = row.cells.iter().filter(|c| c.pos.off >= a.off && c.end <= b.off).collect();
    if between.is_empty() || !between.iter().all(|c| c.ch.is_whitespace()) {
        return None;
    }
    Some((a, b))
}

/// The range of characters the stroke crosses: from the first crossed character to the last.
fn crossed(p: &[[f32; 2]], text: &PageText) -> Option<(Pos, Pos)> {
    let mut lo: Option<Pos> = None;
    let mut hi: Option<Pos> = None;
    for row in &text.rows {
        for c in &row.cells {
            if c.ch.is_whitespace() || c.rect.w <= 0.0 {
                continue;
            }
            // Inset so a stroke running along a line's edge doesn't count every character.
            let r = c.rect.inset(c.rect.w * 0.1, c.rect.h * 0.25, c.rect.w * 0.1, c.rect.h * 0.25);
            let hit = p.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).any(|(a, b)| seg_hits_rect(*a, *b, &r));
            if !hit {
                continue;
            }
            if lo.as_ref().is_none_or(|l| c.pos < *l) {
                lo = Some(c.pos.clone());
            }
            let end = Pos { off: c.end, ..c.pos.clone() };
            if hi.as_ref().is_none_or(|h| end > *h) {
                hi = Some(end);
            }
        }
    }
    let (a, b) = (lo?, hi?);
    (a < b).then_some((a, b))
}

/// The gesture `points` makes on the page's text, if it makes one.
pub fn classify(points: &[[f32; 2]], text: &PageText) -> Option<Gesture> {
    let p = clean(points, text.w, text.h);
    if p.len() < 2 {
        return None;
    }
    let [_, _, w, h] = bbox(&p);
    if w.max(h) < MIN_GESTURE || path_len(&p) < MIN_GESTURE {
        return None;
    }
    // A loop selects. One drawn around no text is not an edit (it stays ink).
    if loop_shape(&p) {
        return enclosed(&p, text).map(|(a, b)| Gesture::Select { a, b });
    }
    // A caret between words puts the insertion point there.
    if let Some(v) = caret_vertex(&p)
        && let Some(at) = text.hit(v[0], v[1]).filter(|at| text.at_gap(at)).map(|at| text.snap_gap(&at))
    {
        return Some(Gesture::Insert { at });
    }
    // A backwards L moves the text after its top to a new line.
    if let Some(top) = backwards_l(&p)
        && let Some(at) = text.hit(top[0], top[1]).map(|at| text.snap_gap(&at))
    {
        return Some(Gesture::NewLine { at });
    }
    // A vertical line inside one word splits it.
    if vertical(&p)
        && let Some(at) = split_point(&p, text)
    {
        return Some(Gesture::Split { at });
    }
    // Characters the stroke crosses: a join's curve passes between its words, over none of them.
    let cross = crossed(&p, text);
    // A curve touching two words joins them.
    if cross.is_none()
        && curved(&p)
        && let Some((a, b)) = join_gap(&p, text)
    {
        return Some(Gesture::Join { a, b });
    }
    // Anything else through text crosses the characters out.
    cross.map(|(a, b)| Gesture::Delete { a, b })
}

/// The text map of `page` as it is laid out: every body line's characters, their boxes and their
/// positions, in the page's order. Turned lines are left out (a gesture there would need the
/// stroke in that line's own frame).
pub fn page_text(s: &Session, layout: &DocLayout, page: usize) -> PageText {
    page_text_in(s, layout, page, None)
}

/// The text map of `page` around the vertical extent `y0..y1` (points), with [`BAND_MARGIN`]
/// either side: the lines a stroke drawn there can reach, and no others. A stroke on a dense page
/// therefore keeps the lines it is drawn on, and the count cap can't push them out of the map.
pub fn page_text_near(s: &Session, layout: &DocLayout, page: usize, y0: f32, y1: f32) -> PageText {
    page_text_in(s, layout, page, Some((y0.min(y1), y0.max(y1))))
}

fn page_text_in(s: &Session, layout: &DocLayout, page: usize, band: Option<(f32, f32)>) -> PageText {
    let pg = layout.pages.get(page);
    let mut text = PageText { page, w: pg.map_or(1.0, |p| p.w.max(1.0)), h: pg.map_or(1.0, |p| p.h.clamp(1.0, 1.0e5)), rows: Vec::new() };
    let Some(pg) = pg else { return text };
    let mut cells = 0usize;
    for line in wordcraft_layout::hit::page_lines(pg, StoryRef::Body) {
        if cells >= MAX_CELLS {
            break;
        }
        if let Some((y0, y1)) = band
            && (line.bottom < y0 - BAND_MARGIN || line.top > y1 + BAND_MARGIN)
        {
            continue;
        }
        let Some(para) = s.doc.para(line.story, line.path) else { continue };
        let carets = line.para.line_carets(line.li);
        let mut row = RowBox { story: line.story, path: line.path.clone(), top: line.top, bottom: line.bottom, cells: Vec::new() };
        for pair in carets.windows(2) {
            if cells >= MAX_CELLS || row.cells.len() >= MAX_ROW_CELLS {
                break;
            }
            let (Some((off, Some(x))), Some((next, Some(nx)))) = (pair.first(), pair.get(1)) else { continue };
            if *next <= *off {
                continue;
            }
            let Some(seg) = para.text.get(*off..*next) else { continue };
            let Some(ch) = seg.chars().next() else { continue };
            let (left, right) = (x.min(*nx), x.max(*nx));
            let rect = Rect::new(line.x + left, line.top, right - left, (line.bottom - line.top).max(1.0));
            row.cells.push(CharBox { pos: Pos { story: line.story, path: line.path.clone(), off: *off }, end: *next, rect, ch });
            cells += 1;
        }
        if !row.cells.is_empty() {
            text.rows.push(row);
        }
    }
    text
}

/// The vertical extent of `points` (finite entries, clamped to height `h`), for a band-limited
/// map. `None` when no entry is finite.
fn band_of(points: &[[f32; 2]], h: f32) -> Option<(f32, f32)> {
    let h = h.clamp(1.0, MAX_COORD);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for [_, y] in points.iter().take(MAX_GESTURE_POINTS.saturating_mul(8)) {
        if !y.is_finite() {
            continue;
        }
        let y = y.clamp(0.0, h);
        lo = lo.min(y);
        hi = hi.max(y);
    }
    (lo <= hi).then_some((lo, hi))
}

/// The text map a stroke is recognised against: only the lines its vertical extent can reach
/// (see [`page_text_near`]), or the whole page when no point is finite (nothing can be read from
/// it). The live preview and the command both use this, so what the stroke shows is what it does.
fn text_for_stroke(s: &Session, layout: &DocLayout, page: usize, points: &[[f32; 2]]) -> PageText {
    let h = layout.pages.get(page).map_or(1.0, |p| p.h);
    match band_of(points, h) {
        Some((y0, y1)) => page_text_near(s, layout, page, y0, y1),
        None => page_text(s, layout, page),
    }
}

/// Recognise the gesture a stroke being drawn makes, for live feedback. `None` while the stroke
/// is too small or makes none: the caller draws it as ink on release. Only the lines the stroke
/// can reach are mapped, so a dense page costs what the stroke meets, not the whole page.
pub fn preview(s: &Session, layout: &DocLayout, page: usize, points: &[[f32; 2]]) -> Option<Gesture> {
    let h = layout.pages.get(page).map_or(1.0, |p| p.h);
    band_of(points, h)?;
    let text = text_for_stroke(s, layout, page, points);
    classify(points, &text)
}

/// `draw.actionPenStroke`: classify the stroke and carry the gesture out through the ordinary
/// edit, selection and revision paths (one undo step for the command). A stroke that makes no
/// gesture is drawn as ink with `draw.stroke`, exactly as a pen would.
pub(super) fn perform(s: &mut Session, v: &Value) -> CmdResult {
    let (page, layout) = super::page_param(s, v)?;
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let (pw, ph) = (pg.w.max(1.0), pg.h.clamp(1.0, 1.0e5));
    let pts = super::points_param(v, MAX_GESTURE_POINTS, pw, ph)?;
    let text = text_for_stroke(s, &layout, page, &pts);
    let Some(gesture) = classify(&pts, &text) else {
        // Unrecognised (or too small): it stays ink. One undo step with the rest of the stroke.
        let mut out = super::stroke(s, v)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("gesture".into(), Value::from("ink"));
        }
        return Ok(out);
    };
    let name = gesture.name();
    match gesture {
        Gesture::Select { a, b } => {
            s.sel = Selection { anchor: a.clone(), focus: b.clone() };
            let mut out = sel_result(s)?;
            out["gesture"] = Value::from("select");
            Ok(out)
        }
        // Deleting and joining both remove a range: the engine's delete path records it as a
        // tracked revision, keeps runs and fields around it, and leaves one undo step.
        Gesture::Delete { a, b } | Gesture::Join { a, b } => {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            s.sel = Selection { anchor: a.clone(), focus: b.clone() };
            let at = delete_selection(s)?;
            Ok(json!({"gesture": name, "anchor": pos_json(&a), "focus": pos_json(&b), "at": pos_json(&at)}))
        }
        // A split puts a space in the word, through the typing path (tracked when Track Changes
        // is on, and content controls and fields are respected).
        Gesture::Split { at } => {
            s.sel = Selection::caret(at.clone());
            type_text(s, " ")?;
            let mut out = sel_result(s)?;
            out["gesture"] = Value::from("split");
            out["at"] = pos_json(&at);
            Ok(out)
        }
        // The caret gesture puts the insertion point down. `text` (a script's, or a recogniser's
        // seam) types the words in the same undo step; otherwise the app's ordinary typing path
        // (keyboard and IME, `text.insert`) does.
        Gesture::Insert { at } => {
            s.sel = Selection::caret(at.clone());
            if let Some(t) = p::str(v, "text").filter(|t| !t.is_empty()) {
                type_text(s, t)?;
            }
            let mut out = sel_result(s)?;
            out["gesture"] = Value::from("insert");
            out["at"] = pos_json(&at);
            Ok(out)
        }
        // A backwards L splits the paragraph at the point: the text after it starts a new line.
        Gesture::NewLine { at } => {
            s.sel = Selection::caret(at.clone());
            let new = split_para(s, &at)?;
            s.sel = Selection::caret(new.clone());
            let mut out = sel_result(s)?;
            out["gesture"] = Value::from("newLine");
            out["at"] = pos_json(&at);
            out["lineStart"] = pos_json(&new);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page with lines of text in a fixed-width test face: each character 10 pt wide.
    fn page(lines: &[&str]) -> PageText {
        let mut rows = Vec::new();
        let path = Path::top(0);
        for (li, line) in lines.iter().enumerate() {
            let (x0, top) = (100.0f32, 100.0 + li as f32 * 20.0);
            let mut cells = Vec::new();
            for (i, ch) in line.char_indices() {
                let x = x0 + (i / ch.len_utf8().max(1)) as f32 * 10.0;
                cells.push(CharBox {
                    pos: Pos { story: StoryRef::Body, path: path.clone(), off: i },
                    end: i + ch.len_utf8(),
                    rect: Rect::new(x, top, 10.0, 12.0),
                    ch,
                });
            }
            rows.push(RowBox { story: StoryRef::Body, path: path.clone(), top, bottom: top + 12.0, cells });
        }
        PageText { page: 0, w: 600.0, h: 800.0, rows }
    }

    /// A vertical stroke at page x from y0 to y1, sampled.
    fn down(x: f32, y0: f32, y1: f32) -> Vec<[f32; 2]> {
        (0..=10).map(|i| [x, y0 + (y1 - y0) * i as f32 / 10.0]).collect()
    }

    /// A horizontal stroke at page y from x0 to x1.
    fn across(x0: f32, x1: f32, y: f32) -> Vec<[f32; 2]> {
        (0..=10).map(|i| [x0 + (x1 - x0) * i as f32 / 10.0, y]).collect()
    }

    /// An ellipse around a box.
    fn loop_around(r: Rect) -> Vec<[f32; 2]> {
        (0..=32)
            .map(|i| {
                let t = i as f32 / 32.0 * std::f32::consts::TAU;
                [r.x + r.w / 2.0 + (r.w / 2.0 + 5.0) * t.cos(), r.y + r.h / 2.0 + (r.h / 2.0 + 5.0) * t.sin()]
            })
            .collect()
    }

    #[test]
    fn a_loop_selects_the_words_it_encloses() {
        let text = page(&["alpha beta gamma"]);
        let g = classify(&loop_around(Rect::new(100.0, 100.0, 50.0, 12.0)), &text);
        assert_eq!(g.as_ref().map(|g| g.name()), Some("select"));
        let Some(Gesture::Select { a, b }) = g else { panic!() };
        assert_eq!((a.off, b.off), (0, 5), "the word alpha");
    }

    #[test]
    fn a_loop_around_two_words_selects_both_and_one_around_nothing_does_nothing() {
        let text = page(&["alpha beta gamma"]);
        let Some(Gesture::Select { a, b }) = classify(&loop_around(Rect::new(100.0, 100.0, 100.0, 12.0)), &text) else { panic!() };
        assert_eq!((a.off, b.off), (0, 10), "alpha beta");
        assert_eq!(classify(&loop_around(Rect::new(400.0, 400.0, 40.0, 12.0)), &text), None);
    }

    #[test]
    fn a_line_a_diagonal_and_a_squiggle_through_text_delete_the_crossed_range() {
        let text = page(&["alpha beta gamma"]);
        let Some(Gesture::Delete { a, b }) = classify(&across(105.0, 200.0, 106.0), &text) else { panic!("line") };
        assert!(a.off <= 1 && b.off >= 9, "{a:?} {b:?}");
        let diagonal: Vec<[f32; 2]> = (0..=10).map(|i| [105.0 + i as f32 * 9.0, 101.0 + i as f32]).collect();
        assert_eq!(classify(&diagonal, &text).map(|g| g.name()), Some("delete"));
        let squiggle: Vec<[f32; 2]> = (0..=40).map(|i| [105.0 + i as f32 * 2.5, 106.0 + (i as f32 * 0.9).sin() * 3.0]).collect();
        assert_eq!(classify(&squiggle, &text).map(|g| g.name()), Some("delete"));
    }

    #[test]
    fn a_curve_touching_two_words_joins_them() {
        let text = page(&["alpha beta gamma"]);
        // From the middle of "alpha" (x 105) down, along, and up to the middle of "beta" (x 165).
        let mut pts: Vec<[f32; 2]> = Vec::new();
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let x = 105.0 + 60.0 * t;
            pts.push([x, 120.0 + (t * std::f32::consts::PI).sin() * 14.0]);
        }
        let Some(Gesture::Join { a, b }) = classify(&pts, &text) else { panic!("join: {pts:?}") };
        assert_eq!((a.off, b.off), (5, 6), "the space between alpha and beta");
    }

    #[test]
    fn a_vertical_line_inside_a_word_splits_it() {
        let text = page(&["alpha beta"]);
        let Some(Gesture::Split { at }) = classify(&down(126.0, 99.0, 113.0), &text) else { panic!("split") };
        assert_eq!(at.off, 3, "inside alpha");
    }

    #[test]
    fn a_caret_between_words_puts_the_insertion_point_there() {
        let text = page(&["alpha beta"]);
        // Drawn from the left tip down to the vertex and up to the right tip, like a person.
        let caret: Vec<[f32; 2]> = (0..=10)
            .map(|i| {
                let t = i as f32 / 10.0;
                [140.0 + 10.0 * t, 101.0 + 17.0 * t]
            })
            .chain((1..=10).map(|i| {
                let t = i as f32 / 10.0;
                [150.0 + 10.0 * t, 118.0 - 17.0 * t]
            }))
            .collect();
        let Some(Gesture::Insert { at }) = classify(&caret, &text) else { panic!("caret") };
        assert_eq!(at.off, 6, "before beta: past the gap's space");
    }

    #[test]
    fn a_backwards_l_moves_the_following_text_to_a_new_line() {
        let text = page(&["alpha beta"]);
        let mut l = down(150.0, 96.0, 120.0);
        let left = across(150.0, 120.0, 120.0);
        l.extend(left.into_iter().skip(1));
        let Some(Gesture::NewLine { at }) = classify(&l, &text) else { panic!("backwards L") };
        assert_eq!(at.off, 6);
    }

    #[test]
    fn tiny_ambiguous_non_finite_and_huge_strokes_never_edit_and_never_panic() {
        let text = page(&["alpha beta"]);
        // Too small.
        assert_eq!(classify(&[[104.0, 106.0], [105.0, 106.0]], &text), None);
        assert_eq!(classify(&[], &text), None);
        // Away from any text.
        assert_eq!(classify(&across(300.0, 400.0, 300.0), &text), None);
        // Non-finite points are dropped; all of them leaves nothing to classify.
        assert_eq!(classify(&[[f32::NAN, 1.0], [f32::INFINITY, 2.0]], &text), None);
        // Huge and long input is clamped and sampled, not walked whole.
        let huge: Vec<[f32; 2]> = (0..100_000).map(|i| [(i % 1000) as f32 * 1.0e6, (i % 300) as f32]).collect();
        let _ = classify(&huge, &text);
        // A vertical line outside the word is not a split (it is a cross-out of nothing here).
        assert_eq!(classify(&down(390.0, 100.0, 112.0), &text), None);
        // A caret drawn inside a word is not an insertion point.
        let caret =
            vec![[130.0, 118.0], [125.0, 111.0], [120.0, 101.0], [125.0, 108.0], [130.0, 116.0], [135.0, 108.0], [140.0, 101.0], [135.0, 111.0]];
        assert_ne!(classify(&caret, &text).map(|g| g.name()), Some("insert"));
    }

    #[test]
    fn enclosed_text_without_spaces_selects_the_characters_covered() {
        let text = page(&["这是一个很长的句子没有空格"]);
        let g = classify(&loop_around(Rect::new(120.0, 100.0, 30.0, 12.0)), &text);
        let Some(Gesture::Select { a, b }) = g else { panic!() };
        assert!(a.off > 0 && b.off < text.rows[0].cells.last().map_or(0, |c| c.end), "a few characters: {a:?} {b:?}");
    }
}
