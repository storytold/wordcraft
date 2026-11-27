//! Calculations through `.docx`, for review in Word and back.
//!
//! Word knows nothing of calculations, so a saved file shows each one as what it displays: an
//! ordinary equation (`σ≔P/A_b=0.6 ksi`, or its `≈`/`⚠` form when stale or in error) inside a
//! rich text content control tagged `wordcraft-calc:<id>` and titled "Calculation". Quoted values
//! are locked `WCCALC` fields showing their result. The sources go in a hidden store
//! ([`CalcStore`], a custom XML part the `.docx` writer adds), each with a fingerprint of its
//! equation as saved.
//!
//! Reading the file back, every tagged equation whose fingerprint still matches becomes a live
//! calculation again; one whose text was changed in Word keeps Word's version and is flagged
//! [`CalcState::EditedOutside`] for the author to resolve, never silently recalculated. Then the
//! document is recalculated.
//!
//! A table marked as calculation data is saved as the ordinary table it is, inside a content
//! control around it tagged `wordcraft-calc-table:<id>` ([`CALC_TABLE_TAG_PREFIX`]), and listed in
//! the store with a fingerprint of its cells' text ([`table_fingerprint`]). Read back, it is data
//! again; its cells are inputs, so values changed in Word are used, and a table whose fingerprint
//! no longer matches is counted ([`crate::RecalcReport::tables_edited`]) for the status line to
//! say so.
//!
//! The fingerprint is FNV-1a 64 of the equation's linear form as WordCraft writes it from its math
//! tree (`to_linear` of the tree), on both sides: the stored display text itself doesn't survive
//! OMML (units lose their quotes, a script base gains parentheses), while the tree's linear form
//! does, byte for byte.

use std::collections::{BTreeMap, HashMap, HashSet};

use wordcraft_doc::calc::{
    CALC_TABLE_TAG_PREFIX, CALC_TAG_PREFIX, CalcRegion, CalcSettings, CalcState, CalcStore, CalcTable, MAX_STORED_REGIONS, StoredRegion, StoredTable,
};
use wordcraft_doc::control::{ControlEvent, ControlKind, control_events, pair_events, walk_controls};
use wordcraft_doc::math::{Math, parse_linear, to_linear};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Block, Blocks, Cell, ContentControl, Document, InlineObject, Path, StoryRef, Table};

use crate::recalc::{assign_ids, assign_table_ids, collect, para_mut, table_at, table_mut, walk_blocks};

/// The title Word shows on a calculation's content control.
pub const CONTROL_TITLE: &str = "Calculation";

/// The title Word shows on a calculation data table's content control.
pub const TABLE_CONTROL_TITLE: &str = "Calculation data";

/// Mixed into the fingerprint saved for a region already edited outside WordCraft, so that it
/// can't match the equation read back: saving a flagged region again must not make Word's text
/// look like WordCraft's.
const EDITED_SALT: &[u8] = b"\0wordcraft-calc:edited\0";

/// FNV-1a, 64 bits.
fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.iter().flat_map(|p| p.iter()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The linear form a fingerprint is taken of: the math tree's, or (no tree) the linear text's,
/// without an equation number (saved as `#`, it comes back from Word as `#(1)`).
fn canonical(linear: &str, math: &Math) -> String {
    let nodes = if math.nodes.is_empty() { parse_linear(linear) } else { math.nodes.clone() };
    to_linear(&crate::render::unnumbered(&nodes).unwrap_or(nodes))
}

/// The fingerprint of an equation as shown: FNV-1a 64 of its canonical linear form.
pub fn fingerprint(linear: &str, math: &Math) -> u64 {
    fnv1a64(&[canonical(linear, math).as_bytes()])
}

/// The fingerprint of a data table's cells: FNV-1a 64 of each cell's text, `\u{1f}` between cells
/// and `\u{1e}` between rows. A cell's text is its paragraphs' final text (tracked deletions left
/// out) joined by `\n`, trimmed; a table nested in a cell adds nothing (a data cell holding one is
/// an error anyway).
pub fn table_fingerprint(t: &Table) -> u64 {
    let mut text = String::new();
    for (r, row) in t.rows.iter().enumerate() {
        if r > 0 {
            text.push('\u{1e}');
        }
        for (c, cell) in row.cells.iter().enumerate() {
            if c > 0 {
                text.push('\u{1f}');
            }
            text.push_str(cell_text(cell).trim());
        }
    }
    fnv1a64(&[text.as_bytes()])
}

fn cell_text(cell: &Cell) -> String {
    let paras: Vec<String> = cell.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.final_text()).collect();
    paras.join("\n")
}

