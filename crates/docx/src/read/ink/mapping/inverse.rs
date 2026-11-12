use super::{El, Format, HashMap, MAX_CHANNELS, MAX_DEPTH, Mapping, Sample, Table, bounded, spend};

pub(super) mod curve;
pub(super) mod discrete;
pub(super) mod formula;
mod product;

impl Mapping {
    /// Compile the forward transform, or a checked inverse when only the reverse map is known.
    pub(in crate::read::ink) fn transform(el: &El, source: &Format, target: &Format, ids: &HashMap<&str, &El>, work: &mut usize) -> Option<Self> {
        let mut mappings = el.children("ink:mapping");
        let forward = mappings.next()?;
        let definition = effective(forward, ids, work)?;
        let reverse = mappings.next();
        if mappings.next().is_some() {
            return None;
        }
        if definition.attr("type").unwrap_or("unknown") != "unknown" {
            return Self::read(forward, source, target, ids, work, 0);
        }
        let forward = Self::read(reverse?, target, source, ids, work, 0)?;
        let backward = forward.inverse(target, source, work)?;
        spend(work, target.channels.len().saturating_add(source.channels.len()))?;
        Some(Self::CheckedInverse { forward: Box::new(forward), backward: Box::new(backward), format: target.clone(), recorded: source.clone() })
    }

    /// Invert dependency-connected components and reject empty or duplicate recovered targets.
    fn inverse(&self, format: &Format, recorded: &Format, work: &mut usize) -> Option<Self> {
        spend(work, 1)?;
        let inverse = if matches!(self, Self::Product(_)) {
            let groups = product::groups(self, work)?;
            Self::Product(groups.iter().map(|group| Self::inverse_component(group, format, recorded, work)).collect::<Option<_>>()?)
        } else {
            Self::inverse_component(&[self], format, recorded, work)?
        };
        let mut targets = Vec::new();
        inverse.targets(&mut targets);
        targets.sort_unstable();
        if targets.is_empty() || targets.windows(2).any(|w| w.first() == w.get(1)) {
            return None;
        }
        Some(inverse)
    }

    /// Prefer finite-domain lookup, then try supported analytic inverses for one component.
    fn inverse_component(maps: &[&Self], format: &Format, recorded: &Format, work: &mut usize) -> Option<Self> {
        if let Some(lookup) = discrete::Lookup::new(maps, format, recorded, work) {
            return Some(Self::InverseDiscrete(lookup));
        }
        for map in maps {
            map.exact_integer_outputs(format, recorded, work)?;
        }
        if let [map] = maps {
            return Some(match map {
                Self::Identity(pairs) => Self::Identity(pairs.iter().map(|&(a, b)| (b, a)).collect()),
                Self::Affine { sources, targets, rows } => {
                    Self::Affine { sources: targets.clone(), targets: sources.clone(), rows: invert(rows, work)? }
                }
                Self::Table(table) => table.inverse(work)?,
                Self::Formula { target, expression } => Self::InverseFormula(formula::inverse(expression, *target, format, work)?),
                _ => return None,
            });
        }
        affine_component(maps, work)
    }

    /// Reject analytic inverses whose integer observations could hide quantization collisions.
    fn exact_integer_outputs(&self, source: &Format, target: &Format, work: &mut usize) -> Option<()> {
        spend(work, source.channels.len().saturating_mul(target.channels.len()).max(1))?;
        let integer = |i: usize| target.channels.get(i).is_none_or(|c| c.integer);
        let discrete = |i: usize| source.channels.get(i).is_some_and(|c| c.integer || c.boolean);
        match self {
            Self::Identity(pairs) => (!pairs.iter().any(|(a, b)| integer(*b) && !discrete(*a))).then_some(()),
            Self::Affine { sources, targets, rows } => {
                for (&target, row) in targets.iter().zip(rows) {
                    if integer(target)
                        && (row.last()?.fract() != 0.0 || sources.iter().zip(row).any(|(&i, &v)| v != 0.0 && (!discrete(i) || v.fract() != 0.0)))
                    {
                        return None;
                    }
                }
                Some(())
            }
            Self::Product(maps) => maps.iter().try_for_each(|m| m.exact_integer_outputs(source, target, work)),
            Self::Formula { target, expression } => {
                if !integer(*target) {
                    return Some(());
                }
                let form = formula::linear(expression, work)?;
                (form.constant.fract() == 0.0 && form.terms.iter().enumerate().all(|(i, &v)| v == 0.0 || discrete(i) && v.fract() == 0.0))
                    .then_some(())
            }
            Self::Table(t) => (!t.targets.iter().any(|(i, _)| integer(*i))).then_some(()),
            _ => None,
        }
    }
}

