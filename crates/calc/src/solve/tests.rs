// Mathcad's saved results are transcribed with all their digits.
#![allow(clippy::excessive_precision)]

use super::{Failure, HOLDS, MAX_SOLVE_STEPS, Mode, Solution, solve, solve_capped};
use crate::CalcError;
use std::f64::consts::PI;

type Sides = Result<Vec<(f64, f64)>, CalcError>;

/// Mathcad's three significant figures: `|a − b| ≤ 0.5·10^(floor(log10|b|) − 2)`.
fn sig3(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5 * 10f64.powi(b.abs().log10().floor() as i32 - 2)
}

fn solved(mode: Mode, x0: &[f64], mut eq: impl FnMut(&[f64]) -> Sides) -> Solution {
    match solve(mode, x0, &mut eq) {
        Ok(Ok(s)) => s,
        other => panic!("expected a solution, got {other:?}"),
    }
}

/// Every equation holds at `x`: its sides differ by at most 1e-9 of the larger one (or of 1).
fn assert_holds(eq: &mut dyn FnMut(&[f64]) -> Sides, x: &[f64]) {
    for (i, (l, r)) in eq(x).expect("eq").into_iter().enumerate() {
        assert!((l - r).abs() <= 1e-9 * l.abs().max(r.abs()).max(1.0), "equation {i} off at {x:?}: {l} vs {r}");
    }
}

fn one(f: impl Fn(f64) -> (f64, f64)) -> impl FnMut(&[f64]) -> Sides {
    move |x: &[f64]| Ok(vec![f(x[0])])
}