/// A copy of `doc` to give a `.docx` writer, and the store to save beside it.
///
/// Every calculation region in the body and table cells becomes a plain equation, showing what it
/// shows now, inside a content control tagged with its id; a repeated id (a copy not yet
/// recalculated) gets the id recalculation would give it. Tracked-deleted regions are saved too
/// (with an id no live one has), so rejecting the deletion, in Word or after reopening, brings
/// the calculation back. The quoted values stay as they are (locked fields).
///
/// Every calculation data table (tracked-deleted ones too, as regions) becomes a plain table with
/// a content control around it tagged with its id (the innermost control on the table), and its
/// id and fingerprint go in the store. `doc` is not changed.
pub fn export(doc: &Document) -> (Document, CalcStore) {
    let mut copy = doc.clone();
    copy.calc_store = None;
    let found = collect(&copy.body);
    let (live_ids, _) = assign_ids(&found);
    // Every region to save, live and tracked-deleted (rejecting the deletion brings one back),
    // in document order, each with an id no other saved region has.
    let mut all: Vec<(&[u32], usize, u64)> = found.regions.iter().zip(&live_ids).map(|(r, &id)| (r.at.path.as_slice(), r.at.obj, id)).collect();
    all.extend(unique_deleted_ids(&live_ids, &found.deleted).into_iter().map(|(at, id)| (at.path.as_slice(), at.obj, id)));
    all.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    // Each paragraph's regions together, last object first, so earlier offsets stay valid.
    let mut by_para: BTreeMap<Vec<u32>, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, (path, obj, _)) in all.iter().enumerate() {
        by_para.entry(path.to_vec()).or_default().push((*obj, i));
    }
    let ids: Vec<u64> = all.iter().map(|&(_, _, id)| id).collect();
    let mut stored: Vec<Option<StoredRegion>> = vec![None; all.len()];
    for (path, mut objs) in by_para {
        let Some(p) = para_mut(&mut copy.body, &path) else { continue };
        let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
        objs.sort_unstable_by(|a, b| b.cmp(a));
        for (k, i) in objs {
            let (Some(&off), Some(&id)) = (anchors.get(k), ids.get(i)) else { continue };
            let Some(InlineObject::Equation { linear, math, calc, .. }) = p.objects.get_mut(k) else { continue };
            let Some(region) = calc.take() else { continue };
            // The edited mark is WordCraft's, not part of the equation Word gets back.
            if region.state == CalcState::EditedOutside {
                crate::render::strip_edited_mark(linear, math);
            }
            let text = canonical(linear, math);
            let fingerprint =
                if region.state == CalcState::EditedOutside { fnv1a64(&[EDITED_SALT, text.as_bytes()]) } else { fnv1a64(&[text.as_bytes()]) };
            let CalcRegion { result_format, worksheet_order, source, unit, precision, numbered, .. } = *region;
            if let Some(slot) = stored.get_mut(i) {
                *slot = Some(StoredRegion { result_format, worksheet_order, id, source, unit, precision, numbered, fingerprint });
            }
            let control = ContentControl {
                tag: format!("{CALC_TAG_PREFIX}{id}"),
                title: CONTROL_TITLE.to_string(),
                kind: ControlKind::RichText,
                ..Default::default()
            };
            let props = p.props_of_char(off).clone();
            // Neither can fail: both offsets are character boundaries of this paragraph.
            if p.insert_object(off + OBJ.len_utf8(), InlineObject::ControlEnd, &props).is_ok() {
                let _ = p.insert_object(off, InlineObject::ControlStart { control: Box::new(control) }, &props);
            }
        }
    }
    let tables = export_tables(doc, &mut copy, &found);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let plots = save_plots(&mut copy);
    (copy, CalcStore { settings, regions: stored.into_iter().flatten().collect(), tables, plots })
}

const PLOT_TAG: &str = "wordcraft-plot:";

