//! An explicit, conservative table join. Moving rows keeps inline relationship markers and
//! embedded-object ids; no paste/adoption or ordinary deletion semantics are involved.

use super::{Block, CmdError, CmdResult, Document, Paragraph, Path, Pos, Selection, Session, Table, Value, sel_result};
use std::sync::Arc;
use wordcraft_doc::InlineObject;
use wordcraft_doc::props::{CharProps, VMerge};
use wordcraft_doc::styles::{TablePart, TableStyleParts};

struct Target {
    table: Path,
    next: Path,
    boundary: usize,
    caret: Path,
}

/// Fresh, non-mutating availability for `table.joinNext`, also used by its command gate.
/// The first version rejects protected/tracked documents, controls, captions, floating tables,
/// kept structural wrappers and row-dependent table styles rather than losing their meaning.
/// Call again at dispatch: a previously enabled menu is not a cached editing target.
pub fn join_next_unavailable(s: &Session) -> Option<&'static str> {
    target(s).err()
}

pub(super) fn status(s: &mut Session, _: &Value) -> CmdResult {
    Ok(match target(s) {
        Ok(t) => serde_json::json!({"available":true,"reason":null,"table":t.table,"nextTable":t.next,"boundaryRow":t.boundary}),
        Err(reason) => serde_json::json!({"available":false,"reason":reason}),
    })
}

fn target(s: &Session) -> Result<Target, &'static str> {
    if s.math.is_some() {
        return Err("leave equation editing before joining tables");
    }
    if s.doc.settings.protection.is_some() || s.doc.settings.protect_formatting {
        return Err("joining tables is unavailable in a protected document");
    }
    if s.doc.settings.track_changes || !s.doc.revisions.is_empty() {
        return Err("joining tables with tracked changes is not supported");
    }
    if !s.sel.is_collapsed() || s.column.is_some() || !s.also_selected.is_empty() {
        return Err("place the caret in the first table without a cell or object selection");
    }
    let story = s.sel.focus.story;
    let (tp, _, _) = s.sel.focus.path.cell().ok_or("the cursor isn't in a table")?;
    if s.doc.para_at(&s.sel.focus).is_none() || s.sel.anchor.story != story || s.sel.anchor.path.cell().is_none_or(|(p, _, _)| p != tp) {
        return Err("the selection must be inside one table");
    }
    let blocks = s.doc.container(story, &tp).ok_or("the table container no longer exists")?;
    // A control starting outside the tables can lock them too. Refuse the whole story in this
    // initial API, including orphan markers, instead of checking only the caret's lock.
    for block in s.doc.story(story).ok_or("the story no longer exists")? {
        safe_block(block, 0)?;
    }
    let first = s.doc.table(story, &tp).ok_or("the current table no longer exists")?;
    let mut ni = tp.last().checked_add(1).ok_or("no following table")?;
    if let Some(Block::Para(p)) = blocks.get(ni).map(Arc::as_ref) {
        // Exact default, including mark, runs, objects, properties and section: an empty
        // heading, bookmark, break or formatted spacer is content, not Split Table's separator.
        let effective = s.doc.styles.resolve_para(&p.props);
        if effective.page_break_before
            || effective.numbering.is_some_and(|n| n.num != 0)
            || effective.outline_level.is_some_and(|n| n < 9)
            // A default paragraph can still paint inherited decoration. Removing that
            // separator would delete visible content even though its direct props are empty.
            || effective.shading.is_some()
            || effective.borders.is_some_and(|b| [b.top, b.left, b.bottom, b.right].into_iter().flatten().any(|side| side.is_visible()))
            || *p != Paragraph::new()
            || s.doc.kept.get().is_some_and(|k| k.para(p).is_some() || k.wrap(p).is_some() || k.group(p).is_some() || k.index(p).is_some())
        {
            return Err("the following paragraph is not a plain empty table separator");
        }
        ni = ni.checked_add(1).ok_or("no following table")?;
    }
    let Some(Block::Table(second)) = blocks.get(ni).map(Arc::as_ref) else {
        return Err("no consecutive table in the same container");
    };
    for t in [first, second] {
        compatible_table(&s.doc, t)?;
    }
    if first.grid != second.grid {
        return Err("the tables have different column grids");
    }
    if first.props != second.props {
        return Err("the tables have different table settings or styles");
    }
    stable_boundary(s, first, second)?;
    if caption(&s.doc, tp.last().checked_sub(1).and_then(|i| blocks.get(i)).map(Arc::as_ref))
        || caption(&s.doc, blocks.get(ni.saturating_add(1)).map(Arc::as_ref))
    {
        return Err("neighboring captions need their relationships reconciled before joining");
    }
    if first.rows.len().saturating_add(second.rows.len()) > wordcraft_doc::table::MAX_ROWS {
        return Err("the joined table would have too many rows");
    }
    // An interior header would stop repeating; a continuation would merge into the preceding
    // table. Preserve both interpretations by refusing that boundary, not changing its flags.
    if second.rows.iter().any(|r| r.props.header) {
        return Err("the next table has repeated header rows");
    }
    if second.rows.first().is_some_and(|r| r.cells.iter().any(|c| c.props.vmerge == VMerge::Continue)) {
        return Err("the next table starts with a vertical merge continuation");
    }
    let mut prefix = tp.with_last(ni).0;
    prefix.push(0);
    let mut caret =
        s.doc.para_paths(story).into_iter().find(|p| p.0.starts_with(&prefix)).ok_or("the next table's first row has no editable paragraph")?;
    *caret.0.get_mut(tp.0.len().saturating_sub(1)).ok_or("the table path is invalid")? = tp.last() as u32;
    *caret.0.get_mut(tp.0.len()).ok_or("the table row path is invalid")? = first.rows.len() as u32;
    Ok(Target { table: tp.clone(), next: tp.with_last(ni), boundary: first.rows.len(), caret })
}

