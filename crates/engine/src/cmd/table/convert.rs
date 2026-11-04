use super::cells::{cell, cell_lang, cell_text, selected_cells, with_table};
use super::layout::autofit;
use super::{
    Block, CmdError, CmdResult, InlineObject, Paragraph, Path, Pos, Selection, Session, SortKey, StoryRef, Table, Value, json, p, para_block,
    parse_number, sel_result, sort_key,
};

/// Sort the table's rows by a column. Numbers are read in each cell's language (`1,5` is one and
/// a half in Polish) and sort first, numerically; other text sorts in the column's language.
pub(super) fn sort_table(s: &mut Session, v: &Value) -> CmdResult {
    let col = p::u64(v, "column").unwrap_or(0) as usize;
    let desc = p::bool(v, "descending").unwrap_or(false);
    let header = p::bool(v, "header");
    let (tp, _, _) = cell(s)?;
    let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    let skip = usize::from(header.unwrap_or(t.rows.first().is_some_and(|r| r.props.header))).min(t.rows.len());
    let column = || t.rows.iter().skip(skip).map(|r| r.cells.get(col));
    let lang = column().flatten().find_map(|c| cell_lang(&s.doc, c)).or_else(|| crate::cmd::default_lang(s));
    // A total order (numbers, then text), so sorting never sees inconsistent comparisons.
    let keys: Vec<(Option<f64>, SortKey)> = column()
        .map(|c| {
            let text = c.map(cell_text).unwrap_or_default();
            let num_lang = c.and_then(|c| cell_lang(&s.doc, c)).or_else(|| lang.clone());
            let num = parse_number(text.trim(), num_lang.as_deref()).filter(|x| x.is_finite());
            (num, if num.is_some() { SortKey::default() } else { sort_key(text.trim(), lang.as_deref()) })
        })
        .collect();
    with_table(s, move |t| {
        if skip > t.rows.len() {
            return;
        }
        let body = t.rows.split_off(skip);
        let mut rows: Vec<_> = keys.iter().cloned().zip(body).collect();
        rows.sort_by(|((a, ka), _), ((b, kb), _)| {
            let o = match (a, b) {
                (Some(x), Some(y)) => x.total_cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => ka.cmp(kb),
            };
            if desc { o.reverse() } else { o }
        });
        t.rows.extend(rows.into_iter().map(|(_, r)| r));
    })
}