fn save_plots(doc: &mut Document) -> Vec<wordcraft_doc::calc::StoredPlot> {
    let mut stored = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Ok(p) = doc.para_mut(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets().into_iter().rev() {
            let Some(InlineObject::Graphic { graphic, .. }) = p.object_at(off) else { continue };
            let Some(plot) = graphic.chart.as_ref().and_then(|chart| chart.plot.clone()) else { continue };
            let id = stored.len() as u64 + 1;
            let control =
                ContentControl { tag: format!("{PLOT_TAG}{id}"), title: "Function Plot".into(), kind: ControlKind::RichText, ..Default::default() };
            let props = p.props_of_char(off).clone();
            if p.insert_object(off + OBJ.len_utf8(), InlineObject::ControlEnd, &props).is_ok() {
                let _ = p.insert_object(off, InlineObject::ControlStart { control: Box::new(control) }, &props);
                stored.push(wordcraft_doc::calc::StoredPlot { id, plot });
            }
        }
    }
    stored
}

fn restore_plots(doc: &mut Document, plots: Vec<wordcraft_doc::calc::StoredPlot>) {
    let mut plots = unique_by_id(plots.into_iter().map(|record| (record.id, record.plot)));
    let mut controls: Vec<_> = doc
        .control_ranges(StoryRef::Body)
        .into_iter()
        .filter_map(|r| Some((r.control.tag.strip_prefix(PLOT_TAG)?.parse::<u64>().ok()?, r)))
        .collect();
    controls.sort_by(|(_, a), (_, b)| (&b.start.path, b.start.off).cmp(&(&a.start.path, a.start.off)));
    for (id, control) in controls {
        if control.start.path != control.end.path {
            continue;
        }
        let Some(plot) = plots.remove(&id) else { continue };
        let Ok(p) = doc.para_mut(StoryRef::Body, &control.start.path) else { continue };
        let off = control.start.off + OBJ.len_utf8();
        let Some(InlineObject::Graphic { graphic, .. }) = p.object_at_mut(off) else { continue };
        let Some(chart) = std::sync::Arc::make_mut(graphic).chart.as_mut() else { continue };
        std::sync::Arc::make_mut(chart).plot = Some(plot);
        let _ = p.delete(control.end.off, control.end.off + OBJ.len_utf8());
        let _ = p.delete(control.start.off, off);
    }
}

/// Wrap each data table of `copy` (found in it as `found`) in its tagged content control and
/// unmark it; the store's entries for them, in document order. Fingerprints are of `doc`'s
/// tables: `copy`'s cells may hold region controls' markers by now.
fn export_tables(doc: &Document, copy: &mut Document, found: &crate::recalc::Found) -> Vec<StoredTable> {
    let live = assign_table_ids(found);
    let mut all: Vec<(&[u32], u64)> = found.tables.iter().zip(&live).map(|(t, &id)| (t.path.as_slice(), id)).collect();
    all.extend(unique_deleted_ids(&live, &found.deleted_tables).into_iter().map(|(path, id)| (path.as_slice(), id)));
    all.sort_unstable();
    let mut stored = Vec::with_capacity(all.len());
    for (path, id) in all {
        let Some(fingerprint) = table_at(&doc.body, path).map(table_fingerprint) else { continue };
        let Some(t) = table_mut(&mut copy.body, path) else { continue };
        t.calc = None;
        // Innermost, so it closes first, just after the table: around the table and nothing else.
        t.controls.open.push(ContentControl {
            tag: format!("{CALC_TABLE_TAG_PREFIX}{id}"),
            title: TABLE_CONTROL_TITLE.to_string(),
            kind: ControlKind::RichText,
            block: true,
            ..Default::default()
        });
        t.controls.close = t.controls.close.saturating_add(1);
        stored.push(StoredTable { id, fingerprint });
    }
    stored
}

/// The tracked-deleted regions (or data tables) with the ids they are saved under: their own,
/// unless a live one (whose ids `live` holds) or an earlier deleted one has it already; then the
/// next id above all of them (or, at the top of the range, the lowest free one).
fn unique_deleted_ids<'a, T>(live: &[u64], deleted: &'a [(T, u64)]) -> Vec<(&'a T, u64)> {
    let mut taken: HashSet<u64> = live.iter().copied().collect();
    let mut next = live.iter().chain(deleted.iter().map(|(_, id)| id)).max().and_then(|m| m.checked_add(1));
    let mut probe: u64 = 0;
    deleted
        .iter()
        .map(|(at, id)| {
            let id = if taken.contains(id) {
                match next {
                    Some(n) => {
                        next = n.checked_add(1);
                        n
                    }
                    None => {
                        while taken.contains(&probe) {
                            probe = probe.saturating_add(1);
                        }
                        probe
                    }
                }
            } else {
                *id
            };
            taken.insert(id);
            (at, id)
        })
        .collect()
}

