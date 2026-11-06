//! Compiled, bounded canvas mappings. Device-to-recorded channel mappings are metadata, not
//! transforms to apply a second time to samples that are already recorded.

use super::{El, MAX_CHANNELS, MAX_DEPTH, bounded, format::Format, number, spend, trace::Sample, trace::Tokens};
use std::collections::HashMap;

mod spline;

/// A mapping compiled to channel indices, so sample evaluation never resolves XML references.
#[derive(Clone)]
pub(super) enum Mapping {
    /// Direct copies into named destination channels.
    Identity(Vec<(usize, usize)>),
    /// A row per destination: source coefficients followed by a constant.
    Affine { sources: Vec<usize>, targets: Vec<usize>, rows: Vec<Vec<f64>> },
    /// Independent maps of the same input; overlapping output channels are invalid.
    Product(Vec<Mapping>),
    /// One scalar Content MathML expression assigned to a destination.
    Formula { target: usize, expression: Expr },
    /// A rectangular lookup grid with source/target column bindings.
    Table(Table),
}

impl Mapping {
    /// Compile a local mapping, resolving references with a hard depth and shared work bound.
    pub fn read(el: &El, source: &Format, target: &Format, ids: &HashMap<&str, &El>, work: &mut usize, depth: usize) -> Option<Self> {
        super::format::charge(el, work, 0)?;
        if depth > MAX_DEPTH {
            return None;
        }
        if let Some(reference) = el.attr("mappingRef") {
            let el = *ids.get(reference.strip_prefix('#')?)?;
            if el.name != "ink:mapping" {
                return None;
            }
            return Self::read(el, source, target, ids, work, depth + 1);
        }
        let map = match el.attr("type").unwrap_or("unknown") {
            "identity" => Self::Identity(target.channels.iter().enumerate().filter_map(|(j, c)| source.index(&c.name).map(|i| (i, j))).collect()),
            "product" => {
                let mut maps = Vec::new();
                for child in el.els() {
                    if child.name != "ink:mapping" || maps.len() >= MAX_CHANNELS {
                        return None;
                    }
                    maps.push(Self::read(child, source, target, ids, work, depth + 1)?);
                }
                Self::Product(maps)
            }
            "affine" => {
                let rows = rows(el.child("ink:affine")?, work)?;
                let n = rows.first()?.len().checked_sub(1)?;
                if n == 0 || n > MAX_CHANNELS || rows.len() > MAX_CHANNELS {
                    return None;
                }
                let sources = bindings(el, "source", source, n)?;
                let targets = if el.els().any(|b| b.name == "ink:bind" && b.attr("target").is_some()) {
                    bindings(el, "target", target, rows.len())?
                } else if el.els().any(|b| b.name == "ink:bind") && rows.len() == sources.len() {
                    sources.iter().map(|i| target.index(&source.channels.get(*i)?.name)).collect::<Option<Vec<_>>>()?
                } else {
                    (rows.len() <= target.regular).then(|| (0..rows.len()).collect())?
                };
                Self::Affine { sources, targets, rows }
            }
            "mathml" => {
                let mut vars = HashMap::new();
                let mut destination = None;
                for b in el.children("ink:bind") {
                    if let Some(name) = b.attr("source") {
                        vars.insert(b.attr("variable")?, source.index(name)?);
                    }
                    if let Some(name) = b.attr("target") {
                        if destination.replace(target.index(name)?).is_some() {
                            return None;
                        }
                        if let Some(var) = b.attr("variable") {
                            vars.insert(var, source.index(name)?);
                        }
                    }
                }
                let math = el.child("mml:math")?;
                let mut children = math.els();
                let expression = Expr::read(children.next()?, &vars, work, 0)?;
                if children.next().is_some() {
                    return None;
                }
                Self::Formula { target: destination?, expression }
            }
            "table" => Self::Table(Table::read(el, source, target, work)?),
            _ => return None,
        };
        // Product maps cannot silently overwrite a sibling map's output.
        let mut assigned = Vec::new();
        map.targets(&mut assigned);
        assigned.sort_unstable();
        if assigned.is_empty() || assigned.windows(2).any(|p| p.first() == p.get(1)) {
            return None;
        }
        Some(map)
    }

