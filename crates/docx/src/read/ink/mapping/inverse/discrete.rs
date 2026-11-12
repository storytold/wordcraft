use super::super::Expr;
use super::{Format, MAX_CHANNELS, MAX_DEPTH, Mapping, Sample, normalize, same, spend};

const MAX_STATES: usize = 1024;

#[derive(Clone)]
struct Entry {
    observed: Vec<f64>,
    recovered: Vec<f64>,
}

#[derive(Clone)]
pub(in crate::read::ink) struct Lookup {
    pub targets: Vec<usize>,
    sources: Vec<usize>,
    exact: Vec<bool>,
    entries: Vec<Entry>,
}

impl Lookup {
    /// Enumerate a bounded discrete domain and record every normalized forward observation.
    pub(super) fn new(maps: &[&Mapping], domain: &Format, range: &Format, work: &mut usize) -> Option<Self> {
        let mut targets = Vec::new();
        let mut sources = Vec::new();
        for map in maps {
            targets.extend(dependencies(map, work)?);
            map.targets(&mut sources);
        }
        targets.sort_unstable();
        targets.dedup();
        sources.sort_unstable();
        if targets.is_empty() || sources.is_empty() || sources.windows(2).any(|p| p.first() == p.get(1)) {
            return None;
        }
        let domains = targets.iter().map(|i| values(domain.channels.get(*i)?, work)).collect::<Option<Vec<_>>>()?;
        let count = domains.iter().try_fold(1usize, |n, v| n.checked_mul(v.len()).filter(|n| *n <= MAX_STATES))?;
        spend(
            work,
            count.saturating_mul(
                domain.channels.len().saturating_add(range.channels.len()).saturating_add(targets.len()).saturating_add(sources.len()),
            ),
        )?;
        let exact = sources
            .iter()
            .map(|i| {
                let c = range.channels.get(*i)?;
                Some(c.integer || c.boolean)
            })
            .collect::<Option<_>>()?;
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let mut input = vec![None; domain.channels.len()];
            let mut index = index;
            for (&target, values) in targets.iter().zip(&domains) {
                *input.get_mut(target)? = Some(*values.get(index.checked_rem(values.len())?)?);
                index = index.checked_div(values.len())?;
            }
            let observed = evaluate(maps, &input, range, work)?;
            let observed = sources.iter().map(|i| observed.get(*i).copied().flatten()).collect::<Option<_>>()?;
            let recovered = targets.iter().map(|i| input.get(*i).copied().flatten()).collect::<Option<_>>()?;
            entries.push(Entry { observed, recovered });
        }
        spend(work, 1)?;
        (!entries.is_empty()).then_some(Self { targets, sources, exact, entries })
    }

    /// Recover values only when the known observations match exactly one enumerated state.
    pub(in crate::read::ink::mapping) fn apply(&self, input: &Sample, output: &mut Sample, work: &mut usize) -> Option<()> {
        spend(work, self.entries.len().saturating_mul(self.sources.len()).saturating_add(self.targets.len()))?;
        let mut selected = None;
        for entry in &self.entries {
            let mut matches = true;
            for ((&source, &value), &exact) in self.sources.iter().zip(&entry.observed).zip(&self.exact) {
                if let Some(expected) = input.get(source).copied().flatten()
                    && !same(expected, value, exact)
                {
                    matches = false;
                    break;
                }
            }
            if matches && selected.replace(entry).is_some() {
                return None;
            }
        }
        let selected = selected?;
        for (&target, &value) in self.targets.iter().zip(&selected.recovered) {
            *output.get_mut(target)? = Some(value);
        }
        Some(())
    }
}

/// Enumerate permitted Boolean or explicitly bounded integer values within the state cap.
fn values(channel: &crate::read::ink::format::Channel, work: &mut usize) -> Option<Vec<f64>> {
    spend(work, 1)?;
    if channel.boolean {
        return Some([0.0, 1.0].into_iter().filter(|v| channel.accepts(*v)).collect());
    }
    if !channel.integer {
        return None;
    }
    let (first, last) = if channel.fraction { (0.0, 1.0) } else { (channel.min?.ceil(), channel.max?.floor()) };
    let count = last - first + 1.0;
    if !super::bounded(first) || !super::bounded(last) || !(1.0..=MAX_STATES as f64).contains(&count) {
        return None;
    }
    spend(work, count as usize)?;
    Some((0..count as usize).map(|i| first + i as f64).filter(|v| channel.accepts(*v)).collect())
}

/// Apply component mappings and normalize observations using the recorded channel format.
fn evaluate(maps: &[&Mapping], input: &Sample, range: &Format, work: &mut usize) -> Option<Sample> {
    let mut output = vec![None; range.channels.len()];
    for map in maps {
        map.apply(input, &mut output, work)?;
    }
    normalize(range, &mut output, work)?;
    Some(output)
}

/// Collect input channels used by a supported mapping, charging traversal work.
pub(super) fn dependencies(map: &Mapping, work: &mut usize) -> Option<Vec<usize>> {
    let mut used = [false; MAX_CHANNELS];
    inputs(map, &mut used, work, 0)?;
    Some(used.iter().enumerate().filter_map(|(i, &v)| v.then_some(i)).collect())
}

/// Mark mapping dependencies recursively within the depth and work limits.
fn inputs(map: &Mapping, used: &mut [bool; MAX_CHANNELS], work: &mut usize, depth: usize) -> Option<()> {
    spend(work, 1)?;
    if depth > MAX_DEPTH {
        return None;
    }
    let mut mark = |i: usize| -> Option<()> {
        *used.get_mut(i)? = true;
        Some(())
    };
    match map {
        Mapping::Identity(pairs) => {
            for &(i, _) in pairs {
                spend(work, 1)?;
                mark(i)?;
            }
        }
        Mapping::Affine { sources, rows, .. } => {
            for row in rows {
                spend(work, sources.len())?;
                for (&i, &coefficient) in sources.iter().zip(row) {
                    if coefficient != 0.0 {
                        mark(i)?;
                    }
                }
            }
        }
        Mapping::Table(table) => {
            spend(work, table.sources.len().saturating_add(table.relative.len()))?;
            for &(i, _, _) in &table.sources {
                mark(i)?;
            }
            for &i in table.relative.iter().flatten() {
                mark(i)?;
            }
        }
        Mapping::Formula { expression, .. } => expression_inputs(expression, used, work, 0)?,
        Mapping::Product(maps) => {
            for map in maps {
                inputs(map, used, work, depth + 1)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// Mark every channel referenced by a bounded MathML expression traversal.
fn expression_inputs(expression: &Expr, used: &mut [bool; MAX_CHANNELS], work: &mut usize, depth: usize) -> Option<()> {
    spend(work, 1)?;
    if depth > MAX_DEPTH {
        return None;
    }
    match expression {
        Expr::Constant(_) => {}
        Expr::Channel(i) => *used.get_mut(*i)? = true,
        Expr::Apply(_, args) => {
            for arg in args {
                expression_inputs(arg, used, work, depth + 1)?;
            }
        }
    }
    Some(())
}
