use super::lex::{Tok, text_symbol};
use super::limits::MAX_NEST;
use super::parser::Parser;
use super::units::{parse_color, spec_widths};
use super::*;

/// Plain text of tokens (arguments that are names, URLs, lengths).
pub(super) fn raw_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Text(x) => s.push_str(x),
            Tok::Tie => s.push('~'),
            Tok::Cmd(c) => match text_symbol(c) {
                Some(sym) => s.push_str(sym),
                None if c == "\\" => s.push(' '),
                None => {
                    s.push('\\');
                    s.push_str(c);
                }
            },
            Tok::Verb(v) | Tok::Math(v, _) => s.push_str(v),
            Tok::Amp => s.push('&'),
            _ => {}
        }
    }
    s.trim().to_string()
}

/// Tokens back as LaTeX source (comments and layout whitespace normalised).
pub(super) fn source_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Text(x) => s.push_str(x),
            Tok::Open => s.push('{'),
            Tok::Close => s.push('}'),
            Tok::Tie => s.push('~'),
            Tok::Amp => s.push('&'),
            Tok::Par => s.push(' '),
            Tok::Cmd(c) => {
                s.push('\\');
                s.push_str(c);
                if c.chars().all(|c| c.is_ascii_alphabetic()) {
                    s.push(' ');
                }
            }
            Tok::Math(m, _) => {
                s.push('$');
                s.push_str(m);
                s.push('$');
            }
            Tok::Verb(v) | Tok::Verbatim(v) => s.push_str(v),
            Tok::Begin(n) => s.push_str(&format!("\\begin{{{n}}}")),
            Tok::End(n) => s.push_str(&format!("\\end{{{n}}}")),
        }
    }
    s.trim().to_string()
}

