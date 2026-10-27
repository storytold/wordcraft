use super::date::{iso_datetime, iso_time};
use super::xlsx::number;
use super::*;

// ---------------------------------------------------------------------------------------------
// OpenDocument

#[derive(Default)]
struct OCell {
    repeat: u64,
    kind: String,
    value: Option<String>,
    text: String,
    paras: usize,
}

/// The sheet names of `content.xml`, and with `want` that sheet's rows (ODF 1.3 §9.1).
pub(super) fn ods_read(xml: &[u8], want: Option<usize>) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut names = Vec::new();
    let mut grid = Grid::default();
    // Nesting of `table:table` (sub-tables inside cells are skipped), and whether the outermost
    // one is the wanted sheet.
    let mut tables = 0usize;
    let mut collecting = false;
    // Cells, pending blank cells, repeat, and the cells' text bytes (`None` once the row is over
    // the text budget and so refused).
    let mut row: Option<(Vec<String>, u64, u64, Option<usize>)> = None;
    let mut cell: Option<OCell> = None;
    let (mut in_p, mut skip) = (0usize, 0usize);
    walk(xml, |ev| match ev {
        Ev::Start(e) => {
            let n = e.local_name();
            let n = n.as_ref();
            if n == b"table" {
                if tables == 0 && names.len() < MAX_SHEETS {
                    collecting = want == Some(names.len());
                    names.push(attr(e, b"name").unwrap_or_else(|| format!("Sheet{}", names.len() + 1)));
                }
                tables += 1;
                return;
            }
            if !collecting || tables != 1 {
                return;
            }
            match n {
                b"table-row" => {
                    let repeat = attr(e, b"number-rows-repeated").and_then(|v| v.trim().parse().ok()).unwrap_or(1u64).max(1);
                    row = Some((Vec::new(), 0, repeat, Some(0)));
                }
                b"table-cell" | b"covered-table-cell" if row.is_some() => {
                    let repeat = attr(e, b"number-columns-repeated").and_then(|v| v.trim().parse().ok()).unwrap_or(1u64).max(1);
                    let kind = attr(e, b"value-type").unwrap_or_default();
                    let kind_lower = kind.trim().to_ascii_lowercase();
                    let value = match kind_lower.as_str() {
                        "float" | "percentage" | "currency" => attr(e, b"value"),
                        "date" => attr(e, b"date-value"),
                        "time" => attr(e, b"time-value"),
                        "boolean" => attr(e, b"boolean-value"),
                        "string" => attr(e, b"string-value"),
                        _ => None,
                    };
                    // A covered cell's own content is hidden by the merge.
                    let covered = n == b"covered-table-cell";
                    cell = Some(OCell {
                        repeat,
                        kind: if covered { String::new() } else { kind },
                        value: if covered { None } else { value },
                        ..Default::default()
                    });
                    if covered {
                        skip += 1;
                    }
                }
                b"annotation" if cell.is_some() => skip += 1,
                b"p" | b"h" if cell.is_some() => {
                    if let Some(c) = cell.as_mut()
                        && skip == 0
                    {
                        if c.paras > 0 {
                            push_capped(&mut c.text, "\n");
                        }
                        c.paras += 1;
                    }
                    in_p += 1;
                }
                b"s" if in_p > 0 && skip == 0 => {
                    let k = attr(e, b"c").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 1_000);
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, &" ".repeat(k));
                    }
                }
                b"tab" if in_p > 0 && skip == 0 => {
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, "\t");
                    }
                }
                b"line-break" if in_p > 0 && skip == 0 => {
                    if let Some(c) = cell.as_mut() {
                        push_capped(&mut c.text, "\n");
                    }
                }
                _ => {}
            }
        }
        Ev::Text(t) => {
            if in_p > 0
                && skip == 0
                && let Some(c) = cell.as_mut()
            {
                push_capped(&mut c.text, &t);
            }
        }
        Ev::End(n) => {
            let n = n.as_slice();
            if n == b"table" {
                tables = tables.saturating_sub(1);
                if tables == 0 {
                    collecting = false;
                }
                return;
            }
            if !collecting || tables != 1 {
                return;
            }
            match n {
                b"p" | b"h" => in_p = in_p.saturating_sub(1),
                b"annotation" => skip = skip.saturating_sub(1),
                b"table-cell" | b"covered-table-cell" => {
                    if n == b"covered-table-cell" {
                        skip = skip.saturating_sub(1);
                    }
                    let (Some(c), Some((cells, pending, _, budget))) = (cell.take(), row.as_mut()) else { return };
                    let Some(bytes) = budget else { return };
                    let text = ods_value(&c);
                    if text.is_empty() {
                        *pending = pending.saturating_add(c.repeat);
                        return;
                    }
                    // Blank cells before this one, then the value as often as it repeats, as far
                    // as the columns and the grid's cells left (all `Grid::push` keeps) allow.
                    let room = MAX_COLS.min(MAX_CELLS.saturating_sub(grid.cells));
                    let blanks = (*pending).min(room.saturating_sub(cells.len()) as u64) as usize;
                    let times = c.repeat.min(room.saturating_sub(cells.len().saturating_add(blanks)) as u64) as usize;
                    // The row's text has to fit what is left of the budget to be kept at all
                    // (`Grid::push`): one that can't is refused now, before the copies are made.
                    *bytes = bytes.saturating_add(text.len().saturating_mul(times));
                    if *bytes > MAX_TEXT_BYTES.saturating_sub(grid.text.0) {
                        cells.clear();
                        *budget = None;
                        grid.refuse();
                        return;
                    }
                    cells.resize(cells.len() + blanks, String::new());
                    *pending = 0;
                    cells.extend(std::iter::repeat_n(text, times));
                }
                b"table-row" => {
                    if let Some((cells, _, repeat, Some(_))) = row.take() {
                        grid.push(cells, repeat);
                    }
                }
                _ => {}
            }
        }
    });
    if names.is_empty() {
        return Err("the spreadsheet has no sheets".into());
    }
    Ok((names, grid.rows))
}

fn ods_value(c: &OCell) -> String {
    let v = c.value.as_deref().map(str::trim);
    let kind = c.kind.trim().to_ascii_lowercase();
    match (kind.as_str(), v) {
        ("float", Some(v)) => v.parse::<f64>().map(number).unwrap_or_else(|_| c.text.clone()),
        ("date", Some(v)) => iso_datetime(v).unwrap_or_else(|| c.text.clone()),
        ("time", Some(v)) => iso_time(v).unwrap_or_else(|| c.text.clone()),
        ("boolean", Some(v)) => if v.eq_ignore_ascii_case("true") || v == "1" { "TRUE" } else { "FALSE" }.into(),
        ("percentage" | "currency", Some(v)) if c.text.trim().is_empty() => v.parse::<f64>().map(number).unwrap_or_else(|_| c.text.clone()),
        ("string", Some(v)) if c.text.trim().is_empty() => {
            // Attribute text obeys the same per-cell cap as `<text:p>` text.
            let mut s = String::new();
            push_capped(&mut s, v);
            s
        }
        _ => c.text.clone(),
    }
}
