use super::merge::refresh;
use super::recipients::cell;
use super::*;

/// A Filter Recipients comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOp {
    Equal,
    NotEqual,
    Less,
    Greater,
    LessOrEqual,
    GreaterOrEqual,
    Blank,
    NotBlank,
    Contains,
    NotContains,
}

impl FilterOp {
    /// Every comparison, in the order the dialog lists them.
    pub const ALL: [FilterOp; 10] = [
        FilterOp::Equal,
        FilterOp::NotEqual,
        FilterOp::Less,
        FilterOp::Greater,
        FilterOp::LessOrEqual,
        FilterOp::GreaterOrEqual,
        FilterOp::Blank,
        FilterOp::NotBlank,
        FilterOp::Contains,
        FilterOp::NotContains,
    ];

    /// The id used in command params.
    pub fn id(self) -> &'static str {
        match self {
            FilterOp::Equal => "equal",
            FilterOp::NotEqual => "notEqual",
            FilterOp::Less => "less",
            FilterOp::Greater => "greater",
            FilterOp::LessOrEqual => "lessOrEqual",
            FilterOp::GreaterOrEqual => "greaterOrEqual",
            FilterOp::Blank => "blank",
            FilterOp::NotBlank => "notBlank",
            FilterOp::Contains => "contains",
            FilterOp::NotContains => "notContains",
        }
    }

    /// The dialog's label (English source text).
    pub fn label(self) -> &'static str {
        match self {
            FilterOp::Equal => "Equal to",
            FilterOp::NotEqual => "Not equal to",
            FilterOp::Less => "Less than",
            FilterOp::Greater => "Greater than",
            FilterOp::LessOrEqual => "Less than or equal",
            FilterOp::GreaterOrEqual => "Greater than or equal",
            FilterOp::Blank => "Is blank",
            FilterOp::NotBlank => "Is not blank",
            FilterOp::Contains => "Contains",
            FilterOp::NotContains => "Does not contain",
        }
    }

    /// An id, a symbol (`=`, `<>`, `<`, `>`, `<=`, `>=`) or a spelled-out alias, any case.
    pub fn parse(t: &str) -> Option<FilterOp> {
        let k: String = t.trim().chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect::<String>().to_ascii_lowercase();
        Some(match k.as_str() {
            "equal" | "equals" | "equalto" | "=" | "==" | "eq" => FilterOp::Equal,
            "notequal" | "notequalto" | "<>" | "!=" | "ne" => FilterOp::NotEqual,
            "less" | "lessthan" | "<" | "lt" => FilterOp::Less,
            "greater" | "greaterthan" | ">" | "gt" => FilterOp::Greater,
            "lessorequal" | "lessthanorequal" | "<=" | "le" => FilterOp::LessOrEqual,
            "greaterorequal" | "greaterthanorequal" | ">=" | "ge" => FilterOp::GreaterOrEqual,
            "blank" | "isblank" | "empty" => FilterOp::Blank,
            "notblank" | "isnotblank" | "notempty" => FilterOp::NotBlank,
            "contains" => FilterOp::Contains,
            "notcontains" | "doesnotcontain" => FilterOp::NotContains,
            _ => return None,
        })
    }

    /// Whether the comparison takes a value (Is blank / Is not blank don't).
    pub fn has_value(self) -> bool {
        !matches!(self, FilterOp::Blank | FilterOp::NotBlank)
    }
}

/// One Filter Recipients row: `field op value`, joined to the row before by And (`or` false) or
/// Or. And binds tighter than Or: `a And b Or c` keeps records matching both a and b, or c.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterRule {
    pub field: String,
    pub op: FilterOp,
    pub value: String,
    pub or: bool,
}

/// One Sort Recipients key.
#[derive(Clone, Debug, PartialEq)]
pub struct SortKey {
    pub field: String,
    pub descending: bool,
}

/// Most filter rows and sort keys a list keeps.
const MAX_RULES: usize = 64;

const MAX_SORT_KEYS: usize = 8;