    /// Collect assigned channels for collision checks, with total size limited at compilation.
    fn targets(&self, out: &mut Vec<usize>) {
        match self {
            Self::Identity(pairs) => out.extend(pairs.iter().map(|p| p.1)),
            Self::Affine { targets, .. } => out.extend(targets),
            Self::Product(maps) => {
                for map in maps {
                    map.targets(out);
                }
            }
            Self::Formula { target, .. } => out.push(*target),
            Self::Table(table) => out.extend(table.targets.iter().map(|p| p.0)),
        }
    }

    /// Evaluate into a fresh canvas sample; every arithmetic operation consumes shared work.
    pub fn apply(&self, input: &Sample, output: &mut Sample, work: &mut usize) -> Option<()> {
        spend(work, 1)?;
        let put = |output: &mut Sample, i: usize, v: f64| -> Option<()> {
            if !bounded(v) {
                return None;
            }
            *output.get_mut(i)? = Some(v);
            Some(())
        };
        match self {
            Self::Identity(pairs) => {
                for &(src, dst) in pairs {
                    spend(work, 1)?;
                    *output.get_mut(dst)? = *input.get(src)?;
                }
            }
            Self::Affine { sources, targets, rows } => {
                for (row, &target) in rows.iter().zip(targets) {
                    spend(work, sources.len() + 1)?;
                    let mut value = *row.last()?;
                    for (&i, &coefficient) in sources.iter().zip(row) {
                        // A zero coefficient need not read an explicitly unknown channel.
                        if coefficient != 0.0 {
                            value += coefficient * input.get(i).copied().flatten()?;
                        }
                    }
                    put(output, target, value)?;
                }
            }
            Self::Product(maps) => {
                for map in maps {
                    map.apply(input, output, work)?;
                }
            }
            Self::Formula { target, expression } => put(output, *target, expression.eval(input, work)?)?,
            Self::Table(table) => table.apply(input, output, work)?,
        }
        Some(())
    }
}

/// Bind matrix rows or columns, with one-based indices and no missing/duplicate assignments.
fn bindings(el: &El, attr: &str, format: &Format, size: usize) -> Option<Vec<usize>> {
    let mut out = vec![None; size];
    let mut any = false;
    for b in el.children("ink:bind") {
        let Some(name) = b.attr(attr) else {
            continue;
        };
        any = true;
        let i = b.attr("column")?.parse::<usize>().ok()?.checked_sub(1)?;
        if out.get_mut(i)?.replace(format.index(name)?).is_some() {
            return None;
        }
    }
    if !any {
        return (size <= format.regular).then(|| (0..size).collect());
    }
    out.into_iter().collect()
}

/// Parse a bounded rectangular matrix from InkML number tokens (including boolean table values).
fn rows(el: &El, work: &mut usize) -> Option<Vec<Vec<f64>>> {
    let text = el.text();
    spend(work, text.len())?;
    let mut tokens = Tokens(text.trim());
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut boolean_values = None;
    let boolean_allowed = el.name != "ink:affine" && el.attr("interpolation").is_none();
    while !tokens.0.is_empty() {
        if rows.len() >= 1024 {
            return None;
        }
        let mut row = Vec::new();
        while !tokens.0.is_empty() && !tokens.0.starts_with(',') {
            if row.len() > MAX_CHANNELS * 2 {
                return None;
            }
            let boolean = tokens.0.starts_with(['T', 'F']);
            if boolean_values.is_some_and(|prior| prior != boolean) || (boolean && !boolean_allowed) {
                return None;
            }
            boolean_values = Some(boolean);
            let value = match tokens.0.as_bytes().first()? {
                b'T' => {
                    tokens.0 = tokens.0.get(1..)?;
                    1.0
                }
                b'F' => {
                    tokens.0 = tokens.0.get(1..)?;
                    0.0
                }
                _ => tokens.number()?,
            };
            row.push(value);
            tokens.space();
        }
        if row.is_empty() || rows.first().is_some_and(|first| first.len() != row.len()) {
            return None;
        }
        rows.push(row);
        if !tokens.0.is_empty() {
            tokens.0 = tokens.0.strip_prefix(',')?;
            tokens.space();
        }
    }
    (!rows.is_empty()).then_some(rows)
}

