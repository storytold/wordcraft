//! The recalculation pass: every calculation region in the document's body, evaluated in
//! dependency order, with unique names, and its display and quoted values rewritten.
//!
//! Stage 1 covers the main body and table cells (to any depth); headers, footers, notes, comments
//! and text boxes are skipped, as is tracked-deleted content. The pass reads the whole document
//! first and changes only the objects whose display, state or id actually changes, so plain
//! equations and untouched paragraphs stay exactly as they were (and shared, for undo).
//!
//! A body table marked as calculation data ([`CalcTable`]) defines one column vector per column
//! (see [`crate::tables`]). Its columns are definitions like regions', placed at the table's
//! position in the document (before anything inside it); what is wrong with them is written to
//! the table's [`CalcTable::errors`], never into its text.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use craft_units::{Formatted, Precision, Scale, UnitError, UnitSystem, format, parse_unit};
use wordcraft_doc::calc::{self, CalcSettings, CalcState, CalcTable, ColumnError, GMeans, QuoteSpec, parse_calc_field};
use wordcraft_doc::math::Arg;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Block, Blocks, Document, InlineObject, Paragraph, Row, Table};

use crate::array::{self, Data};
use crate::ast::{self, keywords_used};
use crate::eval::{MAX_PASS_STEPS, Pass, Val, check_display, check_element, run_stmt_in, unresolved};
use crate::render::{self, MAX_SHOWN_COLS, MAX_SHOWN_ROWS, Rendered, Shown, rendered, shown_result, shows_value};
use crate::tables;
use crate::{CalcError, Env, Stmt, check_name, defined_name, free_names, parse};

mod blocks;

/// What a [`recalc`] pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecalcReport {
    /// Calculation regions in the body (tracked deletions left out).
    pub regions: usize,
    /// Regions left in [`CalcState::Error`].
    pub errors: usize,
    /// Regions given a new id because an earlier region already had theirs (a copy and paste).
    pub reassigned_ids: usize,
    /// Regions edited outside WordCraft ([`CalcState::EditedOutside`]) waiting to be resolved.
    pub edited: usize,
    /// Calculation data tables whose cells were edited outside WordCraft (found when a `.docx` is
    /// read back: [`crate::docx::import`]); their new values are used. Always 0 from [`recalc`].
    pub tables_edited: usize,
}

/// Most names a circular-definition message lists.
const MAX_CYCLE_NAMES: usize = 5;
/// Most values a quoted vector or matrix lists; the rest are counted, never left out silently.
const MAX_QUOTED_VALUES: usize = 50;

/// Recalculate every calculation in the body of `doc`.
///
/// 1. Collects the regions in document order (paragraphs top to bottom, objects left to right,
///    table cells row by row), giving a fresh id to every region whose id an earlier one has.
/// 2. Parses each region's source; a parse error defines nothing. Finds the solve blocks
///    (`Given`, its equations, and the `Find`/`Minerr` that ends it: the one place where the order
///    of regions matters), each closing statement solving its block's equations.
/// 3. The first region in document order defining a name owns it; a later one is a duplicate
///    error (`page_of` maps the owner's id to its page for the message), and a unit, function or
///    constant name is a reserved-name error. Neither defines anything.
/// 4. Evaluates in dependency order. Regions on a cycle are circular errors and every region that
///    depends on a failed name is an upstream error.
/// 5. Rewrites each region's displayed result. A region edited outside the calculator keeps what
///    it shows, with the edited mark after it ([`render::mark_edited`]); it defines nothing, so
///    every region that uses its name is an error ("depends on a calculation edited in Word")
///    and every quoted value of it shows `?`.
/// 6. Rewrites the result of every `WCCALC` field quoting a value; one with no valid value
///    shows `?`.
/// 7. Writes each data table's errors to its [`CalcTable`] (and a fresh id to a pasted copy), only
///    when they change. A data table's columns take part in steps 3 and 4 like regions defining
///    their names, at the table's place; a calculation using a column in error is an upstream
///    table error ("depends on table data with an error").
///
/// Never panics; any document, however malformed, gives a report.
pub fn recalc(doc: &mut Document, page_of: &dyn Fn(u64) -> Option<u32>) -> RecalcReport {
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let found = collect(&doc.body);
    let (ids, reassigned_ids) = assign_ids(&found);
    let table_ids = assign_table_ids(&found);
    let regions = &found.regions;
    let tables = read_tables(&doc.body, &found.tables, &settings);
    let index = Index::build(regions, &ids, &tables, page_of);
    let (env, outcomes, failed) = evaluate(&index, &settings);

    let mut errors = 0;
    let mut edited = 0;
    let mut edits: Vec<Edit> = Vec::new();
    for (i, r) in regions.iter().enumerate() {
        let id = ids.get(i).copied().unwrap_or(r.id);
        if r.state == CalcState::EditedOutside {
            edited += 1;
            // Word's equation stays as it is, with the mark after it so it never looks current.
            let display = object(&doc.body, &r.at).and_then(|o| match o {
                InlineObject::Equation { linear, math, .. } => render::mark_edited(linear, math),
                _ => None,
            });
            if id != r.id || display.is_some() {
                edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: r.state.clone(), display } });
            }
            continue;
        }
        let stmt = index.parsed.get(i);
        let outcome = outcomes.get(i).cloned().unwrap_or(Err(CalcError::Upstream));
        let (state, shown) = match outcome {
            Ok(shown) => (CalcState::Fresh, shown),
            Err(e) => (CalcState::Error(e.to_string()), None),
        };
        if matches!(state, CalcState::Error(_)) {
            errors += 1;
        }
        let parse_failed = Err(CalcError::Upstream);
        let display = number_if(r.numbered, rendered(stmt.unwrap_or(&parse_failed), &r.source, shown.as_ref(), &state));
        edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state, display: Some(display) } });
    }
    for f in &found.fields {
        let result = field_result(f, &index, &env, &failed, &settings);
        edits.push(Edit { at: f.at.clone(), change: Change::Field { result } });
    }
    let table_edits = table_changes(&found.tables, &table_ids, &tables, &index, &outcomes);
    apply(&mut doc.body, edits);
    apply_tables(&mut doc.body, table_edits);
    RecalcReport { regions: regions.len(), errors, reassigned_ids, edited, tables_edited: 0 }
}

/// Every live calculation data table of the body (tables in table cells too, tracked deletions
/// left out) in document order: the path of its block (body block index, then row, cell and block
/// index for each table level) and the id [`recalc`] gives it (a repeated id, from a pasted copy,
/// already replaced by the next id above the largest). Table ids are an id space of their own.
pub fn data_tables(doc: &Document) -> Vec<(Vec<u32>, u64)> {
    let found = collect(&doc.body);
    let ids = assign_table_ids(&found);
    found.tables.into_iter().zip(ids).map(|(t, id)| (t.path, id)).collect()
}

/// Every live calculation region of the body (table cells to any depth, tracked deletions left
/// out) in document order: the path of its paragraph (block index, then row, cell and block index
/// for each table level), the byte offset of its object character there, and the region with the
/// id [`recalc`] gives it (a repeated id already replaced). The one traversal every caller uses,
/// so ids found here are the ids recalculation uses.
pub fn body_regions(doc: &Document) -> Vec<(Vec<u32>, usize, calc::CalcRegion)> {
    let found = collect(&doc.body);
    let (ids, _) = assign_ids(&found);
    found
        .regions
        .into_iter()
        .zip(ids)
        .map(|(r, id)| {
            let region = calc::CalcRegion {
                result_format: r.result_format,
                worksheet_order: r.worksheet_order,
                id,
                source: r.source,
                unit: r.unit,
                precision: r.precision,
                numbered: r.numbered,
                state: r.state,
            };
            (r.at.path, r.off, region)
        })
        .collect()
}

/// Every variable owned by a definition, with its region's id, sorted by name: what a quoted
/// value can show. Owning means the first region in document order that defines the name, with
/// a name that isn't a unit, function or constant. User functions are left out (they have no
/// single value); ids are the ones [`recalc`] gives, a repeated id already replaced.
pub fn defined_names(doc: &Document) -> Vec<(String, u64)> {
    let (ids, _, index) = names_of(doc);
    // A data table's columns have no region's id: they are left out here.
    let mut names: Vec<(String, u64)> = index
        .owners
        .iter()
        .filter(|&(name, &i)| {
            check_name(name).is_ok() && index.is_region(i) && index.parsed.get(i).is_some_and(|s| s.as_ref().is_ok_and(defines_variables))
        })
        .map(|(name, &i)| (name.clone(), ids.get(i).copied().unwrap_or(0)))
        .collect();
    names.sort();
    names
}

