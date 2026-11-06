use super::super::{Expr, Format, MAX_CHANNELS, MAX_DEPTH, Sample, bounded, spend};

pub(super) struct Linear {
    pub terms: [f64; MAX_CHANNELS],
    pub constant: f64,
}

impl Linear {
    fn constant(value: f64) -> Self {
        Self { terms: [0.0; MAX_CHANNELS], constant: value }
    }
    fn scalar(&self) -> bool {
        self.terms.iter().all(|n| *n == 0.0)
    }
    fn scale(mut self, value: f64) -> Option<Self> {
        self.constant *= value;
        for term in &mut self.terms {
            *term *= value;
        }
        self.valid()
    }
    fn add(mut self, other: Self) -> Option<Self> {
        self.constant += other.constant;
        for (a, b) in self.terms.iter_mut().zip(other.terms) {
            *a += b;
        }
        self.valid()
    }
    fn valid(self) -> Option<Self> {
        (bounded(self.constant) && self.terms.iter().all(|n| bounded(*n))).then_some(self)
    }
}

pub(super) fn linear(expression: &Expr, work: &mut usize) -> Option<Linear> {
    spend(work, MAX_CHANNELS + 1)?;
    match expression {
        Expr::Constant(value) => Some(Linear::constant(*value)),
        Expr::Channel(channel) => {
            let mut out = Linear::constant(0.0);
            *out.terms.get_mut(*channel)? = 1.0;
            Some(out)
        }
        Expr::Apply(op, args) => match (op.as_str(), args.as_slice()) {
            ("plus", _) => args.iter().try_fold(Linear::constant(0.0), |out, a| out.add(linear(a, work)?)),
            ("minus", [a]) => linear(a, work)?.scale(-1.0),
            ("minus", [a, b]) => linear(a, work)?.add(linear(b, work)?.scale(-1.0)?),
            ("times", _) => args.iter().try_fold(Linear::constant(1.0), |out, a| {
                let a = linear(a, work)?;
                if a.scalar() {
                    out.scale(a.constant)
                } else if out.scalar() {
                    a.scale(out.constant)
                } else {
                    None
                }
            }),
            ("divide", [a, b]) => {
                let b = linear(b, work)?;
                if !b.scalar() || b.constant == 0.0 {
                    return None;
                }
                linear(a, work)?.scale(1.0 / b.constant)
            }
            _ => expression.eval(&vec![None; MAX_CHANNELS], work).map(Linear::constant),
        },
    }
}

#[derive(Clone, Copy)]
struct Range {
    low: f64,
    high: f64,
}

impl Range {
    fn new(a: f64, b: f64) -> Option<Self> {
        if a.is_nan() || b.is_nan() {
            return None;
        }
        Some(Self { low: a.min(b), high: a.max(b) })
    }
    fn affine(self, a: f64, b: f64) -> Option<Self> {
        Self::new(self.low * a + b, self.high * a + b)
    }
    fn positive(self) -> Option<Self> {
        (self.high > 0.0).then_some(Self { low: self.low.max(0.0), high: self.high })
    }
    fn intersect(self, low: f64, high: f64) -> Option<Self> {
        let range = Self { low: self.low.max(low), high: self.high.min(high) };
        (range.low <= range.high).then_some(range)
    }
}

