//! Layout tab: page setup (margins, orientation, size, columns), breaks, line numbers, hyphenation.

use serde_json::{Value, json};
use wordcraft_doc::props::{Border, Borders};
use wordcraft_doc::section::{
    Columns, DocGrid, DocGridType, LineNumbering, MAX_CHAR_SPACE, MAX_GRID_PITCH, MAX_NOTE_START, MIN_GRID_PITCH, SectionProps, SectionStart,
};
use wordcraft_doc::{Block, Pos, StoryRef};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("layout.margins", "Margins", "Layout › Page Setup", margins).params(
            r#"{"preset"?: "normal|narrow|moderate|wide|mirrored|office2003", "top"?: pt, "bottom"?: pt, "left"?: pt, "right"?: pt, "gutter"?: pt, "mirror"?: bool}"#,
        ),
        CommandSpec::new("layout.orientation", "Orientation", "Layout › Page Setup", |s, v| {
            let land = match p::str(v, "value") {
                Some("landscape") => true,
                Some("portrait") => false,
                _ => !sect(s).landscape,
            };
            with_sect(s, |x| x.set_landscape(land))
        })
        .params(r#"{"value": "portrait|landscape"}"#),
        CommandSpec::new("layout.size", "Size", "Layout › Page Setup", size).params(r#"{"name"?: "Letter|Legal|A4|…", "width"?: pt, "height"?: pt} (each side clamped to 7.2–1584 pt, 0.1–22 in; the section keeps its orientation)"#),
        CommandSpec::new("layout.columns", "Columns", "Layout › Page Setup", columns)
            .params(
                r#"{"count": 1-12, "space"?: pt, "separator"?: bool, "preset"?: "left|right", "widths"?: [[width pt, space after pt], …] (unequal columns; sets the count), "apply"?: "section|document|forward|selection"} (default: selection when text is selected in the main text — section breaks around it — else section)"#,
            ),
        CommandSpec::new("layout.break", "Breaks", "Layout › Page Setup", breaks)
            .params(r#"{"kind": "page|column|textWrapping|nextPage|continuous|evenPage|oddPage"}"#),
        CommandSpec::new("layout.lineNumbers", "Line Numbers", "Layout › Page Setup", line_numbers).params(
            r#"{"value"?: "none|continuous|restartPage|restartSection", "start"?: 1-32767, "countBy"?: 1-100, "distance"?: pt (0 = auto)}"#,
        ),
        CommandSpec::new("layout.hyphenation", "Hyphenation", "Layout › Page Setup", hyphenation)
            .params(r#"{"value"?: bool (automatic; toggles when nothing is given), "zone"?: pt, "caps"?: bool (hyphenate words in capitals), "limit"?: n (consecutive hyphens, 0 = no limit)}"#),
        CommandSpec::new("layout.manualHyphenation", "Manual Hyphenation", "Layout › Page Setup › Hyphenation", manual_hyphenation)
            .params(r#"{"from"?: Pos (default: the caret)} → the next word at a line end that could be hyphenated: {"word", "pos", "points": [char index], "suggest": char index} or {"done": true}"#)
            .pure(),
        CommandSpec::new("layout.pageSetup", "Page Setup", "Layout › Page Setup", |s, v| {
            if let Some(props) = v.get("section") {
                let mut new: SectionProps = serde_json::from_value(props.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
                validate_section(&mut new)?;
                return with_sect(s, |x| *x = new.clone());
            }
            s.push_ui_request(json!({"open": "pageSetup"}));
            Ok(serde_json::to_value(sect(s)).unwrap_or(Value::Null))
        })
        .params(r#"{"section"?: SectionProps} (replaces the whole section, headers and footers included; checked like Size, Margins, Columns, Line Numbers, Footnotes and Page Borders: non-finite numbers and margins that leave no room for text are refused; page sides 7.2–1584 pt, 1–12 columns with 0–300 pt gaps, header/footer/gutter 0–1584 pt, page border width 0.25–6 pt and distance 0–31 pt, note start 1–32767, line numbers start 1–32767, countBy 1–100, distance 0–1584 pt are clamped)"#),
        CommandSpec::new("layout.verticalAlign", "Vertical Alignment", "Layout › Page Setup › Layout", |s, v| {
            let va = wordcraft_doc::section::PageVAlign::parse(p::str(v, "value").unwrap_or_default());
            with_sect(s, |x| x.valign = va)
        })
        .params(r#"{"value": "top|center|justify|bottom"} (also both|justified; anything else is top). Justified shares each page's spare room between its paragraphs and tables"#),
        CommandSpec::new("layout.sectionRtl", "Right-to-Left Section", "Layout › Page Setup", |s, v| {
            let on = p::bool(v, "value").unwrap_or(!sect(s).rtl);
            with_sect(s, |x| x.rtl = on)
        })
        .params(r#"{"value"?: bool} (true: the section's columns run right to left; without a value it toggles)"#),
        CommandSpec::new("layout.textDirection", "Text Direction", "Layout › Page Setup", text_direction).params(
            r#"{"value"?: "horizontal|down|up"} (also lrTb|tbRl|btLr; down: rotate all text 90°, lines top to bottom stacking right to left; up: 270°; default: the next after the current). In a table cell it turns the selected cells, in a text box the box; anywhere else the section(s) of the selection"#,
        ),
        CommandSpec::new("layout.differentFirstPage", "Different First Page", "Header & Footer › Options", |s, v| {
            let on = p::bool(v, "value").unwrap_or(!sect(s).title_page);
            with_sect(s, |x| x.title_page = on)
        }),
        CommandSpec::new("layout.differentOddEven", "Different Odd & Even Pages", "Header & Footer › Options", |s, v| {
            s.doc.settings.even_odd_headers = p::bool(v, "value").unwrap_or(!s.doc.settings.even_odd_headers);
            sel_result(s)
        }),
        CommandSpec::new("layout.pageNumberFormat", "Format Page Numbers", "Insert › Header & Footer › Page Number", |s, v| {
            let fmt = p::str(v, "format").map(wordcraft_doc::section::NumFormat::from_ooxml);
            with_sect(s, |x| {
                if let Some(f) = fmt {
                    x.page_num_format = f;
                }
                // Left out entirely: the numbering keeps its start. Given as null: the pages
                // number on from the section before (the dialog's "Continue from previous
                // section"). A number: numbering starts there.
                if v.get("start").is_some() {
                    x.page_num_start = p::u64(v, "start").map(|n| n.min(100_000) as u32);
                }
            })
        })
        .params(
            r#"{"format"?: "decimal|lowerRoman|upperRoman|lowerLetter|upperLetter", "start"?: n | null} ("start" left out: the numbering keeps its start; null: the pages number on from the section before; a number: numbering starts there)"#,
        ),
        CommandSpec::new("layout.documentGrid", "Document Grid", "Layout › Page Setup › Document Grid", document_grid).params(
            r#"{"type"?: "none|lines|linesAndChars|snapToChars", "linesPerPage"?: n (sets the line pitch from the text height), "charsPerLine"?: n (sets the character pitch from the text width), "linePitch"?: pt, "charSpace"?: n (4096ths of a point added to the default font size: the character pitch)} — left out stays as it is; returns the grid with its lines per page and characters per line"#,
        ),
        CommandSpec::new("layout.section", "Section Properties", "Layout › Page Setup", |s, _| {
            Ok(serde_json::to_value(sect(s)).unwrap_or(Value::Null))
        })
        .pure(),
    ]
}

/// Line Numbers: on with a restart mode (`value`), or off (`none`). `start`, `countBy` and
/// `distance` change the numbering's options and turn it on (keeping its restart mode).
fn line_numbers(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::section::LineNumberRestart as R;
    let mode = p::str(v, "value");
    if mode == Some("none") {
        return with_sect(s, |x| x.line_numbers = None);
    }
    let restart = match mode {
        Some("restartPage") => Some(R::Page),
        Some("restartSection") => Some(R::Section),
        Some("continuous") => Some(R::Continuous),
        Some(x) => return Err(CmdError::Params(format!("unknown value `{x}`"))),
        None => None,
    };
    let start = p::u64(v, "start").map(|n| n.clamp(1, 32_767) as u32);
    let count_by = p::u64(v, "countBy").map(|n| n.clamp(1, 100) as u32);
    let distance = p::f32(v, "distance").map(|d| d.clamp(0.0, 1584.0));
    let options = start.is_some() || count_by.is_some() || distance.is_some();
    with_sect(s, |x| {
        // Options alone keep the current numbering's other settings; a new mode alone starts over.
        let mut l = match (&x.line_numbers, options) {
            (Some(cur), true) => cur.clone(),
            (None, true) => LineNumbering { restart: R::Continuous, ..Default::default() },
            _ => LineNumbering::default(),
        };
        if let Some(r) = restart.or(if options { None } else { Some(R::Continuous) }) {
            l.restart = r;
        }
        if let Some(n) = start {
            l.start = n;
        }
        if let Some(n) = count_by {
            l.count_by = n;
        }
        if let Some(d) = distance {
            l.distance = d;
        }
        x.line_numbers = Some(l);
    })
}

/// Hyphenation: automatic on or off, and its options (Hyphenation Options).
fn hyphenation(s: &mut Session, v: &Value) -> CmdResult {
    let st = &mut s.doc.settings;
    let options = ["zone", "caps", "limit"].iter().any(|k| v.get(*k).is_some_and(|x| !x.is_null()));
    match p::bool(v, "value") {
        Some(on) => st.auto_hyphenation = on,
        None if !options => st.auto_hyphenation = !st.auto_hyphenation,
        None => {}
    }
    if let Some(z) = p::f32(v, "zone") {
        st.hyphenation_zone = z.clamp(0.0, 1584.0);
    }
    if let Some(c) = p::bool(v, "caps") {
        st.hyphenate_caps = c;
    }
    if let Some(n) = p::u64(v, "limit") {
        st.consecutive_hyphen_limit = n.min(32_767) as u32;
    }
    Ok(json!({"value": st.auto_hyphenation, "zone": st.hyphenation_zone, "caps": st.hyphenate_caps, "limit": st.consecutive_hyphen_limit}))
}

/// Manual hyphenation: the next word after `from` that starts a line after a line wrapped at a
/// space, and could end the line before it if hyphenated. `points` are where a hyphen may go
/// (char indices into `word`); `suggest` is the last one whose first part fits on that line.
/// Accepting is `caret.set` to `pos` plus the chosen point, then `text.optionalHyphen`.
fn manual_hyphenation(s: &mut Session, v: &Value) -> CmdResult {
    let from = match v.get("from") {
        Some(f) if !f.is_null() => super::parse_pos(f).ok_or_else(|| CmdError::Params("bad `from`".into()))?,
        _ => s.sel.ordered().0,
    };
    let lim = wordcraft_proof::hyphen::Limits::default();
    let caps = s.doc.settings.hyphenate_caps;
    let layout = s.layout();
    let mut best: Option<(Pos, Value)> = None;
    for page in &layout.pages {
        for it in &page.items {
            let wordcraft_layout::Placed::Lines { story, path, para, l0, l1, .. } = it else { continue };
            if *story != StoryRef::Body {
                continue;
            }
            let Some(text) = s.doc.para_at(&Pos { story: *story, path: path.clone(), off: 0 }).map(|p| p.text.clone()) else { continue };
            for li in (*l0).max(1)..*l1 {
                let (Some(prev), Some(line)) = (para.lines.get(li - 1), para.lines.get(li)) else { continue };
                if prev.end != wordcraft_layout::LineEnd::Wrap || prev.hyphen.is_some() {
                    continue;
                }
                let at = Pos { story: *story, path: path.clone(), off: line.start };
                if at < from || best.as_ref().is_some_and(|(b, _)| *b <= at) {
                    continue;
                }
                let Some(rest) = text.get(line.start..) else { continue };
                let len = rest.char_indices().find(|(_, c)| !(c.is_alphabetic() || *c == '\'' || *c == '\u{2019}')).map_or(rest.len(), |(i, _)| i);
                let Some(word) = rest.get(..len) else { continue };
                // A word with an optional hyphen already breaks where its author said.
                if rest.get(len..).is_some_and(|r| r.starts_with('\u{ad}'))
                    || word.chars().count() < lim.min_word
                    || (!caps && word.chars().all(char::is_uppercase))
                {
                    continue;
                }
                let points = wordcraft_proof::hyphen::hyphen_points(word, &lim);
                // The room left at the end of the line above, and the width of each first part.
                let room = prev.right - prev.end_x();
                let hyphen_w = para
                    .x_of(li, line.start)
                    .zip(para.x_of(li, line.start + word.chars().next().map_or(0, char::len_utf8)))
                    .map_or(6.0, |(a, b)| (b - a) * 0.6);
                let fits = |pt: usize| {
                    let bytes = word.char_indices().nth(pt).map_or(word.len(), |(i, _)| i);
                    match (para.x_of(li, line.start), para.x_of(li, line.start + bytes)) {
                        (Some(a), Some(b)) => (b - a) + hyphen_w <= room,
                        _ => false,
                    }
                };
                let Some(suggest) = points.iter().copied().filter(|pt| fits(*pt)).max() else { continue };
                best = Some((at.clone(), json!({"word": word, "pos": super::pos_json(&at), "points": points, "suggest": suggest})));
            }
        }
    }
    Ok(best.map(|(_, v)| v).unwrap_or_else(|| json!({"done": true})))
}

fn section_of_part(s: &Session, id: u32) -> usize {
    let sections = s.doc.sections();
    for (i, (_, sp)) in sections.iter().enumerate() {
        if [sp.headers.default, sp.headers.first, sp.headers.even, sp.footers.default, sp.footers.first, sp.footers.even].contains(&Some(id)) {
            return i;
        }
    }
    for (bi, block) in s.doc.body.iter().enumerate() {
        if let wordcraft_doc::Block::Para(p) = &**block {
            for o in &p.objects {
                if matches!(o, wordcraft_doc::para::InlineObject::NoteRef { id: nid, .. } if *nid == id) {
                    return s.doc.section_index_of(bi);
                }
            }
        }
    }
    0
}

pub fn section_index_of_focus(s: &Session) -> usize {
    match s.sel.focus.story {
        wordcraft_doc::StoryRef::Body => s.doc.section_index_of(s.sel.focus.path.0.first().copied().unwrap_or(0) as usize),
        wordcraft_doc::StoryRef::Part(id) => section_of_part(s, id),
    }
}

pub fn sect(s: &Session) -> SectionProps {
    let i = section_index_of_focus(s);
    s.doc.sections().get(i).map(|(_, x)| (*x).clone()).unwrap_or_default()
}

/// The last block of each section of the selection.
fn selected_section_ends(s: &Session) -> Vec<usize> {
    let (a, b) = s.sel.ordered();
    let ends: Vec<usize> = s.doc.sections().iter().map(|(e, _)| *e).collect();
    if a.story != wordcraft_doc::StoryRef::Body || b.story != wordcraft_doc::StoryRef::Body {
        let i = section_index_of_focus(s);
        return vec![ends.get(i).copied().unwrap_or(0)];
    }
    let (i0, i1) = (a.path.0.first().copied().unwrap_or(0) as usize, b.path.0.first().copied().unwrap_or(0) as usize);
    let first = ends.iter().position(|e| i0 <= *e).unwrap_or(0);
    let last = ends.iter().position(|e| i1 <= *e).unwrap_or(first);
    (first..=last).map(|k| ends.get(k).copied().unwrap_or(0)).collect()
}

/// Apply to the section(s) of the selection.
pub(crate) fn with_sect(s: &mut Session, f: impl Fn(&mut SectionProps)) -> CmdResult {
    for block in selected_section_ends(s) {
        f(s.doc.section_mut(block));
    }
    Ok(serde_json::to_value(sect(s)).unwrap_or(Value::Null))
}

/// The text direction Layout › Text Direction would change: the caret cell's, the text box's,
/// or the section's.
pub fn text_direction_at(s: &Session) -> wordcraft_doc::props::TextDirection {
    if let Some((tp, r, c)) = s.sel.focus.path.cell() {
        return s.doc.table(s.sel.focus.story, &tp).and_then(|t| t.rows.get(r)?.cells.get(c).map(|cl| cl.props.text_direction)).unwrap_or_default();
    }
    if let Some((_, id)) = super::objects::text_box_target(s) {
        return s.doc.parts.get(&id).map(|p| p.body.vert.turn()).unwrap_or_default();
    }
    sect(s).text_direction
}

/// Layout › Text Direction: like Word's button it works on what the caret is in — the selected
/// table cells, the text box, or else the section.
fn text_direction(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::props::{TextDirection, TextVert};
    let dir = match p::str(v, "value") {
        Some("horizontal" | "lrTb" | "horz") => Some(TextDirection::Horizontal),
        Some("down" | "tbRl" | "vert" | "rotate90") => Some(TextDirection::Down),
        Some("up" | "btLr" | "vert270" | "rotate270") => Some(TextDirection::Up),
        Some(x) => return Err(CmdError::Params(format!("unknown text direction `{x}`"))),
        None => None,
    };
    let dir = dir.unwrap_or(text_direction_at(s).next());
    if s.sel.focus.path.cell().is_some() {
        return super::table::text_direction(s, &json!({"value": dir.ooxml()}));
    }
    if super::objects::text_box_target(s).is_some() {
        let vert = match dir {
            TextDirection::Horizontal => TextVert::Horz,
            TextDirection::Down => TextVert::Vert,
            TextDirection::Up => TextVert::Vert270,
        };
        return super::objects::set_text_body(s, |b| b.vert = vert);
    }
    with_sect(s, |x| x.text_direction = dir)
}

/// The default font size (Normal style), which the character pitch is based on.
fn grid_font_size(s: &Session) -> f32 {
    let size = s.doc.styles.resolve_char(Some("Normal"), &wordcraft_doc::CharProps::default()).size;
    if size.is_finite() { size.clamp(1.0, 1584.0) } else { 10.5 }
}

fn grid_result(x: &SectionProps, font_size: f32) -> Value {
    let g = x.doc_grid.unwrap_or_default();
    json!({
        "type": if g.kind == DocGridType::Default { "none" } else { g.kind.ooxml() },
        "linePitch": g.line_pitch(),
        "charSpace": g.char_space,
        "charPitch": g.char_pitch(font_size),
        "linesPerPage": x.grid_lines_per_page(),
        "charsPerLine": x.grid_chars_per_line(font_size),
    })
}

/// Layout › Page Setup › Document Grid: the grid type, lines per page and characters per line.
fn document_grid(s: &mut Session, v: &Value) -> CmdResult {
    let kind = match p::str(v, "type") {
        None => None,
        Some("none" | "default") => Some(DocGridType::Default),
        Some(t) => Some(DocGridType::from_ooxml(t).ok_or_else(|| CmdError::Params(format!("unknown grid type `{t}`")))?),
    };
    let lines = p::u64(v, "linesPerPage").map(|n| n.clamp(1, 1000) as f32);
    let chars = p::u64(v, "charsPerLine").map(|n| n.clamp(1, 1000) as f32);
    let line_pitch = p::f32(v, "linePitch").map(|x| x.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH));
    let char_space =
        v.get("charSpace").and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(-(MAX_CHAR_SPACE as f64), MAX_CHAR_SPACE as f64) as i32);
    let size = grid_font_size(s);
    if kind.is_none() && lines.is_none() && chars.is_none() && line_pitch.is_none() && char_space.is_none() {
        return Ok(grid_result(&sect(s), size));
    }
    with_sect(s, |x| {
        let mut g = x.doc_grid.unwrap_or_default();
        if let Some(k) = kind {
            g.kind = k;
        }
        if let Some(n) = lines {
            g.line_pitch = (x.text_height() / n).clamp(MIN_GRID_PITCH, MAX_GRID_PITCH);
        }
        if let Some(pt) = line_pitch {
            g.line_pitch = pt;
        }
        if let Some(n) = chars {
            g.char_space = DocGrid::char_space_for(x.text_width() / n, size);
        }
        if let Some(cs) = char_space {
            g.char_space = cs;
        }
        x.doc_grid = Some(g);
    })?;
    Ok(grid_result(&sect(s), size))
}

/// Layout › Margins presets: id and top, bottom, left, right in points.
pub const MARGIN_PRESETS: [(&str, [f32; 4]); 6] = [
    ("normal", [72.0, 72.0, 72.0, 72.0]),
    ("narrow", [36.0, 36.0, 36.0, 36.0]),
    ("moderate", [72.0, 72.0, 54.0, 54.0]),
    ("wide", [72.0, 72.0, 144.0, 144.0]),
    ("mirrored", [72.0, 72.0, 90.0, 72.0]),
    ("office2003", [72.0, 72.0, 90.0, 90.0]),
];

/// The Margins preset the caret's section is set to, if any (Mirrored only with mirror margins
/// on, the others only with it off), so the menu can mark it.
pub fn margins_preset_at(s: &Session) -> Option<&'static str> {
    let x = sect(s);
    let m = [x.margin_top, x.margin_bottom, x.margin_left, x.margin_right];
    MARGIN_PRESETS
        .iter()
        .find(|(id, p)| (*id == "mirrored") == s.doc.settings.mirror_margins && p.iter().zip(m).all(|(a, b)| (a - b).abs() < 0.5))
        .map(|(id, _)| *id)
}

/// The Columns preset the caret's section matches: `one`, `two`, `three` (equal columns), or
/// `left` / `right` (two columns, the narrower one on that side).
pub fn columns_preset_at(s: &Session) -> Option<&'static str> {
    let c = sect(s).columns;
    let first = c.widths.first().map(|w| w.0);
    let equal = c.widths.iter().all(|w| first.is_some_and(|f| (w.0 - f).abs() < 0.5));
    match (c.count, c.widths.as_slice()) {
        (1, _) => Some("one"),
        (2, [a, b]) if a.0 < b.0 - 0.5 => Some("left"),
        (2, [a, b]) if a.0 > b.0 + 0.5 => Some("right"),
        (2, _) if equal => Some("two"),
        (3, _) if equal => Some("three"),
        _ => None,
    }
}

/// Checks a whole section handed to `layout.pageSetup` by the rules of Size, Margins, Columns,
/// Line Numbers, Footnotes and Page Borders: a non-finite number or margins that leave no room
/// for text is an error; the rest is clamped to the ranges those commands use.
fn validate_section(x: &mut SectionProps) -> Result<(), CmdError> {
    let bad = |what: &str| CmdError::Params(format!("section: {what} must be a finite number"));
    let finite = [x.page_w, x.page_h, x.margin_top, x.margin_bottom, x.margin_left, x.margin_right, x.header, x.footer, x.gutter, x.columns.space];
    if finite.iter().any(|n| !n.is_finite()) {
        return Err(bad("sizes, margins, distances and column gaps"));
    }
    if x.columns.widths.iter().any(|(w, sp)| !w.is_finite() || !sp.is_finite()) {
        return Err(bad("column widths"));
    }
    if x.line_numbers.as_ref().is_some_and(|l| !l.distance.is_finite()) {
        return Err(bad("the line number distance"));
    }
    if x.doc_grid.as_ref().is_some_and(|g| !g.line_pitch.is_finite()) {
        return Err(bad("the grid pitch"));
    }
    if x.page_borders.as_ref().is_some_and(|b| border_sides(b).any(|d| !d.width.is_finite() || !d.space.is_finite())) {
        return Err(bad("page border widths and distances"));
    }
    x.page_w = x.page_w.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE);
    x.page_h = x.page_h.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE);
    // Like Size, the orientation follows the sides.
    if x.page_w != x.page_h {
        x.landscape = x.page_w > x.page_h;
    }
    x.margin_top = x.margin_top.max(0.0);
    x.margin_bottom = x.margin_bottom.max(0.0);
    x.margin_left = x.margin_left.max(0.0);
    x.margin_right = x.margin_right.max(0.0);
    x.header = x.header.clamp(0.0, x.page_h);
    x.footer = x.footer.clamp(0.0, x.page_h);
    x.gutter = x.gutter.clamp(0.0, MAX_PAGE_SIDE);
    // A page narrower than the usual text room (the smallest is 0.1") needs only margins that
    // leave the whole of it: zero margins, gutter included.
    let room = |side: f32| MIN_TEXT_ROOM.min(side);
    if x.margin_left + x.margin_right + x.gutter > x.page_w - room(x.page_w) || x.margin_top + x.margin_bottom > x.page_h - room(x.page_h) {
        return Err(CmdError::Params("margins leave no room for text".into()));
    }
    x.columns.space = x.columns.space.clamp(0.0, 300.0);
    x.columns.widths.truncate(12);
    for (w, sp) in &mut x.columns.widths {
        *w = w.clamp(18.0, MAX_PAGE_SIDE);
        *sp = sp.clamp(0.0, 300.0);
    }
    // As in Columns: custom widths set the count; a single width is no custom layout.
    if x.columns.widths.len() > 1 {
        x.columns.count = x.columns.widths.len() as u32;
    } else {
        x.columns.widths.clear();
        x.columns.count = x.columns.count.clamp(1, 12);
    }
    if let Some(l) = &mut x.line_numbers {
        l.start = l.start.clamp(1, 32_767);
        l.count_by = l.count_by.clamp(1, 100);
        l.distance = l.distance.clamp(0.0, 1584.0);
    }
    if let Some(g) = &mut x.doc_grid {
        g.line_pitch = g.line_pitch.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH);
        g.char_space = g.char_space.clamp(-MAX_CHAR_SPACE, MAX_CHAR_SPACE);
    }
    x.page_num_start = x.page_num_start.map(|n| n.min(100_000));
    for n in [&mut x.footnote_pr, &mut x.endnote_pr] {
        n.num_start = n.num_start.map(|v| v.clamp(1, MAX_NOTE_START));
    }
    // Page Borders: width 0.25–6 pt, distance from the text 0–31 pt.
    if let Some(b) = &mut x.page_borders {
        for d in border_sides_mut(b) {
            d.width = d.width.clamp(0.25, 6.0);
            d.space = d.space.clamp(0.0, 31.0);
        }
    }
    Ok(())
}

