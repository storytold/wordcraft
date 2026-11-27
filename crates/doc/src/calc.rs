//! Data for inline calculations: the payload an equation carries when it is a calculation
//! region, the document's calculation settings, and the `WCCALC` field code of a quoted value.
//!
//! Plain data only. Evaluating a region is `wordcraft-calc`'s job; this crate depends on neither
//! it nor the units crate, so [`UnitSystem`] is a local copy that the evaluator maps across.

use serde::{Deserialize, Serialize};

/// A calculation region: what an [`crate::para::InlineObject::Equation`] carries when it is a
/// calculation rather than typeset math. The equation's `linear` and `math` are the displayed
/// result (rewritten by the recalculation pass); `source` is what the user typed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalcRegion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    /// Source evaluation order for a worksheet whose regions were rearranged for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worksheet_order: Option<u64>,

    /// Stable within the document; names the region in diagnostics and results.
    pub id: u64,
    /// The calculation as written (`σ_max := P / A`).
    pub source: String,
    /// Unit the result is shown in (`ksi`); `None` = whatever the value comes out in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Significant decimals for this region; `None` = the document's [`CalcSettings::precision`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    /// Shown with an automatic equation number, like a numbered plain equation (`#`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub numbered: bool,
    #[serde(default)]
    pub state: CalcState,
}

impl CalcRegion {
    /// Bytes this region owns beyond its own size: its source, unit and error message, all of
    /// which come from files. What the paste and clone cost guards count.
    pub fn heap_bytes(&self) -> usize {
        let error = if let CalcState::Error(message) = &self.state { message.len() } else { 0 };
        std::mem::size_of::<CalcRegion>()
            .saturating_add(self.source.len())
            .saturating_add(self.unit.as_ref().map_or(0, String::len))
            .saturating_add(error)
    }
}

/// A document table marked as calculation data: each column defines a global column vector named
/// by its heading (`D_o (in)`), of the values in the rows below it. What a table carries when it is
/// marked; the recalculation pass rewrites `errors`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CalcTable {
    /// Stable within the document, in an id space of its own (separate from regions' ids).
    pub id: u64,
    /// What is wrong with the table's data as of the last recalculation; empty when every column
    /// reads. Never written into the table's text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ColumnError>,
}

/// One error of a [`CalcTable`]: about the column `col` (counted from 0), or the whole table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnError {
    /// The column, counted from 0; `None` for an error about the whole table.
    pub col: Option<u32>,
    /// What is wrong, written for the author (`D_o row 2 isn't a number`).
    pub message: String,
}

/// Where a [`CalcRegion`] stands with respect to its displayed result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CalcState {
    /// The displayed result matches `source`.
    #[default]
    Fresh,
    /// An input changed (manual calculation mode): the displayed result is out of date.
    Stale,
    /// Evaluating `source` failed; the message says why.
    Error(String),
    /// The equation's text was edited by something other than the calculator (another editor,
    /// a round trip through a format that doesn't carry the payload): the result can't be trusted.
    EditedOutside,
}

/// The unit system calculations show results in. Local to this crate; `wordcraft-calc` maps it
/// to the units crate's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitSystem {
    #[default]
    UsCustomary,
    Si,
}

/// What `g` means in a document's calculations. Mathcad's convention is the default: `g` is
/// standard gravity and the gram is `gm`. The SI convention makes `g` the gram (gravity is then
/// `g_0`). It belongs to the document, so a report calculates the same on every computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GMeans {
    #[default]
    Gravity,
    Gram,
}

/// Numeric presentation of a result, independent of its calculation and display unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberStyle {
    #[default]
    General,
    Decimal,
    Scientific,
    Engineering,
    Fraction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResultFormat {
    pub style: NumberStyle,
    pub decimals: u8,
    pub trailing_zeros: bool,
    pub engineering_exponents: bool,
    pub exponential_threshold: u8,
    pub e_notation: bool,
    pub accuracy: u8,
    pub mixed_numbers: bool,
}

