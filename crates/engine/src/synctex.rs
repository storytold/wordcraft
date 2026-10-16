//! SyncTeX forward/inverse search for the TeX sidecar.
//!
//! Tectonic (and pdfTeX/XeTeX) emit a `.synctex(.gz)` sidecar beside the PDF
//! when asked (`--synctex` / `-synctex=1`). This module parses that file and
//! answers two jumps without ever touching the document model:
//!
//! * forward: source line -> page plus rectangle in the compiled output;
//! * inverse: page plus point -> source file tag plus line.
//!
//! Coordinates are PDF points (1/72 inch) with a top-left origin (x right,
//! y down), so a preview PNG rasterised at `dpi` maps linearly via
//! `pixel = point * dpi / 72`. The `.synctex` native units are TeX scaled
//! points (65536 per point) scaled by `Unit * Magnification / 1000`; the
//! header almost always reads `Unit:1 Magnification:1000 Offset:0`, and any
//! other header is honoured on a best-effort basis. Box records store the
//! baseline (`y`) plus the height above it (`h`) and depth below it
//! (`depth`): the top edge is `y - h`, the full height `h + depth`.
//!
//! The parser is hostile-input safe: garbage, truncation and absurd numbers
//! cost entries, never panics. Only `flate2` (already in the lockfile via
//! `zip`) is used for `.synctex.gz`; no new crates are introduced.

use std::collections::HashMap;

/// Largest raw `.synctex(.gz)` blob accepted (bytes on disk).
pub const MAX_SYNCTEX_BYTES: u64 = 8 << 20;
/// Largest decompressed SyncTeX text parsed (bytes).
pub const MAX_SYNCTEX_TEXT: usize = 8 << 20;
/// Most entries kept; the rest are dropped, parsing still succeeds.
pub const MAX_SYNCTEX_ENTRIES: usize = 200_000;
/// Most `Input:` mappings kept.
pub const MAX_INPUTS: usize = 4096;
/// Longest single input path kept (bytes, cut at a char boundary).
pub const MAX_INPUT_PATH: usize = 4096;
/// Largest page number honoured (larger pages are skipped).
pub const MAX_PAGE: u32 = 1_000_000;
/// Reasonable coordinate window in points (entries outside are skipped).
pub const COORD_MIN: f64 = -5_000.0;
pub const COORD_MAX: f64 = 10_000.0;
/// Largest box edge kept in points (larger boxes are skipped).
pub const MAX_EDGE: f64 = 10_000.0;
/// Minimum forward rectangle edge when expanding a bare point.
pub const MIN_POINT_W: f64 = 10.0;
/// Minimum forward rectangle height when expanding a bare point.
pub const MIN_POINT_H: f64 = 12.0;
/// How far (points) an inverse click may stray from any record.
pub const INVERSE_SNAP: f64 = 50.0;

/// One rectangle in PDF points, top-left origin, y down.
#[derive(Clone, Debug, PartialEq)]
pub struct SynctexRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One parsed record: the source tag plus line, the page it lands on, and
/// the rectangle (points, top-left). Point records (`g`, `k`, zero-area
/// boxes) keep `w == 0` and/or `h == 0` with `is_box == false`.
#[derive(Clone, Debug, PartialEq)]
pub struct SynctexEntry {
    pub tag: u32,
    pub line: u32,
    pub page: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub is_box: bool,
}

/// A parsed SyncTeX file: input-tag to path plus flat records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SynctexDoc {
    pub inputs: HashMap<u32, String>,
    pub entries: Vec<SynctexEntry>,
}

/// Scale from native units to points: `Unit * Magnification/1000 / 65536`.
fn scale_for(unit: f64, mag: f64) -> f64 {
    if !unit.is_finite() || !mag.is_finite() || unit <= 0.0 || mag <= 0.0 {
        return 1.0 / 65_536.0;
    }
    let s = unit * (mag / 1000.0) / 65_536.0;
    if s.is_finite() && s > 0.0 && s < 1.0 { s } else { 1.0 / 65_536.0 }
}

/// Parse one `f64` header value, falling back to `dflt` on any trouble.
fn header_f64(after: &str, dflt: f64) -> f64 {
    let t = after.trim();
    if t.is_empty() {
        return dflt;
    }
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() => v,
        _ => dflt,
    }
}