fn compatible_table(doc: &Document, t: &Table) -> Result<(), &'static str> {
    if t.grid.is_empty()
        || t.grid.len() > wordcraft_doc::table::MAX_COLS
        || t.grid.iter().any(|w| !w.is_finite() || *w <= 0.0)
        || !t.grid.iter().sum::<f32>().is_finite()
        || t.rows.is_empty()
    {
        return Err("the table has an invalid or empty grid");
    }
    if t.props.float.is_some() {
        return Err("joining floating tables is not supported");
    }
    if t.props.caption.is_some() || t.props.description.is_some() {
        return Err("joining tables with captions or descriptions is not supported");
    }
    if doc.kept.get().is_some_and(|k| k.table(t).is_some() || k.table_wrap(t).is_some()) {
        return Err("joining tables inside preserved format wrappers is not supported");
    }
    if let Some(id) = &t.props.style {
        let style = doc.styles.table_style(id).ok_or("the table style cannot be resolved")?;
        row_styles(&style.parts)?;
    }
    let mut body = false;
    for (ri, row) in t.rows.iter().enumerate() {
        let n = row.first_grid().saturating_add(row.cells.iter().map(|c| c.span()).fold(0, usize::saturating_add)).saturating_add(row.grid_after());
        if n != t.grid.len() || row.cells.is_empty() || row.cells.iter().any(|c| !(1..=63).contains(&c.props.span)) {
            return Err("a table row does not fit its column grid");
        }
        body |= !row.props.header;
        if body && row.props.header {
            return Err("repeated header rows must be a prefix");
        }
        if row.props.exceptions.is_some() {
            return Err("row-level table exceptions need a join policy first");
        }
        let mut g = row.first_grid();
        for cell in &row.cells {
            if cell.props.vmerge == VMerge::Continue
                && (ri == 0
                    || !t
                        .cell_at_grid(ri - 1, g)
                        // Covering the column is insufficient: shifted equal-span cells do
                        // not form the same vertical merge in Table::vertical_extent.
                        .filter(|&ci| t.grid_col(ri - 1, ci) == g)
                        .and_then(|ci| t.rows.get(ri - 1)?.cells.get(ci))
                        .is_some_and(|above| above.props.vmerge != VMerge::None && above.span() == cell.span()))
            {
                return Err("a vertical merge has no matching cell above it");
            }
            g = g.saturating_add(cell.span());
        }
    }
    Ok(())
}