/// Whether `stmt` defines variables (each with a value to quote): `x := …` or `[a; b] := …`, not
/// a function.
fn defines_variables(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Define { params, .. } => params.is_empty(),
        Stmt::DefineMany { .. } => true,
        Stmt::Eval { .. } | Stmt::Given | Stmt::Constraint { .. } => false,
    }
}

/// Every vector owned by a data table's column, with its table's id and the column (counted from
/// 0), sorted by name: what a quoted value can show besides [`defined_names`]. A column owns its
/// name when it is the first definition of it in document order and its heading's name reads
/// (also when its values don't: a quoted value of it shows `?` until they do). Table ids are the
/// ones [`recalc`] gives ([`data_tables`]), an id space of their own: never a region's id.
pub fn defined_columns(doc: &Document) -> Vec<(String, u64, u32)> {
    let (_, table_ids, index) = names_of(doc);
    let mut columns: Vec<(String, u64, u32)> = index
        .owners
        .iter()
        .filter(|&(name, &i)| check_name(name).is_ok() && !index.is_region(i))
        .filter_map(|(name, &i)| {
            let &(t, col) = index.columns.get(i.checked_sub(index.regions)?)?;
            Some((name.clone(), *table_ids.get(t)?, col))
        })
        .collect();
    columns.sort();
    columns
}

/// The regions' ids and the data tables' ids as [`recalc`] gives them (a repeated id replaced),
/// and who owns each name (data table columns included).
fn names_of(doc: &Document) -> (Vec<u64>, Vec<u64>, Index) {
    let found = collect(&doc.body);
    let (ids, _) = assign_ids(&found);
    let table_ids = assign_table_ids(&found);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let tables = read_tables(&doc.body, &found.tables, &settings);
    let index = Index::build(&found.regions, &ids, &tables, &|_| None);
    (ids, table_ids, index)
}

/// Resolve the worksheet once, then feed its functions and variables to the plot sampler.
pub fn sample_plots(doc: &Document, plots: &[wordcraft_doc::chart::PlotSpec]) -> Vec<Result<Vec<(f64, f64)>, CalcError>> {
    let (_, _, index) = names_of(doc);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let (env, _, _) = evaluate(&index, &settings);
    let budget = crate::eval::Pass::new(2_000_000);
    plots.iter().map(|plot| crate::eval::sample_plot(&mut env.clone(), plot, &budget)).collect()
}

/// Whether a quoted value of the variable `name` can show it in `unit`: the unit must read
/// ([`parse_unit`]) and, when the variable has a value now, be of its dimension (every element's,
/// for a vector or matrix), and `°C`/`°F` only for an absolute temperature. A variable with no
/// value now (an error) is checked when it has one; until then the quoted value shows `?`.
pub fn check_quote_unit(doc: &Document, name: &str, unit: &str) -> Result<(), CalcError> {
    let (_, _, index) = names_of(doc);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let (env, _, _) = evaluate(&index, &settings);
    env.display_scale(unit)?;
    match env.data(name) {
        Some(data) => check_display(&env, unit, data),
        None => Ok(()),
    }
}

/// Whether a quoted value of the variable `name` can show its element `at` (one index, or a row
/// and a column, counted from the document's origin): the variable's value now must be a vector
/// or matrix (`x isn't a vector`) holding that element, with the message `name[…]` would give in
/// a calculation. A variable with no value now (an error) is checked when it has one; until then
/// the quoted value shows `?`. Evaluates a copy of the calculations: `doc` never changes.
pub fn check_quote_index(doc: &Document, name: &str, at: (u32, Option<u32>)) -> Result<(), CalcError> {
    let (_, _, index) = names_of(doc);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let (env, _, _) = evaluate(&index, &settings);
    match env.data(name) {
        Some(data) => check_element(data, name, env.origin(), at),
        None => Ok(()),
    }
}

/// Whether `name` may be given to a new calculation, or to the region `editing` (whose own
/// definition of the name doesn't count): [`CalcError::ReservedName`] for a unit, function or
/// constant name, [`CalcError::Duplicate`] when another region already owns it (a variable or a
/// function) or a data table's column does.
pub fn check_new_name(doc: &Document, name: &str, editing: Option<u64>) -> Result<(), CalcError> {
    check_name(name)?;
    // Ids as recalculation gives them, so a pasted copy's repeated id can't stand for its owner.
    let (ids, _, index) = names_of(doc);
    let duplicate = || Err(CalcError::Duplicate { name: name.to_string(), page: None });
    match index.owners.get(name) {
        Some(&i) if !index.is_region(i) => duplicate(),
        Some(&i) => match ids.get(i) {
            Some(&owner) if Some(owner) != editing => duplicate(),
            _ => Ok(()),
        },
        None => Ok(()),
    }
}

/// Mark every current ([`CalcState::Fresh`]) region in the body [`CalcState::Stale`], for manual
/// recalculation: its result is redrawn grey after `≈`, so it never looks current. A region whose
/// id an earlier one has (a pasted copy) gets the id [`recalc`] would give it. Quoted values
/// (`WCCALC` fields) get a leading `≈ ` too; [`recalc`] puts back the exact value.
pub fn mark_stale(doc: &mut Document) {
    let found = collect(&doc.body);
    // A pasted copy gets its own id now, as recalculation would give it: the id stored on every
    // region is unique after any command, so nothing that names a region by it reaches another.
    let (ids, _) = assign_ids(&found);
    let mut edits = Vec::new();
    for (i, r) in found.regions.iter().enumerate() {
        let id = ids.get(i).copied().unwrap_or(r.id);
        if r.state != CalcState::Fresh {
            if id != r.id {
                edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: r.state.clone(), display: None } });
            }
            continue;
        }
        let stmt = parse(&r.source);
        let old = object(&doc.body, &r.at).and_then(|o| match o {
            InlineObject::Equation { linear, .. } => shown_result(linear),
            _ => None,
        });
        let shown = stmt.as_ref().ok().filter(|s| shows_value(s)).and(old);
        let display = number_if(r.numbered, rendered(&stmt, &r.source, shown.as_ref(), &CalcState::Stale));
        edits.push(Edit { at: r.at.clone(), change: Change::Region { id, state: CalcState::Stale, display: Some(display) } });
    }
    // A quoted value is out of date too: it gets the same `≈`, once.
    for f in &found.fields {
        if let Some(InlineObject::Field { result, .. }) = object(&doc.body, &f.at)
            && result != "?"
            && !result.starts_with('≈')
        {
            edits.push(Edit { at: f.at.clone(), change: Change::Field { result: format!("≈ {result}") } });
        }
    }
    apply(&mut doc.body, edits);
}

// ---------------------------------------------------------------------------------------------
// Collecting
// ---------------------------------------------------------------------------------------------

/// Where an object sits: the path to its paragraph (body block index, then row, cell and block
/// index for each table level) and its index in the paragraph's objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct At {
    pub(crate) path: Vec<u32>,
    pub(crate) obj: usize,
}

pub(crate) struct FoundRegion {
    result_format: Option<calc::ResultFormat>,
    worksheet_order: Option<u64>,
    pub(crate) at: At,
    /// Byte offset of its object character in the paragraph's text.
    off: usize,
    pub(crate) id: u64,
    source: String,
    unit: Option<String>,
    precision: Option<u8>,
    numbered: bool,
    state: CalcState,
}

struct FoundField {
    at: At,
    spec: QuoteSpec,
}

/// A live calculation data table of the body.
pub(crate) struct FoundTable {
    /// The path of its block: body block index, then row, cell and block index for each table level.
    pub(crate) path: Vec<u32>,
    pub(crate) id: u64,
    /// The errors it carries now.
    errors: Vec<ColumnError>,
}

#[derive(Default)]
pub(crate) struct Found {
    pub(crate) regions: Vec<FoundRegion>,
    fields: Vec<FoundField>,
    /// Ids of tracked-deleted regions: never reused for a reassigned id.
    deleted_ids: Vec<u64>,
    /// Where each tracked-deleted region is, with its id (in document order).
    pub(crate) deleted: Vec<(At, u64)>,
    /// Calculation data tables, in document order (a table before anything inside it).
    pub(crate) tables: Vec<FoundTable>,
    /// Where each tracked-deleted data table is, with its id (in document order): its id is never
    /// reused for a reassigned one.
    pub(crate) deleted_tables: Vec<(Vec<u32>, u64)>,
}

