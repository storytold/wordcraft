//! The numerical core of `Given`/`Find`/`Minerr`: solves a small system of equations by
//! Levenberg–Marquardt with a forward-difference Jacobian.
//!
//! It works on plain `f64` in SI base units and knows nothing about units, names or the parser; the
//! evaluator converts at its boundary. The systems are small and dense (a handful of unknowns), so
//! the damped normal equations are solved by Gaussian elimination.

use crate::CalcError;

/// What to do with the equations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Solve them exactly: every equation must hold at the answer.
    Find,
    /// Get as close as possible: minimize the sum of squared differences.
    Minerr,
}

/// The unknowns' values, in the order of the starting guesses.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Solution {
    pub(crate) x: Vec<f64>,
}

/// Why no solution came back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Failure {
    /// Converged, but some equation still doesn't hold (Find only): the largest |lhs − rhs| over
    /// the equation's scale at the answer ([`HOLDS`]).
    NoSolution { worst: f64 },
    /// Hit MAX_SOLVE_STEPS.
    NotConverged { steps: u32 },
    /// An equation isn't a finite number at the guess, or near it where the Jacobian is measured, so
    /// the solve can't go on (Find and Minerr alike).
    NonFinite,
}

/// The step limit and the "holds" tolerance; the guide states both.
///
/// An equation holds at the answer when `|lhs − rhs| ≤ HOLDS · scale`, its scale there being
/// `max(|lhs|, |rhs|, Σⱼ |∂(lhs − rhs)/∂xⱼ| · sizeⱼ)`: the sides' own size, or what a change of
/// the unknowns by their own size does to the difference, whichever is larger. An unknown's size is
/// its size at the answer, `|xⱼ|`, but at least `HOLDS · |x0ⱼ|` (`HOLDS` when the guess is 0), so
/// an unknown that has gone to 0 is measured against its guess. It is judged at the answer only,
/// never at the guess or a point passed through, so a difference that is small only next to huge
/// sides (or a huge guess) doesn't hold. Sides that are exactly equal hold whatever the scale (so a
/// scale of 0 never divides 0 by 0). And where the solve stopped because no move reduces the differences
/// at all (a least-squares minimum), only the sides' own size counts: moving can't help there.
pub(crate) const MAX_SOLVE_STEPS: u32 = 200;
pub(crate) const HOLDS: f64 = 1e-10;

/// The damping before the first step, and the point past which no damping can help. Damping is
/// relative: unknown `k` gets this times its own diagonal entry of `JᵀJ` added (Marquardt), so the
/// solve behaves the same whatever the magnitudes of the quantities (Minerr's residuals are raw SI).
const LAMBDA_START: f64 = 1e-3;
const LAMBDA_STUCK: f64 = 1e16;
/// An unknown's gradient this small relative to its column of `J` times `‖r‖` is stationary.
const GRADIENT_FLAT: f64 = 1e-15;
/// The smallest damping weight, relative to the largest diagonal entry of `JᵀJ`.
const DIAG_FLOOR: f64 = 1e-30;
/// A step this small, relative to the unknowns, has reached the limit of double precision.
const STEP_TINY: f64 = 1e-15;
/// The relative size of the forward-difference step: the square root of the machine epsilon.
const DIFF_STEP: f64 = 1.490_116_119_384_765_6e-8;

/// One equation's two sides at some point.
type Sides = Vec<(f64, f64)>;

/// How the iteration scales the residuals: by the larger of each equation's sides at the guess and
/// at the current point (1 if all are 0), so the scale tracks the iterate and the guess, never a
/// point passed through on the way. This only steers the iteration; whether an equation holds is
/// judged at the answer alone ([`HOLDS`]).
struct Scales {
    mode: Mode,
    /// The largest |side| of each equation at the guess.
    first: Vec<f64>,
    /// The largest |side| of each equation at the current point.
    current: Vec<f64>,
}

impl Scales {
    fn new(mode: Mode, sides: &Sides) -> Scales {
        let first: Vec<f64> = sides.iter().map(|&(l, r)| l.abs().max(r.abs())).collect();
        Scales { mode, current: first.clone(), first }
    }

    /// No scaling at all: the raw differences of the sides.
    fn raw() -> Scales {
        Scales { mode: Mode::Minerr, first: Vec::new(), current: Vec::new() }
    }

    /// Moves the current point to the one where the sides are `sides`.
    fn note(&mut self, sides: &Sides) {
        self.current = sides.iter().map(|&(l, r)| l.abs().max(r.abs())).collect();
    }