fn caption(doc: &Document, block: Option<&Block>) -> bool {
    let Some(Block::Para(p)) = block else { return false };
    p.props
        .style
        .as_deref()
        .is_some_and(|id| doc.styles.chain(id).iter().any(|st| st.name.eq_ignore_ascii_case("Caption") || st.id.eq_ignore_ascii_case("Caption")))
        || p.objects.iter().any(|o| {
            matches!(o, InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. }
            if instr.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("SEQ")))
        })
}

fn stable_boundary(s: &Session, first: &Table, second: &Table) -> Result<(), &'static str> {
    for t in [first, second] {
        if t.props.space_before.is_some_and(|v| v != 0.0) || t.props.space_after.is_some_and(|v| v != 0.0) {
            return Err("table spacing needs an explicit join policy first");
        }
        let mut borders = t.props.style.as_deref().and_then(|id| s.doc.styles.table_style(id)).and_then(|st| st.parts.borders).unwrap_or_default();
        if let Some(own) = t.props.borders {
            borders.overlay(&own);
        }
        if borders.top != borders.between || borders.bottom != borders.between {
            return Err("the outside and inside horizontal table borders differ");
        }
        stable_width(t)?;
    }
    // Start-aligned tables can have different origins even with equal table properties:
    // the first cell's border (current mode), or margin (legacy mode), contributes to x.
    if !matches!(first.props.align, Some(wordcraft_doc::props::Align::Center | wordcraft_doc::props::Align::Right)) {
        let (a, b) = (start_offset(s, first), start_offset(s, second));
        if !a.is_finite() || !b.is_finite() || (a - b).abs() > 0.01 {
            return Err("the first cells give the tables different start origins");
        }
    }
    let (Some(a), Some(b)) = (first.rows.last(), second.rows.first()) else { return Err("the tables contain no rows") };
    let mut x = a.first_grid();
    for cell in &a.cells {
        let end = x.saturating_add(cell.span());
        let mut y = b.first_grid();
        for other in &b.cells {
            let z = y.saturating_add(other.span());
            if x < z && y < end && cell.props.borders.unwrap_or_default().bottom != other.props.borders.unwrap_or_default().top {
                return Err("the cell borders facing the joined boundary differ");
            }
            y = z;
        }
        x = end;
    }
    Ok(())
}

fn stable_width(t: &Table) -> Result<(), &'static str> {
    // AutoFit considers all rows: concatenating two independently fitted tables can change
    // both grids. The bounded AutoFit-to-Contents preview is not a proof of stable layout.
    if !t.props.fixed {
        return Err("automatic sizing needs explicit fixed layout before joining tables");
    }
    for row in &t.rows {
        let mut g = row.first_grid();
        for cell in &row.cells {
            // The fixed renderer may replace a stale grid with consistent unspanned cell
            // preferences. Mixed preferences after a join stop that replacement. Decline
            // contradictions instead of rewriting either grid or imported cell widths.
            let width = t.grid.get(g).copied().unwrap_or_default();
            if cell.props.width_pct.is_some()
                || cell.props.width.is_some_and(|v| !v.is_finite() || v <= 0.0 || (cell.span() == 1 && (v - width).abs() > 0.01))
            {
                return Err("preferred cell widths must agree with the fixed grid before joining");
            }
            g = g.saturating_add(cell.span());
        }
    }
    Ok(())
}

fn start_offset(s: &Session, t: &Table) -> f32 {
    if s.doc.settings.compat_mode < 15 {
        return -wordcraft_layout::cell_margins(&s.doc, t, 0, 0)[1];
    }
    let mut borders = t.props.style.as_deref().and_then(|id| s.doc.styles.table_style(id)).and_then(|st| st.parts.borders).unwrap_or_default();
    if let Some(own) = t.props.borders {
        borders.overlay(&own);
    }
    t.rows
        .first()
        .and_then(|r| r.cells.first())
        .and_then(|c| c.props.borders.and_then(|b| b.left))
        .or(borders.left)
        .filter(wordcraft_doc::props::Border::is_visible)
        .map_or(0.0, |b| b.width.clamp(0.0, 12.0) / 2.0)
}