/// Parse one `i64`, returning `None` on any trouble (never panics).
fn parse_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<i64>().ok()
}

/// Parse one `u32`, returning `None` on any trouble (never panics).
fn parse_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<u32>().ok()
}

/// Split `tag,line:rest` into `(tag, line, rest)`.
fn split_tag_line(s: &str) -> Option<(u32, u32, &str)> {
    let (head, tail) = s.split_once(':')?;
    let (tag_s, line_s) = head.split_once(',')?;
    Some((parse_u32(tag_s)?, parse_u32(line_s)?, tail))
}

/// Split `x,y` into `(x, y)`.
fn split_xy(s: &str) -> Option<(i64, i64)> {
    let (xs, ys) = s.split_once(',')?;
    Some((parse_i64(xs)?, parse_i64(ys)?))
}

/// Parse box geometry `x,y:w,h,depth` into raw integers.
fn parse_box_tail(tail: &str) -> Option<(i64, i64, i64, i64, i64)> {
    let (xy, whd) = tail.split_once(':')?;
    let (x, y) = split_xy(xy)?;
    let mut it = whd.split(',');
    let w = parse_i64(it.next()?)?;
    let h = parse_i64(it.next()?)?;
    let d = parse_i64(it.next()?)?;
    if it.next().is_some() {
        return None;
    }
    Some((x, y, w, h, d))
}

/// Parse kern geometry `x,y:width` into raw integers.
fn parse_kern_tail(tail: &str) -> Option<(i64, i64, i64)> {
    let (xy, w) = tail.split_once(':')?;
    let (x, y) = split_xy(xy)?;
    if w.contains(',') || w.contains(':') {
        return None;
    }
    Some((x, y, parse_i64(w)?))
}

/// Parse point geometry `x,y` into raw integers.
fn parse_point_tail(tail: &str) -> Option<(i64, i64)> {
    if tail.contains(':') {
        return None;
    }
    split_xy(tail)
}

/// Convert a raw native coordinate to points.
fn to_pt(raw: i64, off: f64, scale: f64) -> Option<f64> {
    // `raw as f64` never panics; huge values become approximate but finite.
    let v = (raw as f64 + off) * scale;
    if v.is_finite() { Some(v) } else { None }
}

/// Convert a raw native extent to points (`None` when negative or wild).
fn extent_pt(raw: i64, scale: f64) -> Option<f64> {
    if raw < 0 {
        return None;
    }
    let v = raw as f64 * scale;
    if v.is_finite() && v <= MAX_EDGE { Some(v) } else { None }
}

/// Build a box entry from raw integers (`None` when insane).
#[allow(clippy::too_many_arguments)]
fn make_box(tag: u32, line: u32, page: u32, x: i64, y: i64, w: i64, h: i64, d: i64, x_off: f64, y_off: f64, scale: f64) -> Option<SynctexEntry> {
    if page == 0 || page > MAX_PAGE || line == 0 {
        return None;
    }
    let y_top_raw = y.checked_sub(h)?;
    let h_tot_raw = h.checked_add(d)?;
    if w < 0 || h < 0 || d < 0 {
        return None;
    }
    let left = to_pt(x, x_off, scale)?;
    let top = to_pt(y_top_raw, y_off, scale)?;
    let w_pt = extent_pt(w, scale)?;
    let h_pt = extent_pt(h_tot_raw, scale)?;
    if !(left.is_finite() && top.is_finite() && w_pt.is_finite() && h_pt.is_finite()) {
        return None;
    }
    if left < COORD_MIN || left > COORD_MAX || top < COORD_MIN || top > COORD_MAX {
        return None;
    }
    let is_box = w_pt > 1e-9 && h_pt > 1e-9;
    Some(SynctexEntry { tag, line, page, x: left, y: top, w: w_pt, h: h_pt, is_box })
}