/// A scalar expression from the restricted Content MathML subset in InkML §6.1.1.
#[derive(Clone)]
pub(super) enum Expr {
    /// Literal, named constant, or bound channel.
    Constant(f64),
    /// Source channel index.
    Channel(usize),
    /// Supported operator with recursively compiled arguments.
    Apply(String, Vec<Expr>),
}

impl Expr {
    /// Compile only the InkML-permitted MathML names; ignore attributes except numeric type.
    fn read(el: &El, vars: &HashMap<&str, usize>, work: &mut usize, depth: usize) -> Option<Self> {
        spend(work, 1)?;
        if depth > MAX_DEPTH {
            return None;
        }
        Some(match el.name.as_str() {
            "mml:cn" if el.els().next().is_none() => {
                let text = el.text();
                spend(work, text.len())?;
                let value = number(text.trim())?;
                match el.attr("type") {
                    None | Some("real") => {}
                    Some("integer") if value.fract() == 0.0 => {}
                    _ => return None,
                }
                Self::Constant(value)
            }
            "mml:ci" if el.els().next().is_none() => {
                let text = el.text();
                spend(work, text.len())?;
                Self::Channel(*vars.get(text.trim())?)
            }
            "mml:pi" => Self::Constant(std::f64::consts::PI),
            "mml:exponentiale" => Self::Constant(std::f64::consts::E),
            "mml:true" => Self::Constant(1.0),
            "mml:false" => Self::Constant(0.0),
            "mml:apply" => {
                let mut children = el.els();
                let op = children.next()?.name.strip_prefix("mml:")?;
                if !matches!(
                    op,
                    "plus"
                        | "minus"
                        | "times"
                        | "divide"
                        | "quotient"
                        | "rem"
                        | "power"
                        | "root"
                        | "min"
                        | "max"
                        | "abs"
                        | "floor"
                        | "ceiling"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "arcsin"
                        | "arccos"
                        | "arctan"
                        | "exp"
                        | "ln"
                        | "log"
                        | "and"
                        | "or"
                        | "xor"
                        | "not"
                        | "eq"
                        | "neq"
                        | "gt"
                        | "lt"
                        | "geq"
                        | "leq"
                ) {
                    return None;
                }
                let mut args = Vec::new();
                let mut qualifier = None;
                for child in children {
                    if matches!(child.name.as_str(), "mml:degree" | "mml:logbase") {
                        if (child.name == "mml:degree" && op != "root") || (child.name == "mml:logbase" && op != "log") || qualifier.is_some() {
                            return None;
                        }
                        let mut parts = child.els();
                        qualifier = Some(Self::read(parts.next()?, vars, work, depth + 1)?);
                        if parts.next().is_some() {
                            return None;
                        }
                    } else {
                        args.push(Self::read(child, vars, work, depth + 1)?);
                    }
                    if args.len() > MAX_CHANNELS {
                        return None;
                    }
                }
                if matches!(op, "root" | "log") {
                    args.push(qualifier.unwrap_or(Self::Constant(if op == "root" { 2.0 } else { 10.0 })));
                }
                Self::Apply(op.into(), args)
            }
            _ => return None,
        })
    }