impl Default for ResultFormat {
    fn default() -> Self {
        Self {
            style: NumberStyle::General,
            decimals: 3,
            trailing_zeros: false,
            engineering_exponents: false,
            exponential_threshold: 3,
            e_notation: false,
            accuracy: 12,
            mixed_numbers: false,
        }
    }
}

/// Document-wide calculation settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalcSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    pub system: UnitSystem,
    /// Decimals shown when a region sets none. Consumers clamp it to what they can display.
    pub precision: u8,
    /// Recalculate only on request (regions go [`CalcState::Stale`] instead).
    pub manual: bool,
    /// What `g` stands for (see [`GMeans`]).
    pub g: GMeans,
    /// What an array's first element is numbered: 0 (Mathcad's convention) or 1. The evaluator
    /// reads any other value as 0. Not written when 0, so a document without arrays saves as it
    /// did before the setting existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub origin: u8,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

impl Default for CalcSettings {
    fn default() -> Self {
        CalcSettings { result_format: None, system: UnitSystem::UsCustomary, precision: 3, manual: false, g: GMeans::Gravity, origin: 0 }
    }
}

/// The tag prefix of the content control a calculation is saved in (`.docx`): the region's id
/// follows (`wordcraft-calc:7`). WordCraft's own bookkeeping, never shown as a form field.
pub const CALC_TAG_PREFIX: &str = "wordcraft-calc:";

/// The tag prefix of the content control a calculation data table is saved in (`.docx`): the
/// table's id follows (`wordcraft-calc-table:2`). Table ids are an id space of their own, so the
/// prefix differs from [`CALC_TAG_PREFIX`] (neither starts with the other).
pub const CALC_TABLE_TAG_PREFIX: &str = "wordcraft-calc-table:";

/// Most entries (regions and tables together) a [`CalcStore`] read from a file may hold; a larger
/// one is ignored.
pub const MAX_STORED_REGIONS: usize = 100_000;

/// Largest serialized [`CalcStore`] read from a file, bytes; a larger one is ignored.
pub const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;

/// What a saved file (`.docx`) keeps of its calculations beside the ordinary equations and tables
/// Word shows: the document's settings, each region's source, keyed by the id in its content
/// control's tag ([`CALC_TAG_PREFIX`]), and each calculation data table's id and fingerprint
/// ([`CALC_TABLE_TAG_PREFIX`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalcStore {
    pub settings: CalcSettings,
    pub regions: Vec<StoredRegion>,
    /// Not written when empty, so a document without data tables saves as it did before tables
    /// could be data (and a store without it is still valid).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<StoredTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plots: Vec<StoredPlot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredPlot {
    pub id: u64,
    pub plot: crate::chart::PlotSpec,
}

impl CalcStore {
    /// Nothing worth saving: no regions, no tables and the default settings.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.tables.is_empty() && self.plots.is_empty() && self.settings == CalcSettings::default()
    }
}

/// One calculation data table of a [`CalcStore`]. A table's values are its cells' text, which Word
/// keeps, so nothing else is stored: the table comes back as data whatever its cells hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTable {
    pub id: u64,
    /// Hash of the table's cell text as saved: when the table read back hashes differently, its
    /// data was edited outside WordCraft (it is used all the same: cells are inputs).
    pub fingerprint: u64,
}

/// One region of a [`CalcStore`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredRegion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    /// Source evaluation order for a worksheet whose regions were rearranged for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worksheet_order: Option<u64>,

    pub id: u64,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub numbered: bool,
    /// Hash of the equation as saved: when the equation read back hashes differently, it was
    /// edited outside WordCraft.
    pub fingerprint: u64,
}

/// The field type of a quoted value: a locked [`crate::para::InlineObject::Field`] whose code is
/// built by [`calc_field_instr`] and whose result is the value as it was quoted.
pub const CALC_FIELD: &str = "WCCALC";

/// Longest field code [`parse_calc_field`] accepts. Field codes come from files; a calculation
/// name and unit are short.
pub const MAX_INSTR_BYTES: usize = 1024;

/// Highest precision switch [`parse_calc_field`] accepts.
const MAX_PRECISION: u8 = 15;