/// Table Layout › Data › Convert to Text: the selected rows, or the whole table with the caret
/// in one cell, become paragraphs. Each row is one paragraph with its cells set apart by the
/// separator (a tab, a comma or another character); with `paragraph` each cell paragraph
/// stays a separate paragraph. The text keeps its formatting and objects, a table nested in a cell stays
/// a table, and rows above and below the selected ones stay a table each.
pub(super) fn to_text(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let sep = match p::str(v, "separator") {
        None | Some("tab") => Some('\t'),
        Some("comma") => Some(','),
        Some("paragraph") => None,
        Some("other") => match p::str(v, "char").and_then(|c| c.chars().next()) {
            // U+FFFC marks an object in the text, so it cannot set the cells apart.
            Some(c) if c != '\n' && c != '\r' && c != wordcraft_doc::para::OBJ => Some(c),
            _ => return Err(CmdError::Params("`char` (the separating character) is required with `other`".into())),
        },
        Some(x) => return Err(CmdError::Params(format!("unknown separator `{x}`"))),
    };
    let story = s.sel.focus.story;
    let mut t = s.doc.table(story, &tp).cloned().ok_or_else(|| CmdError::Failed("no table".into()))?;
    // Several cells selected: their rows. A caret (or a selection inside one cell): every row.
    let cells = selected_cells(s);
    let (r0, r1) = match (cells.iter().map(|x| x.0).min(), cells.iter().map(|x| x.0).max()) {
        (Some(a), Some(b)) if cells.len() > 1 || s.cell_range().is_some() => (a, b.min(t.rows.len().saturating_sub(1))),
        _ => (0, t.rows.len().saturating_sub(1)),
    };
    // `split_off` ends the vertical merges and row content controls that a cut crosses, and opens
    // them again (without their ids) below it.
    let mut tail = t.split_off(r1 + 1);
    let rows = t.split_off(r0).rows;
    t.controls.close = std::mem::take(&mut tail.controls.close);
    let mut blocks = Vec::new();
    if !t.rows.is_empty() {
        let mut head = Table { props: t.props.clone(), grid: t.grid.clone(), rows: std::mem::take(&mut t.rows), controls: Default::default() };
        head.controls.open = std::mem::take(&mut t.controls.open);
        blocks.push(Block::Table(head));
    }
    let text_from = blocks.len();
    for row in rows {
        row_to_text(row, sep, &mut blocks);
    }
    // Rows without cells (only from a damaged file) still leave a paragraph in their place.
    if blocks.len() == text_from {
        blocks.push(Block::Para(Paragraph::new()));
    }
    // The caret goes to the first converted paragraph (into a nested table when there is none).
    let caret = match blocks.iter().enumerate().skip(text_from).find(|(_, b)| matches!(b, Block::Para(_))) {
        Some((k, _)) => tp.with_last(tp.last() + k),
        None => Path(tp.with_last(tp.last() + text_from).0.into_iter().chain([0, 0, 0]).collect()),
    };
    if !tail.rows.is_empty() {
        tail.controls.close = std::mem::take(&mut t.controls.close);
        // A floating table keeps its position in the part above; the part below goes in the text
        // flow.
        if text_from > 0 {
            tail.props.float = None;
        }
        blocks.push(Block::Table(tail));
    }
    // A content control around the table keeps both ends: on the text where no table part is
    // left to carry them.
    let (open, close) = (std::mem::take(&mut t.controls.open), std::mem::take(&mut t.controls.close));
    match blocks.get_mut(text_from) {
        Some(Block::Para(p)) => {
            for (k, mut control) in open.into_iter().enumerate() {
                control.block = true;
                let start = InlineObject::ControlStart { control: Box::new(control) };
                p.insert_object(k * wordcraft_doc::para::OBJ.len_utf8(), start, &Default::default())?;
            }
        }
        Some(Block::Table(x)) => {
            let inner = std::mem::replace(&mut x.controls.open, open);
            x.controls.open.extend(inner);
        }
        None => {}
    }
    match blocks.last_mut() {
        Some(Block::Para(p)) => {
            // The count is read from the file; at most 1,024 ends go onto the text.
            for _ in 0..close.min(1024) {
                let end = p.len();
                p.insert_object(end, InlineObject::ControlEnd, &Default::default())?;
            }
        }
        Some(Block::Table(x)) => x.controls.close = x.controls.close.saturating_add(close),
        None => {}
    }
    s.doc.remove_block(story, &tp)?;
    for (k, b) in blocks.into_iter().enumerate() {
        s.doc.insert_block(story, &tp.with_last(tp.last() + k), b)?;
    }
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: caret, off: 0 }));
    sel_result(s)
}

/// One row of Convert to Text as blocks: a paragraph per row (and per further paragraph in a
/// cell) with the cells set apart by `sep`, or each cell's blocks as they are without one.
fn row_to_text(row: wordcraft_doc::Row, sep: Option<char>, out: &mut Vec<Block>) {
    let Some(sep) = sep else {
        for b in row.cells.into_iter().flat_map(|c| c.blocks) {
            out.push(match std::sync::Arc::unwrap_or_clone(b) {
                Block::Para(mut para) => {
                    para.section = None;
                    para.touch();
                    Block::Para(para)
                }
                other => other,
            });
        }
        return;
    };
    let mut line: Option<Paragraph> = None;
    for (ci, cell) in row.cells.into_iter().enumerate() {
        if ci > 0 {
            // The separator takes no formatting from the text before it.
            line.get_or_insert_with(Paragraph::new).append(Paragraph::with_text(sep.encode_utf8(&mut [0; 4]), Default::default()));
        }
        let mut first = true;
        for b in cell.blocks {
            match std::sync::Arc::unwrap_or_clone(b) {
                Block::Para(mut para) => {
                    para.section = None;
                    match line.as_mut() {
                        Some(p) if first => p.append(para),
                        _ => {
                            if let Some(done) = line.take() {
                                out.push(Block::Para(done));
                            }
                            para.touch();
                            line = Some(para);
                        }
                    }
                }
                other => {
                    // A nested table stays a table between the row's paragraphs.
                    if let Some(done) = line.take() {
                        out.push(Block::Para(done));
                    }
                    out.push(other);
                }
            }
            first = false;
        }
    }
    if let Some(done) = line {
        out.push(Block::Para(done));
    }
}