/// Round integer samples and validate known values against their channel domains.
pub(super) fn normalize(format: &Format, values: &mut Sample, work: &mut usize) -> Option<()> {
    spend(work, format.channels.len())?;
    if values.len() != format.channels.len() {
        return None;
    }
    for (channel, value) in format.channels.iter().zip(values) {
        if let Some(v) = value {
            if channel.integer {
                *v = v.round();
            }
            if !channel.accepts(*v) {
                return None;
            }
        }
    }
    Some(())
}

/// Compare bounded values exactly for discrete channels or with a relative floating-point tolerance.
pub(super) fn same(expected: f64, actual: f64, exact: bool) -> bool {
    let tolerance = if exact { 0.0 } else { 128.0 * f64::EPSILON * expected.abs().max(actual.abs()).max(1.0) };
    bounded(expected) && bounded(actual) && (expected - actual).abs() <= tolerance
}

/// Validate recovered samples by replaying the original map against all known observations.
pub(super) fn check(forward: &Mapping, format: &Format, recorded: &Format, input: &Sample, output: &mut Sample, work: &mut usize) -> Option<()> {
    spend(work, input.len().saturating_add(output.len()))?;
    normalize(format, output, work)?;
    let mut recovered = vec![None; input.len()];
    forward.apply(output, &mut recovered, work)?;
    normalize(recorded, &mut recovered, work)?;
    for ((expected, actual), channel) in input.iter().zip(recovered).zip(&recorded.channels) {
        if let (Some(expected), Some(actual)) = (expected, actual)
            && !same(*expected, actual, channel.integer || channel.boolean)
        {
            return None;
        }
    }
    Some(())
}

/// Follow local mapping references within depth and work limits to find their definition.
fn effective<'a>(mut el: &'a El, ids: &HashMap<&str, &'a El>, work: &mut usize) -> Option<&'a El> {
    for _ in 0..=MAX_DEPTH {
        spend(work, el.attrs.len().max(1))?;
        if el.name != "ink:mapping" {
            return None;
        }
        let Some(reference) = el.attr("mappingRef") else {
            return Some(el);
        };
        el = *ids.get(reference.strip_prefix('#')?)?;
    }
    None
}

type AffineRow = (usize, Vec<(usize, f64)>, f64);

/// Collect affine equations from identity, matrix, product and linear MathML mappings.
fn affine_rows(mapping: &Mapping, rows: &mut Vec<AffineRow>, work: &mut usize) -> Option<()> {
    spend(work, 1)?;
    match mapping {
        Mapping::Identity(pairs) => rows.extend(pairs.iter().map(|&(src, dst)| (dst, vec![(src, 1.0)], 0.0))),
        Mapping::Affine { sources, targets, rows: matrix } => {
            spend(work, sources.len().saturating_mul(targets.len()))?;
            for (&target, row) in targets.iter().zip(matrix) {
                rows.push((target, sources.iter().copied().zip(row.iter().copied()).collect(), *row.last()?));
            }
        }
        Mapping::Product(maps) => {
            for map in maps {
                affine_rows(map, rows, work)?;
            }
        }
        Mapping::Formula { target, expression } => {
            let form = formula::linear(expression, work)?;
            let terms = form.terms.iter().enumerate().filter_map(|(i, &v)| (v != 0.0).then_some((i, v))).collect();
            rows.push((*target, terms, form.constant));
        }
        _ => return None,
    }
    (rows.len() <= MAX_CHANNELS).then_some(())
}

/// Assemble and invert the joint affine system for a dependency-connected component.
fn affine_component(maps: &[&Mapping], work: &mut usize) -> Option<Mapping> {
    let mut rows = Vec::new();
    for map in maps {
        affine_rows(map, &mut rows, work)?;
    }
    let mut sources: Vec<_> = rows.iter().flat_map(|r| r.1.iter().map(|p| p.0)).collect();
    sources.sort_unstable();
    sources.dedup();
    rows.sort_by_key(|r| r.0);
    let targets: Vec<_> = rows.iter().map(|r| r.0).collect();
    if sources.len() > targets.len() || sources.len() > MAX_CHANNELS {
        return None;
    }
    let mut matrix = Vec::new();
    for (_, terms, constant) in rows {
        let mut row = vec![0.0; sources.len() + 1];
        for (source, coefficient) in terms {
            *row.get_mut(sources.binary_search(&source).ok()?)? += coefficient;
        }
        *row.last_mut()? = constant;
        matrix.push(row);
    }
    Some(Mapping::Affine { sources: targets, targets: sources, rows: invert(&matrix, work)? })
}