/// What a quoted value (`WCCALC` field) shows: the variable, the unit and precision it is shown
/// with (`None` = the variable's own and the document's), and for a vector or matrix, the one
/// element to show (an index, or a row and a column, counted from the document's origin; `None` =
/// the whole list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuoteSpec {
    pub name: String,
    pub unit: Option<String>,
    pub precision: Option<u8>,
    pub index: Option<(u32, Option<u32>)>,
}

/// The field code quoting `spec`: `WCCALC σ \u ksi \p 2`, or `WCCALC D \i 2` and `WCCALC M \i 1,2`
/// for an element, each switch left out when `None`. A unit with spaces is double-quoted
/// (`\u "kN m"`). The name is an identifier (no whitespace); a `"` in a unit, which no unit has, is
/// dropped, and an empty unit counts as none.
pub fn calc_field_instr(spec: &QuoteSpec) -> String {
    let mut instr = format!("{CALC_FIELD} {}", spec.name);
    if let Some(unit) = &spec.unit {
        let unit: String = unit.chars().filter(|&c| c != '"').collect();
        if !unit.trim().is_empty() {
            if unit.chars().any(char::is_whitespace) {
                instr.push_str(&format!(" \\u \"{unit}\""));
            } else {
                instr.push_str(&format!(" \\u {unit}"));
            }
        }
    }
    if let Some(precision) = spec.precision {
        instr.push_str(&format!(" \\p {precision}"));
    }
    match spec.index {
        Some((i, None)) => instr.push_str(&format!(" \\i {i}")),
        Some((r, Some(c))) => instr.push_str(&format!(" \\i {r},{c}")),
        None => {}
    }
    instr
}

/// Splits a field code into words; a word starting with `"` runs to the closing `"` and may hold
/// spaces. `None` for an unterminated quote or a closing quote followed by more word.
fn words(instr: &str) -> Option<Vec<(String, bool)>> {
    let mut out = Vec::new();
    let mut chars = instr.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { return Some(out) };
        let mut word = String::new();
        if first == '"' {
            chars.next();
            loop {
                match chars.next()? {
                    '"' => break,
                    c => word.push(c),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return None;
            }
            out.push((word, true));
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                word.push(c);
            }
            out.push((word, false));
        }
    }
}

/// A whole number written with digits only: `u32::from_str` takes a leading `+`; a field code doesn't.
fn digits<T: std::str::FromStr>(text: &str) -> Option<T> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Reads a [`calc_field_instr`] code back. `None` if it isn't a `WCCALC` code, is over
/// [`MAX_INSTR_BYTES`], has no name, repeats or doesn't know a switch, gives a precision over 15,
/// or an index that isn't one whole number or two separated by a comma.
pub fn parse_calc_field(instr: &str) -> Option<QuoteSpec> {
    if instr.len() > MAX_INSTR_BYTES {
        return None;
    }
    let mut words = words(instr)?.into_iter();
    let (kind, quoted) = words.next()?;
    if quoted || !kind.eq_ignore_ascii_case(CALC_FIELD) {
        return None;
    }
    let (name, quoted) = words.next()?;
    if quoted || name.is_empty() || name.starts_with('\\') {
        return None;
    }
    let mut spec = QuoteSpec { name, unit: None, precision: None, index: None };
    while let Some((switch, quoted)) = words.next() {
        if quoted {
            return None;
        }
        let (value, _) = words.next()?;
        match switch.as_str() {
            "\\u" if spec.unit.is_none() && !value.trim().is_empty() && !value.starts_with('\\') => spec.unit = Some(value),
            "\\p" if spec.precision.is_none() => spec.precision = Some(digits::<u8>(&value).filter(|&p| p <= MAX_PRECISION)?),
            "\\i" if spec.index.is_none() => {
                spec.index = Some(match value.split_once(',') {
                    Some((r, c)) => (digits(r)?, Some(digits(c)?)),
                    None => (digits(&value)?, None),
                });
            }
            _ => return None,
        }
    }
    Some(spec)
}

#[cfg(test)]
mod tests;