/// Build a point entry from raw integers (`None` when insane).
fn make_point(tag: u32, line: u32, page: u32, x: i64, y: i64, x_off: f64, y_off: f64, scale: f64) -> Option<SynctexEntry> {
    if page == 0 || page > MAX_PAGE || line == 0 {
        return None;
    }
    let px = to_pt(x, x_off, scale)?;
    let py = to_pt(y, y_off, scale)?;
    if !px.is_finite() || !py.is_finite() {
        return None;
    }
    if px < COORD_MIN || px > COORD_MAX || py < COORD_MIN || py > COORD_MAX {
        return None;
    }
    Some(SynctexEntry { tag, line, page, x: px, y: py, w: 0.0, h: 0.0, is_box: false })
}

/// Build a kern entry (a horizontal segment with no height).
fn make_kern(tag: u32, line: u32, page: u32, x: i64, y: i64, w: i64, x_off: f64, y_off: f64, scale: f64) -> Option<SynctexEntry> {
    if page == 0 || page > MAX_PAGE || line == 0 {
        return None;
    }
    if w < 0 {
        return None;
    }
    let px = to_pt(x, x_off, scale)?;
    let py = to_pt(y, y_off, scale)?;
    let w_pt = extent_pt(w, scale)?;
    if !px.is_finite() || !py.is_finite() || !w_pt.is_finite() {
        return None;
    }
    if px < COORD_MIN || px > COORD_MAX || py < COORD_MIN || py > COORD_MAX {
        return None;
    }
    Some(SynctexEntry { tag, line, page, x: px, y: py, w: w_pt, h: 0.0, is_box: false })
}

/// Cut a path to the length cap at a char boundary.
fn cap_path(raw: &str) -> String {
    let t = raw.trim();
    if t.len() <= MAX_INPUT_PATH {
        return t.to_string();
    }
    let mut cut = MAX_INPUT_PATH;
    while cut > 0 && !t.is_char_boundary(cut) {
        cut -= 1;
    }
    t.get(..cut).map(str::to_string).unwrap_or_default()
}

