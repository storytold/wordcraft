//! Curve fitting, Mathcad's way: a fit is worked out once into a coefficient vector, and `interp`
//! evaluates it.
//!
//! - `cspline(vx, vy)`, `lspline(vx, vy)`, `pspline(vx, vy)`: the cubic spline through every point
//!   with cubic, linear or parabolic ends; `regress(vx, vy, k)`: the least-squares polynomial of
//!   degree `k`. Each gives a column vector (below).
//! - `interp(vs, vx, vy, x)`: the value at each `x` of the curve `vs` describes, over the data it
//!   was made from.
//! - `slope(vx, vy)` and `intercept(vx, vy)`: the least-squares line.
//!
//! The coefficient vector, its elements counted from the document's index base:
//!
//! - Element 0 is the kind (1 `lspline`, 2 `pspline`, 3 `cspline`, 4 `regress`), element 1 is 3,
//!   element 2 is the degree `k` of a `regress` and 0 for a spline: plain numbers.
//! - From element 3 on, a spline has its second derivatives at each `vx` (in `vy/vx²`), and a
//!   `regress` has its coefficients `a_0 … a_k` (`a_j` in `vy/vx^j`), `a_0` first as in Mathcad.
//!   The header values are WordCraft's own.
//!
//! As the other interpolations do, a spline never extrapolates: `interp` outside `vx` is the error
//! `spline` gives. A `regress` is a formula, and evaluates anywhere. A vector that isn't one of
//! these, or that was made over data of another size or units, is refused.

use super::{Ctx, Points, arg_name, charge, count, finite, fits, items, one_dim, same_length, second_derivatives, vector};
use crate::array::{self, Data, Grid, MAX_ELEMENTS};
use crate::eval::{Val, picked};
use crate::{CalcError, Expr};
use craft_units::{Dim, Quantity, UnitError};

/// How close to dependent a column of the fit may be (relative to its own size, once the earlier
/// columns are taken out) before it is refused: every column is independent of the others, but a
/// degree needing more distinct `x` values than there are makes some dependent.
const DEPENDENT: f64 = 1e-10;

fn syntax(message: String) -> CalcError {
    CalcError::Syntax(message)
}

/// `y_dim / x_dim^power`: the dimension of a coefficient.
fn per_power(y_dim: Dim, x_dim: Dim, power: i32) -> Result<Dim, CalcError> {
    x_dim.powi(power).and_then(|d| y_dim.div(d)).ok_or(CalcError::Unit(UnitError::Overflow))
}

/// The coefficient vector with `header` and then `data`, as a column.
fn coefficients(header: [f64; 3], data: Vec<Quantity>) -> Result<Data, CalcError> {
    let head = header.into_iter().map(|v| Val::plain(Quantity::dimensionless(v)));
    let items: Vec<Val> = head.chain(data.into_iter().map(Val::plain)).collect();
    Grid::new(items.len(), 1, items).map(Data::Grid)
}

/// `cspline(vx, vy)`, `lspline(vx, vy)` or `pspline(vx, vy)`: the second derivatives of the spline
/// through the points, with the ends named in [`ends`]. `vx` must be strictly increasing, of one
/// dimension, and as long as `vy`; at least 3 points, or 2 for `lspline`.
pub(super) fn spline(name: &str, args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [xs, ys] = args else {
        return Err(CalcError::Arity(name.to_string(), 2));
    };
    let names = (arg_name(exprs, 0, "vx"), arg_name(exprs, 1, "vy"));
    // The header is the 3 results besides the points' own.
    let points = Points::new(name, (xs, ys), names, 3, cx)?;
    let (kind, least) = match name {
        "lspline" => (1.0, 2),
        "pspline" => (2.0, 3),
        _ => (3.0, 3),
    };
    if points.xv.len() < least {
        return Err(syntax(format!("{name} needs at least {least} points in {}", names.0)));
    }
    let second = match name {
        "lspline" => Some(second_derivatives(&points.xv, &points.yv)),
        "pspline" => ends(&points.xv, &points.yv, false),
        _ => ends(&points.xv, &points.yv, true),
    };
    let second = second.ok_or(CalcError::NonFinite)?;
    let dim = per_power(points.y_dim, points.x_dim, 2)?;
    let data = second.into_iter().map(|m| finite(Quantity::new(m, dim))).collect::<Result<_, _>>()?;
    coefficients([kind, 3.0, 0.0], data)
}