fn border_sides(b: &Borders) -> impl Iterator<Item = &Border> {
    [&b.top, &b.left, &b.bottom, &b.right, &b.between, &b.inside_v].into_iter().flatten()
}

fn border_sides_mut(b: &mut Borders) -> impl Iterator<Item = &mut Border> {
    [&mut b.top, &mut b.left, &mut b.bottom, &mut b.right, &mut b.between, &mut b.inside_v].into_iter().flatten()
}

fn margins(s: &mut Session, v: &Value) -> CmdResult {
    let preset = p::str(v, "preset");
    let (t, b, l, r) = match preset {
        Some(x) => match MARGIN_PRESETS.iter().find(|(id, _)| *id == x) {
            Some((_, [t, b, l, r])) => (*t, *b, *l, *r),
            None => return Err(CmdError::Params(format!("unknown preset `{x}`"))),
        },
        None => {
            let c = sect(s);
            (
                p::f32(v, "top").unwrap_or(c.margin_top),
                p::f32(v, "bottom").unwrap_or(c.margin_bottom),
                p::f32(v, "left").unwrap_or(c.margin_left),
                p::f32(v, "right").unwrap_or(c.margin_right),
            )
        }
    };
    // Negative margins count as zero before the fit check, so one can't offset another.
    let (t, b, l, r) = (t.max(0.0), b.max(0.0), l.max(0.0), r.max(0.0));
    let gutter = p::f32(v, "gutter").map(|g| g.max(0.0));
    let cur = sect(s);
    if !margins_fit((cur.page_w, cur.page_h), (t, b, l, r), gutter.unwrap_or(cur.gutter)) {
        return Err(CmdError::Params("margins leave no room for text".into()));
    }
    // A preset picks mirrored or not, as does `mirror` (Page Setup's Multiple pages); plain numbers
    // alone (a ruler drag, an agent) keep the setting.
    if let Some(on) = p::bool(v, "mirror").or(preset.map(|k| k == "mirrored")) {
        s.doc.settings.mirror_margins = on;
    }
    with_sect(s, |x| {
        x.margin_top = t;
        x.margin_bottom = b;
        x.margin_left = l;
        x.margin_right = r;
        if let Some(g) = gutter {
            x.gutter = g;
        }
    })
}