/// Table Layout › Data › Formula: inserts a formula field at the caret, evaluated now and again
/// by Update Field. What a formula can use is described in [`super::table_formula`]. A formula
/// that doesn't evaluate (an unknown function, a syntax error) is an error and inserts nothing.
pub(super) fn formula(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let raw = p::str(v, "formula").unwrap_or("=SUM(ABOVE)");
    let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    let default_lang = crate::cmd::default_lang(s);
    let instr = auto_formula_instr(t, r, c, raw, default_lang.as_deref());
    let text = crate::cmd::table_formula::evaluate_field_with(t, Some(&s.doc), default_lang.as_deref(), r, c, &instr)?;
    let props = s.typing_props();
    let at = s.sel.focus.clone();
    let end = s.doc.insert_object(
        &at,
        wordcraft_doc::InlineObject::Field {
            instr: instr.clone(),
            result: text.clone(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
            code_xml: String::new(),
        },
        &props,
    )?;
    s.sel = Selection::caret(end);
    Ok(json!({"result": text, "formula": instr}))
}

/// A formula as a field stores it: upper case, with one leading `=`. The `\#` numeric picture and
/// the `\*` switches are kept as typed.
pub(crate) fn formula_instr(raw: &str) -> String {
    let body = raw.trim();
    let body = body.strip_prefix('=').unwrap_or(body);
    match crate::cmd::table_formula::split_switch(body) {
        (formula, Some(switch)) => format!("={} {switch}", formula.trim().to_ascii_uppercase()),
        (formula, None) => format!("={}", formula.trim().to_ascii_uppercase()),
    }
}

/// A bare function name (`=SUM`, no parentheses) takes its direction from the cell: ABOVE when a
/// cell above holds a number, else LEFT.
fn auto_formula_instr(t: &Table, r: usize, c: usize, raw: &str, lang: Option<&str>) -> String {
    let instr = formula_instr(raw);
    let body = instr.strip_prefix('=').unwrap_or(&instr);
    if !body.contains('(') && matches!(body, "SUM" | "AVERAGE" | "COUNT" | "MAX" | "MIN" | "PRODUCT") {
        let g = t.grid_col(r, c);
        let has_above = (0..r).rev().any(|row| {
            t.cell_at_grid(row, g)
                .and_then(|ci| t.rows.get(row)?.cells.get(ci))
                .and_then(|cl| crate::cmd::table_formula::cell_number_in(cl, lang))
                .is_some()
        });
        let dir = if has_above { "ABOVE" } else { "LEFT" };
        format!("={body}({dir})")
    } else {
        instr
    }
}

/// The paragraphs Convert Text to Table would convert: the selected ones, as text. Refused
/// rather than dropping content when a table lies between them.
pub fn text_table_lines(s: &Session) -> Result<Vec<String>, CmdError> {
    let (a, b) = s.sel.ordered();
    if a.path.parent() != b.path.parent() || a.story != b.story {
        return Err(CmdError::Failed("select paragraphs to convert".into()));
    }
    let lines: Vec<String> =
        (a.path.last()..=b.path.last()).filter_map(|i| s.doc.para(a.story, &a.path.with_last(i)).map(|p| p.plain_text())).collect();
    // Refuse rather than drop content: a table between the paragraphs would be removed with them.
    if lines.len() != b.path.last() - a.path.last() + 1 {
        return Err(CmdError::Failed("select only paragraphs to convert".into()));
    }
    Ok(lines)
}

/// The cells Convert Text to Table makes of `lines`, row by row, and the column count. `sep`
/// `None` separates at paragraphs: each paragraph is a cell, `columns` (default 1) to a row.
/// Otherwise each paragraph starts a row; `columns` defaults to the most fields in one, and a
/// paragraph with more fields than that carries on over the next rows.
pub fn text_table_cells(lines: &[String], sep: Option<char>, columns: Option<usize>) -> (Vec<Vec<&str>>, usize) {
    let Some(sep) = sep else {
        let cols = columns.unwrap_or(1).clamp(1, wordcraft_doc::table::MAX_COLS);
        return (lines.chunks(cols).map(|ch| ch.iter().map(String::as_str).collect()).collect(), cols);
    };
    let cols = columns.unwrap_or_else(|| lines.iter().map(|l| l.split(sep).count()).max().unwrap_or(1)).max(1);
    let mut rows = Vec::new();
    for l in lines {
        let fields: Vec<&str> = l.split(sep).collect();
        rows.extend(fields.chunks(cols.min(wordcraft_doc::table::MAX_COLS)).map(<[&str]>::to_vec));
    }
    (rows, cols)
}

/// The cells Convert Text to Table makes of `paras`, row by row, and the column count, as
/// [`text_table_cells`] makes them of their text. Each cell is its field of the source paragraph
/// cut out of it, trimmed of surrounding white space, so its formatting, objects (pictures,
/// fields, notes…) and tracked changes go with it.
fn text_table_paras(paras: Vec<Paragraph>, sep: Option<char>, columns: Option<usize>) -> Result<(Vec<Vec<Paragraph>>, usize), CmdError> {
    use wordcraft_doc::para::OBJ;
    use wordcraft_doc::table::MAX_COLS;
    // Each paragraph's fields as byte ranges; an object is never a separator.
    let fields: Vec<Vec<std::ops::Range<usize>>> = paras
        .iter()
        .map(|p| {
            let mut out = Vec::new();
            let mut from = 0;
            if let Some(sep) = sep.filter(|c| *c != OBJ) {
                for (i, c) in p.text.char_indices().filter(|(_, c)| *c == sep) {
                    out.push(from..i);
                    from = i + c.len_utf8();
                }
            }
            out.push(from..p.text.len());
            out
        })
        .collect();
    let cols = match sep {
        None => columns.unwrap_or(1).clamp(1, MAX_COLS),
        Some(_) => columns.unwrap_or_else(|| fields.iter().map(Vec::len).max().unwrap_or(1)).max(1),
    };
    let per_row = cols.min(MAX_COLS);
    let mut rows: Vec<Vec<Paragraph>> = Vec::new();
    let mut row: Vec<Paragraph> = Vec::new();
    for (mut work, ranges) in paras.into_iter().zip(fields) {
        // The section break stays out of the cells.
        work.section = None;
        let mut pieces = Vec::with_capacity(ranges.len());
        // Last field first, so the earlier offsets stay valid.
        for r in ranges.into_iter().rev() {
            let field = work.text.get(r.clone()).unwrap_or("");
            let start = r.start + (field.len() - field.trim_start().len());
            let end = (r.start + field.trim_end().len()).max(start);
            let mut piece = work.split_off(start)?;
            piece.split_off(end - start)?;
            pieces.push(piece);
        }
        pieces.reverse();
        for piece in pieces {
            row.push(piece);
            if row.len() == per_row {
                rows.push(std::mem::take(&mut row));
            }
        }
        // With a separator, each paragraph starts a row.
        if sep.is_some() && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    Ok((rows, cols))
}

/// Word's largest Initial column width, 22 in, in points.
const MAX_INITIAL_WIDTH: f64 = 22.0 * 72.0;

pub(super) fn from_text(s: &mut Session, v: &Value) -> CmdResult {
    let sep = match p::str(v, "separator") {
        None | Some("tab") => Some('\t'),
        Some("comma") => Some(','),
        Some("paragraph") => None,
        Some("other") => match p::str(v, "char").and_then(|c| c.chars().next()) {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => return Err(CmdError::Params("`char` (the separating character) is required with `other`".into())),
        },
        Some(x) => return Err(CmdError::Params(format!("unknown separator `{x}`"))),
    };
    let fit = match p::str(v, "autofit") {
        None | Some("initial") => None,
        Some(m @ ("fixed" | "contents" | "window")) => Some(m),
        Some(x) => return Err(CmdError::Params(format!("unknown AutoFit behavior `{x}`"))),
    };
    // Initial column width: 0 (or none) is Auto, the text width shared out; Word takes 0 to 22 in.
    let col_width = match v.get("width") {
        // Only the Initial column width uses it; AutoFit sizes the columns itself.
        _ if matches!(fit, Some("contents" | "window")) => 0.0,
        None | Some(Value::Null) => 0.0,
        Some(w) => match w.as_f64() {
            Some(w) if (0.0..=MAX_INITIAL_WIDTH).contains(&w) => w as f32,
            _ => return Err(CmdError::Params("`width` must be between 0 and 1584 pt (22 in)".into())),
        },
    };
    let columns = p::u64(v, "columns").map(|n| n.clamp(1, wordcraft_doc::table::MAX_COLS as u64) as usize);
    let (a, b) = s.sel.ordered();
    // Checks the selection: only paragraphs, a table between them is refused.
    text_table_lines(s)?;
    let paras: Vec<Paragraph> = (a.path.last()..=b.path.last()).filter_map(|i| s.doc.para(a.story, &a.path.with_last(i)).cloned()).collect();
    let (cells, cols) = text_table_paras(paras, sep, columns)?;
    // A table has at most 63 columns; a longer row would lose its last fields.
    if cols > wordcraft_doc::table::MAX_COLS {
        return Err(CmdError::Failed("a table can have at most 63 columns".into()));
    }
    if cells.len() > wordcraft_doc::table::MAX_ROWS {
        return Err(CmdError::Failed(format!("a table can have at most {} rows", wordcraft_doc::table::MAX_ROWS)));
    }
    let width = crate::cmd::page::sect(s).text_width();
    let mut t = Table::new(cells.len().max(1), cols, width);
    if col_width > 0.0 {
        // Every column gets that preferred width, even past the margins (layout fits the table
        // to the text width, as Word draws it); the table itself has no preferred width.
        t.grid.iter_mut().for_each(|w| *w = col_width);
        t.rows.iter_mut().flat_map(|row| row.cells.iter_mut()).for_each(|c| c.props.width = Some(col_width));
    }
    for (r, l) in cells.into_iter().enumerate() {
        for (c, para) in l.into_iter().enumerate() {
            if let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
                cl.blocks = vec![para_block(para)];
            }
        }
    }
    for i in (a.path.last()..=b.path.last()).rev() {
        s.doc.remove_block(a.story, &a.path.with_last(i))?;
    }
    s.doc.insert_block(a.story, &a.path, Block::Table(t))?;
    let mut first = a.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: a.story, path: Path(first), off: 0 });
    match fit {
        Some(mode) => autofit(s, &json!({ "mode": mode })),
        None => sel_result(s),
    }
}