/// The second derivatives at every point of the cubic spline through (`x`, `y`) (`x` strictly
/// increasing, at least 3 points) that ends parabolically (`M_0 = M_1` and `M_{n-1} = M_{n-2}`) or,
/// for `cubic`, with `M` running in a straight line in `x` across the two intervals at each end
/// (Mathcad's `cspline`). Three points give the one parabola through them either way. `None` when
/// the system has no solution (it has, for any such points).
fn ends(x: &[f64], y: &[f64], cubic: bool) -> Option<Vec<f64>> {
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| if let [a, b] = w { b - a } else { 0.0 }).collect();
    let slope: Vec<f64> = y.windows(2).zip(&h).map(|(w, h)| if let [a, b] = w { (b - a) / h } else { 0.0 }).collect();
    // One row per inner point i: [below, diagonal, above, right side], for
    // h[i-1]·M[i-1] + 2(h[i-1] + h[i])·M[i] + h[i]·M[i+1] = 6(slope[i] - slope[i-1]).
    let mut rows: Vec<[f64; 4]> = h
        .windows(2)
        .zip(slope.windows(2))
        .filter_map(|(hw, sw)| match (hw, sw) {
            ([h0, h1], [s0, s1]) => Some([*h0, 2.0 * (h0 + h1), *h1, 6.0 * (s1 - s0)]),
            _ => None,
        })
        .collect();
    // The two intervals at each end, and whether the ends are cubic (not for 3 points: then the
    // two end conditions are the same one, and the parabola is the answer).
    let cubic = cubic && n > 3;
    let (h0, h1) = (h.first().copied()?, h.get(1).copied()?);
    let (ha, hb) = (h.get(n.checked_sub(3)?).copied()?, h.get(n.checked_sub(2)?).copied()?);
    // The end point's M is a multiple of M_1 (and M_2): fold it into the first row, as the
    // unknowns are the inner points' only.
    if let Some([below, diag, above, _]) = rows.first_mut() {
        if cubic {
            // (h0+h1)·M_1 = h1·M_0 + h0·M_2, so M_0 = ((h0+h1)·M_1 − h0·M_2) / h1.
            *diag += *below * (h0 + h1) / h1;
            *above -= *below * h0 / h1;
        } else {
            // M_0 = M_1.
            *diag += *below;
        }
        *below = 0.0;
    }
    if let Some([below, diag, above, _]) = rows.last_mut() {
        if cubic {
            *diag += *above * (ha + hb) / ha;
            *below -= *above * hb / ha;
        } else {
            *diag += *above;
        }
        *above = 0.0;
    }
    let inner = thomas(&rows)?;
    let (m1, m_last) = (inner.first().copied()?, inner.last().copied()?);
    let (m0, m_end) = if cubic {
        (((h0 + h1) * m1 - h0 * inner.get(1).copied()?) / h1, ((ha + hb) * m_last - hb * inner.get(inner.len().checked_sub(2)?).copied()?) / ha)
    } else {
        (m1, m_last)
    };
    Some(std::iter::once(m0).chain(inner).chain(std::iter::once(m_end)).collect())
}