/// Decompress (or pass through) one `.synctex(.gz)` blob into text.
pub fn decompress_synctex(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() as u64 > MAX_SYNCTEX_BYTES {
        return Err(format!("SyncTeX blob is over the {} byte limit", MAX_SYNCTEX_BYTES));
    }
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use std::io::Read;
        let decoder = flate2::read::GzDecoder::new(bytes);
        let mut limited = decoder.take(MAX_SYNCTEX_TEXT as u64 + 1);
        let mut out = String::new();
        match limited.read_to_string(&mut out) {
            Ok(_) => {
                if out.len() > MAX_SYNCTEX_TEXT {
                    return Err(format!("SyncTeX text is over the {MAX_SYNCTEX_TEXT} byte limit"));
                }
                Ok(out)
            }
            Err(e) => Err(format!("could not gunzip SyncTeX data: {e}")),
        }
    } else {
        if bytes.len() > MAX_SYNCTEX_TEXT {
            return Err(format!("SyncTeX text is over the {MAX_SYNCTEX_TEXT} byte limit"));
        }
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Parse SyncTeX text into inputs plus flat records. Garbage is skipped:
/// hostile text costs entries, never an error beyond size caps (and never a
/// panic). An empty document parses to an empty `SynctexDoc`.
pub fn parse_synctex_text(text: &str) -> Result<SynctexDoc, String> {
    if text.len() > MAX_SYNCTEX_TEXT {
        return Err(format!("SyncTeX text is over the {MAX_SYNCTEX_TEXT} byte limit"));
    }
    // Header first: magnification, unit and offsets (defaults match tectonic).
    let mut mag = 1000.0;
    let mut unit = 1.0;
    let mut x_off = 0.0;
    let mut y_off = 0.0;
    for raw in text.lines() {
        if raw.len() > 8192 {
            continue;
        }
        let line = raw.trim();
        if let Some(after) = line.strip_prefix('M') {
            // Avoid accidental matches: check the full key.
            if let Some(v) = line.strip_prefix("Magnification:") {
                let m = header_f64(v, 1000.0);
                if m > 0.0 && m < 100_000.0 {
                    mag = m;
                }
            } else if let Some(v) = line.strip_prefix("Unit:") {
                let u = header_f64(v, 1.0);
                if u > 0.0 && u < 1_000_000.0 {
                    unit = u;
                }
            } else {
                let _ = after;
            }
        } else if let Some(v) = line.strip_prefix("X Offset:") {
            let o = header_f64(v, 0.0);
            if o.abs() < 10_000_000.0 {
                x_off = o;
            }
        } else if let Some(v) = line.strip_prefix("Y Offset:") {
            let o = header_f64(v, 0.0);
            if o.abs() < 10_000_000.0 {
                y_off = o;
            }
        }
    }
    let scale = scale_for(unit, mag);
    let mut doc = SynctexDoc::default();
    let mut in_content = false;
    let mut pages: Vec<u32> = Vec::new();
    for raw in text.lines() {
        if doc.entries.len() >= MAX_SYNCTEX_ENTRIES {
            break;
        }
        if raw.len() > 8192 {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("Input:") {
            if doc.inputs.len() >= MAX_INPUTS {
                continue;
            }
            if let Some(rest) = line.strip_prefix("Input:") {
                // Tag ends at the first colon; the path keeps the rest (it
                // may itself contain colons, e.g. `C:\...`).
                if let Some((tag_s, path)) = rest.split_once(':')
                    && let Some(tag) = parse_u32(tag_s)
                {
                    let p = cap_path(path);
                    if !p.is_empty() {
                        doc.inputs.entry(tag).or_insert(p);
                    }
                }
            }
            continue;
        }
        if line == "Content:" {
            in_content = true;
            continue;
        }
        if line == "Postamble:" {
            in_content = false;
            continue;
        }
        if !in_content {
            continue;
        }
        if line.starts_with('!') {
            continue;
        }
        if line.starts_with('{') {
            if let Some(n) = parse_u32(line.get(1..).unwrap_or_default())
                && n > 0
                && n <= MAX_PAGE
                && pages.len() < 4096
            {
                pages.push(n);
            }
            continue;
        }
        if line.starts_with('}') {
            pages.pop();
            continue;
        }
        let Some(page) = pages.last().copied() else { continue };
        if page == 0 || page > MAX_PAGE {
            continue;
        }
        if line == ")" || line == "]" {
            continue;
        }
        let first = line.as_bytes().first().copied().unwrap_or(0);
        if first == b'[' || first == b'(' {
            let Some(body) = line.get(1..) else { continue };
            let Some((tag, ln, tail)) = split_tag_line(body) else { continue };
            let Some((x, y, w, h, d)) = parse_box_tail(tail) else { continue };
            if let Some(e) = make_box(tag, ln, page, x, y, w, h, d, x_off, y_off, scale) {
                doc.entries.push(e);
            }
            continue;
        }
        if first == b'g' || first == b'G' {
            let Some(body) = line.get(1..) else { continue };
            let Some((tag, ln, tail)) = split_tag_line(body) else { continue };
            let Some((x, y)) = parse_point_tail(tail) else { continue };
            if let Some(e) = make_point(tag, ln, page, x, y, x_off, y_off, scale) {
                doc.entries.push(e);
            }
            continue;
        }
        if first == b'k' || first == b'K' {
            let Some(body) = line.get(1..) else { continue };
            let Some((tag, ln, tail)) = split_tag_line(body) else { continue };
            if let Some((x, y, w)) = parse_kern_tail(tail) {
                if let Some(e) = make_kern(tag, ln, page, x, y, w, x_off, y_off, scale) {
                    doc.entries.push(e);
                }
            } else if let Some((x, y)) = parse_point_tail(tail)
                && let Some(e) = make_point(tag, ln, page, x, y, x_off, y_off, scale)
            {
                doc.entries.push(e);
            }
            continue;
        }
        if first.is_ascii_alphabetic() {
            // `h`, `v`, `r`, `m`, `x` and friends: try box, then kern, then
            // point. Anything else is skipped.
            let Some(body) = line.get(1..) else { continue };
            let Some((tag, ln, tail)) = split_tag_line(body) else { continue };
            if let Some((x, y, w, h, d)) = parse_box_tail(tail) {
                if let Some(e) = make_box(tag, ln, page, x, y, w, h, d, x_off, y_off, scale) {
                    doc.entries.push(e);
                }
            } else if let Some((x, y, w)) = parse_kern_tail(tail) {
                if let Some(e) = make_kern(tag, ln, page, x, y, w, x_off, y_off, scale) {
                    doc.entries.push(e);
                }
            } else if let Some((x, y)) = parse_point_tail(tail)
                && let Some(e) = make_point(tag, ln, page, x, y, x_off, y_off, scale)
            {
                doc.entries.push(e);
            }
            continue;
        }
    }
    Ok(doc)
}

/// Parse one raw `.synctex(.gz)` blob (gzipped or plain) into a document.
pub fn parse_synctex_bytes(bytes: &[u8]) -> Result<SynctexDoc, String> {
    let text = decompress_synctex(bytes)?;
    parse_synctex_text(&text)
}

/// The path recorded for one input tag, if any.
pub fn file_for_tag(doc: &SynctexDoc, tag: u32) -> Option<&str> {
    doc.inputs.get(&tag).map(String::as_str)
}

/// Accumulator for a per-page union of rectangles.
struct Union {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    count: usize,
}

impl Union {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Option<Self> {
        if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
            return None;
        }
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let max_x = x + w;
        let max_y = y + h;
        if !(max_x.is_finite() && max_y.is_finite()) {
            return None;
        }
        Some(Union { min_x: x, min_y: y, max_x, max_y, count: 1 })
    }

    fn add(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
            return;
        }
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let ex = x + w;
        let ey = y + h;
        if !(ex.is_finite() && ey.is_finite()) {
            return;
        }
        if x < self.min_x {
            self.min_x = x;
        }
        if y < self.min_y {
            self.min_y = y;
        }
        if ex > self.max_x {
            self.max_x = ex;
        }
        if ey > self.max_y {
            self.max_y = ey;
        }
        self.count = self.count.saturating_add(1);
    }

    fn rect(&self) -> Option<SynctexRect> {
        let w = self.max_x - self.min_x;
        let h = self.max_y - self.min_y;
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return None;
        }
        if w > MAX_EDGE || h > MAX_EDGE {
            return None;
        }
        Some(SynctexRect { x: self.min_x, y: self.min_y, w, h })
    }

    fn area(&self) -> f64 {
        let w = self.max_x - self.min_x;
        let h = self.max_y - self.min_y;
        if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }
}