/// Why `layout.size` refuses a page its margins don't fit.
pub const MARGINS_DONT_FIT: &str = "The margins don't fit on a page this size; make the margins smaller first.";

/// Text room the margins must leave on a page, each way, points.
pub const MIN_TEXT_ROOM: f32 = 36.0;

/// Whether margins (top, bottom, left, right) and a gutter leave text room on a page (width,
/// height), all in points: the rule `layout.margins` and `layout.size` both apply, and Page
/// Setup checks before it runs either.
pub fn margins_fit(page: (f32, f32), (t, b, l, r): (f32, f32, f32, f32), gutter: f32) -> bool {
    l + r + gutter <= page.0 - MIN_TEXT_ROOM && t + b <= page.1 - MIN_TEXT_ROOM
}

/// The smallest and largest page side, points (0.1" and 22", as Word allows).
pub const MIN_PAGE_SIDE: f32 = 7.2;
pub const MAX_PAGE_SIDE: f32 = 1584.0;

fn size(s: &mut Session, v: &Value) -> CmdResult {
    let (w, h) = if let Some(n) = p::str(v, "name") {
        let (_, w, h) = wordcraft_geom::PAPER_SIZES
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(n))
            .ok_or_else(|| CmdError::Params(format!("unknown paper `{n}`")))?;
        (*w, *h)
    } else {
        (p::req_f32(v, "width")?, p::req_f32(v, "height")?)
    };
    // Word's range per side is 0.1"–22" (#1433); `p::f32` already refused NaN and infinities.
    let (w, h) = (w.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE), h.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE));
    // Each section keeps its orientation: the long side across in landscape, down in portrait.
    let oriented = |x: &SectionProps| if x.landscape { (w.max(h), w.min(h)) } else { (w.min(h), w.max(h)) };
    let sections = s.doc.sections();
    let fits = |block: usize| {
        sections
            .iter()
            .find(|(e, _)| *e == block)
            .is_none_or(|(_, x)| margins_fit(oriented(x), (x.margin_top, x.margin_bottom, x.margin_left, x.margin_right), x.gutter))
    };
    if !selected_section_ends(s).into_iter().all(fits) {
        return Err(CmdError::Params(MARGINS_DONT_FIT.into()));
    }
    with_sect(s, |x| (x.page_w, x.page_h) = oriented(x))
}