/// The solution of the tridiagonal system whose rows are [below, diagonal, above, right side]
/// (Thomas algorithm: stable for the diagonally dominant systems of splines).
fn thomas(rows: &[[f64; 4]]) -> Option<Vec<f64>> {
    let (mut above, mut right) = (Vec::with_capacity(rows.len()), Vec::with_capacity(rows.len()));
    let (mut c, mut d) = (0.0, 0.0);
    for [below, diag, up, rhs] in rows {
        let w = diag - below * c;
        if w == 0.0 || !w.is_finite() {
            return None;
        }
        d = (rhs - below * d) / w;
        c = up / w;
        above.push(c);
        right.push(d);
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut next = 0.0;
    for (c, d) in above.iter().zip(&right).rev() {
        next = d - c * next;
        out.push(next);
    }
    out.reverse();
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// `regress(vx, vy, k)`: the coefficients `a_0 … a_k` of the polynomial of degree `k` that fits
/// the points best in the least-squares sense. `k` is a whole number from 1 to `n − 1`.
pub(super) fn regress(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [xs, ys, k] = args else {
        return Err(CalcError::Arity("regress".to_string(), 3));
    };
    let (xs_name, ys_name, k_name) = (arg_name(exprs, 0, "vx"), arg_name(exprs, 1, "vy"), arg_name(exprs, 2, "k"));
    let (xs, ys) = (vector("regress", xs, xs_name)?, vector("regress", ys, ys_name)?);
    same_length("regress", (xs_name, xs.len()), (ys_name, ys.len()))?;
    let n = xs.len();
    if n < 2 {
        return Err(syntax(format!("regress needs at least 2 points in {xs_name}")));
    }
    let bad_degree = || syntax(format!("regress needs {k_name} to be a whole number from 1 to {}", n.saturating_sub(1)));
    let Data::One(degree) = k else {
        return Err(bad_degree());
    };
    let k = whole(degree.q.value).filter(|&k| degree.q.dim == Dim::ONE && (1..n).contains(&k)).ok_or_else(bad_degree)?;
    // The matrix is n × (k + 1), and building it costs that times k + 1 (each column is made
    // orthogonal to the earlier ones); it is refused before it is built if it is too large.
    let columns = k.saturating_add(1);
    n.checked_mul(columns).filter(|&size| size <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    charge(cx, n.saturating_mul(columns).saturating_mul(columns).saturating_add(columns))?;
    let x_dim = one_dim(xs, xs_name, cx.origin)?;
    let y_dim = one_dim(ys, ys_name, cx.origin)?;
    let xv: Vec<f64> = xs.iter().map(|v| v.q.value).collect();
    let yv: Vec<f64> = ys.iter().map(|v| v.q.value).collect();
    let a =
        polynomial_fit(&xv, &yv, k).ok_or_else(|| syntax(format!("regress can't fit degree {k} to {xs_name}: it has too few different values")))?;
    let mut data = Vec::with_capacity(columns);
    for (j, a) in (0..).zip(a) {
        data.push(finite(Quantity::new(a, per_power(y_dim, x_dim, j)?))?);
    }
    coefficients([4.0, 3.0, f64::from(u32::try_from(k).unwrap_or(u32::MAX))], data)
}

/// `v` as a count, if it is a whole number that fits one.
fn whole(v: f64) -> Option<usize> {
    // A whole number below 2^31, so the conversions are exact.
    (v.is_finite() && v.fract() == 0.0 && (0.0..=2_147_483_647.0).contains(&v)).then_some(v as usize)
}

/// The coefficients `a_0 … a_k` of the least-squares polynomial of degree `k` through (`x`, `y`),
/// at least `k + 1` points. `x` is centered and scaled to −1…1 and the system solved by an
/// orthogonal factorization (normal equations lose half the digits, and more as `x` moves away
/// from 0); the polynomial is then turned back into powers of `x`. `None` when the points have too
/// few different `x` values for the degree.
fn polynomial_fit(x: &[f64], y: &[f64], k: usize) -> Option<Vec<f64>> {
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    let scale = x.iter().map(|v| (v - mean).abs()).fold(0.0, f64::max);
    if scale == 0.0 || !scale.is_finite() {
        return None;
    }
    let u: Vec<f64> = x.iter().map(|v| (v - mean) / scale).collect();
    // The columns 1, u, u² … u^k.
    let mut columns: Vec<Vec<f64>> = vec![vec![1.0; u.len()]];
    for _ in 0..k {
        let next = columns.last()?.iter().zip(&u).map(|(p, u)| p * u).collect();
        columns.push(next);
    }
    let c = least_squares(&columns, y)?;
    // Σ c_i ((x − mean)/scale)^i as powers of x: add each term's expansion in turn, the powers of
    // (x − mean) built up one factor at a time.
    let mut a = vec![0.0; k + 1];
    let mut power = vec![1.0];
    for (i, ci) in (0..).zip(&c) {
        if i > 0 {
            let shifted = std::iter::once(0.0).chain(power.iter().copied());
            let same = power.iter().copied().chain(std::iter::once(0.0));
            power = shifted.zip(same).map(|(up, same)| up - mean * same).collect();
        }
        let weight = ci / scale.powi(i);
        a.iter_mut().zip(&power).for_each(|(a, p)| *a += weight * p);
    }
    a.iter().all(|v| v.is_finite()).then_some(a)
}

/// The `c` minimizing `|Σ c_j · columns[j] − y|`: each column made orthogonal to the earlier ones
/// (twice over, which keeps them orthogonal to rounding), then the triangular system solved.
/// `None` for a column that is (nearly) a combination of the earlier ones.
fn least_squares(columns: &[Vec<f64>], y: &[f64]) -> Option<Vec<f64>> {
    let norm = |v: &[f64]| v.iter().map(|a| a * a).sum::<f64>().sqrt();
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    // The orthonormal columns, and the triangle: `triangle[j]` is column j's weights on columns 0…j.
    let mut q: Vec<Vec<f64>> = Vec::with_capacity(columns.len());
    let mut triangle: Vec<Vec<f64>> = Vec::with_capacity(columns.len());
    for column in columns {
        let size = norm(column);
        let mut v = column.clone();
        let mut weights = vec![0.0; q.len() + 1];
        for _ in 0..2 {
            for (qi, w) in q.iter().zip(&mut weights) {
                let d = dot(qi, &v);
                v.iter_mut().zip(qi).for_each(|(a, b)| *a -= d * b);
                *w += d;
            }
        }
        let rest = norm(&v);
        // Also false for NaN.
        if rest.partial_cmp(&(DEPENDENT * size)) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        *weights.last_mut()? = rest;
        q.push(v.iter().map(|a| a / rest).collect());
        triangle.push(weights);
    }
    // R c = Qᵀy, from the last unknown up.
    let at = |t: &[Vec<f64>], col: usize, row: usize| t.get(col).and_then(|c| c.get(row)).copied();
    let b: Vec<f64> = q.iter().map(|qj| dot(qj, y)).collect();
    let mut c = vec![0.0; q.len()];
    for j in (0..q.len()).rev() {
        let mut tail = 0.0;
        for later in j + 1..q.len() {
            tail += at(&triangle, later, j)? * c.get(later).copied()?;
        }
        *c.get_mut(j)? = (b.get(j).copied()? - tail) / at(&triangle, j, j)?;
    }
    c.iter().all(|v| v.is_finite()).then_some(c)
}

/// `slope(vx, vy)` or `intercept(vx, vy)`: the slope (in `vy/vx`) or the intercept (in `vy`) of the
/// least-squares line through the points. `vx` needs at least 2 different values.
pub(super) fn line(name: &str, args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [xs, ys] = args else {
        return Err(CalcError::Arity(name.to_string(), 2));
    };
    let (xs_name, ys_name) = (arg_name(exprs, 0, "vx"), arg_name(exprs, 1, "vy"));
    let (xs, ys) = (vector(name, xs, xs_name)?, vector(name, ys, ys_name)?);
    same_length(name, (xs_name, xs.len()), (ys_name, ys.len()))?;
    if xs.len() < 2 {
        return Err(syntax(format!("{name} needs at least 2 points in {xs_name}")));
    }
    charge(cx, xs.len())?;
    let x_dim = one_dim(xs, xs_name, cx.origin)?;
    let y_dim = one_dim(ys, ys_name, cx.origin)?;
    let n = xs.len() as f64;
    let (mean_x, mean_y) = (xs.iter().map(|v| v.q.value).sum::<f64>() / n, ys.iter().map(|v| v.q.value).sum::<f64>() / n);
    let (mut sxx, mut sxy) = (0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        let dx = x.q.value - mean_x;
        sxx += dx * dx;
        sxy += dx * (y.q.value - mean_y);
    }
    if sxx == 0.0 || !sxx.is_finite() {
        return Err(syntax(format!("{name} needs {xs_name} to have more than one different value")));
    }
    let slope = sxy / sxx;
    let q = if name == "slope" { Quantity::new(slope, per_power(y_dim, x_dim, 1)?) } else { Quantity::new(mean_y - slope * mean_x, y_dim) };
    Ok(Data::One(Val::plain(finite(q)?)))
}

/// `interp(vs, vx, vy, x)`: the value at each element of `x` of the curve `vs` (from `cspline`,
/// `lspline`, `pspline` or `regress`) describes, over the data `vx` and `vy` it was made from.
/// A spline is refused outside `vx`, as `spline` is; a `regress` is a polynomial, and goes on.
pub(super) fn interp(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [vs, xs, ys, x] = args else {
        return Err(CalcError::Arity("interp".to_string(), 4));
    };
    let (vs_name, xs_name, ys_name) = (arg_name(exprs, 0, "vs"), arg_name(exprs, 1, "vx"), arg_name(exprs, 2, "vy"));
    let foreign =
        |why: String| syntax(format!("interp needs {vs_name} from cspline, lspline, pspline or regress over {xs_name} and {ys_name} — {why}"));
    let curve = vector("interp", vs, vs_name)?;
    // The header: the kind, 3, and the degree, all plain numbers.
    let [kind, mark, degree, data @ ..] = curve else {
        let n = curve.len();
        return Err(foreign(format!("it has {n} element{}, not even a header", if n == 1 { "" } else { "s" })));
    };
    if [kind, mark, degree].iter().any(|v| v.q.dim != Dim::ONE) || mark.q.value != 3.0 {
        return Err(foreign("it isn't one".to_string()));
    }
    let Some(kind) = (1u8..=4).find(|&k| f64::from(k) == kind.q.value) else {
        return Err(foreign("it isn't one".to_string()));
    };
    let n = count(xs);
    if kind < 4 {
        if degree.q.value != 0.0 {
            return Err(foreign("it isn't one".to_string()));
        }
        if data.len() != n {
            return Err(foreign(format!("it has {} points, and {xs_name} has {n}", data.len())));
        }
        let points = Points::new("interp", (xs, ys), (xs_name, ys_name), items(x).len(), cx)?;
        let dim = per_power(points.y_dim, points.x_dim, 2)?;
        if data.iter().any(|v| !v.q.dim.same_ignoring_angle(dim)) {
            return Err(foreign("its units aren't those of the data".to_string()));
        }
        let second: Vec<f64> = data.iter().map(|v| v.q.value).collect();
        return points.at(x, &second, cx);
    }
    let k = whole(degree.q.value).filter(|&k| k >= 1 && data.len() == k + 1).ok_or_else(|| foreign("it isn't one".to_string()))?;
    let (xv, yv) = (vector("interp", xs, xs_name)?, vector("interp", ys, ys_name)?);
    same_length("interp", (xs_name, xv.len()), (ys_name, yv.len()))?;
    if xv.len() <= k {
        return Err(foreign(format!("a fit of degree {k} needs more than {k} points, and {xs_name} has {}", xv.len())));
    }
    // Each element of `x` is a polynomial of k + 1 terms.
    charge(cx, xv.len().saturating_add(data.len()).saturating_add(items(x).len().saturating_mul(k + 1)))?;
    let x_dim = one_dim(xv, xs_name, cx.origin)?;
    let y_dim = one_dim(yv, ys_name, cx.origin)?;
    for (j, a) in (0..).zip(data) {
        if !a.q.dim.same_ignoring_angle(per_power(y_dim, x_dim, j)?) {
            return Err(foreign("its units aren't those of the data".to_string()));
        }
    }
    let t = picked(yv);
    array::map(x, cx.origin, cx.lenient, |x| {
        fits(x.q, x_dim)?;
        // Horner's rule, from the highest power.
        let value = data.iter().rev().fold(0.0, |acc, a| acc * x.q.value + a.q.value);
        Ok(Val { q: finite(Quantity::new(value, y_dim))?, t })
    })
}
