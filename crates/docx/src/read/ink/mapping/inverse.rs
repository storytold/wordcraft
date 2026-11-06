use super::{El, Format, HashMap, MAX_CHANNELS, MAX_DEPTH, Mapping, Table, bounded, spend};

impl Mapping {
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
        Self::read(reverse?, target, source, ids, work, 0)?.inverse(work)
    }

    fn inverse(&self, work: &mut usize) -> Option<Self> {
        spend(work, 1)?;
        let inverse = match self {
            Self::Identity(pairs) => Self::Identity(pairs.iter().map(|&(a, b)| (b, a)).collect()),
            Self::Affine { sources, targets, rows } => {
                let inverse = invert(rows, work)?;
                Self::Affine { sources: targets.clone(), targets: sources.clone(), rows: inverse }
            }
            Self::Product(maps) => {
                let mut rows = Vec::new();
                if affine_rows(self, &mut rows, work).is_some() {
                    let mut sources: Vec<_> = rows.iter().flat_map(|r| r.1.iter().map(|p| p.0)).collect();
                    sources.sort_unstable();
                    sources.dedup();
                    rows.sort_by_key(|r| r.0);
                    let targets: Vec<_> = rows.iter().map(|r| r.0).collect();
                    if sources.len() != targets.len() || sources.len() > MAX_CHANNELS {
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
                    Self::Affine { sources: targets, targets: sources, rows: invert(&matrix, work)? }
                } else {
                    Self::Product(maps.iter().map(|m| m.inverse(work)).collect::<Option<Vec<_>>>()?)
                }
            }
            Self::Table(table) => Self::Table(table.inverse(work)?),
            Self::Formula { .. } => return None,
        };
        let mut targets = Vec::new();
        inverse.targets(&mut targets);
        targets.sort_unstable();
        if targets.windows(2).any(|w| w.first() == w.get(1)) {
            return None;
        }
        Some(inverse)
    }
}

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
        _ => return None,
    }
    (rows.len() <= MAX_CHANNELS).then_some(())
}

fn invert(rows: &[Vec<f64>], work: &mut usize) -> Option<Vec<Vec<f64>>> {
    let n = rows.len();
    if n == 0 || n > MAX_CHANNELS || rows.iter().any(|r| r.len() != n + 1) {
        return None;
    }
    spend(work, n.saturating_mul(n).saturating_mul(n + 1).saturating_mul(4))?;
    let mut matrix = Vec::with_capacity(n);
    for (i, row) in rows.iter().enumerate() {
        let mut r = row.get(..n)?.to_vec();
        r.resize(n * 2, 0.0);
        *r.get_mut(n + i)? = 1.0;
        matrix.push(r);
    }
    for i in 0..n {
        let pivot = (i..n).max_by(|&a, &b| {
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
    fn inverse(&self, work: &mut usize) -> Option<Self> {
        let [(source, xcol, _)] = self.sources.as_slice() else {
            return None;
        };
        let [(target, ycol)] = self.targets.as_slice() else {
            return None;
        };
        if self.interpolation != "linear" || self.relative.iter().any(Option::is_some) || self.rows.len() < 2 {
            return None;
        }
        spend(work, self.rows.len().saturating_mul(16))?;
        let mut rows: Vec<Vec<f64>> = self.rows.iter().map(|r| Some(vec![*r.get(*xcol)?, *r.get(*ycol)?])).collect::<Option<_>>()?;
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
        Some(Self {
            sources: vec![(*target, 0, knots)],
            targets: vec![(*source, 1)],
            relative: vec![None],
            rows,
            interpolation: "linear".into(),
            spline: None,
        })
    }
}