    /// Evaluate under the global budget, rejecting invalid domains and nonfinite intermediates.
    fn eval(&self, input: &Sample, work: &mut usize) -> Option<f64> {
        spend(work, 1)?;
        let value = match self {
            Self::Constant(v) => *v,
            Self::Channel(i) => input.get(*i).copied().flatten()?,
            Self::Apply(op, args) => {
                let v = args.iter().map(|a| a.eval(input, work)).collect::<Option<Vec<_>>>()?;
                let a = *v.first()?;
                let binary = || (v.len() == 2).then(|| v.get(1).copied()).flatten();
                let unary = || (v.len() == 1).then_some(a);
                let flag = |v: bool| if v { 1.0 } else { 0.0 };
                match op.as_str() {
                    "plus" => v.iter().sum(),
                    "times" => v.iter().product(),
                    "min" => v.iter().copied().reduce(f64::min)?,
                    "max" => v.iter().copied().reduce(f64::max)?,
                    "minus" if v.len() == 1 => -a,
                    "minus" => a - binary()?,
                    "divide" => a / binary()?,
                    "quotient" => (a / binary()?).trunc(),
                    "rem" => a % binary()?,
                    "power" => a.powf(binary()?),
                    "root" => {
                        let b = binary()?;
                        if a < 0.0 && b.fract() == 0.0 && b.abs() % 2.0 == 1.0 { -(-a).powf(1.0 / b) } else { a.powf(1.0 / b) }
                    }
                    "log" => a.log(binary()?),
                    "abs" => unary()?.abs(),
                    "floor" => unary()?.floor(),
                    "ceiling" => unary()?.ceil(),
                    "sin" => unary()?.sin(),
                    "cos" => unary()?.cos(),
                    "tan" => unary()?.tan(),
                    "arcsin" => unary()?.asin(),
                    "arccos" => unary()?.acos(),
                    "arctan" => unary()?.atan(),
                    "exp" => unary()?.exp(),
                    "ln" => unary()?.ln(),
                    "not" => flag(unary()? == 0.0),
                    "and" => flag(v.iter().all(|n| *n != 0.0)),
                    "or" => flag(v.iter().any(|n| *n != 0.0)),
                    "xor" => flag(v.iter().filter(|n| **n != 0.0).count() % 2 == 1),
                    "eq" => flag(a == binary()?),
                    "neq" => flag(a != binary()?),
                    "gt" => flag(a > binary()?),
                    "lt" => flag(a < binary()?),
                    "geq" => flag(a >= binary()?),
                    "leq" => flag(a <= binary()?),
                    _ => return None,
                }
            }
        };
        bounded(value).then_some(value)
    }
}

/// A complete rectangular lookup grid. Evaluation scans bounded rows, charging each lookup.
#[derive(Clone)]
pub(super) struct Table {
    /// Source channel, table column and sorted unique knots for each independent variable.
    sources: Vec<(usize, usize, Vec<f64>)>,
    /// Destination channel and table column.
    targets: Vec<(usize, usize)>,
    /// Optional input channel added for relative output.
    relative: Vec<Option<usize>>,
    /// Matrix of input/output knots.
    rows: Vec<Vec<f64>>,
    /// Grid interpolation policy.
    interpolation: String,
    /// Natural cubic coefficients for a univariate cubic table, otherwise absent.
    spline: Option<spline::Spline>,
}