fn columns(s: &mut Session, v: &Value) -> CmdResult {
    let preset = p::str(v, "preset");
    let space = p::f32(v, "space").unwrap_or(36.0).clamp(0.0, 300.0);
    let sep = p::bool(v, "separator").unwrap_or(false);
    let tw = sect(s).text_width();
    // Unequal columns: (width, space after) each, as the Columns dialog sends them.
    let custom = match v.get("widths").filter(|w| !w.is_null()) {
        Some(w) => Some(column_widths(w, tw)?),
        None => None,
    };
    let count = match &custom {
        Some(c) => c.len() as u64,
        None => p::u64(v, "count").unwrap_or(match preset {
            Some("left") | Some("right") => 2,
            _ => 1,
        }),
    };
    if !(1..=12).contains(&count) {
        return Err(CmdError::Params("columns must be 1–12".into()));
    }
    let widths = match (custom, preset) {
        (Some(c), _) if c.len() > 1 => c,
        (Some(_), _) => Vec::new(),
        (None, Some("left")) => vec![((tw - space) / 3.0, space), ((tw - space) * 2.0 / 3.0, 0.0)],
        (None, Some("right")) => vec![((tw - space) * 2.0 / 3.0, space), ((tw - space) / 3.0, 0.0)],
        _ => Vec::new(),
    };
    // Equal columns keep one spacing; unequal ones remember the first gap for a later switch back.
    let space = widths.first().map(|w| w.1).unwrap_or(space);
    let cols = Columns { count: count as u32, space, separator: sep, widths };
    // With text selected the gallery works on that text, as in Word (#2024).
    let default = if selection_sectionable(s) { "selection" } else { "section" };
    match p::str(v, "apply").unwrap_or(default) {
        "section" => with_sect(s, |x| x.columns = cols.clone()),
        "document" => {
            let ends: Vec<usize> = s.doc.sections().iter().map(|(e, _)| *e).collect();
            for e in ends {
                s.doc.section_mut(e).columns = cols.clone();
            }
            Ok(serde_json::to_value(sect(s)).unwrap_or(Value::Null))
        }
        // From the caret on: a continuous section break first, then the new section's columns.
        "forward" => {
            let start = s.sel.ordered().0;
            s.sel = Selection::caret(start);
            breaks(s, &json!({"kind": "continuous"}))?;
            with_sect(s, |x| x.columns = cols.clone())
        }
        "selection" => {
            selection_to_sections(s)?;
            with_sect(s, |x| x.columns = cols.clone())
        }
        x => Err(CmdError::Params(format!("unknown apply `{x}` (section, document, forward or selection)"))),
    }
}