/// The calculation regions, quoted values and calculation data tables of the body, in document
/// order.
pub(crate) fn collect(body: &Blocks) -> Found {
    let mut found = Found::default();
    walk_blocks(body, &mut |path, b| match b {
        Block::Para(p) => visit(path, p, &mut found),
        Block::Table(t) => visit_table(path, t, &mut found),
    });
    found
}

/// Every paragraph of `blocks` in document order (table cells row by row, to any depth), with its
/// path: block index, then row, cell and block index for each table level.
fn walk_paras<'a>(blocks: &'a Blocks, f: &mut dyn FnMut(&[u32], &'a Paragraph)) {
    walk_blocks(blocks, &mut |path, b| {
        if let Block::Para(p) = b {
            f(path, p);
        }
    });
}

/// Every paragraph and table of `blocks` in document order (a table, then its cells row by row,
/// to any depth), with its path: block index, then row, cell and block index for each table level.
pub(crate) fn walk_blocks<'a>(blocks: &'a Blocks, f: &mut dyn FnMut(&[u32], &'a Block)) {
    for (i, b) in blocks.iter().enumerate() {
        let top = u32::try_from(i).unwrap_or(u32::MAX);
        match &**b {
            Block::Para(_) => f(&[top], b),
            Block::Table(_) => {
                // An explicit stack, last first, so cells come off it row by row.
                let mut todo: Vec<(&Block, Vec<u32>)> = vec![(&**b, vec![top])];
                while let Some((b, path)) = todo.pop() {
                    f(&path, b);
                    if let Block::Table(t) = b {
                        for (r, row) in t.rows.iter().enumerate().rev() {
                            for (c, cell) in row.cells.iter().enumerate().rev() {
                                for (j, cb) in cell.blocks.iter().enumerate().rev() {
                                    let mut inner = path.clone();
                                    inner.extend([r, c, j].map(|k| u32::try_from(k).unwrap_or(u32::MAX)));
                                    todo.push((&**cb, inner));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Note `t` when it is calculation data: live, or (its text all tracked-deleted) its id kept from
/// reuse.
fn visit_table(path: &[u32], t: &Table, found: &mut Found) {
    let Some(c) = &t.calc else { return };
    if inside_deletion(t) {
        found.deleted_tables.push((path.to_vec(), c.id));
    } else {
        found.tables.push(FoundTable { path: path.to_vec(), id: c.id, errors: c.errors.clone() });
    }
}

/// Whether `t` sits inside a tracked deletion: it has text and all of it, at any depth, is
/// tracked-deleted (the model has no tracked table deletion; deleting a table with tracking on
/// marks its text deleted).
fn inside_deletion(t: &Table) -> bool {
    tables::deleted_only(t.rows.iter().flat_map(|r| &r.cells))
}

/// Whether `o` is a calculation region or a quoted value (`WCCALC` field; the field type is the
/// whole first word of its code).
pub fn is_calc_object(o: &InlineObject) -> bool {
    match o {
        InlineObject::Equation { calc, .. } => calc.is_some(),
        InlineObject::Field { instr, .. } => {
            let code = instr.trim_start();
            code.get(..calc::CALC_FIELD.len()).is_some_and(|w| w.eq_ignore_ascii_case(calc::CALC_FIELD))
                && code.get(calc::CALC_FIELD.len()..).is_some_and(|rest| rest.chars().next().is_none_or(char::is_whitespace))
        }
        _ => false,
    }
}

/// One calculation object as far as the recalculation pass is concerned: what it reads, and what
/// it writes (so a display or quoted value changed by anything else is drawn again).
#[derive(PartialEq)]
enum Item<'a> {
    Region {
        story: Option<u32>,
        deleted: bool,
        region: &'a calc::CalcRegion,
        linear: &'a str,
    },
    Field {
        story: Option<u32>,
        deleted: bool,
        instr: &'a str,
        result: &'a str,
        dirty: bool,
    },
    /// A calculation data table: what it reads (its rows) and what the pass writes (its errors).
    Table {
        story: Option<u32>,
        data: &'a CalcTable,
        rows: Rows<'a>,
    },
}

/// A data table's rows as [`calc_unchanged`] compares them: each cell's blocks by address first
/// (an edit copies only the blocks it changes), then by value.
struct Rows<'a>(&'a [Row]);

impl PartialEq for Rows<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(other.0).all(|(a, b)| {
                a.props == b.props
                    && a.cells.len() == b.cells.len()
                    && a.cells.iter().zip(&b.cells).all(|(x, y)| {
                        x.props == y.props
                            && x.blocks.len() == y.blocks.len()
                            && x.blocks.iter().zip(&y.blocks).all(|(p, q)| Arc::ptr_eq(p, q) || p == q)
                    })
            })
    }
}

/// Every calculation object and data table of `doc` in document order (a table before its
/// cells): the body's, then each other story's.
fn items(doc: &Document) -> Vec<Item<'_>> {
    let mut out = Vec::new();
    let stories = std::iter::once((None, &doc.body)).chain(doc.parts.iter().map(|(&k, part)| (Some(k), &part.blocks)));
    for (story, blocks) in stories {
        walk_blocks(blocks, &mut |_, b| {
            let p = match b {
                Block::Para(p) => p,
                Block::Table(t) => {
                    if let Some(data) = &t.calc {
                        out.push(Item::Table { story, data, rows: Rows(&t.rows) });
                    }
                    return;
                }
            };
            if !p.objects.iter().any(is_calc_object) {
                return;
            }
            let deleted = p.deleted_ranges();
            let mut anchors = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i);
            for o in &p.objects {
                let deleted = anchors.next().is_some_and(|off| deleted.iter().any(|r| r.contains(&off)));
                match o {
                    InlineObject::Equation { linear, calc: Some(region), .. } => out.push(Item::Region { story, deleted, region, linear }),
                    InlineObject::Field { instr, result, dirty, .. } if is_calc_object(o) => {
                        out.push(Item::Field { story, deleted, instr, result, dirty: *dirty });
                    }
                    _ => {}
                }
            }
        });
    }
    out
}

/// Whether the pass has nothing to do after a command turned `before` into `after`: the same
/// calculation settings and the same calculation objects and data tables in the same order (each
/// region's id, source, unit, precision, state and display; each quoted value's code and result;
/// whether each is tracked-deleted and which story it is in; each data table's id, errors and
/// rows), and, in automatic mode, no result out of date. Typing ordinary text outside data
/// tables leaves all of that alone, so it costs no recalculation (and in manual mode marks
/// nothing stale).
pub fn calc_unchanged(before: &Document, after: &Document) -> bool {
    if before.calc_settings != after.calc_settings {
        return false;
    }
    let now = items(after);
    let manual = after.calc_settings.as_ref().is_some_and(|c| c.manual);
    // In automatic mode a stale result is left from a pass that failed: the next edit retries.
    let stale = !manual && now.iter().any(|i| matches!(i, Item::Region { region, .. } if region.state == CalcState::Stale));
    !stale && items(before) == now
}

/// What a calculation outside the main text says instead of a result.
pub const NOT_CALCULATED_HERE: &str = "not calculated here";

/// Calculations live in the main text and its tables only. One that lands anywhere else — pasted
/// into a text box, header, footer, note or comment, or read from a file that has it there — is
/// never recalculated, so it must not look current:
///
/// - a calculation region becomes a plain equation showing its calculation with no result, then
///   a red `⚠` and "not calculated here";
/// - a quoted value (`WCCALC` field) shows `?`.
///
/// Returns how many objects it changed. Paragraphs with nothing to change are not copied.
pub fn freeze_outside_body(doc: &mut Document) -> usize {
    let mut todo: Vec<(u32, Vec<u32>)> = Vec::new();
    for (&id, part) in &doc.parts {
        walk_paras(&part.blocks, &mut |path, p| {
            if p.objects.iter().any(needs_freezing) {
                todo.push((id, path.to_vec()));
            }
        });
    }
    let mut changed = 0;
    for (id, path) in todo {
        let Some(part) = doc.parts.get_mut(&id) else { continue };
        let Some(p) = para_mut(&mut part.blocks, &path) else { continue };
        for o in &mut p.objects {
            if needs_freezing(o) {
                freeze(o);
                changed += 1;
            }
        }
        p.touch();
    }
    changed
}

/// `display`, numbered when the region is ([`render::numbered`]).
fn number_if(numbered: bool, display: Rendered) -> Rendered {
    if numbered { render::numbered(display) } else { display }
}

/// Whether [`freeze_outside_body`] has anything to do to `o`.
fn needs_freezing(o: &InlineObject) -> bool {
    match o {
        InlineObject::Equation { calc, .. } => calc.is_some(),
        InlineObject::Field { result, dirty, .. } => is_calc_object(o) && (result != "?" || *dirty),
        _ => false,
    }
}

fn freeze(o: &mut InlineObject) {
    match o {
        InlineObject::Equation { linear, math, calc, .. } => {
            let Some(region) = calc.take() else { return };
            let state = CalcState::Error(NOT_CALCULATED_HERE.to_string());
            let mut display = rendered(&parse(&region.source), &region.source, None, &state);
            render::push_note(&mut display, NOT_CALCULATED_HERE);
            let display = number_if(region.numbered, display);
            *linear = display.linear;
            set_nodes(math, display.math);
        }
        InlineObject::Field { result, dirty, .. } => {
            *result = "?".to_string();
            *dirty = false;
        }
        _ => {}
    }
}

fn visit(path: &[u32], p: &Paragraph, found: &mut Found) {
    if !p.objects.iter().any(is_calc_object) {
        return;
    }
    // Object k sits at the k-th object character; one past them is shown nowhere.
    let anchors: Vec<usize> = p.text.char_indices().filter(|&(_, c)| c == OBJ).map(|(i, _)| i).collect();
    let deleted = p.deleted_ranges();
    for (k, o) in p.objects.iter().enumerate() {
        let Some(&off) = anchors.get(k) else { break };
        let is_deleted = deleted.iter().any(|r| r.contains(&off));
        let at = || At { path: path.to_vec(), obj: k };
        match o {
            InlineObject::Equation { calc: Some(c), .. } => {
                if is_deleted {
                    found.deleted_ids.push(c.id);
                    found.deleted.push((at(), c.id));
                } else {
                    found.regions.push(FoundRegion {
                        result_format: c.result_format,
                        worksheet_order: c.worksheet_order,
                        at: at(),
                        off,
                        id: c.id,
                        source: c.source.clone(),
                        unit: c.unit.clone(),
                        precision: c.precision,
                        numbered: c.numbered,
                        state: c.state.clone(),
                    });
                }
            }
            InlineObject::Field { instr, .. } if !is_deleted => {
                if let Some(spec) = parse_calc_field(instr) {
                    found.fields.push(FoundField { at: at(), spec });
                }
            }
            _ => {}
        }
    }
}

/// Each region's id, the first of any repeated id kept and the rest given the next unused one;
/// and how many were reassigned.
pub(crate) fn assign_ids(found: &Found) -> (Vec<u64>, usize) {
    let ids: Vec<u64> = found.regions.iter().map(|r| r.id).collect();
    unique_ids(&ids, &found.deleted_ids)
}

/// Each data table's id, as [`assign_ids`] gives regions theirs. Tables have an id space of
/// their own.
pub(crate) fn assign_table_ids(found: &Found) -> Vec<u64> {
    let ids: Vec<u64> = found.tables.iter().map(|t| t.id).collect();
    let deleted: Vec<u64> = found.deleted_tables.iter().map(|&(_, id)| id).collect();
    unique_ids(&ids, &deleted).0
}

/// `ids` with the first of any repeated id kept and the rest given the next one above the
/// largest (of `ids` and `reserved`), or the lowest unused when the top of the range is taken;
/// and how many were reassigned.
fn unique_ids(ids: &[u64], reserved: &[u64]) -> (Vec<u64>, usize) {
    let mut used: HashSet<u64> = ids.iter().chain(reserved).copied().collect();
    let mut next = used.iter().max().and_then(|m| m.checked_add(1));
    let mut probe: u64 = 0;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut reassigned = 0;
    let ids = ids
        .iter()
        .map(|&old| {
            if seen.insert(old) {
                return old;
            }
            reassigned += 1;
            let id = match next {
                Some(n) => {
                    next = n.checked_add(1);
                    n
                }
                // The top of the range is taken: the lowest free id. There are far fewer regions
                // than ids, so one is always found.
                None => {
                    while used.contains(&probe) {
                        probe = probe.saturating_add(1);
                    }
                    probe
                }
            };
            used.insert(id);
            seen.insert(id);
            id
        })
        .collect();
    (ids, reassigned)
}

// ---------------------------------------------------------------------------------------------
// Names and evaluation
// ---------------------------------------------------------------------------------------------

/// The definitions one live data table makes.
struct TableDefs {
    /// The path of the table's block.
    path: Vec<u32>,
    /// Each column (counted from 0): the name its heading gives, when it reads (also when the
    /// values don't, so the name is owned and its users are told the table has an error), and
    /// its definition.
    cols: Vec<(u32, Option<String>, Result<Stmt, CalcError>)>,
    /// An error for the whole table.
    error: Option<CalcError>,
}

/// The definitions each live data table of `body` makes, read with the document's meaning of `g`.
fn read_tables(body: &Blocks, tables: &[FoundTable], settings: &CalcSettings) -> Vec<TableDefs> {
    let g_is_gravity = settings.g == GMeans::Gravity;
    tables
        .iter()
        .map(|ft| {
            let Some(t) = table_at(body, &ft.path) else { return TableDefs { path: ft.path.clone(), cols: Vec::new(), error: None } };
            let (stmts, error) = tables::column_stmts(t, g_is_gravity);
            let cols = stmts
                .into_iter()
                .map(|(col, stmt)| {
                    let name = match &stmt {
                        Ok(stmt) => defined_name(stmt).map(str::to_string),
                        Err(_) => tables::column_name(t, col),
                    };
                    (col, name, stmt)
                })
                .collect();
            TableDefs { path: ft.path.clone(), cols, error }
        })
        .collect()
}

/// Every definition, parsed, and who owns each name. The definitions are the regions, in
/// document order, then the data tables' columns, table by table; `order` lists them all in
/// document order, a table's columns at the table's place (before anything inside it).
struct Index {
    parsed: Vec<Result<Stmt, CalcError>>,
    /// An error found before evaluation: parse, reserved name, duplicate, or a column whose
    /// values don't read.
    early: Vec<Option<CalcError>>,
    /// Name → index of the definition that owns it. A name refused as a unit, function or constant
    /// is owned by its first (failed) definition, so its uses fail.
    owners: HashMap<String, usize>,
    /// Regions edited outside the calculator: they keep owning their name but define nothing.
    edited: Vec<bool>,
    /// Each region's own precision.
    precision: Vec<Option<u8>>,
    formats: Vec<Option<calc::ResultFormat>>,
    /// The names each definition defines: a region's (several for `[a; b] := …`), or a column's
    /// heading (also when the column's values don't read).
    names: Vec<Vec<String>>,
    /// How many definitions are regions (the first ones).
    regions: usize,
    /// Each column definition (index `regions + k`): its table (index into the tables read) and
    /// column.
    columns: Vec<(usize, u32)>,
    /// Every definition, in document order.
    order: Vec<usize>,
    /// The solve blocks among the regions, each closing statement rewritten into its solve.
    blocks: Vec<blocks::Block>,
}

impl Index {
    fn build(regions: &[FoundRegion], ids: &[u64], tables: &[TableDefs], page_of: &dyn Fn(u64) -> Option<u32>) -> Index {
        let mut parsed: Vec<Result<Stmt, CalcError>> = regions.iter().map(|r| parse(&r.source)).collect();
        let mut region_order: Vec<_> = (0..regions.len()).collect();
        region_order.sort_by_key(|&i| regions[i].worksheet_order.unwrap_or(u64::MAX));
        let blocks = blocks::resolve(&mut parsed, region_order);
        let mut names: Vec<Vec<String>> =
            parsed.iter().map(|s| s.as_ref().map(|s| ast::defined_names(s).into_iter().map(str::to_string).collect()).unwrap_or_default()).collect();
        let mut columns = Vec::new();
        // Where each definition is: its paragraph's path and object, or its table's path and column.
        let mut places: Vec<(&[u32], usize, usize)> = regions.iter().enumerate().map(|(i, r)| (r.at.path.as_slice(), r.at.obj, i)).collect();
        for (t, defs) in tables.iter().enumerate() {
            for (col, name, stmt) in &defs.cols {
                places.push((&defs.path, usize::try_from(*col).unwrap_or(usize::MAX), parsed.len()));
                columns.push((t, *col));
                names.push(name.iter().cloned().collect());
                parsed.push(stmt.clone());
            }
        }
        // A table's path is a prefix of (so sorts before) every path inside it. Regions are in
        // document order already; the sort is stable.
        if !columns.is_empty() {
            places.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        }
        // Imported worksheets can display calculations in an appendix while solving them
        // at their source positions. Stable sort keeps ordinary content in document order.
        if regions.iter().any(|r| r.worksheet_order.is_some()) {
            places.sort_by_key(|&(_, _, i)| regions.get(i).and_then(|r| r.worksheet_order).unwrap_or(u64::MAX));
        }
        let order: Vec<usize> = places.into_iter().map(|(_, _, i)| i).collect();
        let mut owners: HashMap<String, usize> = HashMap::new();
        let mut early: Vec<Option<CalcError>> = vec![None; parsed.len()];
        // The first definition of a name in document order owns it.
        for &i in &order {
            let region = i < regions.len();
            let own = match parsed.get(i) {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            };
            // A region with a parse error defines nothing; a column whose values don't read still
            // owns its heading's name.
            let defines = match names.get(i) {
                Some(defines) if !defines.is_empty() && !(region && own.is_some()) => defines,
                _ => {
                    if let Some(slot) = early.get_mut(i) {
                        *slot = own;
                    }
                    continue;
                }
            };
            // Each name of `[a; b] := …` on its own: the definition owns every name it can, and
            // fails with the first that it can't.
            let mut error = None;
            for name in defines {
                let refused = if let Err(e) = check_name(name) {
                    // A name refused as a unit, function or constant (a document from before it
                    // was reserved) is still owned by its first definition: that definition has
                    // failed, so every use of the name is an upstream error rather than quietly
                    // meaning the unit.
                    if matches!(e, CalcError::ReservedName(_)) {
                        owners.entry(name.clone()).or_insert(i);
                    }
                    Some(e)
                } else if let Some(&owner) = owners.get(name) {
                    // A column has no page of its own to name.
                    let page = if owner < regions.len() { ids.get(owner).and_then(|&id| page_of(id)) } else { None };
                    Some(CalcError::Duplicate { name: name.clone(), page })
                } else {
                    owners.insert(name.clone(), i);
                    None
                };
                error = error.or(refused);
            }
            if let Some(slot) = early.get_mut(i) {
                *slot = error.or(own);
            }
        }
        // What breaks a block rule, unless the region has failed already. A `Find` with no block
        // to end is one, unless the document defines `Find` itself: then it is a use of that
        // failed definition, which the graph makes an upstream error.
        let strays = blocks.strays.into_iter().filter(|(_, kind)| !owners.contains_key(kind.name())).map(|(i, kind)| (i, unresolved(kind)));
        for (i, e) in blocks.errors.into_iter().chain(strays) {
            if let Some(slot @ None) = early.get_mut(i) {
                *slot = Some(e);
            }
        }
        // A word that is a keyword now but that a definition tries to use as a name (a document
        // from before it was a keyword): every calculation using it as an operator fails with the
        // word named, rather than quietly changing meaning.
        // Each word once (there are five), so the scan below stays linear in the regions.
        let mut defined_words: Vec<&str> = Vec::new();
        for s in &parsed {
            if let Err(CalcError::Keyword(word)) = s
                && !defined_words.contains(&word.as_str())
            {
                defined_words.push(word);
            }
        }
        if !defined_words.is_empty() {
            for (i, stmt) in parsed.iter().enumerate() {
                let edited = regions.get(i).is_some_and(|r| r.state == CalcState::EditedOutside);
                let word = stmt.as_ref().ok().filter(|_| !edited).and_then(|s| keywords_used(s).into_iter().find(|w| defined_words.contains(w)));
                if let (Some(word), Some(slot)) = (word, early.get_mut(i))
                    && slot.is_none()
                {
                    *slot = Some(CalcError::Keyword(word.to_string()));
                }
            }
        }
        let edited = regions.iter().map(|r| r.state == CalcState::EditedOutside).chain(columns.iter().map(|_| false)).collect();
        let precision = regions.iter().map(|r| r.precision).chain(columns.iter().map(|_| None)).collect();
        let formats = regions.iter().map(|r| r.result_format).collect();
        let mut index =
            Index { parsed, early, owners, edited, precision, formats, names, regions: regions.len(), columns, order, blocks: blocks.blocks };
        // A region's own display unit applies when its source names none.
        for (stmt, r) in index.parsed.iter_mut().zip(regions) {
            if let (Ok(stmt), Some(unit)) = (stmt, r.unit.as_deref().filter(|u| !u.trim().is_empty())) {
                match stmt {
                    Stmt::Eval { show, .. } | Stmt::Define { show: Some(show), .. } | Stmt::DefineMany { show: Some(show), .. }
                        if show.unit.is_none() =>
                    {
                        show.unit = Some(unit.to_string());
                    }
                    _ => {}
                }
            }
        }
        index
    }

    /// Whether definition `i` is a region (rather than a data table's column).
    fn is_region(&self, i: usize) -> bool {
        i < self.regions
    }
}

/// A region's result as shown, when it shows one.
type Outcome = Result<Option<Shown>, CalcError>;

/// A region in the dependency graph. Indexes are positions in the region list; the graph's own
/// lists are read with `get`, so a wrong index is a skipped edge rather than a panic.
#[derive(Default)]
struct Node {
    outcome: Option<Outcome>,
    failed: bool,
    /// Failed because it, or a region it depends on (at any distance), was edited outside.
    edited: bool,
    /// Failed because it is a data table's column in error, or depends on one (at any distance).
    table: bool,
    /// Regions owning a name this one uses (itself, for `x := x + 1`).
    deps: Vec<usize>,
    /// Regions using the name this one owns.
    users: Vec<usize>,
    /// Dependencies not yet evaluated.
    pending: usize,
}

/// The dependency graph of `index`'s definitions, before evaluation: each node's outcome when it
/// failed before running, and its edges.
fn graph(index: &Index) -> Vec<Node> {
    let mut nodes: Vec<Node> = index.early.iter().map(|e| Node { outcome: e.clone().map(Err), ..Node::default() }).collect();
    // Edges run from the definition owning a name to each region using it. Only owned names
    // count: anything else (a unit, constant, built-in, or nothing) is left for the evaluator. An
    // owner that has already failed (a data table's column in error) is waited for by nobody.
    for &i in &index.order {
        let Some(Ok(stmt)) = index.parsed.get(i) else { continue };
        // An edited region uses nothing: it fails on its own, whatever its source says. A block's
        // `Given` and equations use nothing either: their names are its closing region's.
        if nodes.get(i).is_none_or(|n| n.outcome.is_some())
            || index.edited.get(i).copied().unwrap_or(false)
            || matches!(stmt, Stmt::Given | Stmt::Constraint { .. })
        {
            continue;
        }
        let mut names = free_names(stmt);
        if let Some(unit) = show_unit(stmt)
            && !names.iter().any(|name| name == unit.trim())
            && index.owners.contains_key(unit.trim())
            && parse_unit(&unit).is_err()
        {
            names.push(unit.trim().to_owned());
        }
        for name in names {
            if let Some(&owner) = index.owners.get(&name) {
                depend(&mut nodes, i, owner);
            }
        }
    }
    // A block's closing region depends on each region of the block, so a failed or edited one
    // fails it; and on the document's own (failed) definition of `Find` or `Minerr`, if any, which
    // its call means instead.
    for block in &index.blocks {
        let c = block.closing;
        if nodes.get(c).is_none_or(|n| n.outcome.is_some()) || index.edited.get(c).copied().unwrap_or(false) {
            continue;
        }
        for &m in &block.members {
            depend(&mut nodes, c, m);
        }
        if let Some(&owner) = index.owners.get(block.kind.name()) {
            depend(&mut nodes, c, owner);
        }
    }
    nodes
}

/// Match downstream calculation errors with their sole failing upstream region, when one can
/// be established from the same dependencies used for recalculation.
pub fn worksheet_error_roots(doc: &Document) -> HashMap<u64, u64> {
    #[derive(Clone, Copy, PartialEq)]
    enum Cause {
        None,
        One(u64),
        Multiple,
    }
    let found = collect(&doc.body);
    let (ids, _) = assign_ids(&found);
    let settings = doc.calc_settings.clone().unwrap_or_default();
    let tables = read_tables(&doc.body, &found.tables, &settings);
    let index = Index::build(&found.regions, &ids, &tables, &|_| None);
    let graph = graph(&index);
    let upstream = CalcError::Upstream.to_string();
    let downstream: Vec<_> = found.regions.iter().map(|r| matches!(&r.state, CalcState::Error(error) if *error == upstream)).collect();
    let failed: Vec<_> = found.regions.iter().map(|r| matches!(&r.state, CalcState::Error(_))).collect();
    let mut causes: Vec<_> = ids.iter().enumerate().map(|(i, id)| if failed[i] && !downstream[i] { Cause::One(*id) } else { Cause::None }).collect();
    let mut pending = vec![0; ids.len()];
    let mut users = vec![Vec::new(); ids.len()];
    for (i, is_downstream) in downstream.iter().copied().enumerate() {
        if is_downstream {
            for &dependency in &graph[i].deps {
                if failed.get(dependency) == Some(&true) {
                    pending[i] += 1;
                    users[dependency].push(i);
                }
            }
        }
    }
    let mut ready: Vec<_> = pending.iter().enumerate().filter_map(|(i, &left)| (left == 0).then_some(i)).collect();
    while let Some(i) = ready.pop() {
        for &user in &users[i] {
            causes[user] = match (causes[user], causes[i]) {
                (_, Cause::None) => Cause::Multiple,
                (Cause::None, cause) => cause,
                (left, right) if left == right => left,
                _ => Cause::Multiple,
            };
            pending[user] -= 1;
            if pending[user] == 0 {
                ready.push(user);
            }
        }
    }
    ids.into_iter()
        .enumerate()
        .filter_map(|(i, id)| match causes[i] {
            Cause::One(root) if downstream[i] && pending[i] == 0 => Some((id, root)),
            _ => None,
        })
        .collect()
}

/// Make region `i` depend on definition `on`: it waits for it, unless `on` has failed already.
fn depend(nodes: &mut [Node], i: usize, on: usize) {
    let waits = nodes.get(on).is_some_and(|o| o.outcome.is_none());
    if waits && let Some(o) = nodes.get_mut(on) {
        o.users.push(i);
    }
    if let Some(node) = nodes.get_mut(i) {
        node.deps.push(on);
        if waits {
            node.pending += 1;
        }
    }
}

/// Evaluate every region in dependency order: the environment of defined names, each region's
/// outcome, and which regions failed.
///
/// A result is formatted for display as part of its statement: one that can't be shown is an
/// error like any other, so its name is defined by nothing and every use of it is an upstream
/// error (nothing is defined unless the whole statement succeeds).
fn evaluate(index: &Index, settings: &CalcSettings) -> (Env, Vec<Outcome>, Vec<bool>) {
    let mut nodes = graph(index);
    let mut env = Env::new();
    env.set_g_is_gravity(settings.g == wordcraft_doc::calc::GMeans::Gravity);
    env.set_origin(settings.origin);
    env.set_display(unit_system(settings), Precision::new(settings.precision));
    // One budget for the whole pass: a hostile document errors instead of taking seconds.
    let pass = Pass::new(MAX_PASS_STEPS);
    // Record that definition `i` failed: using any name it owns is then an upstream error.
    let fail = |env: &mut Env, node: &mut Node, i: usize| {
        node.failed = true;
        for name in index.names.get(i).into_iter().flatten() {
            if index.owners.get(name) == Some(&i) {
                env.mark_failed(name);
            }
        }
    };
    // A data table's column whose values don't read, but which owns its name, has failed already.
    // So has a region with an error found before running (its name is a unit, function or
    // constant, it uses a word that another definition tries to name, or it breaks a block rule):
    // using a name it owns is an upstream error, and a block it is part of fails.
    for (i, node) in nodes.iter_mut().enumerate() {
        if node.outcome.is_none() {
            continue;
        }
        node.table = !index.is_region(i);
        fail(&mut env, node, i);
    }
    // Kahn's algorithm, in document order among the ready definitions.
    let mut ready: VecDeque<usize> =
        index.order.iter().copied().filter(|&i| nodes.get(i).is_some_and(|n| n.outcome.is_none() && n.pending == 0)).collect();
    while let Some(i) = ready.pop_front() {
        let failed_dep =
            |edited: bool| nodes.get(i).is_some_and(|n| n.deps.iter().any(|&d| nodes.get(d).is_some_and(|dn| dn.failed && (dn.edited || !edited))));
        let (upstream, upstream_edited) = (failed_dep(false), failed_dep(true));
        let upstream_table = nodes.get(i).is_some_and(|n| n.deps.iter().any(|&d| nodes.get(d).is_some_and(|dn| dn.failed && dn.table)));
        let own_edit = index.edited.get(i).copied().unwrap_or(false);
        let outcome = match index.parsed.get(i) {
            _ if own_edit => Err(CalcError::EditedOutside),
            _ if upstream_edited => Err(CalcError::UpstreamEdited),
            _ if upstream_table => Err(CalcError::UpstreamTable),
            _ if upstream => Err(CalcError::Upstream),
            Some(Ok(stmt)) => run_stmt_in(&mut env, stmt, &pass).and_then(|data| {
                let p = precision(index.precision.get(i).copied().flatten(), settings);
                data.map(|d| {
                    let scale = show_unit(stmt).as_deref().map(|unit| env.display_scale(unit)).transpose()?;
                    let style = index.formats.get(i).copied().flatten().or(settings.result_format);
                    let style = style.map(|mut style| {
                        if index.formats.get(i).is_some_and(Option::is_none)
                            && let Some(decimals) = index.precision.get(i).copied().flatten()
                        {
                            style.decimals = decimals;
                        }
                        style
                    });
                    format_data(&d, scale.as_ref(), unit_system(settings), p, style, env.origin())
                })
                .transpose()
            }),
            Some(Err(e)) => Err(e.clone()),
            None => Err(CalcError::Upstream),
        };
        let Some(node) = nodes.get_mut(i) else { continue };
        if outcome.is_err() {
            node.edited = own_edit || upstream_edited;
            node.table = !node.edited && (!index.is_region(i) || upstream_table);
            fail(&mut env, node, i);
        }
        node.outcome = Some(outcome);
        let users = std::mem::take(&mut node.users);
        for &u in &users {
            if let Some(un) = nodes.get_mut(u) {
                un.pending = un.pending.saturating_sub(1);
                if un.pending == 0 {
                    ready.push_back(u);
                }
            }
        }
        if let Some(node) = nodes.get_mut(i) {
            node.users = users;
        }
    }
    // What is left is on a cycle or depends on one.
    let left: Vec<bool> = nodes.iter().map(|n| n.outcome.is_none()).collect();
    if left.contains(&true) {
        let users: Vec<&[usize]> = nodes.iter().map(|n| n.users.as_slice()).collect();
        let sccs = strongly_connected(&users, &left);
        for mut members in sccs {
            let self_loop = || members.first().is_some_and(|&v| nodes.get(v).is_some_and(|n| n.deps.contains(&v)));
            if members.len() < 2 && !self_loop() {
                continue;
            }
            members.sort_unstable();
            // Every name the members define (`[a; b] := …` defines several).
            let mut all = members.iter().flat_map(|&v| index.parsed.get(v).and_then(|s| s.as_ref().ok()).map(ast::defined_names).unwrap_or_default());
            let mut names: Vec<&str> = all.by_ref().take(MAX_CYCLE_NAMES).collect();
            if all.next().is_some() {
                names.push("…");
            }
            let message = names.join(", ");
            for &v in &members {
                if let Some(node) = nodes.get_mut(v) {
                    node.outcome = Some(Err(CalcError::Circular(message.clone())));
                    fail(&mut env, node, v);
                }
            }
        }
        // The rest depend, through other leftovers, on a cycle.
        for (i, node) in nodes.iter_mut().enumerate() {
            if node.outcome.is_none() {
                node.outcome = Some(Err(CalcError::Upstream));
                fail(&mut env, node, i);
            }
        }
    }
    let failed = nodes.iter().map(|n| n.failed).collect();
    let outcomes = nodes.into_iter().map(|n| n.outcome.unwrap_or(Err(CalcError::Upstream))).collect();
    (env, outcomes, failed)
}

/// Tarjan's bookkeeping for one node.
#[derive(Clone, Copy, Default)]
struct Visit {
    index: Option<usize>,
    low: usize,
    on_stack: bool,
}

/// The strongly connected components of the graph `succ` restricted to the nodes in `within`
/// (Tarjan's algorithm with an explicit stack instead of recursion).
fn strongly_connected(succ: &[&[usize]], within: &[bool]) -> Vec<Vec<usize>> {
    let inside = |v: usize| within.get(v).copied().unwrap_or(false);
    let mut visits = vec![Visit::default(); succ.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut counter = 0usize;
    // Number `v` and put it on the stack.
    let mut open = |visits: &mut Vec<Visit>, stack: &mut Vec<usize>, v: usize| {
        if let Some(x) = visits.get_mut(v) {
            *x = Visit { index: Some(counter), low: counter, on_stack: true };
        }
        counter += 1;
        stack.push(v);
    };
    for root in 0..succ.len() {
        if !inside(root) || visits.get(root).is_none_or(|x| x.index.is_some()) {
            continue;
        }
        open(&mut visits, &mut stack, root);
        // Frames of (node, next successor to look at).
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&mut (v, ref mut next)) = calls.last_mut() {
            let edges = succ.get(v).copied().unwrap_or_default();
            if let Some(&w) = edges.get(*next) {
                *next += 1;
                if !inside(w) {
                    continue;
                }
                let seen = visits.get(w).copied().unwrap_or_default();
                match seen.index {
                    None => {
                        open(&mut visits, &mut stack, w);
                        calls.push((w, 0));
                    }
                    Some(wi) if seen.on_stack => {
                        if let Some(x) = visits.get_mut(v) {
                            x.low = x.low.min(wi);
                        }
                    }
                    Some(_) => {}
                }
                continue;
            }
            calls.pop();
            let done = visits.get(v).copied().unwrap_or_default();
            if let Some(&(parent, _)) = calls.last()
                && let Some(x) = visits.get_mut(parent)
            {
                x.low = x.low.min(done.low);
            }
            if done.index == Some(done.low) {
                let mut scc = Vec::new();
                while let Some(w) = stack.pop() {
                    if let Some(x) = visits.get_mut(w) {
                        x.on_stack = false;
                    }
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(scc);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

fn unit_system(settings: &CalcSettings) -> UnitSystem {
    match settings.system {
        calc::UnitSystem::UsCustomary => UnitSystem::UsCustomary,
        calc::UnitSystem::Si => UnitSystem::Si,
    }
}

fn precision(own: Option<u8>, settings: &CalcSettings) -> Precision {
    Precision::new(own.unwrap_or(settings.precision))
}

/// The unit a statement asks its value to be shown in.
fn show_unit(stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::Eval { show, .. } | Stmt::Define { show: Some(show), .. } | Stmt::DefineMany { show: Some(show), .. } => show.unit.clone(),
        Stmt::Define { show: None, .. } | Stmt::DefineMany { show: None, .. } | Stmt::Given | Stmt::Constraint { .. } => None,
    }
}

/// `q` as number and unit text, in `unit` when given, else in the natural unit of `sys`.
/// A result as it is shown: one value or text, or a vector or matrix of its first [`MAX_SHOWN_ROWS`] rows
/// and [`MAX_SHOWN_COLS`] columns (each element in `unit` when given); only those are formatted.
/// An element that can't be formatted is named, counted from `origin`.
fn format_data(
    d: &Data,
    scale: Option<&Scale>,
    sys: UnitSystem,
    p: Precision,
    style: Option<calc::ResultFormat>,
    origin: u8,
) -> Result<Shown, CalcError> {
    let show = |v: Val| crate::number_format::quantity(v.q, scale, sys, p, style);
    match d {
        Data::One(v) => Ok(Shown::One(show(*v)?)),
        Data::Grid(g) => {
            let (rows, cols) = (g.rows.min(MAX_SHOWN_ROWS), g.cols.min(MAX_SHOWN_COLS));
            let shown: Vec<Val> = (0..rows).filter_map(|r| g.items.get(r.saturating_mul(g.cols)..)?.get(..cols)).flatten().copied().collect();
            let (items, unit) = format_elements(&shown, &g.items, scale, show).map_err(|(k, e)| {
                // Element `k` of those shown, as an element of the whole grid.
                let (r, c) = (k.checked_div(cols).unwrap_or(0), k.checked_rem(cols).unwrap_or(0));
                array::element_error(g.rows, g.cols, r.saturating_mul(g.cols).saturating_add(c), origin, e.into())
            })?;
            Ok(Shown::Grid { rows, cols, items, unit, total_rows: g.rows, total_cols: g.cols })
        }
        Data::Text(t) => Ok(Shown::Text(t.clone())),
        Data::MixedGrid(g) => {
            let (rows, cols) = (g.rows.min(MAX_SHOWN_ROWS), g.cols.min(MAX_SHOWN_COLS));
            let items = (0..rows)
                .filter_map(|r| g.items.get(r * g.cols..)?.get(..cols))
                .flatten()
                .map(|cell| match cell {
                    array::Cell::Number(value) => {
                        show(*value).map(|shown| if shown.unit.is_empty() { shown.number } else { format!("{} {}", shown.number, shown.unit) })
                    }
                    array::Cell::Text(text) => Ok(text.clone()),
                    array::Cell::Nested(value) => Ok(value.shape().map_or_else(|| "Value".into(), |(r, c)| format!("{r} × {c}"))),
                })
                .collect::<Result<_, _>>()?;
            Ok(Shown::TextGrid { rows, cols, items, total_rows: g.rows, total_cols: g.cols })
        }
        Data::TextGrid(g) => {
            let (rows, cols) = (g.rows.min(MAX_SHOWN_ROWS), g.cols.min(MAX_SHOWN_COLS));
            let items: Vec<String> = (0..rows).filter_map(|r| g.items.get(r.saturating_mul(g.cols)..)?.get(..cols)).flatten().cloned().collect();
            Ok(Shown::TextGrid { rows, cols, items, total_rows: g.rows, total_cols: g.cols })
        }
    }
}

/// The elements `shown` (the first of `all`) formatted, in `scale` when given, else each in its
/// natural unit. When `scale` is given or every element of `all` has the same dimension, they
/// share one unit: it is returned (unless it is empty, for plain numbers) and the items carry
/// none. An error comes with the element it is about.
fn format_elements(
    shown: &[Val],
    all: &[Val],
    scale: Option<&Scale>,
    show: impl Fn(Val) -> Result<Formatted, UnitError>,
) -> Result<(Vec<Formatted>, Option<String>), (usize, UnitError)> {
    let shared = scale.is_some() || all.windows(2).all(|w| matches!(w, [a, b] if a.q.dim == b.q.dim));
    let mut items = shown.iter().enumerate().map(|(k, v)| show(*v).map_err(|e| (k, e))).collect::<Result<Vec<_>, _>>()?;
    let mut common = None;
    if shared {
        for f in &mut items {
            common = Some(std::mem::take(&mut f.unit));
        }
    }
    Ok((items, common.filter(|u| !u.is_empty())))
}

/// The text a quoted value shows: `"600 psi"`; a vector as a list (`"0.84, 1.05 in"`), a matrix's
/// rows separated by `;` (`"1, 2; 3, 4"`), and past [`MAX_QUOTED_VALUES`] the count of the rest
/// (`"0, 1, …, 49, … (60 values)"`); one element with an index. `"?"` when its name has no valid
/// value, its unit doesn't fit, or its index is outside the value.
fn field_result(f: &FoundField, index: &Index, env: &Env, failed: &[bool], settings: &CalcSettings) -> String {
    let spec = &f.spec;
    let valid = index
        .owners
        .get(&spec.name)
        .is_some_and(|&i| !failed.get(i).copied().unwrap_or(true) && index.parsed.get(i).is_some_and(|s| s.as_ref().is_ok_and(defines_variables)));
    let data = match (env.data(&spec.name), spec.index) {
        _ if !valid => None,
        (Some(Data::Grid(g)), Some(at)) => array::element_at(g, env.origin(), at).map(|v| Cow::Owned(Data::One(v))),
        (Some(Data::TextGrid(g)), Some(at)) => array::element_at(g, env.origin(), at).map(|t| Cow::Owned(Data::Text(t))),
        (Some(Data::MixedGrid(g)), Some(at)) => array::element_at(g, env.origin(), at).map(|cell| Cow::Owned(cell.data())),
        (Some(d), None) => Some(Cow::Borrowed(d)),
        _ => None,
    };
    let Some(data) = data else { return "?".to_string() };
    let unit = spec.unit.as_deref();
    // A difference shown in °C or °F would be a wrong number.
    if unit.is_some_and(|u| check_display(env, u, &data).is_err()) {
        return "?".to_string();
    }
    let Ok(scale) = unit.map(|unit| env.display_scale(unit)).transpose() else { return "?".into() };
    let (sys, p) = (unit_system(settings), precision(spec.precision, settings));
    match &*data {
        Data::One(v) => match format(v.q, scale.as_ref(), sys, p) {
            Ok(shown) if shown.unit.is_empty() => render::text_number(&shown.number),
            Ok(shown) => format!("{} {}", render::text_number(&shown.number), shown.unit),
            Err(_) => "?".to_string(),
        },
        Data::Grid(g) => quoted_list(g, scale.as_ref(), sys, p).unwrap_or_else(|| "?".to_string()),
        // A text is quoted as it is (a unit was refused above).
        Data::Text(t) => t.clone(),
        Data::TextGrid(g) => quoted_texts(g),
        Data::MixedGrid(_) => "?".to_string(),
    }
}

/// A vector or matrix of texts as a quoted list, laid out as [`quoted_list`] does numbers: the
/// first [`MAX_QUOTED_VALUES`] texts row by row, `, ` between them (`; ` between a matrix's rows),
/// then `, … (N values)` when there are more.
fn quoted_texts(g: &array::Grid<String>) -> String {
    let shown = g.items.get(..MAX_QUOTED_VALUES).unwrap_or(&g.items);
    let matrix = g.rows > 1 && g.cols > 1;
    let mut text = String::new();
    for (k, t) in shown.iter().enumerate() {
        if k > 0 {
            text.push_str(if matrix && k.checked_rem(g.cols) == Some(0) { "; " } else { ", " });
        }
        text.push_str(t);
    }
    if g.items.len() > shown.len() {
        text.push_str(&format!(", … ({} values)", g.items.len()));
    }
    text
}

/// A vector or matrix as a quoted list: its first [`MAX_QUOTED_VALUES`] values row by row, `, `
/// between them (`; ` between a matrix's rows), with their shared unit once after them or each its
/// own; then `, … (N values)` when there are more.
fn quoted_list(g: &array::Grid, scale: Option<&Scale>, sys: UnitSystem, p: Precision) -> Option<String> {
    let shown = g.items.get(..MAX_QUOTED_VALUES).unwrap_or(&g.items);
    let (items, common) = format_elements(shown, &g.items, scale, |v| format(v.q, scale, sys, p)).ok()?;
    let matrix = g.rows > 1 && g.cols > 1;
    let mut text = String::new();
    for (k, f) in items.iter().enumerate() {
        if k > 0 {
            text.push_str(if matrix && k.checked_rem(g.cols) == Some(0) { "; " } else { ", " });
        }
        text.push_str(&render::text_number(&f.number));
        if !f.unit.is_empty() {
            text.push(' ');
            text.push_str(&f.unit);
        }
    }
    if let Some(unit) = common {
        text.push(' ');
        text.push_str(&unit);
    }
    if g.items.len() > shown.len() {
        text.push_str(&format!(", … ({} values)", g.items.len()));
    }
    Some(text)
}

// ---------------------------------------------------------------------------------------------
// Writing back
// ---------------------------------------------------------------------------------------------

enum Change {
    Region { id: u64, state: CalcState, display: Option<Rendered> },
    Field { result: String },
}

struct Edit {
    at: At,
    change: Change,
}

/// The paragraph at `path`, if there is one.
fn para<'a>(body: &'a Blocks, path: &[u32]) -> Option<&'a Paragraph> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &Block = body.get(first as usize)?;
    loop {
        match block {
            Block::Para(p) => return rest.is_empty().then_some(p),
            Block::Table(t) => {
                let [r, c, j, tail @ ..] = rest else { return None };
                block = t.rows.get(*r as usize)?.cells.get(*c as usize)?.blocks.get(*j as usize)?;
                rest = tail;
            }
        }
    }
}

/// The paragraph at `path`, to change: only the blocks on the way to it are copied when shared.
pub(crate) fn para_mut<'a>(body: &'a mut Blocks, path: &[u32]) -> Option<&'a mut Paragraph> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &mut Block = Arc::make_mut(body.get_mut(first as usize)?);
    loop {
        match block {
            Block::Para(p) => return rest.is_empty().then_some(p),
            Block::Table(t) => {
                let [r, c, j, tail @ ..] = rest else { return None };
                block = Arc::make_mut(t.rows.get_mut(*r as usize)?.cells.get_mut(*c as usize)?.blocks.get_mut(*j as usize)?);
                rest = tail;
            }
        }
    }
}

fn object<'a>(body: &'a Blocks, at: &At) -> Option<&'a InlineObject> {
    para(body, &at.path)?.objects.get(at.obj)
}

/// Whether applying `change` to `o` would change it.
fn differs(o: &InlineObject, change: &Change) -> bool {
    match (o, change) {
        (InlineObject::Equation { linear, math, calc: Some(c), .. }, Change::Region { id, state, display }) => {
            c.id != *id || c.state != *state || display.as_ref().is_some_and(|d| *linear != d.linear || math.nodes != d.math || !math.omml.is_empty())
        }
        (InlineObject::Field { result: old, dirty, .. }, Change::Field { result }) => old != result || *dirty,
        _ => false,
    }
}

fn change_object(o: &mut InlineObject, change: Change) {
    match (o, change) {
        (InlineObject::Equation { linear, math, calc: Some(c), .. }, Change::Region { id, state, display }) => {
            c.id = id;
            c.state = state;
            if let Some(d) = display {
                *linear = d.linear;
                set_nodes(math, d.math);
            }
        }
        (InlineObject::Field { result: old, dirty, .. }, Change::Field { result }) => {
            *old = result;
            *dirty = false;
        }
        _ => {}
    }
}

/// New structure for an equation: the source markup it was read with no longer matches it.
fn set_nodes(math: &mut wordcraft_doc::math::Math, nodes: Arg) {
    math.nodes = nodes;
    math.omml.clear();
}

/// The table at `path`, if there is one.
pub(crate) fn table_at<'a>(body: &'a Blocks, path: &[u32]) -> Option<&'a Table> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &Block = body.get(first as usize)?;
    loop {
        let Block::Table(t) = block else { return None };
        let [r, c, j, tail @ ..] = rest else { return Some(t) };
        block = t.rows.get(*r as usize)?.cells.get(*c as usize)?.blocks.get(*j as usize)?;
        rest = tail;
    }
}

