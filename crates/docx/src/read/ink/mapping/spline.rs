//! Natural cubic splines for univariate InkML lookup tables. InkML specifies cubic splines
//! without endpoint derivatives; this renderer chooses zero second derivatives at both ends.

use super::{bounded, spend};

/// Precomputed natural spline coefficients for one or more output channels on a shared axis.
#[derive(Clone)]
pub(super) struct Spline {
    /// Strictly increasing input knots.
    x: Vec<f64>,
    /// Output knot values, one vector per output channel.
    y: Vec<Vec<f64>>,
    /// Output second derivatives at each knot.
    second: Vec<Vec<f64>>,
}

impl Spline {
    /// Compute coefficients in linear time and bounded memory, rejecting singular/nonfinite inputs.
    pub fn new(rows: &[Vec<f64>], source: usize, targets: &[usize], work: &mut usize) -> Option<Self> {
        if rows.len() < 4 {
            return None;
        }
        spend(work, rows.len().saturating_mul(targets.len() + 1).saturating_mul(16))?;
        let mut ordered: Vec<_> = rows.iter().collect();
        ordered.sort_by(|a, b| a.get(source).copied().unwrap_or(f64::NAN).total_cmp(&b.get(source).copied().unwrap_or(f64::NAN)));
        let x = ordered.iter().map(|r| r.get(source).copied()).collect::<Option<Vec<_>>>()?;
        if x.windows(2).any(|w| !matches!(w,[a,b] if a<b)) {
            return None;
        }
        let mut y = Vec::new();
        let mut second = Vec::new();
        for &target in targets {
            let values = ordered.iter().map(|r| r.get(target).copied()).collect::<Option<Vec<_>>>()?;
            let mut deriv = vec![0.0; x.len()];
            let mut rhs = vec![0.0; x.len()];
            for i in 1..x.len().saturating_sub(1) {
                let (lo, hi) = (x.get(i)? - x.get(i - 1)?, x.get(i + 1)? - x.get(i)?);
                let ratio = lo / (lo + hi);
                let pivot = ratio * deriv.get(i - 1)? + 2.0;
                let coefficient = (ratio - 1.0) / pivot;
                let slopes = (values.get(i + 1)? - values.get(i)?) / hi - (values.get(i)? - values.get(i - 1)?) / lo;
                let value = (6.0 * slopes / (lo + hi) - ratio * rhs.get(i - 1)?) / pivot;
                if !bounded(coefficient) || !bounded(value) {
                    return None;
                }
                *deriv.get_mut(i)? = coefficient;
                *rhs.get_mut(i)? = value;
            }
            for i in (0..x.len().saturating_sub(1)).rev() {
                let value = deriv.get(i)? * deriv.get(i + 1)? + rhs.get(i)?;
                if !bounded(value) {
                    return None;
                }
                *deriv.get_mut(i)? = value;
            }
            y.push(values);
            second.push(deriv);
        }
        Some(Self { x, y, second })
    }

    pub(super) fn component(&self, index: usize, work: &mut usize) -> Option<Self> {
        spend(work, self.x.len().saturating_mul(3))?;
        Some(Self { x: self.x.clone(), y: vec![self.y.get(index)?.clone()], second: vec![self.second.get(index)?.clone()] })
    }

    pub(super) fn inverse_domain(&self, relative: bool, work: &mut usize) -> Option<(Vec<f64>, Vec<f64>)> {
        let [y] = self.y.as_slice() else {
            return None;
        };
        let [d] = self.second.as_slice() else {
            return None;
        };
        if y.len() != self.x.len() || d.len() != y.len() {
            return None;
        }
        spend(work, y.len().saturating_mul(64))?;
        let values: Vec<_> = y.iter().zip(&self.x).map(|(y, x)| y + if relative { *x } else { 0.0 }).collect();
        if !values.iter().all(|v| bounded(*v)) {
            return None;
        }
        let direction = if values.first()? < values.last()? { 1.0 } else { -1.0 };
        for i in 1..self.x.len() {
            let h = self.x.get(i)? - self.x.get(i - 1)?;
            let (a, b) = (*d.get(i - 1)?, *d.get(i)?);
            let delta = values.get(i)? - values.get(i - 1)?;
            if delta * direction <= 0.0 {
                return None;
            }
            let quadratic = h * (b - a) * 0.5;
            let linear = h * a;
            let constant = delta / h - h * (2.0 * a + b) / 6.0;
            let derivative = |t: f64| (quadratic * t * t + linear * t + constant) * direction;
            let vertex = -linear / (2.0 * quadratic);
            if ![quadratic, linear, constant].iter().all(|v| v.is_finite())
                || derivative(0.0) < 0.0
                || derivative(1.0) < 0.0
                || ((0.0..1.0).contains(&vertex) && derivative(vertex) < 0.0)
            {
                return None;
            }
        }
        Some((self.x.clone(), values))
    }

    /// Interpolate in the declared domain; endpoint knots are returned exactly.
    pub fn at(&self, value: f64, work: &mut usize) -> Option<Vec<f64>> {
        spend(work, self.y.len().saturating_mul(16).saturating_add(self.x.len().ilog2() as usize + 1))?;
        if value < *self.x.first()? || value > *self.x.last()? {
            return None;
        }
        let hi = self.x.partition_point(|x| *x < value);
        if self.x.get(hi).is_some_and(|x| *x == value) {
            return self.y.iter().map(|v| v.get(hi).copied()).collect();
        }
        let lo = hi.checked_sub(1)?;
        let h = self.x.get(hi)? - self.x.get(lo)?;
        let a = (self.x.get(hi)? - value) / h;
        let b = (value - self.x.get(lo)?) / h;
        self.y
            .iter()
            .zip(&self.second)
            .map(|(y, d)| {
                let value = a * y.get(lo)? + b * y.get(hi)? + ((a * a * a - a) * d.get(lo)? + (b * b * b - b) * d.get(hi)?) * h * h / 6.0;
                bounded(value).then_some(value)
            })
            .collect()
    }
}