fn row_styles(parts: &TableStyleParts) -> Result<(), &'static str> {
    // Appending changes first/last-row and band membership. Direct cell formatting is kept,
    // but this first API does not materialize conditional styles into overriding properties.
    for p in [
        TablePart::FirstRow,
        TablePart::LastRow,
        TablePart::Band1Horz,
        TablePart::Band2Horz,
        TablePart::NwCell,
        TablePart::NeCell,
        TablePart::SwCell,
        TablePart::SeCell,
    ] {
        if !parts.part(p).is_empty() {
            return Err("joining tables with row-dependent style regions is not supported");
        }
    }
    Ok(())
}

fn tracked(p: &CharProps) -> bool {
    p.ins.is_some() || p.del.is_some() || p.fmt_change.is_some()
}

fn safe_block(block: &Block, depth: usize) -> Result<(), &'static str> {
    if depth > wordcraft_doc::table::MAX_TABLE_DEPTH {
        return Err("the table nesting limit is exceeded");
    }
    match block {
        Block::Para(p) => {
            if tracked(&p.mark) || p.runs.iter().any(|r| tracked(&r.props)) || p.props.fmt_change.is_some() || p.props.num_change.is_some() {
                return Err("joining tables with tracked formatting is not supported");
            }
            if p.objects.iter().any(|o| {
                matches!(
                    o,
                    InlineObject::ControlStart { .. } | InlineObject::ControlEnd | InlineObject::PermStart { .. } | InlineObject::PermEnd { .. }
                )
            }) {
                return Err("joining tables in a story with content controls or permission ranges is not supported");
            }
            if p.objects.iter().any(|o| {
                matches!(o, InlineObject::FieldStart { .. } | InlineObject::FieldEnd | InlineObject::Opaque { .. })
                    || matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().starts_with('='))
            }) {
                return Err("range fields, table formulas and opaque content need a join policy first");
            }
        }
        Block::Table(t) => {
            if !t.controls.is_empty() || t.rows.iter().any(|r| !r.controls.is_empty() || r.cells.iter().any(|c| !c.controls.is_empty())) {
                return Err("joining tables in a story with content controls is not supported");
            }
            if t.props.fmt_change.is_some()
                || t.rows.iter().any(|r| r.props.fmt_change.is_some() || r.cells.iter().any(|c| c.props.fmt_change.is_some()))
            {
                return Err("joining tables with tracked formatting is not supported");
            }
            for b in t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks) {
                if b.as_para().is_some_and(|p| p.section.is_some()) {
                    return Err("joining tables containing section boundaries is not supported");
                }
                safe_block(b, depth + 1)?;
            }
        }
    }
    Ok(())
}

pub(super) fn join_next(s: &mut Session, _: &Value) -> CmdResult {
    let t = target(s).map_err(|why| CmdError::Disabled(why.into()))?;
    let story = s.sel.focus.story;
    let blocks = s.doc.container_mut(story, &t.table)?;
    let removed = blocks.remove(t.next.last());
    let Block::Table(second) = Arc::unwrap_or_clone(removed) else {
        return Err(CmdError::Failed("the next table no longer exists".into()));
    };
    if t.next.last() != t.table.last() + 1 {
        blocks.remove(t.table.last() + 1);
    }
    s.doc.table_mut(story, &t.table)?.rows.extend(second.rows);
    // The validated first moved paragraph can be inside a nested table. Its unchanged suffix
    // and new parent-row offset are known before editing, not found by clamping a stale path.
    s.sel = Selection::caret(Pos { story, path: t.caret, off: 0 });
    s.column = None;
    s.pending = None;
    sel_result(s)
}