/// Compute a bounded affine left inverse using pivoted elimination; reject rank-deficient systems.
fn invert(rows: &[Vec<f64>], work: &mut usize) -> Option<Vec<Vec<f64>>> {
    let m = rows.len();
    let n = rows.first()?.len().checked_sub(1)?;
    if n == 0 || n > MAX_CHANNELS || m < n || m > MAX_CHANNELS || rows.iter().any(|r| r.len() != n + 1) {
        return None;
    }
    spend(work, m.saturating_mul(n).saturating_mul(n + m).saturating_mul(4))?;
    let mut matrix = Vec::with_capacity(m);
    for (i, row) in rows.iter().enumerate() {
        let mut r = row.get(..n)?.to_vec();
        r.resize(n + m, 0.0);
        *r.get_mut(n + i)? = 1.0;
        matrix.push(r);
    }
    for i in 0..n {
        let pivot = (i..m).max_by(|&a, &b| {
            matrix
                .get(a)
                .and_then(|r| r.get(i))
                .copied()
                .unwrap_or(0.0)
                .abs()
                .total_cmp(&matrix.get(b).and_then(|r| r.get(i)).copied().unwrap_or(0.0).abs())
        })?;
        matrix.swap(i, pivot);
        let divisor = *matrix.get(i)?.get(i)?;
        if divisor == 0.0 {
            return None;
        }
        for value in matrix.get_mut(i)? {
            *value /= divisor;
            if !bounded(*value) {
                return None;
            }
        }
        let row = matrix.get(i)?.clone();
        for (j, other) in matrix.iter_mut().enumerate() {
            if j == i {
                continue;
            }
            let factor = *other.get(i)?;
            for (value, coefficient) in other.iter_mut().zip(&row) {
                *value -= factor * coefficient;
                if !bounded(*value) {
                    return None;
                }
            }
        }
    }
    matrix
        .iter()
        .take(n)
        .map(|row| {
            let mut inverse = row.get(n..)?.to_vec();
            let offset: f64 = inverse.iter().zip(rows).map(|(a, r)| Some(a * r.last()?)).sum::<Option<f64>>()?;
            if !bounded(offset) {
                return None;
            }
            inverse.push(-offset);
            Some(inverse)
        })
        .collect()
}

impl Table {
    /// Find a table output axis that uniquely reconstructs its single input channel.
    fn inverse(&self, work: &mut usize) -> Option<Mapping> {
        for target in 0..self.targets.len() {
            spend(work, 1)?;
            if let Some(mapping) = self.inverse_axis(target, work) {
                return Some(mapping);
            }
            if *work == 0 {
                return None;
            }
        }
        None
    }

    /// Invert one monotone linear or cubic table output, including its relative identity term.
    fn inverse_axis(&self, axis: usize, work: &mut usize) -> Option<Mapping> {
        let [(source, xcol, _)] = self.sources.as_slice() else {
            return None;
        };
        let (target, ycol) = self.targets.get(axis)?;
        if self.rows.len() < 2 {
            return None;
        }
        let relative = match self.relative.get(axis)? {
            None => false,
            Some(i) if i == source => true,
            _ => return None,
        };
        if let Some(spline) = &self.spline {
            let spline = spline.component(axis, work)?;
            return Some(Mapping::InverseCurve(curve::Curve::new(spline, *target, *source, relative, work)?));
        }
        if self.interpolation != "linear" {
            return None;
        }
        spend(work, self.rows.len().saturating_mul(16))?;
        let mut rows: Vec<Vec<f64>> = self
            .rows
            .iter()
            .map(|r| {
                let x = *r.get(*xcol)?;
                let y = *r.get(*ycol)? + if relative { x } else { 0.0 };
                bounded(y).then_some(vec![x, y])
            })
            .collect::<Option<_>>()?;
        rows.sort_by(|a, b| a.first().copied().unwrap_or(0.0).total_cmp(&b.first().copied().unwrap_or(0.0)));
        let mut direction = None;
        for pair in rows.windows(2) {
            let delta = pair.get(1)?.get(1)? - pair.first()?.get(1)?;
            if delta == 0.0 || direction.is_some_and(|d| d != delta.is_sign_positive()) {
                return None;
            }
            direction = Some(delta.is_sign_positive());
        }
        for row in &mut rows {
            row.swap(0, 1);
        }
        rows.sort_by(|a, b| a.first().copied().unwrap_or(0.0).total_cmp(&b.first().copied().unwrap_or(0.0)));
        let knots = rows.iter().map(|r| r.first().copied()).collect::<Option<_>>()?;
        Some(Mapping::Table(Self {
            sources: vec![(*target, 0, knots)],
            targets: vec![(*source, 1)],
            relative: vec![None],
            rows,
            interpolation: "linear".into(),
            spline: None,
            boolean: false,
        }))
    }
}