    /// Equation `i`'s scale: Find divides by it; Minerr works on the raw difference.
    fn of(&self, i: usize) -> f64 {
        match self.mode {
            Mode::Minerr => 1.0,
            Mode::Find => {
                let scale = self.first.get(i).copied().unwrap_or(0.0).max(self.current.get(i).copied().unwrap_or(0.0));
                if scale > 0.0 { scale } else { 1.0 }
            }
        }
    }

    fn residuals(&self, sides: &Sides) -> Vec<f64> {
        sides.iter().enumerate().map(|(i, &(l, r))| (l - r) / self.of(i)).collect()
    }
}

/// `eq(x)` gives each equation's two sides, in SI, at `x` (also SI). An `Err` from `eq` stops the
/// solve and is returned as is.
///
/// Unknown `j` is scaled by `|x0[j]|` (by 1 when that is 0) so a guess of zero works. Every trial
/// step counts toward [`MAX_SOLVE_STEPS`]; with `n` unknowns `eq` is called at most
/// `(MAX_SOLVE_STEPS + 1)·(n + 1) + n` times (the last `n` measure the Jacobian at the answer when
/// Find's check needs it).
#[cfg(test)]
pub(crate) fn solve(
    mode: Mode,
    x0: &[f64],
    eq: &mut dyn FnMut(&[f64]) -> Result<Vec<(f64, f64)>, CalcError>,
) -> Result<Result<Solution, Failure>, CalcError> {
    solve_charged(mode, x0, eq, &mut |_| Ok(()))
}

/// [`solve`], telling `work` how many steps of arithmetic each Jacobian costs besides the calls of
/// `eq`: `n · (n + m)` for `n` unknowns and `m` equations (the matrix products and the elimination
/// are cubic in the unknowns). An `Err` from `work` stops the solve and is returned as is.
pub(crate) fn solve_charged(
    mode: Mode,
    x0: &[f64],
    eq: &mut dyn FnMut(&[f64]) -> Result<Vec<(f64, f64)>, CalcError>,
    work: &mut dyn FnMut(u64) -> Result<(), CalcError>,
) -> Result<Result<Solution, Failure>, CalcError> {
    solve_capped(mode, x0, eq, work, MAX_SOLVE_STEPS)
}

/// [`solve_charged`] with at most `cap` steps. At the cap, Find returns the point reached if every
/// equation holds there; otherwise, and always for Minerr, the solve hasn't converged.
pub(super) fn solve_capped(
    mode: Mode,
    x0: &[f64],
    eq: &mut dyn FnMut(&[f64]) -> Result<Vec<(f64, f64)>, CalcError>,
    work: &mut dyn FnMut(u64) -> Result<(), CalcError>,
    cap: u32,
) -> Result<Result<Solution, Failure>, CalcError> {
    let Some(mut sides) = evaluate(eq, x0, None)? else {
        return Ok(Err(Failure::NonFinite));
    };
    let unit: Vec<f64> = x0.iter().map(|&v| if v == 0.0 { 1.0 } else { v.abs() }).collect();
    let mut scales = Scales::new(mode, &sides);
    let mut x = x0.to_vec();
    let (mut lambda, mut nu) = (LAMBDA_START, 2.0);
    let mut steps = 0u32;
    // Stopped because no move of the unknowns reduces the differences at all: a least-squares
    // minimum, which is a solution only if the sides agree by themselves.
    let mut stationary = false;
    'outer: loop {
        // Every equation holds exactly: done, with no Jacobian to measure (an exact guess next to
        // a domain edge, as `√(−x) == 0` from 0, has none).
        if sides.iter().all(|&(l, r)| l == r) {
            break;
        }
        let r = scales.residuals(&sides);
        let cost = half_square_sum(&r);
        let count = |len: usize| u64::try_from(len).unwrap_or(u64::MAX);
        let (n, m) = (count(x.len()), count(sides.len()));
        work(n.saturating_mul(n.saturating_add(m)))?;
        let Some(jac) = jacobian(eq, &x, &unit, &sides, &scales)? else {
            return Ok(Err(Failure::NonFinite));
        };
        let grad = jt_times(&jac, &r, x.len());
        let jtj = normal_matrix(&jac, x.len());
        let diag: Vec<f64> = (0..x.len()).map(|k| jtj[k * x.len() + k]).collect();
        // Flat when every unknown's gradient is negligible against its own column of J, so an
        // unknown of a tiny magnitude isn't judged by the size of the others.
        let r_norm = norm(&r);
        let flat = |j: usize, g: f64| g.abs() <= GRADIENT_FLAT * diag[j].sqrt() * r_norm;
        if cost == 0.0 {
            break;
        }
        if grad.iter().enumerate().all(|(j, &g)| flat(j, g)) {
            stationary = true;
            break;
        }
        // Marquardt damping: each unknown's own diagonal entry (floored), so unknowns of very
        // different magnitudes are each damped in proportion to their own sensitivity.
        let floor = DIAG_FLOOR * diag.iter().copied().fold(0.0, f64::max);
        let diag: Vec<f64> = diag.into_iter().map(|d| d.max(floor)).collect();
        loop {
            if steps >= cap {
                let held = mode == Mode::Find && finish(mode, &x, &sides, &unit, false, eq)?.is_ok();
                return Ok(if held { Ok(Solution { x }) } else { Err(Failure::NotConverged { steps }) });
            }
            if lambda > LAMBDA_STUCK {
                break 'outer;
            }
            steps += 1;
            let trial = damped_step(&jtj, &grad, lambda, &diag).and_then(|d| {
                let moved: Vec<f64> = x.iter().zip(&d).zip(&unit).map(|((&xj, &dj), &uj)| xj + uj * dj).collect();
                moved.iter().all(|v| v.is_finite()).then_some((d, moved))
            });
            let Some((d, moved)) = trial else {
                (lambda, nu) = (lambda * nu, nu * 2.0);
                continue;
            };
            let Some(new_sides) = evaluate(eq, &moved, Some(sides.len()))? else {
                (lambda, nu) = (lambda * nu, nu * 2.0);
                continue;
            };
            let predicted = 0.5 * d.iter().zip(&grad).zip(&diag).map(|((&dj, &gj), &dk)| dj * (lambda * dk * dj - gj)).sum::<f64>();
            let rho = cost_drop(&r, &scales.residuals(&new_sides)) / predicted;
            if rho.is_nan() || rho <= 0.0 {
                (lambda, nu) = (lambda * nu, nu * 2.0);
                continue;
            }
            lambda *= (1.0 - (2.0 * rho - 1.0).powi(3)).max(1.0 / 3.0);
            nu = 2.0;
            let tiny = norm(&d) <= STEP_TINY * norm_scaled(&x, &unit);
            x = moved;
            scales.note(&new_sides);
            sides = new_sides;
            if tiny {
                break 'outer;
            }
            continue 'outer;
        }
    }
    finish(mode, &x, &sides, &unit, stationary, eq)
}