#[test]
fn one_unknown() {
    let mut eq = one(|x| (x * x, 2.0));
    let s = solved(Mode::Find, &[1.0], &mut eq);
    assert!((s.x[0] - 2f64.sqrt()).abs() <= 1e-12 * 2f64.sqrt(), "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);
}

#[test]
fn zero_guess() {
    let mut eq = one(|x| (x, 3.0));
    let s = solved(Mode::Find, &[0.0], &mut eq);
    assert!((s.x[0] - 3.0).abs() <= 1e-12 * 3.0, "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);

    let mut eq = one(|x| (x * x, 4.0));
    match solve(Mode::Find, &[0.0], &mut eq) {
        Ok(Ok(s)) => {
            assert!((s.x[0].abs() - 2.0).abs() <= 1e-9, "{}", s.x[0]);
            assert_holds(&mut eq, &s.x);
        }
        Ok(Err(Failure::NoSolution { worst })) => assert!(!worst.is_nan()),
        Ok(Err(_)) | Err(_) => {}
    }

    let mut eq = one(|x| (x, 1e-6));
    let s = solved(Mode::Find, &[0.0], &mut eq);
    assert!((s.x[0] - 1e-6).abs() <= 1e-12 * 1e-6, "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);
}

const G: f64 = 32.174048556430442;
const F: f64 = 0.03;
const LENGTHS: [f64; 3] = [800.0, 600.0, 700.0]; // L12, L24, L23
const DIAMETERS: [f64; 3] = [1.0, 1.2, 2.1]; // d12, d24, d23

/// Head loss `f·L/d·V|V|/(2g)` for pipe `p`.
fn loss(p: usize, v: f64) -> f64 {
    F * LENGTHS[p] / DIAMETERS[p] * v * v.abs() / (2.0 * G)
}

fn area(p: usize) -> f64 {
    PI * DIAMETERS[p] * DIAMETERS[p] / 4.0
}

fn reservoirs(x: &[f64]) -> Sides {
    let (v12, v23, v24) = (x[0], x[1], x[2]);
    let (z1, z3, z4) = (838.0, 850.0, 805.0);
    Ok(vec![(z1 - loss(0, v12) - loss(1, v24), z4), (z1 - loss(0, v12) + loss(2, v23), z3), (area(0) * v12 + area(2) * v23 - area(1) * v24, 0.0)])
}

#[test]
fn three_reservoirs() {
    let gpm = 448.83116883;
    let s = solved(Mode::Find, &[3.0, 5.0, 5.0], reservoirs);
    assert_holds(&mut reservoirs, &s.x);
    let q12 = area(0) * s.x[0] * gpm;
    let q24 = area(1) * s.x[2] * gpm;
    let q23 = area(2) * s.x[1] * gpm;
    assert!(sig3(q12, -1589.5412631603367), "{q12}");
    assert!(sig3(q24, 6697.8072260949511), "{q24}");
    assert!(sig3(q23, 8287.3484892554116), "{q23}");
}

#[test]
fn colebrook() {
    let (re, e) = (955288.76846737019, 0.00022553564716200979);
    let mut eq = one(|f: f64| (1.0 / f.sqrt(), -2.0 * (e / 3.7 + 2.51 / (re * f.sqrt())).log10()));
    let s = solved(Mode::Find, &[0.1], &mut eq);
    let want = 0.014988971068715593;
    assert!(sig3(s.x[0], want), "{}", s.x[0]);
    assert!((s.x[0] - want).abs() <= 1e-12 * want, "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);
}

#[test]
fn find_no_solution() {
    let mut eq = one(|x| (x * x, -1.0));
    match solve(Mode::Find, &[1.0], &mut eq) {
        Ok(Err(Failure::NoSolution { worst })) => assert!(worst > HOLDS, "{worst}"),
        other => panic!("expected NoSolution, got {other:?}"),
    }
}

#[test]
fn minerr_closest() {
    let s = solved(Mode::Minerr, &[1.0], one(|x| (x * x, -1.0)));
    assert!(s.x[0].abs() < 1e-6, "{}", s.x[0]);

    let s = solved(Mode::Minerr, &[0.0], |x: &[f64]| Ok(vec![(x[0], 1.0), (x[0], 3.0)]));
    assert!((s.x[0] - 2.0).abs() <= 1e-12 * 2.0, "{}", s.x[0]);
}

#[test]
fn underdetermined_nearest() {
    let mut eq = |x: &[f64]| Ok(vec![(x[0] + x[1], 1.0)]);
    let s = solved(Mode::Find, &[0.0, 0.0], &mut eq);
    assert!((s.x[0] + s.x[1] - 1.0).abs() <= 1e-12, "{:?}", s.x);
    assert!((s.x[0] - 0.5).abs() < 1e-6 && (s.x[1] - 0.5).abs() < 1e-6, "{:?}", s.x);
    assert_holds(&mut eq, &s.x);
}

#[test]
fn holds_with_zero_side() {
    let mut eq = one(|x| (x - 3.0, 0.0));
    let s = solved(Mode::Find, &[1.0], &mut eq);
    assert!((s.x[0] - 3.0).abs() <= 1e-9, "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);
}

#[test]
fn step_cap() {
    let mut calls = 0u64;
    let mut eq = |x: &[f64]| {
        calls += 1;
        Ok(vec![((1.0 / x[0]).sin(), 2.0)])
    };
    let out = solve(Mode::Find, &[0.01], &mut eq);
    assert!(matches!(out, Ok(Err(Failure::NoSolution { .. } | Failure::NotConverged { .. }))), "{out:?}");
    // At most (MAX_SOLVE_STEPS + 1)·(n + 1) + n calls, n = 1.
    assert!(calls <= (u64::from(MAX_SOLVE_STEPS) + 1) * 2 + 1, "{calls} calls");
}

#[test]
fn step_cap_counts_every_unknown() {
    // A rough surface in three unknowns: the bound is (MAX_SOLVE_STEPS + 1)·(n + 1).
    let mut calls = 0u64;
    let mut eq = |x: &[f64]| {
        calls += 1;
        Ok(vec![((1.0 / x[0]).sin() + x[1] * x[2], 2.0), (x[1].cos() * (1.0 / x[2]).sin(), 3.0)])
    };
    let out = solve(Mode::Minerr, &[0.01, 1.0, 0.02], &mut eq);
    assert_eq!(out, Ok(Err(Failure::NotConverged { steps: MAX_SOLVE_STEPS })));
    assert!(calls <= (u64::from(MAX_SOLVE_STEPS) + 1) * 4, "{calls} calls");
}

#[test]
fn eq_error_stops() {
    let mut calls = 0;
    let mut eq = |x: &[f64]| {
        calls += 1;
        if calls == 3 { Err(CalcError::TooMuchWork) } else { Ok(vec![(x[0] * x[0], 2.0)]) }
    };
    assert_eq!(solve(Mode::Find, &[1.0], &mut eq).unwrap_err(), CalcError::TooMuchWork);
}

#[test]
fn find_scale_ignores_points_passed_through() {
    // A narrow bump of height 1e4 sits at 1/1.001, on the way from the guess to the root at 1. The
    // sides must be judged against the guess and the final point, not against the bump: a result
    // that is off by 3e-8 of 1 doesn't hold.
    let bump = |x: f64| 1e4 * (-((x - 1.0 / 1.001) / 1e-5).powi(2)).exp();
    let mut eq = one(|x| (x + bump(x), 1.0 + bump(x)));
    let out = solve(Mode::Find, &[0.0], &mut eq);
    match out {
        Ok(Ok(s)) => {
            assert!((s.x[0] - 1.0).abs() <= 1e-9, "{}", s.x[0]);
            assert_holds(&mut eq, &s.x);
        }
        Ok(Err(Failure::NoSolution { worst })) => assert!(worst > HOLDS, "{worst}"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn minerr_small_quantities() {
    // Residuals of 1e-12 SI are as real as residuals of 1.
    let s = solved(Mode::Minerr, &[1e-12], |x: &[f64]| Ok(vec![(x[0], 2e-12), (x[0], 4e-12)]));
    assert!((s.x[0] - 3e-12).abs() <= 1e-9 * 3e-12, "{}", s.x[0]);

    let s = solved(Mode::Minerr, &[1e-12], one(|x| (x, 2e-12)));
    assert!((s.x[0] - 2e-12).abs() <= 1e-9 * 2e-12, "{}", s.x[0]);

    let s = solved(Mode::Minerr, &[1e-6], one(|x| (x * x, 2e-12)));
    let want = 2e-12f64.sqrt();
    assert!((s.x[0] - want).abs() <= 1e-9 * want, "{}", s.x[0]);
}

#[test]
fn non_finite_at_the_guess() {
    for mode in [Mode::Find, Mode::Minerr] {
        // 1/x is infinite at 0.
        let out = solve(mode, &[0.0], &mut one(|x| (1.0 / x, 2.0)));
        assert_eq!(out, Ok(Err(Failure::NonFinite)), "{mode:?}");
        let out = solve(mode, &[1.0], &mut one(|x| (x, f64::NAN)));
        assert_eq!(out, Ok(Err(Failure::NonFinite)), "{mode:?}");
    }
}

#[test]
fn non_finite_where_the_jacobian_is_measured() {
    // Finite at the guess, NaN just above it.
    for mode in [Mode::Find, Mode::Minerr] {
        let out = solve(mode, &[1.0], &mut one(|x| (if x > 1.0 { f64::NAN } else { x }, 3.0)));
        assert_eq!(out, Ok(Err(Failure::NonFinite)), "{mode:?}");
    }
}

#[test]
fn minerr_mixed_magnitudes() {
    let close = |got: &[f64], want: &[f64]| {
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() <= 1e-9 * w.abs(), "{got:?} vs {want:?}");
        }
    };
    let s = solved(Mode::Minerr, &[1e-12, 1e3], |x: &[f64]| Ok(vec![(x[0], 2e-12), (x[1], 5e3)]));
    close(&s.x, &[2e-12, 5e3]);

    let s = solved(Mode::Minerr, &[1e-12, 1e3], |x: &[f64]| Ok(vec![(x[0] * x[1], 1e-8), (x[1], 5e3)]));
    close(&s.x, &[2e-12, 5e3]);

    let s = solved(Mode::Minerr, &[1e-12, 1e3], |x: &[f64]| Ok(vec![(x[0], 2e-12), (x[0], 4e-12), (x[1], 5e3), (x[1], 7e3)]));
    close(&s.x, &[3e-12, 6e3]);
}

#[test]
fn holds_is_judged_at_the_stopping_point() {
    // From 30 the left side starts near 1e13 and falls to 1 far to the left: 0.5 against 1 is no
    // solution, however small it is next to the sides at the guess.
    let out = solve(Mode::Find, &[30.0], &mut one(|x| (x.exp() + 1.0, 0.5)));
    assert!(matches!(out, Ok(Err(Failure::NoSolution { worst })) if worst > 0.1), "{out:?}");
    // x² = −1 has no real root, from any guess: a huge one mustn't make the miss look small.
    for x0 in [1e3, 1e6, 1e8, 1e9, 1e12, 1e15, -1e10, 1e22] {
        let out = solve(Mode::Find, &[x0], &mut one(|x| (x * x, -1.0)));
        assert!(matches!(out, Ok(Err(Failure::NoSolution { worst })) if worst > 0.1), "from {x0}: {out:?}");
    }
    // Equations that contradict each other, from guesses far larger than any answer.
    for x0 in [1e9, 1e12, 1e15, 1e22, 1e30] {
        for other in [3.0, 3.1] {
            let out = solve(Mode::Find, &[x0], &mut |x: &[f64]| Ok(vec![(x[0], 1.0), (x[0], other)]));
            assert!(matches!(out, Ok(Err(Failure::NoSolution { .. }))), "x == 1, x == {other} from {x0}: {out:?}");
        }
    }
    for x0 in [1e9, 1e12, 1e15] {
        let out = solve(Mode::Find, &[x0, x0], &mut |x: &[f64]| Ok(vec![(x[0] + x[1], 1.0), (x[0] + x[1], 3.0)]));
        assert!(matches!(out, Ok(Err(Failure::NoSolution { .. }))), "x + y == 1, x + y == 3 from {x0}: {out:?}");
    }
}

#[test]
fn a_root_at_zero_holds() {
    // The solve ends a hair from 0, where both sides are tiny: that is still the root.
    let cases: [(f64, fn(f64) -> (f64, f64)); 7] = [
        (5.0, |x| (x, 0.0)),
        (0.1, |x| (x.sin(), 0.0)),
        (3.0, |x| (x.sin(), 0.0)),
        (1.0, |x| (x * x.exp(), 0.0)),
        (0.5, |x| (2.0 * x + x * x, 0.0)),
        (0.5, |x| (x.tan(), 0.0)),
        (1.0, |x| (x.exp() - 1.0, 0.0)),
    ];
    for (x0, f) in cases {
        let mut eq = one(f);
        let s = solved(Mode::Find, &[x0], &mut eq);
        assert_holds(&mut eq, &s.x);
        let root = if x0 == 3.0 { PI } else { 0.0 };
        assert!((s.x[0] - root).abs() <= 1e-12, "from {x0}: {}", s.x[0]);
    }
}

#[test]
fn an_exact_guess_next_to_a_domain_edge() {
    // √(−x) = 0 holds at the guess 0; just above it √(−x) isn't a number, so no Jacobian can be
    // measured there, and none is needed.
    let mut eq = one(|x| ((-x).sqrt(), 0.0));
    let s = solved(Mode::Find, &[0.0], &mut eq);
    assert_eq!(s.x, [0.0]);
    assert_holds(&mut eq, &s.x);
}

#[test]
fn at_the_step_cap_an_answer_that_holds_is_returned() {
    let mut eq = one(|x| (x * x, 2.0));
    // Newton from 1 has √2 to double precision after five steps; the sixth, too small to matter,
    // is what ends the solve. Capped at five, the answer holds, so it is returned.
    let s = match solve_capped(Mode::Find, &[1.0], &mut eq, &mut |_| Ok(()), 5) {
        Ok(Ok(s)) => s,
        other => panic!("{other:?}"),
    };
    assert!((s.x[0] - 2f64.sqrt()).abs() <= 1e-12, "{}", s.x[0]);
    assert_holds(&mut eq, &s.x);
    // Capped at one step (at 1.5), it doesn't hold: no convergence, as before.
    assert_eq!(solve_capped(Mode::Find, &[1.0], &mut eq, &mut |_| Ok(()), 1), Ok(Err(Failure::NotConverged { steps: 1 })));
    // Minerr has no "holds" to check: at the cap it hasn't converged.
    assert_eq!(solve_capped(Mode::Minerr, &[1.0], &mut eq, &mut |_| Ok(()), 5), Ok(Err(Failure::NotConverged { steps: 5 })));
}

#[test]
fn three_reservoirs_from_a_zero_guess() {
    let s = solved(Mode::Find, &[0.0, 5.0, 5.0], reservoirs);
    assert_holds(&mut reservoirs, &s.x);
    let gpm = 448.83116883;
    assert!(sig3(area(0) * s.x[0] * gpm, -1589.5412631603367), "{:?}", s.x);
    assert!(sig3(area(1) * s.x[2] * gpm, 6697.8072260949511), "{:?}", s.x);
    assert!(sig3(area(2) * s.x[1] * gpm, 8287.3484892554116), "{:?}", s.x);
}