/// Whether the selection can become sections of its own: text selected in the main text, outside
/// tables.
pub fn selection_sectionable(s: &Session) -> bool {
    let (a, b) = s.sel.ordered();
    !s.sel.is_collapsed() && [a, b].iter().all(|p| p.story == StoryRef::Body && p.path.depth() == 0)
}

/// Columns for the selected text (#2024), as Word does it: a continuous section break where the
/// selection starts and one where it ends, splitting paragraphs there, unless that end already
/// meets a section boundary (the document's start or end, or a section break). The selection keeps
/// its text, which is then the section(s) between.
fn selection_to_sections(s: &mut Session) -> Result<(), CmdError> {
    if !selection_sectionable(s) {
        return Err(CmdError::Params("selected text in the main text, outside tables, is needed".into()));
    }
    let old = s.sel.clone();
    let (a, mut b) = old.ordered();
    let block = |p: &Pos| p.path.0.first().copied().unwrap_or(0) as usize;
    let len = |s: &Session, p: &Pos| s.doc.para_at(p).map(|q| q.len()).unwrap_or(0);
    let ends: Vec<usize> = s.doc.sections().iter().map(|(e, _)| *e).collect();
    let after_break = |i: usize| i.checked_sub(1).is_some_and(|j| ends.contains(&j));
    // A selection that took a section break with it ends before that break.
    if b.off == 0 && block(&b) > block(&a) && after_break(block(&b)) {
        let prev = Pos { path: wordcraft_doc::Path::top(block(&b) - 1), off: 0, ..b.clone() };
        b = Pos { off: len(s, &prev), ..prev };
    }
    let last_block = s.doc.body.len().saturating_sub(1);
    let end_break = !(b.off == len(s, &b) && (ends.contains(&block(&b)) || block(&b) == last_block));
    let start_break = !(a.off == 0 && (block(&a) == 0 || after_break(block(&a))));
    // The end first: the start's position doesn't move.
    if end_break {
        s.sel = Selection::caret(b.clone());
        breaks(s, &json!({"kind": "continuous"}))?;
    }
    let (first, last) = if start_break {
        s.sel = Selection::caret(a.clone());
        breaks(s, &json!({"kind": "continuous"}))?;
        let first = s.sel.focus.clone();
        let last = shifted_by_split(&b, &a, &first);
        (first, last)
    } else {
        (a, b)
    };
    s.sel = if old.anchor <= old.focus { Selection { anchor: first, focus: last } } else { Selection { anchor: last, focus: first } };
    Ok(())
}