/// Forward search: where does `line` land? Returns the best page plus the
/// union of that line's boxes there. Exact-area boxes win; when the line
/// only owns points (glue/kern, zero-area absolutes), the smallest enclosing
/// boxes are used, falling back to the points' bounding box expanded to a
/// minimum readable size. Returns `None` when the line has no records.
pub fn forward_search(doc: &SynctexDoc, line: u32) -> Option<(u32, SynctexRect)> {
    if line == 0 {
        return None;
    }
    // Direct boxes for this line, grouped by page.
    let mut unions: HashMap<u32, Union> = HashMap::new();
    for e in &doc.entries {
        if e.line != line || !e.is_box {
            continue;
        }
        if e.page == 0 || e.page > MAX_PAGE {
            continue;
        }
        if !(e.x.is_finite() && e.y.is_finite() && e.w.is_finite() && e.h.is_finite()) {
            continue;
        }
        if e.w <= 0.0 || e.h <= 0.0 {
            continue;
        }
        match unions.get_mut(&e.page) {
            Some(u) => u.add(e.x, e.y, e.w, e.h),
            None => {
                if let Some(u) = Union::new(e.x, e.y, e.w, e.h) {
                    unions.insert(e.page, u);
                }
            }
        }
    }
    let mut best: Option<(u32, SynctexRect, f64)> = None;
    for (page, u) in &unions {
        let Some(r) = u.rect() else { continue };
        let a = u.area();
        let better = match &best {
            None => true,
            Some((bp, _, ba)) => a > *ba || (a == *ba && *page < *bp),
        };
        if better {
            best = Some((*page, r, a));
        }
    }
    if let Some((page, rect, _)) = best {
        return Some((page, rect));
    }
    // No direct boxes: collect this line's point positions per page.
    let mut points: HashMap<u32, Vec<(f64, f64)>> = HashMap::new();
    for e in &doc.entries {
        if e.line != line {
            continue;
        }
        if e.page == 0 || e.page > MAX_PAGE {
            continue;
        }
        if !e.x.is_finite() || !e.y.is_finite() {
            continue;
        }
        // For boxes that somehow missed `is_box` (zero height), use the top
        // edge as the point; for kerns use the segment centre.
        let (px, py) = if e.w > 0.0 && !e.is_box { (e.x + e.w / 2.0, e.y) } else { (e.x, e.y) };
        if !px.is_finite() || !py.is_finite() {
            continue;
        }
        points.entry(e.page).or_default().push((px, py));
    }
    if points.is_empty() {
        return None;
    }
    // Prefer the smallest enclosing box for each point.
    let mut enc: HashMap<u32, Union> = HashMap::new();
    for (page, pts) in &points {
        for (px, py) in pts {
            let mut inner: Option<(f64, &SynctexEntry)> = None;
            for e in &doc.entries {
                if e.page != *page || !e.is_box {
                    continue;
                }
                if !(e.x.is_finite() && e.y.is_finite() && e.w.is_finite() && e.h.is_finite()) {
                    continue;
                }
                if e.w <= 0.0 || e.h <= 0.0 {
                    continue;
                }
                if *px < e.x || *px > e.x + e.w || *py < e.y || *py > e.y + e.h {
                    continue;
                }
                let a = e.w * e.h;
                if !a.is_finite() {
                    continue;
                }
                let better = match inner {
                    None => true,
                    Some((ba, _)) => a < ba,
                };
                if better {
                    inner = Some((a, e));
                }
            }
            if let Some((_, b)) = inner {
                match enc.get_mut(page) {
                    Some(u) => u.add(b.x, b.y, b.w, b.h),
                    None => {
                        if let Some(u) = Union::new(b.x, b.y, b.w, b.h) {
                            enc.insert(*page, u);
                        }
                    }
                }
            }
        }
    }
    let mut best_enc: Option<(u32, SynctexRect, f64)> = None;
    for (page, u) in &enc {
        let Some(r) = u.rect() else { continue };
        let a = u.area();
        let better = match &best_enc {
            None => true,
            Some((bp, _, ba)) => a > *ba || (a == *ba && *page < *bp),
        };
        if better {
            best_enc = Some((*page, r, a));
        }
    }
    if let Some((page, rect, _)) = best_enc {
        return Some((page, rect));
    }
    // Last resort: bounding box of the points, expanded to a minimum size.
    let mut best_pts: Option<(u32, SynctexRect, usize)> = None;
    for (page, pts) in &points {
        if pts.is_empty() {
            continue;
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (px, py) in pts {
            if *px < min_x {
                min_x = *px;
            }
            if *py < min_y {
                min_y = *py;
            }
            if *px > max_x {
                max_x = *px;
            }
            if *py > max_y {
                max_y = *py;
            }
        }
        if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
            continue;
        }
        let mut w = max_x - min_x;
        let mut h = max_y - min_y;
        if !w.is_finite() || !h.is_finite() {
            continue;
        }
        let mut x = min_x;
        let mut y = min_y;
        if w < MIN_POINT_W {
            x -= (MIN_POINT_W - w) / 2.0;
            w = MIN_POINT_W;
        }
        if h < MIN_POINT_H {
            y -= (MIN_POINT_H - h) / 2.0;
            h = MIN_POINT_H;
        }
        if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
            continue;
        }
        if w <= 0.0 || h <= 0.0 || w > MAX_EDGE || h > MAX_EDGE {
            continue;
        }
        let better = match &best_pts {
            None => true,
            Some((bp, _, bn)) => pts.len() > *bn || (pts.len() == *bn && *page < *bp),
        };
        if better {
            best_pts = Some((*page, SynctexRect { x, y, w, h }, pts.len()));
        }
    }
    best_pts.map(|(page, rect, _)| (page, rect))
}