pub(super) fn quick_table(s: &mut Session, v: &Value) -> CmdResult {
    let kind = p::str(v, "kind").unwrap_or("tabular");
    let data: Vec<Vec<&str>> = match kind {
        "calendar" => {
            let mut rows = vec![vec!["M", "T", "W", "T", "F", "S", "S"]];
            let days = [
                "", "", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22",
                "23", "24", "25", "26", "27", "28", "29", "30", "31", "", "", "",
            ];
            for ch in days.chunks(7) {
                rows.push(ch.to_vec());
            }
            rows
        }
        "matrix" => vec![
            vec!["", "Column A", "Column B", "Column C"],
            vec!["Row 1", "1", "2", "3"],
            vec!["Row 2", "4", "5", "6"],
            vec!["Row 3", "7", "8", "9"],
        ],
        _ => vec![
            vec!["Item", "Needed"],
            vec!["Books", "1"],
            vec!["Magazines", "3"],
            vec!["Notebooks", "1"],
            vec!["Paper pads", "1"],
            vec!["Pens", "3"],
            vec!["Pencils", "2"],
        ],
    };
    let cols = data.iter().map(Vec::len).max().unwrap_or(1);
    let width = crate::cmd::page::sect(s).text_width();
    let mut t = Table::new(data.len(), cols, width);
    t.props.style = Some(if kind == "calendar" { "GridTable1Light" } else { "GridTable4AccentBlue" }.into());
    for (r, row) in data.iter().enumerate() {
        for (c, txt) in row.iter().enumerate() {
            if let Some(cl) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                cl.blocks = vec![para_block(Paragraph::with_text(txt, Default::default()))];
            }
        }
    }
    if let Some(r) = t.rows.first_mut() {
        r.props.header = true;
    }
    let story = s.sel.focus.story;
    let at = crate::cmd::delete_selection(s)?;
    let new = s.doc.split_paragraph(&at)?;
    s.doc.insert_block(story, &new.path, Block::Table(t))?;
    let mut first = new.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(first), off: 0 });
    let _ = StoryRef::Body;
    sel_result(s)
}