/// The region id a content control tag names, if it is a calculation's.
fn tagged_id(tag: &str) -> Option<Option<u64>> {
    tag.strip_prefix(CALC_TAG_PREFIX).map(|id| id.parse().ok())
}

/// The data table id a content control tag names, if it is a calculation data table's.
fn tagged_table_id(tag: &str) -> Option<Option<u64>> {
    tag.strip_prefix(CALC_TABLE_TAG_PREFIX).map(|id| id.parse().ok())
}

/// Whether the body holds a calculation's or a calculation data table's content control (as read
/// from a `.docx`).
pub fn has_calc_controls(doc: &Document) -> bool {
    let mut found = false;
    walk_controls(&doc.body, &mut |c| found |= c.tag.starts_with(CALC_TAG_PREFIX) || c.tag.starts_with(CALC_TABLE_TAG_PREFIX));
    found
}

/// Turn a document read from a `.docx` back into a live one, with the store read beside it.
///
/// For each content control in the body tagged as a calculation (in document order), the first
/// plain equation inside it gets its stored region back: [`CalcState::Fresh`] when its
/// fingerprint matches, [`CalcState::EditedOutside`] (Word's text and structure kept) when it
/// doesn't. Every tagged control's markers are removed, matched or not. Left as plain math: a
/// control naming no stored region, a second control naming the same one (a copy made in Word),
/// and every control when there is no store. A stored region whose control is gone is dropped.
///
/// Then each content control in the body tagged as a calculation data table is removed, wherever
/// its markers are, and the first table inside it (in document order, the first control naming an
/// id claims it) is calculation data again when the store lists its id. Its cells are used as they
/// are; when their fingerprint differs from the stored one the table counts as edited in Word
/// ([`crate::RecalcReport::tables_edited`]). The fingerprint is used for nothing else.
///
/// The store is untrusted: one of over [`MAX_STORED_REGIONS`] entries (regions and tables
/// together) counts as none, and an id stored twice counts as not stored. The store's settings
/// become the document's. Finally the document is recalculated (regions edited outside keep their
/// display, marked) and the pass's report returned. A document with no calculations and no
/// calculation controls is left exactly as it is.
pub fn import(doc: &mut Document, store: Option<CalcStore>) -> crate::RecalcReport {
    doc.calc_store = None;
    let mut by_id: HashMap<u64, StoredRegion> = HashMap::new();
    let mut by_table: HashMap<u64, StoredTable> = HashMap::new();
    if let Some(store) = store.filter(|s| s.regions.len().saturating_add(s.tables.len()) <= MAX_STORED_REGIONS) {
        restore_plots(doc, store.plots);
        by_id = unique_by_id(store.regions.into_iter().map(|r| (r.id, r)));
        by_table = unique_by_id(store.tables.into_iter().map(|t| (t.id, t)));
        doc.calc_settings = (store.settings != CalcSettings::default()).then_some(store.settings);
    }

    let mut ranges: Vec<_> = doc.control_ranges(StoryRef::Body).into_iter().filter_map(|r| tagged_id(&r.control.tag).map(|id| (id, r))).collect();
    ranges.sort_by(|(_, a), (_, b)| (&a.start.path, a.start.off).cmp(&(&b.start.path, b.start.off)));

    let mut markers: BTreeMap<Path, Vec<usize>> = BTreeMap::new();
    for (id, r) in ranges {
        markers.entry(r.start.path.clone()).or_default().push(r.start.off);
        markers.entry(r.end.path.clone()).or_default().push(r.end.off);
        let Some(id) = id else { continue };
        let Ok(p) = doc.para_mut(StoryRef::Body, &r.start.path) else { continue };
        let end = if r.end.path == r.start.path { r.end.off } else { p.text.len() };
        let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
        let first = anchors
            .iter()
            .enumerate()
            .find(|&(k, &off)| off > r.start.off && off < end && matches!(p.objects.get(k), Some(InlineObject::Equation { calc: None, .. })));
        let Some((k, _)) = first else { continue };
        // Claimed by the first control naming it.
        let Some(stored) = by_id.remove(&id) else { continue };
        if let Some(InlineObject::Equation { linear, math, calc, .. }) = p.objects.get_mut(k) {
            let state = if fingerprint(linear, math) == stored.fingerprint { CalcState::Fresh } else { CalcState::EditedOutside };
            let StoredRegion { result_format, worksheet_order, id, source, unit, precision, numbered, .. } = stored;
            *calc = Some(Box::new(CalcRegion { result_format, worksheet_order, id, source, unit, precision, numbered, state }));
            p.touch();
        }
    }
    for (path, mut offs) in markers {
        let Ok(p) = doc.para_mut(StoryRef::Body, &path) else { continue };
        offs.sort_unstable();
        offs.dedup();
        for off in offs.into_iter().rev() {
            if matches!(p.object_at(off), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) {
                let _ = p.delete(off, off + OBJ.len_utf8());
            }
        }
    }

    let mut tables_edited = 0;
    for (path, id) in strip_table_controls(&mut doc.body) {
        let (Some(path), Some(id)) = (path, id) else { continue };
        if !table_at(&doc.body, &path).is_some_and(|t| t.calc.is_none()) {
            continue;
        }
        // Claimed by the first control naming it.
        let Some(stored) = by_table.remove(&id) else { continue };
        let Some(t) = table_mut(&mut doc.body, &path) else { continue };
        if table_fingerprint(t) != stored.fingerprint {
            tables_edited += 1;
        }
        t.calc = Some(Box::new(CalcTable { id, errors: Vec::new() }));
    }
    let report = crate::recalc(doc, &|_| None);
    crate::RecalcReport { tables_edited, ..report }
}