impl Table {
    /// Validate bindings and completeness before accepting a table for evaluation.
    fn read(el: &El, source: &Format, target: &Format, work: &mut usize) -> Option<Self> {
        let table = el.child("ink:table")?;
        let rows = rows(table, work)?;
        let width = rows.first()?.len();
        let interpolation = table.attr("interpolation").unwrap_or("linear");
        if !matches!(interpolation, "floor" | "middle" | "ceiling" | "linear" | "cubic") {
            return None;
        }
        let relative = match table.attr("apply") {
            None | Some("absolute") => false,
            Some("relative") => true,
            _ => return None,
        };
        let mut sources = Vec::new();
        let mut targets = Vec::new();
        let mut additions = Vec::new();
        let mut assigned = vec![false; width];
        for b in el.children("ink:bind") {
            let column = b.attr("column")?.parse::<usize>().ok()?.checked_sub(1)?;
            if *assigned.get(column)? {
                return None;
            }
            *assigned.get_mut(column)? = true;
            match (b.attr("source"), b.attr("target")) {
                (Some(name), None) => {
                    let mut knots = rows.iter().map(|r| r.get(column).copied()).collect::<Option<Vec<_>>>()?;
                    knots.sort_by(f64::total_cmp);
                    knots.dedup();
                    sources.push((source.index(name)?, column, knots));
                }
                (None, Some(name)) => {
                    targets.push((target.index(name)?, column));
                    additions.push(if relative { Some(source.index(name)?) } else { None });
                }
                _ => return None,
            }
        }
        if sources.is_empty() || targets.is_empty() || sources.len() > MAX_CHANNELS || targets.len() > MAX_CHANNELS || assigned.contains(&false) {
            return None;
        }
        let total = sources.iter().try_fold(1usize, |n, (_, _, v)| n.checked_mul(v.len()))?;
        if total != rows.len() {
            return None;
        }
        let mut keys = Vec::with_capacity(rows.len());
        for row in &rows {
            spend(work, sources.len())?;
            let mut key = 0usize;
            for (_, col, knots) in &sources {
                let value = row.get(*col)?;
                key = key.checked_mul(knots.len())?.checked_add(knots.binary_search_by(|v| v.total_cmp(value)).ok()?)?;
            }
            keys.push(key);
        }
        keys.sort_unstable();
        if keys.windows(2).any(|w| w.first() == w.get(1)) {
            return None;
        }
        let spline = if interpolation == "cubic" {
            let [(_, source_column, _)] = sources.as_slice() else {
                return None;
            };
            let columns: Vec<_> = targets.iter().map(|p| p.1).collect();
            Some(spline::Spline::new(&rows, *source_column, &columns, work)?)
        } else {
            None
        };
        Some(Self { sources, targets, relative: additions, rows, interpolation: interpolation.into(), spline })
    }

    /// Evaluate a grid/spline and apply optional relative output to the original named channel.
    fn apply(&self, input: &Sample, output: &mut Sample, work: &mut usize) -> Option<()> {
        let values = if let Some(spline) = &self.spline {
            let channel = self.sources.first()?.0;
            spline.at(input.get(channel).copied().flatten()?, work)?
        } else {
            self.grid(input, work)?
        };
        for (((channel, _), add), mut value) in self.targets.iter().zip(&self.relative).zip(values) {
            if let Some(i) = add {
                value += input.get(*i).copied().flatten()?;
            }
            if !bounded(value) {
                return None;
            }
            *output.get_mut(*channel)? = Some(value);
        }
        Some(())
    }

    /// Interpolate complete tensor-product grids without exponential allocations.
    fn grid(&self, input: &Sample, work: &mut usize) -> Option<Vec<f64>> {
        let mut ranges = Vec::with_capacity(self.sources.len());
        for (channel, _, knots) in &self.sources {
            let value = input.get(*channel).copied().flatten()?;
            let hi = knots.partition_point(|x| *x < value);
            let exact = knots.get(hi).is_some_and(|x| *x == value);
            let low = if exact { knots.get(hi).copied() } else { hi.checked_sub(1).and_then(|i| knots.get(i)).copied() };
            let high = knots.get(hi).copied();
            let (a, b, t) = match self.interpolation.as_str() {
                "floor" => {
                    let a = low?;
                    (a, a, 0.0)
                }
                "ceiling" => {
                    let b = high?;
                    (b, b, 0.0)
                }
                "middle" => {
                    let (a, b) = (low?, high?);
                    let v = if value - a < b - value { a } else { b };
                    (v, v, 0.0)
                }
                "linear" => {
                    let (a, b) = (low?, high?);
                    (a, b, if a == b { 0.0 } else { (value - a) / (b - a) })
                }
                _ => return None,
            };
            ranges.push((a, b, t));
        }
        let mut values = vec![0.0; self.targets.len()];
        for row in &self.rows {
            spend(work, self.sources.len() + self.targets.len())?;
            let mut weight = 1.0;
            for ((_, col, _), &(a, b, t)) in self.sources.iter().zip(&ranges) {
                let v = *row.get(*col)?;
                weight *= if a == b && v == a {
                    1.0
                } else if v == a {
                    1.0 - t
                } else if v == b {
                    t
                } else {
                    0.0
                };
            }
            if weight != 0.0 {
                for (v, (_, col)) in values.iter_mut().zip(&self.targets) {
                    *v += row.get(*col)? * weight;
                }
            }
        }
        Some(values)
    }
}