/// The table at `path`, to change: only the blocks on the way to it are copied when shared.
pub(crate) fn table_mut<'a>(body: &'a mut Blocks, path: &[u32]) -> Option<&'a mut Table> {
    let (&first, mut rest) = path.split_first()?;
    let mut block: &mut Block = Arc::make_mut(body.get_mut(first as usize)?);
    loop {
        let Block::Table(t) = block else { return None };
        let [r, c, j, tail @ ..] = rest else { return Some(t) };
        block = Arc::make_mut(t.rows.get_mut(*r as usize)?.cells.get_mut(*c as usize)?.blocks.get_mut(*j as usize)?);
        rest = tail;
    }
}

/// The data tables whose id or errors change, with what they become: the error for the whole
/// table first, then each column's in order. A column left undefined by the whole table's error
/// adds nothing of its own.
fn table_changes(found: &[FoundTable], ids: &[u64], tables: &[TableDefs], index: &Index, outcomes: &[Outcome]) -> Vec<(Vec<u32>, CalcTable)> {
    let mut errors: Vec<Vec<ColumnError>> =
        tables.iter().map(|t| t.error.iter().map(|e| ColumnError { col: None, message: e.to_string() }).collect()).collect();
    for (k, &(t, col)) in index.columns.iter().enumerate() {
        if let Some(Err(e)) = outcomes.get(index.regions.saturating_add(k))
            && *e != CalcError::UpstreamTable
            && let Some(list) = errors.get_mut(t)
        {
            list.push(ColumnError { col: Some(col), message: e.to_string() });
        }
    }
    found
        .iter()
        .zip(errors)
        .enumerate()
        .filter_map(|(k, (ft, errors))| {
            let id = ids.get(k).copied().unwrap_or(ft.id);
            (id != ft.id || errors != ft.errors).then(|| (ft.path.clone(), CalcTable { id, errors }))
        })
        .collect()
}