/// `entries` by id, without any id given more than once (which of them is right can't be known).
fn unique_by_id<T>(entries: impl Iterator<Item = (u64, T)>) -> HashMap<u64, T> {
    let mut by_id: HashMap<u64, T> = HashMap::new();
    let mut repeated: HashSet<u64> = HashSet::new();
    for (id, entry) in entries {
        if by_id.insert(id, entry).is_some() {
            repeated.insert(id);
        }
    }
    for id in repeated {
        by_id.remove(&id);
    }
    by_id
}

/// One marker of a content control to remove: in a paragraph, or on a table's wrap.
enum Marker {
    /// A `ControlStart` or `ControlEnd` at byte `off` of the paragraph at `para`.
    InPara { para: Vec<u32>, off: usize },
    /// The `nth` control opening before the table at `table`.
    TableOpen { table: Vec<u32>, nth: usize },
    /// One of the controls closing after the table at `table`.
    TableClose { table: Vec<u32> },
}

/// Remove every content control in the body tagged as a calculation data table (both markers of
/// a whole one, the marker of one without a partner), wherever its markers are. Returns, in
/// document order, the path of the first table inside each (`None` when there is none) and the id
/// its tag names (`None` when it names none). Paths are block index, then row, cell and block
/// index for each table level. Nothing is copied when the body has no such control.
fn strip_table_controls(body: &mut Blocks) -> Vec<(Option<Vec<u32>>, Option<u64>)> {
    let mut any = false;
    walk_controls(body, &mut |c| any |= c.tag.starts_with(CALC_TABLE_TAG_PREFIX));
    if !any {
        return Vec::new();
    }
    // Every block list a control can be in: the body, and each table cell to any depth.
    let mut containers: Vec<Vec<u32>> = vec![Vec::new()];
    walk_blocks(body, &mut |path, b| {
        if let Block::Table(t) = b {
            for (r, row) in t.rows.iter().enumerate() {
                for c in 0..row.cells.len() {
                    let mut cell = path.to_vec();
                    cell.extend([r, c].map(|k| u32::try_from(k).unwrap_or(u32::MAX)));
                    containers.push(cell);
                }
            }
        }
    });
    let mut markers: Vec<Marker> = Vec::new();
    let mut found: Vec<(Option<Vec<u32>>, Option<u64>)> = Vec::new();
    for prefix in containers {
        let Some(bl) = container(body, &prefix) else { continue };
        tagged_tables_in(bl, &prefix, &mut markers, &mut found);
    }

    let mut in_paras: BTreeMap<Vec<u32>, Vec<usize>> = BTreeMap::new();
    let mut opens: BTreeMap<Vec<u32>, Vec<usize>> = BTreeMap::new();
    let mut closes: BTreeMap<Vec<u32>, u32> = BTreeMap::new();
    for m in markers {
        match m {
            Marker::InPara { para, off } => in_paras.entry(para).or_default().push(off),
            Marker::TableOpen { table, nth } => opens.entry(table).or_default().push(nth),
            Marker::TableClose { table } => *closes.entry(table).or_default() += 1,
        }
    }
    for (path, mut offs) in in_paras {
        let Some(p) = para_mut(body, &path) else { continue };
        // Last first, so earlier offsets stay valid.
        offs.sort_unstable();
        offs.dedup();
        for off in offs.into_iter().rev() {
            if matches!(p.object_at(off), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)) {
                let _ = p.delete(off, off + OBJ.len_utf8());
            }
        }
        p.touch();
    }
    for (path, mut nths) in opens {
        let Some(t) = table_mut(body, &path) else { continue };
        nths.sort_unstable();
        nths.dedup();
        for nth in nths.into_iter().rev() {
            if nth < t.controls.open.len() {
                t.controls.open.remove(nth);
            }
        }
    }
    for (path, n) in closes {
        if let Some(t) = table_mut(body, &path) {
            t.controls.close = t.controls.close.saturating_sub(n);
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The block list at `prefix`: the body for an empty one, else a table cell's (the table's path,
/// then row and cell).
fn container<'a>(body: &'a Blocks, prefix: &[u32]) -> Option<&'a Blocks> {
    let [table @ .., r, c] = prefix else { return Some(body) };
    Some(&table_at(body, table)?.rows.get(*r as usize)?.cells.get(*c as usize)?.blocks)
}

