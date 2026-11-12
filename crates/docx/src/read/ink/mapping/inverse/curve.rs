use super::super::{bounded, spend, spline::Spline};

#[derive(Clone)]
pub(in crate::read::ink) struct Curve {
    pub source: usize,
    pub target: usize,
    spline: Spline,
    x: Vec<f64>,
    y: Vec<f64>,
    relative: bool,
}

impl Curve {
    pub(super) fn new(spline: Spline, source: usize, target: usize, relative: bool, work: &mut usize) -> Option<Self> {
        let (x, y) = spline.inverse_domain(relative, work)?;
        Some(Self { source, target, spline, x, y, relative })
    }

    pub(in crate::read::ink::mapping) fn at(&self, value: f64, work: &mut usize) -> Option<f64> {
        spend(work, 16)?;
        let (first, last) = (*self.y.first()?, *self.y.last()?);
        if !bounded(value) || value < first.min(last) || value > first.max(last) {
            return None;
        }
        let increasing = first < last;
        let hi = self.y.partition_point(|v| if increasing { *v < value } else { *v > value });
        if self.y.get(hi) == Some(&value) {
            return self.x.get(hi).copied();
        }
        let mut left = *self.x.get(hi.checked_sub(1)?)?;
        let mut right = *self.x.get(hi)?;
        for _ in 0..64 {
            spend(work, 1)?;
            let middle = left + (right - left) * 0.5;
            if middle == left || middle == right {
                break;
            }
            let actual = self.value(middle, work)?;
            if actual == value {
                return Some(middle);
            }
            if (actual < value) == increasing {
                left = middle;
            } else {
                right = middle;
            }
        }
        let a = self.value(left, work)?;
        let b = self.value(right, work)?;
        Some(if (a - value).abs() < (b - value).abs() { left } else { right })
    }

    /// Evaluates the spline at `x`, applying relative output and finite-value checks.
    fn value(&self, x: f64, work: &mut usize) -> Option<f64> {
        let y = self.spline.at(x, work)?.first().copied()? + if self.relative { x } else { 0.0 };
        bounded(y).then_some(y)
    }
}