/// The outcome once the iteration has stopped at `x`, where the equations' sides are `sides`: for
/// Find, a solution only if every equation holds there ([`HOLDS`]).
///
/// The sides' own size is tried as the scale first: the full scale is never smaller, so an equation
/// that holds against it holds. Only when that fails, and the solve didn't stop at a `stationary`
/// point, is the Jacobian measured at `x` (`n` more calls of `eq`, each unknown moved by a step
/// relative to its size there) for the full scale; if it can't be measured there, the answer
/// doesn't hold.
fn finish(
    mode: Mode,
    x: &[f64],
    sides: &Sides,
    unit: &[f64],
    stationary: bool,
    eq: &mut dyn FnMut(&[f64]) -> Result<Sides, CalcError>,
) -> Result<Result<Solution, Failure>, CalcError> {
    let solution = Ok(Solution { x: x.to_vec() });
    if mode == Mode::Minerr {
        return Ok(solution);
    }
    let sizes = worst_miss(sides, None);
    if sizes <= HOLDS {
        return Ok(solution);
    }
    if stationary {
        return Ok(Err(Failure::NoSolution { worst: sizes }));
    }
    // Each unknown's size at the answer, at least HOLDS of its guess's (`unit`).
    let size: Vec<f64> = x.iter().zip(unit).map(|(&v, &u)| v.abs().max(HOLDS * u)).collect();
    let Some(slopes) = jacobian(eq, x, &size, sides, &Scales::raw())? else {
        return Ok(Err(Failure::NoSolution { worst: sizes }));
    };
    let worst = worst_miss(sides, Some(&slopes));
    Ok(if worst <= HOLDS { solution } else { Err(Failure::NoSolution { worst }) })
}

/// The largest `|lhs − rhs| / scale` over the equations. The scale is the larger side, or with
/// `slopes` (the Jacobian of `lhs − rhs` with respect to the scaled unknowns, row-major, one row per
/// equation) also the row's sum of absolute slopes ([`HOLDS`]). Sides that are equal miss by 0,
/// whatever their scale.
fn worst_miss(sides: &Sides, slopes: Option<&[f64]>) -> f64 {
    let n = slopes.map_or(0, |s| s.len() / sides.len().max(1));
    let worst = |(i, &(l, r)): (usize, &(f64, f64))| {
        let miss = (l - r).abs();
        if miss == 0.0 {
            return 0.0;
        }
        let moved = slopes.and_then(|s| s.get(i * n..(i + 1) * n)).map_or(0.0, |row| row.iter().map(|v| v.abs()).sum::<f64>());
        miss / l.abs().max(r.abs()).max(moved)
    };
    sides.iter().enumerate().map(worst).fold(0.0, f64::max)
}