/// Two cell values compared: as numbers when both are numbers, else as text ignoring case.
/// Orders two cells: numbers (compared numerically) before text (compared
/// case-insensitively). This is a total order, so `sort_by` stays consistent
/// (and can't panic) on columns that mix numbers and text, such as `9`, `10`, `1A`.
pub(super) fn compare_cells(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Greater, Less};
    let number = |s: &str| s.parse::<f64>().ok().filter(|x| x.is_finite());
    let (a, b) = (a.trim(), b.trim());
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Less,
        (None, Some(_)) => Greater,
        (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn rule_holds(r: &FilterRule, cell: &str) -> bool {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let ord = || compare_cells(cell, &r.value);
    let lower = || (cell.to_lowercase(), r.value.trim().to_lowercase());
    match r.op {
        FilterOp::Equal => ord() == Equal,
        FilterOp::NotEqual => ord() != Equal,
        FilterOp::Less => ord() == Less,
        FilterOp::Greater => ord() == Greater,
        FilterOp::LessOrEqual => ord() != Greater,
        FilterOp::GreaterOrEqual => ord() != Less,
        FilterOp::Blank => cell.trim().is_empty(),
        FilterOp::NotBlank => !cell.trim().is_empty(),
        FilterOp::Contains => {
            let (c, v) = lower();
            c.contains(&v)
        }
        FilterOp::NotContains => {
            let (c, v) = lower();
            !c.contains(&v)
        }
    }
}

/// The records (indices into `rows`) that `filter` keeps, in `sort` order (stable: ties keep the
/// list's order). Rules and keys naming a field that isn't in `headers` are ignored.
pub fn select_records(headers: &[String], rows: &[Vec<String>], filter: &[FilterRule], sort: &[SortKey]) -> Vec<usize> {
    let col = |f: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(f.trim()));
    let rules: Vec<(Option<usize>, &FilterRule)> = filter.iter().map(|r| (col(&r.field), r)).filter(|(c, _)| c.is_some()).collect();
    let keep = |row: &Vec<String>| {
        if rules.is_empty() {
            return true;
        }
        // Or-groups of And-ed rules.
        let mut any = false;
        let mut all = true;
        for (k, (c, r)) in rules.iter().enumerate() {
            if k > 0 && r.or {
                any |= all;
                all = true;
            }
            let cell = c.and_then(|c| row.get(c)).map(String::as_str).unwrap_or("");
            all = all && rule_holds(r, cell);
        }
        any || all
    };
    let mut out: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| keep(r)).map(|(i, _)| i).collect();
    let keys: Vec<(usize, bool)> = sort.iter().filter_map(|k| col(&k.field).map(|c| (c, k.descending))).collect();
    if !keys.is_empty() {
        out.sort_by(|a, b| {
            for (c, desc) in &keys {
                let get = |i: &usize| rows.get(*i).and_then(|r| r.get(*c)).map(String::as_str).unwrap_or("");
                let o = compare_cells(get(a), get(b));
                let o = if *desc { o.reverse() } else { o };
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    out
}

/// Recompute the merged records from the list, its filter and its sort; back to the first.
pub(super) fn apply_filter(s: &mut Session) {
    let m = &mut s.merge;
    let headers = &m.headers;
    m.filter.retain(|r| headers.iter().any(|h| h.eq_ignore_ascii_case(r.field.trim())));
    m.sort.retain(|k| headers.iter().any(|h| h.eq_ignore_ascii_case(k.field.trim())));
    m.rows = select_records(&m.headers, &m.source, &m.filter, &m.sort).into_iter().filter_map(|i| m.source.get(i).cloned()).collect();
    m.record = 0;
    if m.rows.is_empty() {
        m.preview = false;
    }
    refresh_shown(s);
}

/// [`refresh`] for the non-mutating recipient-list commands: in a protected document the merge
/// fields' shown results stay as they are, since these commands skip the protection checks and
/// undo (Preview Results and the record buttons, which mutate, show the new list's values).
pub(super) fn refresh_shown(s: &mut Session) {
    if !crate::cmd::protect::enforced(&s.doc) {
        refresh(s);
    }
}

/// The canonical field name for `f` (any case), or a params error.
fn known_field(headers: &[String], f: &str) -> Result<String, CmdError> {
    headers
        .iter()
        .find(|h| h.eq_ignore_ascii_case(f.trim()))
        .cloned()
        .ok_or_else(|| CmdError::Params(format!("no field named `{f}` in the recipient list")))
}

pub(super) fn parse_rules(headers: &[String], v: Option<&Value>) -> Result<Vec<FilterRule>, CmdError> {
    let arr = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(CmdError::Params("`rules` must be an array of {field, op, value, join}".into())),
    };
    if arr.len() > MAX_RULES {
        return Err(CmdError::Params(format!("at most {MAX_RULES} filter rules")));
    }
    let mut out = Vec::new();
    for r in arr {
        let field = p::str(r, "field").ok_or_else(|| CmdError::Params("each rule needs a `field`".into()))?;
        let op = p::str(r, "op").or_else(|| p::str(r, "comparison")).unwrap_or("equal");
        let op = FilterOp::parse(op).ok_or_else(|| {
            CmdError::Params(format!("unknown `op` `{op}`; one of {}", FilterOp::ALL.iter().map(|o| o.id()).collect::<Vec<_>>().join(", ")))
        })?;
        let or = match p::str(r, "join").map(|j| j.trim().to_ascii_lowercase()) {
            None => false,
            Some(j) if j == "and" => false,
            Some(j) if j == "or" => true,
            Some(_) => return Err(CmdError::Params("`join` must be \"and\" or \"or\"".into())),
        };
        let value = r.get("value").map(cell).unwrap_or_default();
        out.push(FilterRule { field: known_field(headers, field)?, op, value, or });
    }
    Ok(out)
}

pub(super) fn parse_sort(headers: &[String], v: Option<&Value>) -> Result<Vec<SortKey>, CmdError> {
    let arr: Vec<Value> = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.clone(),
        Some(o @ (Value::Object(_) | Value::String(_))) => vec![o.clone()],
        Some(_) => return Err(CmdError::Params("`sort` must be an array of {field, descending}".into())),
    };
    if arr.len() > MAX_SORT_KEYS {
        return Err(CmdError::Params(format!("at most {MAX_SORT_KEYS} sort keys")));
    }
    arr.iter()
        .map(|k| {
            let (field, descending) = match k {
                Value::String(f) => (f.as_str(), false),
                _ => (
                    p::str(k, "field").ok_or_else(|| CmdError::Params("each sort key needs a `field`".into()))?,
                    p::bool(k, "descending").unwrap_or(false),
                ),
            };
            Ok(SortKey { field: known_field(headers, field)?, descending })
        })
        .collect()
}

pub(super) fn rules_json(m: &MergeState) -> Value {
    json!(
        m.filter
            .iter()
            .map(|r| json!({"field": r.field, "op": r.op.id(), "value": r.value, "join": if r.or { "or" } else { "and" }}))
            .collect::<Vec<_>>()
    )
}

pub(super) fn sort_json(m: &MergeState) -> Value {
    json!(m.sort.iter().map(|k| json!({"field": k.field, "descending": k.descending})).collect::<Vec<_>>())
}

pub(super) fn filter_result(s: &Session) -> CmdResult {
    Ok(json!({"records": s.merge.rows.len(), "of": s.merge.source.len(), "rules": rules_json(&s.merge), "sort": sort_json(&s.merge)}))
}