enum Step {
    Affine(f64, f64),
    Unary(&'static str),
    Binary(&'static str, f64, bool),
}

struct Chain {
    channel: usize,
    range: Range,
    steps: Vec<Step>,
}

#[derive(Clone)]
pub(in crate::read::ink) struct Scalar {
    pub target: usize,
    source: usize,
    expression: Expr,
    range: Range,
}

impl Scalar {
    pub(in crate::read::ink::mapping) fn at(&self, input: &Sample, work: &mut usize) -> Option<f64> {
        spend(work, 1)?;
        let value = input.get(self.source).copied().flatten()?;
        if value < self.range.low || value > self.range.high {
            return None;
        }
        self.expression.eval(input, work)
    }
}

pub(super) fn inverse(expression: &Expr, input: usize, format: &Format, work: &mut usize) -> Option<Scalar> {
    let chain = chain(expression, format, work, 0)?;
    let mut value = Expr::Channel(input);
    let mut depth = 0;
    for step in chain.steps.into_iter().rev() {
        spend(work, 3)?;
        value = match step {
            Step::Affine(a, b) => {
                let shifted = if b == 0.0 {
                    value
                } else {
                    depth += 1;
                    Expr::Apply("minus".into(), vec![value, Expr::Constant(b)])
                };
                if a == 1.0 {
                    shifted
                } else {
                    depth += 1;
                    Expr::Apply("divide".into(), vec![shifted, Expr::Constant(a)])
                }
            }
            Step::Unary(op) => {
                depth += 1;
                Expr::Apply(op.into(), vec![value])
            }
            Step::Binary(op, constant, first) => {
                depth += 1;
                let args = if first { vec![Expr::Constant(constant), value] } else { vec![value, Expr::Constant(constant)] };
                Expr::Apply(op.into(), args)
            }
        };
        if depth > MAX_DEPTH {
            return None;
        }
    }
    Some(Scalar { target: chain.channel, source: input, expression: value, range: chain.range })
}

fn chain(expression: &Expr, format: &Format, work: &mut usize, depth: usize) -> Option<Chain> {
    spend(work, 1)?;
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(form) = linear(expression, work) {
        let mut terms = form.terms.iter().enumerate().filter(|(_, n)| **n != 0.0);
        let (channel, &coefficient) = terms.next()?;
        if terms.next().is_some() {
            return None;
        }
        let channel_format = format.channels.get(channel)?;
        let (low, high) = if channel_format.boolean || channel_format.fraction {
            (0.0, 1.0)
        } else {
            (channel_format.min.unwrap_or(f64::NEG_INFINITY), channel_format.max.unwrap_or(f64::INFINITY))
        };
        return Some(Chain {
            channel,
            range: Range::new(low, high)?.affine(coefficient, form.constant)?,
            steps: vec![Step::Affine(coefficient, form.constant)],
        });
    }
    let Expr::Apply(op, args) = expression else {
        return None;
    };
    let mut child = None;
    let mut constants = Vec::new();
    let mut variable = 0;
    for (i, arg) in args.iter().enumerate() {
        spend(work, 1)?;
        if let Some(value) = linear(arg, work).filter(Linear::scalar).map(|n| n.constant) {
            constants.push((i, value));
        } else {
            if child.is_some() {
                return None;
            }
            variable = i;
            child = Some(chain(arg, format, work, depth + 1)?);
        }
    }
    let mut child = child?;
    let (range, steps) = step(op, args.len(), variable, &constants, child.range)?;
    child.range = range;
    child.steps.extend(steps.into_iter().rev());
    (child.steps.len() <= MAX_DEPTH).then_some(child)
}

fn step(op: &str, arity: usize, variable: usize, constants: &[(usize, f64)], range: Range) -> Option<(Range, Vec<Step>)> {
    let unary = || (arity == 1).then_some(());
    let constant = || if arity == 2 { constants.first().map(|c| c.1) } else { None };
    let affine = |a: f64, b: f64| {
        if !bounded(a) || !bounded(b) || a == 0.0 {
            return None;
        }
        Some((range.affine(a, b)?, vec![Step::Affine(a, b)]))
    };
    match op {
        "plus" => affine(1.0, constants.iter().map(|c| c.1).sum()),
        "times" => affine(constants.iter().map(|c| c.1).product(), 0.0),
        "minus" if arity == 1 => affine(-1.0, 0.0),
        "minus" => {
            if variable == 0 {
                affine(1.0, -constant()?)
            } else {
                affine(-1.0, constant()?)
            }
        }
        "divide" if variable == 0 => affine(1.0 / constant()?, 0.0),
        "divide" => {
            let c = constant()?;
            if c == 0.0 {
                return None;
            }
            let out = if range.low < 0.0 && range.high > 0.0 {
                Range::new(f64::NEG_INFINITY, f64::INFINITY)?
            } else {
                Range::new(c / range.low, c / range.high)?
            };
            Some((out, vec![Step::Binary("divide", c, true)]))
        }
        "abs" => {
            unary()?;
            if range.low >= 0.0 {
                affine(1.0, 0.0)
            } else if range.high <= 0.0 {
                affine(-1.0, 0.0)
            } else {
                None
            }
        }
        "min" if constants.iter().all(|c| c.1 >= range.high) => affine(1.0, 0.0),
        "max" if constants.iter().all(|c| c.1 <= range.low) => affine(1.0, 0.0),
        "exp" => {
            unary()?;
            Some((Range::new(range.low.exp(), range.high.exp())?, vec![Step::Unary("ln")]))
        }
        "ln" => {
            unary()?;
            let range = range.positive()?;
            Some((Range::new(range.low.ln(), range.high.ln())?, vec![Step::Unary("exp")]))
        }
        "log" if variable == 0 => {
            let base = constant()?;
            if base <= 0.0 || base == 1.0 {
                return None;
            }
            let range = range.positive()?;
            Some((Range::new(range.low.log(base), range.high.log(base))?, vec![Step::Binary("power", base, true)]))
        }
        "power" if variable == 1 => {
            let base = constant()?;
            if base <= 0.0 || base == 1.0 {
                return None;
            }
            Some((Range::new(base.powf(range.low), base.powf(range.high))?, vec![Step::Binary("log", base, false)]))
        }
        "power" => power(range, constant()?),
        "root" if variable == 0 => root(range, constant()?),
        "sin" | "cos" | "tan" => {
            unary()?;
            trig(op, range)
        }
        "arcsin" => {
            unary()?;
            let range = range.intersect(-1.0, 1.0)?;
            Some((Range::new(range.low.asin(), range.high.asin())?, vec![Step::Unary("sin")]))
        }
        "arccos" => {
            unary()?;
            let range = range.intersect(-1.0, 1.0)?;
            Some((Range::new(range.low.acos(), range.high.acos())?, vec![Step::Unary("cos")]))
        }
        "arctan" => {
            unary()?;
            Some((Range::new(range.low.atan(), range.high.atan())?, vec![Step::Unary("tan")]))
        }
        _ => None,
    }
}

fn root(range: Range, degree: f64) -> Option<(Range, Vec<Step>)> {
    if degree == 0.0 {
        return None;
    }
    if degree.fract() == 0.0 && degree.abs() % 2.0 == 1.0 {
        let root = |x: f64| x.signum() * x.abs().powf(1.0 / degree);
        let out = if degree < 0.0 && range.low <= 0.0 && range.high >= 0.0 {
            Range::new(f64::NEG_INFINITY, f64::INFINITY)?
        } else {
            Range::new(root(range.low), root(range.high))?
        };
        Some((out, vec![Step::Binary("power", degree, false)]))
    } else {
        power(range, 1.0 / degree)
    }
}

fn power(mut range: Range, exponent: f64) -> Option<(Range, Vec<Step>)> {
    if exponent == 0.0 || !bounded(exponent) {
        return None;
    }
    let integer = exponent.fract() == 0.0;
    let odd = integer && exponent.abs() % 2.0 == 1.0;
    let negative = integer && !odd && range.high <= 0.0;
    if integer && !odd && range.low < 0.0 && range.high > 0.0 {
        return None;
    }
    if !integer {
        range = range.intersect(0.0, f64::INFINITY)?;
    }
    let out = if odd && exponent < 0.0 && range.low <= 0.0 && range.high >= 0.0 {
        Range::new(f64::NEG_INFINITY, f64::INFINITY)?
    } else {
        Range::new(range.low.powf(exponent), range.high.powf(exponent))?
    };
    let mut steps = vec![Step::Binary("root", exponent, false)];
    if negative {
        steps.push(Step::Affine(-1.0, 0.0));
    }
    Some((out, steps))
}

fn trig(op: &str, range: Range) -> Option<(Range, Vec<Step>)> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let phase = if op == "cos" { 0.0 } else { FRAC_PI_2 };
    let n = ((range.low + phase) / PI).floor();
    let left = n * PI - phase;
    let right = (n + 1.0) * PI - phase;
    if range.low < left || range.high > right || (op == "tan" && (range.low <= left || range.high >= right)) {
        return None;
    }
    let even = n.rem_euclid(2.0) == 0.0;
    let (a, offset, function, low, high) = match op {
        "sin" => (if even { 1.0 } else { -1.0 }, n * PI, "arcsin", range.low.sin(), range.high.sin()),
        "cos" => (if even { 1.0 } else { -1.0 }, if even { n * PI } else { (n + 1.0) * PI }, "arccos", range.low.cos(), range.high.cos()),
        "tan" => (1.0, n * PI, "arctan", range.low.tan(), range.high.tan()),
        _ => return None,
    };
    let steps = vec![Step::Unary(function), Step::Affine(a, -offset * a)];
    Some((Range::new(low, high)?, steps))
}