/// Calls `eq` at `x`. `None` when a side isn't finite, or (given `len`) there aren't `len` equations.
fn evaluate(eq: &mut dyn FnMut(&[f64]) -> Result<Sides, CalcError>, x: &[f64], len: Option<usize>) -> Result<Option<Sides>, CalcError> {
    let sides = eq(x)?;
    let ok = len.is_none_or(|n| n == sides.len()) && sides.iter().all(|&(l, r)| l.is_finite() && r.is_finite());
    Ok(ok.then_some(sides))
}

/// The Jacobian of the scaled residuals with respect to the scaled unknowns, row by row, by forward
/// differences. `None` when a difference isn't finite. The scales stay fixed while it is built.
fn jacobian(
    eq: &mut dyn FnMut(&[f64]) -> Result<Sides, CalcError>,
    x: &[f64],
    unit: &[f64],
    sides: &Sides,
    scales: &Scales,
) -> Result<Option<Vec<f64>>, CalcError> {
    let (m, n) = (sides.len(), x.len());
    let base = scales.residuals(sides);
    let mut jac = vec![0.0; m * n];
    for j in 0..n {
        let mut moved = x.to_vec();
        let h = DIFF_STEP * x[j].abs().max(unit[j]);
        moved[j] = x[j] + h;
        let actual = (moved[j] - x[j]) / unit[j];
        let Some(shifted) = evaluate(eq, &moved, Some(m))? else {
            return Ok(None);
        };
        for (i, (new, old)) in scales.residuals(&shifted).into_iter().zip(&base).enumerate() {
            jac[i * n + j] = (new - old) / actual;
        }
    }
    Ok(jac.iter().all(|v| v.is_finite()).then_some(jac))
}

/// `Jᵀ·r` for the `m × n` row-major `jac`.
fn jt_times(jac: &[f64], r: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    for (row, &ri) in jac.chunks_exact(n.max(1)).zip(r) {
        for (o, &a) in out.iter_mut().zip(row) {
            *o += a * ri;
        }
    }
    out
}

/// `JᵀJ`, row-major `n × n`.
fn normal_matrix(jac: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n * n];
    for row in jac.chunks_exact(n.max(1)) {
        for (a, &ra) in row.iter().enumerate() {
            for (b, &rb) in row.iter().enumerate() {
                out[a * n + b] += ra * rb;
            }
        }
    }
    out
}

/// Solves `(JᵀJ + λ·diag(diag))·d = −g`. `None` when the system is singular or the answer isn't finite.
fn damped_step(jtj: &[f64], grad: &[f64], lambda: f64, diag: &[f64]) -> Option<Vec<f64>> {
    let n = grad.len();
    let mut a = jtj.to_vec();
    for k in 0..n {
        a[k * n + k] += lambda * diag[k];
    }
    let mut b: Vec<f64> = grad.iter().map(|g| -g).collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|&p, &q| a[p * n + col].abs().total_cmp(&a[q * n + col].abs()))?;
        let diag = a[pivot * n + col];
        if diag == 0.0 || !diag.is_finite() {
            return None;
        }
        if pivot != col {
            for c in 0..n {
                a.swap(col * n + c, pivot * n + c);
            }
            b.swap(col, pivot);
        }
        for row in col + 1..n {
            let factor = a[row * n + col] / a[col * n + col];
            for c in col..n {
                a[row * n + c] -= factor * a[col * n + c];
            }
            b[row] -= factor * b[col];
        }
    }
    for row in (0..n).rev() {
        let tail: f64 = (row + 1..n).map(|c| a[row * n + c] * b[c]).sum();
        b[row] = (b[row] - tail) / a[row * n + row];
    }
    b.iter().all(|v| v.is_finite()).then_some(b)
}

fn half_square_sum(r: &[f64]) -> f64 {
    0.5 * r.iter().map(|v| v * v).sum::<f64>()
}

/// `½‖before‖² − ½‖after‖²`, computed as a sum of `(a − b)(a + b)` terms so a tiny drop isn't lost
/// to the rounding of two nearly equal totals.
fn cost_drop(before: &[f64], after: &[f64]) -> f64 {
    0.5 * before.iter().zip(after).map(|(&a, &b)| (a - b) * (a + b)).sum::<f64>()
}

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|a| a * a).sum::<f64>().sqrt()
}

/// The length of `x` in scaled units (each unknown divided by its scale).
fn norm_scaled(x: &[f64], unit: &[f64]) -> f64 {
    norm(&x.iter().zip(unit).map(|(&v, &u)| v / u).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests;