/// The markers of the calculation data table controls in the block list `bl` (at `prefix`) to
/// remove, and for each control the path of the first table inside it and the id it names.
fn tagged_tables_in(bl: &Blocks, prefix: &[u32], markers: &mut Vec<Marker>, found: &mut Vec<(Option<Vec<u32>>, Option<u64>)>) {
    let events = control_events(bl);
    if events.is_empty() {
        return;
    }
    let (pairs, _) = pair_events(&events);
    let partner: HashMap<usize, usize> = pairs.iter().map(|&(o, c, _)| (o, c)).collect();
    let at = |block: usize| {
        let mut path = prefix.to_vec();
        path.push(u32::try_from(block).unwrap_or(u32::MAX));
        path
    };
    let marker = |e: &ControlEvent, nth: usize| match (e, e.off()) {
        (_, Some(off)) => Marker::InPara { para: at(e.block()), off },
        (ControlEvent::Open { .. }, None) => Marker::TableOpen { table: at(e.block()), nth },
        (ControlEvent::Close { .. }, None) => Marker::TableClose { table: at(e.block()) },
    };
    // Which of its table's opens each table open is.
    let mut table_opens: HashMap<usize, usize> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        let nth = match e {
            ControlEvent::Open { block, off: None, .. } => {
                let n = table_opens.entry(*block).or_insert(0);
                *n += 1;
                *n - 1
            }
            _ => 0,
        };
        let ControlEvent::Open { block, control, .. } = e else { continue };
        let Some(id) = tagged_table_id(&control.tag) else { continue };
        markers.push(marker(e, nth));
        let close = partner.get(&i).and_then(|&c| events.get(c));
        if let Some(close) = close {
            markers.push(marker(close, 0));
        }
        // The first table from the control's start to its end (a control without an end: only
        // the table it opens before).
        let last = close.map_or(*block, ControlEvent::block);
        let table = (*block..=last).find(|&k| bl.get(k).is_some_and(|b| b.as_table().is_some())).map(at);
        found.push((table, id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_the_reference_values() {
        assert_eq!(fnv1a64(&[b""]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(&[b"a"]), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(&[b"foo", b"bar"]), fnv1a64(&[b"foobar"]));
    }

    #[test]
    fn tags_name_ids() {
        assert_eq!(tagged_id("wordcraft-calc:42"), Some(Some(42)));
        assert_eq!(tagged_id("wordcraft-calc:x"), Some(None));
        assert_eq!(tagged_id("other"), None);
    }

    #[test]
    fn a_deleted_region_sharing_a_live_id_is_saved_under_its_own() {
        use wordcraft_doc::{CharProps, Paragraph, para_block};
        let eq = |id, source: &str| InlineObject::Equation {
            linear: String::new(),
            display: false,
            math: Math::default(),
            calc: Some(Box::new(CalcRegion {
                result_format: None,
                worksheet_order: None,
                id,
                source: source.into(),
                unit: None,
                precision: None,
                numbered: false,
                state: CalcState::Fresh,
            })),
        };
        let mut p = Paragraph::with_text("", CharProps::default());
        p.insert_object(0, eq(1, "a := 1"), &CharProps { del: Some(1), ..Default::default() }).unwrap();
        let end = p.len();
        p.insert_object(end, eq(1, "b := 2"), &CharProps::default()).unwrap();
        let mut doc = Document::new();
        doc.body = vec![para_block(p)];
        let (_, store) = export(&doc);
        let mut got: Vec<(u64, &str)> = store.regions.iter().map(|r| (r.id, r.source.as_str())).collect();
        got.sort();
        assert_eq!(got, vec![(1, "b := 2"), (2, "a := 1")]);
    }

    #[test]
    fn table_fingerprint_separates_cells_and_rows() {
        use wordcraft_doc::Row;
        let t = |rows: &[&[&str]]| Table {
            rows: rows.iter().map(|r| Row { cells: r.iter().map(|c| Cell::with_text(c)).collect(), ..Default::default() }).collect(),
            ..Default::default()
        };
        assert_eq!(table_fingerprint(&t(&[&["a", " b "], &["c", "d"]])), fnv1a64(&["a\u{1f}b\u{1e}c\u{1f}d".as_bytes()]));
        assert_ne!(table_fingerprint(&t(&[&["a", "b"]])), table_fingerprint(&t(&[&["a"], &["b"]])));
    }

    #[test]
    fn deleted_and_nested_data_tables_round_trip_in_memory() {
        use wordcraft_doc::{CharProps, Paragraph, Row, para_block};
        let table = |id, text: &str, props: &CharProps| Table {
            rows: ["H", text]
                .iter()
                .map(|t| Row {
                    cells: vec![Cell { blocks: vec![para_block(Paragraph::with_text(t, props.clone()))], ..Default::default() }],
                    ..Default::default()
                })
                .collect(),
            calc: Some(Box::new(CalcTable { id, errors: Vec::new() })),
            ..Default::default()
        };
        let deleted = CharProps { del: Some(1), ..Default::default() };
        // A live table, a tracked-deleted one with the same id, and one nested in a plain table's cell.
        let mut outer = Table { rows: vec![Row { cells: vec![Cell::default()], ..Default::default() }], ..Default::default() };
        outer.rows[0].cells[0].blocks = vec![std::sync::Arc::new(Block::Table(table(5, "3", &CharProps::default())))];
        let mut doc = Document::new();
        doc.body = vec![
            std::sync::Arc::new(Block::Table(table(1, "1", &CharProps::default()))),
            std::sync::Arc::new(Block::Table(table(1, "2", &deleted))),
            std::sync::Arc::new(Block::Table(outer)),
        ];
        let (mut copy, store) = export(&doc);
        let ids: Vec<u64> = store.tables.iter().map(|t| t.id).collect();
        assert_eq!(ids, [1, 6, 5], "the deleted table is saved under an id of its own, above every other");
        let marked = |d: &Document| -> Vec<Option<u64>> {
            let mut out = Vec::new();
            walk_blocks(&d.body, &mut |_, b| {
                if let Block::Table(t) = b {
                    out.push(t.calc.as_ref().map(|c| c.id));
                }
            });
            out
        };
        assert_eq!(marked(&copy), [None, None, None, None]);
        assert_eq!(copy.body[0].as_table().map(|t| t.controls.close), Some(1));
        let report = import(&mut copy, Some(store));
        assert_eq!(report.tables_edited, 0);
        assert_eq!(marked(&copy), [Some(1), Some(6), None, Some(5)]);
        let mut wrapped = false;
        walk_controls(&copy.body, &mut |_| wrapped = true);
        assert!(!wrapped, "every control removed");
    }

    #[test]
    fn a_plain_document_is_left_alone() {
        let mut doc = Document::from_text("Plain\nText");
        let before = doc.clone();
        import(&mut doc, None);
        assert_eq!(doc, before);
        let (copy, store) = export(&doc);
        assert_eq!(copy, doc);
        assert!(store.is_empty());
    }
}