/// Inverse search: which source record owns `x, y` on `page`? Returns the
/// owning `(tag, line)`. The smallest containing box wins; with no container,
/// the nearest point record within snapping range wins; otherwise `None`.
pub fn inverse_search(doc: &SynctexDoc, page: u32, x: f64, y: f64) -> Option<(u32, u32)> {
    if page == 0 || page > MAX_PAGE {
        return None;
    }
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let mut best: Option<(f64, u32, u32, u32)> = None;
    for e in &doc.entries {
        if e.page != page || !e.is_box {
            continue;
        }
        if !(e.x.is_finite() && e.y.is_finite() && e.w.is_finite() && e.h.is_finite()) {
            continue;
        }
        if e.w <= 0.0 || e.h <= 0.0 {
            continue;
        }
        if x < e.x || x > e.x + e.w || y < e.y || y > e.y + e.h {
            continue;
        }
        let a = e.w * e.h;
        if !a.is_finite() {
            continue;
        }
        let better = match best {
            None => true,
            Some((ba, bt, bl, _)) => a < ba || (a == ba && (e.tag < bt || (e.tag == bt && e.line < bl))),
        };
        if better {
            best = Some((a, e.tag, e.line, e.page));
        }
    }
    if let Some((_, tag, line, _)) = best {
        return Some((tag, line));
    }
    // No container: nearest point-like record on this page.
    let mut near: Option<(f64, u32, u32)> = None;
    for e in &doc.entries {
        if e.page != page {
            continue;
        }
        // Boxes missed above: distance to the rectangle edge.
        let d = if e.is_box && e.w > 0.0 && e.h > 0.0 && e.x.is_finite() && e.y.is_finite() && e.w.is_finite() && e.h.is_finite() {
            let dx = if x < e.x {
                e.x - x
            } else if x > e.x + e.w {
                x - (e.x + e.w)
            } else {
                0.0
            };
            let dy = if y < e.y {
                e.y - y
            } else if y > e.y + e.h {
                y - (e.y + e.h)
            } else {
                0.0
            };
            dx * dx + dy * dy
        } else {
            if !e.x.is_finite() || !e.y.is_finite() {
                continue;
            }
            let mut px = e.x;
            let py = e.y;
            if e.w > 0.0 && e.w.is_finite() {
                // Kern segment: nearest point on the segment.
                if x >= e.x && x <= e.x + e.w {
                    px = x;
                } else if x > e.x + e.w {
                    px = e.x + e.w;
                }
            }
            let dx = x - px;
            let dy = y - py;
            dx * dx + dy * dy
        };
        if !d.is_finite() {
            continue;
        }
        let better = match near {
            None => true,
            Some((bd, bt, bl)) => d < bd || (d == bd && (e.tag < bt || (e.tag == bt && e.line < bl))),
        };
        if better {
            near = Some((d, e.tag, e.line));
        }
    }
    match near {
        Some((d2, tag, line)) => {
            if d2 <= INVERSE_SNAP * INVERSE_SNAP {
                Some((tag, line))
            } else {
                None
            }
        }
        None => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod synctex_tests {
    use super::*;

    const FIXTURE: &str = "SyncTeX Version:1\nInput:1:/tmp/wcsyncprobe/main.tex\nInput:2:\nInput:3:\nInput:4:\nInput:5:\nInput:6:\nOutput:pdf\nMagnification:1000\nUnit:1\nX Offset:0\nY Offset:0\nContent:\n!165\n{1\n[1,6:4736287,46220575:26673152,41484288,0\n(1,6:4736287,4736287:0,0,0\n)\n[1,6:8799519,46220575:22609920,40435712,0\n[1,6:8799519,6571295:22609920,786432,0\n(1,6:8799519,6571295:22609920,0,0\nh1,6:8799519,6571295:22609920,0,0\n)\n]\n[1,6:8799519,44254495:22609920,36044800,0\n(1,4:8799519,8865055:22609920,454820,7208\nh1,3:8799519,8865055:983040,0,0\ng1,3:11475354,8865055\ng1,3:12713329,8865055\nk1,4:31409439,8865055:17494835\ng1,4:31409439,8865055\n)\n(1,6:8799519,9651487:22609920,462029,135003\nh1,5:8799519,9651487:983040,0,0\ng1,5:12003574,9651487\ng1,5:15139472,9651487\nk1,6:31409439,9651487:14884536\ng1,6:31409439,9651487\n)\n]\n(1,6:8799519,46220575:22609920,436470,0\n(1,6:8799519,46220575:22609920,436470,0\nk1,6:19940639,46220575:11141120\nk1,6:31409439,46220575:11141120\n)\n)\n]\n]\n!776\n}1\nInput:7:\n!17\nPostamble:\nCount:34\n!24\nPost scriptum:\n";

    #[test]
    fn fixture_forward_lands_on_page_one_with_sane_rect() {
        let doc = parse_synctex_text(FIXTURE).unwrap();
        assert!(!doc.entries.is_empty());
        let Some((page, rect)) = forward_search(&doc, 3) else { panic!("no forward hit") };
        assert_eq!(page, 1);
        assert!(rect.w > 0.0 && rect.h > 0.0);
        assert!(rect.x.is_finite() && rect.y.is_finite() && rect.w.is_finite() && rect.h.is_finite());
        assert!(rect.w < 1000.0 && rect.h < 1000.0);
    }

    #[test]
    fn fixture_inverse_of_forward_center_returns_line_within_one() {
        let doc = parse_synctex_text(FIXTURE).unwrap();
        let Some((page, rect)) = forward_search(&doc, 3) else { panic!("no forward hit") };
        let cx = rect.x + rect.w / 2.0;
        let cy = rect.y + rect.h / 2.0;
        let Some((_tag, line)) = inverse_search(&doc, page, cx, cy) else { panic!("no inverse hit") };
        assert!((line as i64 - 3).abs() <= 1, "line {line} too far from 3");
    }

    #[test]
    fn forward_unknown_line_returns_none() {
        let doc = parse_synctex_text(FIXTURE).unwrap();
        assert!(forward_search(&doc, 9999).is_none());
        assert!(forward_search(&doc, 0).is_none());
    }

    #[test]
    fn inverse_far_point_returns_none() {
        let doc = parse_synctex_text(FIXTURE).unwrap();
        assert!(inverse_search(&doc, 1, 5000.0, 5000.0).is_none());
        assert!(inverse_search(&doc, 0, 100.0, 100.0).is_none());
        assert!(inverse_search(&doc, 1, f64::NAN, 100.0).is_none());
    }

    #[test]
    fn malformed_inputs_never_panic() {
        let bad = [
            "",
            "\n",
            "garbage without colons\n::: \n",
            "SyncTeX Version:1\nContent:\n{notanumber\n[abc\n(1,x:1,2:3,4,5\n",
            "Input:99999999999999999999:toolong\nContent:\n{1\n[1,1:9999999999999999999,9999999999999999999:9999999999999999999,9999999999999999999,9999999999999999999\n",
            "Magnification:banana\nUnit:nope\nX Offset:xxx\nContent:\n{1\n[1,1:1,2:3,4,5\n}1\n",
            "Content:\n{1\n] \n) \n!999\n}1\nPostamble:\n",
            "\u{0}\u{fffd} binary \nInput:1:\n",
            "Input:1:a\nInput:1:b\nContent:\n{1\n(1,1:10,20:30,40,50\n}1\n",
        ];
        for s in bad {
            let doc = parse_synctex_text(s).unwrap();
            let _ = forward_search(&doc, 1);
            let _ = inverse_search(&doc, 1, 10.0, 10.0);
            let _ = file_for_tag(&doc, 1);
        }
        let empty = parse_synctex_bytes(&[]).unwrap();
        assert!(empty.entries.is_empty());
        let gibberish = [0u8, 159, 255, 10, 0x1f, 0x8b, 0, 1, 2, 3];
        let _ = parse_synctex_bytes(&gibberish);
    }

    #[test]
    fn gzip_bytes_parse_like_plain_text() {
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(FIXTURE.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        assert!(gz.len() >= 2 && gz[0] == 0x1f && gz[1] == 0x8b);
        let from_gz = parse_synctex_bytes(&gz).unwrap();
        let from_txt = parse_synctex_text(FIXTURE).unwrap();
        assert_eq!(from_gz, from_txt);
        let plain = parse_synctex_bytes(FIXTURE.as_bytes()).unwrap();
        assert_eq!(plain, from_txt);
    }

    #[test]
    fn oversize_blob_is_a_clean_error() {
        let big = vec![b'a'; MAX_SYNCTEX_TEXT + 1];
        assert!(parse_synctex_bytes(&big).is_err());
        assert!(parse_synctex_text(&"a".repeat(MAX_SYNCTEX_TEXT + 1)).is_err());
    }
}