/// `[[width, space after], …]` in points: 1–12 columns, each at least 0.25", that fit the text
/// width (`tw`).
fn column_widths(v: &Value, tw: f32) -> Result<Vec<(f32, f32)>, CmdError> {
    let bad = || CmdError::Params("widths: [[width, space after], …] in points, 1–12 columns".into());
    let a = v.as_array().filter(|a| (1..=12).contains(&a.len())).ok_or_else(bad)?;
    let mut out = Vec::with_capacity(a.len());
    for (i, c) in a.iter().enumerate() {
        let num = |k: usize| c.get(k).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite());
        let w = num(0).ok_or_else(bad)?;
        // The last column has no space after it.
        let sp = if i + 1 == a.len() { 0.0 } else { num(1).unwrap_or(0.0) };
        if w < 18.0 || !(0.0..=300.0).contains(&sp) {
            return Err(CmdError::Params("each column must be at least 0.25\" wide, with 0–300 pt after it".into()));
        }
        out.push((w, sp));
    }
    let total: f32 = out.iter().map(|(w, sp)| w + sp).sum();
    if total > tw + 1.0 {
        return Err(CmdError::Params(format!("the columns ({total:.0} pt) are wider than the text ({tw:.0} pt)")));
    }
    Ok(out)
}

fn breaks(s: &mut Session, v: &Value) -> CmdResult {
    let kind = p::str(v, "kind").unwrap_or("page");
    let start = match kind {
        "page" => return super::text::specs().iter().find(|c| c.id == "text.pageBreak").map(|c| (c.run)(s, v)).unwrap_or_else(|| sel_result(s)),
        "column" => return super::text::specs().iter().find(|c| c.id == "text.columnBreak").map(|c| (c.run)(s, v)).unwrap_or_else(|| sel_result(s)),
        "textWrapping" => {
            return super::text::specs().iter().find(|c| c.id == "text.lineBreak").map(|c| (c.run)(s, v)).unwrap_or_else(|| sel_result(s));
        }
        "nextPage" => SectionStart::NextPage,
        "continuous" => SectionStart::Continuous,
        "evenPage" => SectionStart::EvenPage,
        "oddPage" => SectionStart::OddPage,
        x => return Err(CmdError::Params(format!("unknown break `{x}`"))),
    };
    // A section break goes just before the selection and never replaces it, as in Word (#852:
    // selected text was deleted, so the first section lost its text).
    let old = s.sel.clone();
    let (at, end) = old.ordered();
    if at.story != wordcraft_doc::StoryRef::Body || at.path.depth() > 0 {
        return Err(CmdError::Failed("section breaks go in the main text".into()));
    }
    // Section break: the paragraph before the caret ends a section with the current props; the
    // new section (after) takes them too and starts as requested.
    s.sel = Selection::caret(at.clone());
    let cur = sect(s);
    let new = match s.doc.split_paragraph(&at) {
        Ok(p) => p,
        Err(e) => {
            s.sel = old;
            return Err(e.into());
        }
    };
    let ended = at.path.clone();
    let para = s.doc.para_mut(at.story, &ended)?;
    let mut props = cur;
    // The start type belongs to the section that begins after the break.
    let next_block = new.path.last();
    para.section = Some(Box::new(props.clone()));
    props.start = start;
    *s.doc.section_mut(next_block) = props;
    // The selection keeps its text, which now starts the new section.
    let first = Pos { off: 0, ..new };
    let last = shifted_by_split(&end, &at, &first);
    s.sel = if old.anchor <= old.focus { Selection { anchor: first, focus: last } } else { Selection { anchor: last, focus: first } };
    let _ = Block::Para;
    sel_result(s)
}

/// Where `p` lands after the top-level body paragraph at `at` was split there, the second half
/// starting at `new`: later text in that paragraph moves into the new one, later blocks move down.
fn shifted_by_split(p: &Pos, at: &Pos, new: &Pos) -> Pos {
    if p.story != at.story {
        return p.clone();
    }
    if p.path == at.path {
        return if p.off >= at.off { Pos { off: p.off.saturating_sub(at.off), ..new.clone() } } else { p.clone() };
    }
    let mut q = p.clone();
    if let (Some(first), Some(i)) = (q.path.0.first_mut(), at.path.0.first())
        && *first > *i
    {
        *first = first.saturating_add(1);
    }
    q
}

#[cfg(test)]
mod tests;