/// Write each data table's new id and errors (from [`table_changes`]), copying only the blocks on
/// the way to it. The table's text is never touched.
fn apply_tables(body: &mut Blocks, changes: Vec<(Vec<u32>, CalcTable)>) {
    for (path, data) in changes {
        if let Some(t) = table_mut(body, &path)
            && t.calc.is_some()
        {
            t.calc = Some(Box::new(data));
        }
    }
}

/// Apply the edits that change something, copying only the paragraphs they touch.
fn apply(body: &mut Blocks, edits: Vec<Edit>) {
    let mut edits: Vec<Edit> = edits.into_iter().filter(|e| object(body, &e.at).is_some_and(|o| differs(o, &e.change))).collect();
    // Bring each paragraph's edits (its regions' and its fields') together, so it is copied and
    // touched once. The sort is stable: within a paragraph, edits keep their order.
    edits.sort_by(|a, b| a.at.path.cmp(&b.at.path));
    let mut rest = edits.drain(..).peekable();
    while let Some(first) = rest.next() {
        let path = first.at.path.clone();
        let mut group = vec![first];
        while let Some(e) = rest.next_if(|e| e.at.path == path) {
            group.push(e);
        }
        let Some(p) = para_mut(body, &path) else { continue };
        for e in group {
            if let Some(o) = p.objects.get_mut(e.at.obj) {
                change_object(o, e.change);
            }
        }
        p.touch();
    }
}