/// A `tabular` body as a table.
pub(super) fn table(toks: &[Tok], spec: &str, depth: usize) -> FTable {
    let (spec_cols, widths) = spec_widths(spec);
    let mut has_rules = spec.contains('|');
    // Split rows at `\\` and cells at `&`, outside braces and nested environments.
    let mut rows: Vec<Vec<&[Tok]>> = Vec::new();
    let mut row: Vec<&[Tok]> = Vec::new();
    let mut start = 0usize;
    let mut brace = 0usize;
    let mut env = 0usize;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Open => brace += 1,
            Tok::Close => brace = brace.saturating_sub(1),
            Tok::Begin(_) => env += 1,
            Tok::End(_) => env = env.saturating_sub(1),
            Tok::Cmd(c) if brace == 0 && env == 0 && matches!(c.as_str(), "hline" | "cline" | "toprule" | "midrule" | "bottomrule") => {
                has_rules = true;
            }
            Tok::Amp if brace == 0 && env == 0 => {
                row.push(toks.get(start..i).unwrap_or_default());
                start = i + 1;
            }
            Tok::Cmd(c) if brace == 0 && env == 0 && (c == "\\" || c == "tabularnewline") => {
                row.push(toks.get(start..i).unwrap_or_default());
                rows.push(std::mem::take(&mut row));
                start = i + 1;
            }
            _ => {}
        }
        if rows.len() >= wordcraft_doc::table::MAX_ROWS {
            break;
        }
    }
    if start < toks.len() {
        row.push(toks.get(start..).unwrap_or_default());
    }
    if !row.is_empty() {
        rows.push(row);
    }
    let mut out = FTable { rows: Vec::new(), widths: Vec::new(), borderless: !has_rules };
    // Rows still covered by a `\multirow` above, per grid column.
    let mut covering: Vec<u32> = Vec::new();
    for r in rows {
        let mut cells = Vec::new();
        let mut g = 0usize;
        for cell_toks in r {
            let mut cell = Cell::default();
            let mut content = trim_cell(cell_toks);
            // `\multicolumn{n}{spec}{content}`, `\multirow{n}{width}{content}`. Each wrapper
            // rescans its content, so only MAX_NEST of them are unwrapped (a cell nests one or
            // two); further ones stay in the content.
            let mut sub = Parser::new(content, depth + 1);
            let mut wrappers = 0usize;
            loop {
                sub.skip_spaces();
                match sub.t.get(sub.pos) {
                    Some(Tok::Cmd(c)) if c == "multicolumn" && wrappers < MAX_NEST => {
                        wrappers += 1;
                        sub.pos += 1;
                        cell.colspan = sub.arg_raw().trim().parse::<u32>().unwrap_or(1).clamp(1, 63);
                        let _ = sub.arg_raw();
                        content = sub.arg_tokens();
                        sub = Parser::new(content, depth + 1);
                    }
                    Some(Tok::Cmd(c)) if (c == "multirow" || c == "multirow*") && wrappers < MAX_NEST => {
                        wrappers += 1;
                        sub.pos += 1;
                        let _ = sub.opt_arg();
                        cell.rowspan = sub.arg_raw().trim().parse::<i64>().map(|n| n.unsigned_abs().clamp(1, 1000) as u32).unwrap_or(1);
                        let _ = sub.opt_arg();
                        let _ = sub.arg_raw();
                        let _ = sub.opt_arg();
                        content = sub.arg_tokens();
                        sub = Parser::new(content, depth + 1);
                    }
                    Some(Tok::Cmd(c)) if c == "cellcolor" => {
                        sub.pos += 1;
                        let model = sub.opt_arg();
                        let spec = sub.arg_raw();
                        cell.shading = parse_color(model.as_deref(), &spec);
                    }
                    _ => break,
                }
            }
            if covering.get(g).copied().unwrap_or(0) > 0 {
                cell.covered = true;
            }
            if !cell.covered && depth < MAX_NEST {
                sub.run(None);
                let (blocks, _) = sub.finish();
                cell.blocks = blocks;
            }
            let span = cell.colspan as usize;
            if covering.len() < g + span {
                covering.resize(g + span, 0);
            }
            for k in g..g + span {
                if let Some(c) = covering.get_mut(k)
                    && cell.rowspan > 1
                    && !cell.covered
                {
                    *c = cell.rowspan;
                }
            }
            g += span;
            cells.push(cell);
        }
        for c in covering.iter_mut() {
            *c = c.saturating_sub(1);
        }
        // `\hline` alone after the last `\\` is not a row.
        let empty = cells.iter().all(|c| !c.covered && c.blocks.is_empty());
        if empty && cells.len() <= 1 {
            continue;
        }
        out.rows.push(cells);
    }
    let cols = out.cols();
    if !widths.is_empty() && widths.len() == cols && spec_cols == cols {
        out.widths = widths;
    }
    out
}

/// A cell's tokens without the rules (`\hline`…) and spaces around them.
fn trim_cell(toks: &[Tok]) -> &[Tok] {
    let skip = |t: &Tok| match t {
        Tok::Cmd(c) => {
            matches!(c.as_str(), "hline" | "toprule" | "midrule" | "bottomrule" | "endhead" | "endfirsthead" | "endfoot" | "endlastfoot" | "noalign")
        }
        Tok::Text(s) => s.trim().is_empty(),
        _ => false,
    };
    let mut a = 0;
    while let Some(t) = toks.get(a) {
        if skip(t) {
            a += 1;
        } else if matches!(t, Tok::Cmd(c) if c == "cline" || c == "cmidrule" || c == "addlinespace") {
            // `\cline{2-3}`, `\cmidrule(lr){1-2}`: the command and its argument.
            a += 1;
            while let Some(Tok::Text(s)) = toks.get(a) {
                if s.trim().is_empty() || s.trim_start().starts_with('(') || s.trim_start().starts_with('[') {
                    a += 1;
                } else {
                    break;
                }
            }
            if toks.get(a) == Some(&Tok::Open) {
                while let Some(t) = toks.get(a) {
                    a += 1;
                    if *t == Tok::Close {
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }
    let mut b = toks.len();
    while b > a && toks.get(b - 1).is_some_and(skip) {
        b -= 1;
    }
    toks.get(a..b).unwrap_or_default()
}
