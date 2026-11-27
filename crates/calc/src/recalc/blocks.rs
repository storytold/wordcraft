//! Solve blocks in a document: a `Given` region, optional guesses, equations, and the region
//! that ends it with `Find(…)` or `Minerr(…)`. The one place where the order of regions matters.
//!
//! [`resolve`] walks the regions in document order (tracked deletions are already left out, so they
//! neither open, close nor join a block), rewrites each block's closing statement so its
//! `Find`/`Minerr` call becomes an [`Expr::Solve`] holding the block's equations, and reports what
//! breaks the block rules as errors on the regions at fault.

use crate::{CalcError, Expr, SolveKind, Stmt};

/// A `Given` that nothing ends.
const UNCLOSED: &str = "this Given has no Find or Minerr below it to end its block";
/// An equation that no `Given` opens a block for.
const NEEDS_GIVEN: &str = "an equation needs a Given above it";
/// A region that is neither an equation nor an initial guess for this solve.
const INVALID_MEMBER: &str = "a Given block accepts only starting values for its unknowns followed by equations";
/// A block that ends with no equation in it.
const NO_EQUATIONS: &str = "a Given block needs at least one equation";

/// A closed block: the region that ends it and the regions above it that make it up.
pub(super) struct Block {
    /// The closing region: `name := Find(…)`, `name(params) := Find(…)`, `[a; b] := Find(…)` or
    /// `Find(…) =` (or `Minerr`).
    pub(super) closing: usize,
    /// Which call ends it.
    pub(super) kind: SolveKind,
    /// The `Given` and every region between it and the closing one, in document order. A failed
    /// or edited member fails the closing region.
    pub(super) members: Vec<usize>,
}

/// What [`resolve`] found.
#[derive(Default)]
pub(super) struct Resolved {
    /// Every closed block, in document order.
    pub(super) blocks: Vec<Block>,
    /// The regions that break a block rule, with their errors.
    pub(super) errors: Vec<(usize, CalcError)>,
    /// The regions that would end a block but have none to end, with the call they make: an error
    /// ([`crate::eval::unresolved`]) unless the document defines that call's name itself, when the call is a
    /// use of that (failed) definition.
    pub(super) strays: Vec<(usize, SolveKind)>,
}

/// The block being read: its members, candidate guesses, equations, and errors. Those are only errors once a `Find` or `Minerr` ends the block: until then a
/// lone `Given` (just typed, or left behind) breaks nothing below it.
#[derive(Default)]
struct Open {
    members: Vec<usize>,
    equations: Vec<(Expr, Expr)>,
    guesses: Vec<(usize, String)>,
    errors: Vec<(usize, CalcError)>,
}

/// Find the blocks among `regions` (each region's parsed statement, in document order), rewriting
/// each closing statement into an [`Expr::Solve`] of its block's equations.
///
/// - A `Given` opens a block. One that comes while a block is open leaves that block unended (see
///   below) and opens a new one.
/// - A plain definition before the first equation may supply a starting value for an unknown.
///   Each equation (`a == b`) joins the block. Other calculations are errors; a `Find` or
///   `Minerr` inside an expression is one at once, as it is anywhere ([`crate::eval`]).
/// - The first statement whose whole right side, or whole expression, is a `Find`/`Minerr` call
///   ends it. The call's arguments must be names, at least one, each once, and the block must hold
///   at least one equation. Outside a block such a call is a stray: see [`Resolved::strays`].
/// - An equation outside a block is an error.
/// - A `Given` that nothing ends is an error on that `Given` alone: nothing below it is an error
///   for being in its block, so the rest of the document keeps working, and its equations show
///   nothing, as a block's do.
pub(super) fn resolve(regions: &mut [Result<Stmt, CalcError>], order: impl IntoIterator<Item = usize>) -> Resolved {
    let mut out = Resolved::default();
    let syntax = |message: &str| CalcError::Syntax(message.to_string());
    let mut open: Option<Open> = None;
    for i in order {
        let Some(stmt) = regions.get_mut(i) else { continue };
        let Ok(stmt) = stmt else {
            // A region that doesn't parse still takes its place in an open block, which fails with it.
            if let Some(block) = &mut open {
                block.members.push(i);
            }
            continue;
        };
        if let Some(call) = solve_call(stmt) {
            let Expr::Call(name, args) = call else { continue };
            let kind = if name == "Find" { SolveKind::Find } else { SolveKind::Minerr };
            let Some(block) = open.take() else {
                out.strays.push((i, kind));
                continue;
            };
            // The block has ended: what is wrong inside it counts now.
            out.errors.extend(block.errors);
            let unknowns: Option<Vec<String>> = args
                .iter()
                .map(|a| match a {
                    Expr::Name(n) => Some(n.clone()),
                    _ => None,
                })
                .collect();
            for (region, name) in block.guesses {
                if !unknowns.as_ref().is_some_and(|names| names.contains(&name)) {
                    out.errors.push((region, syntax(INVALID_MEMBER)));
                }
            }
            let twice = unknowns.as_ref().and_then(|u| u.iter().enumerate().find(|&(k, n)| u[..k].contains(n)).map(|(_, n)| n.clone()));
            match unknowns.filter(|u| !u.is_empty()) {
                Some(_) if block.equations.is_empty() => out.errors.push((i, syntax(NO_EQUATIONS))),
                Some(_) if let Some(name) = twice => out.errors.push((i, CalcError::Syntax(format!("{name} is listed twice")))),
                Some(unknowns) => {
                    *call = Expr::Solve { kind, unknowns, equations: block.equations };
                    out.blocks.push(Block { closing: i, kind, members: block.members });
                }
                None => {
                    let k = kind.name();
                    out.errors.push((i, CalcError::Syntax(format!("{k}'s arguments are the unknowns, like {k}(x, y)"))));
                }
            }
            continue;
        }
        match (stmt, &mut open) {
            (Stmt::Given, block) => {
                // A block still open has no end: it is unended, and this Given starts another.
                if let Some(given) = block.take().and_then(|b| b.members.first().copied()) {
                    out.errors.push((given, syntax(UNCLOSED)));
                }
                open = Some(Open { members: vec![i], ..Open::default() });
            }
            (Stmt::Constraint { lhs, rhs }, Some(block)) => {
                block.members.push(i);
                block.equations.push((lhs.clone(), rhs.clone()));
            }
            (Stmt::Constraint { .. }, None) => out.errors.push((i, syntax(NEEDS_GIVEN))),
            (other, Some(block)) => {
                block.members.push(i);
                // A Find inside an expression is an error whether or not the block is ever ended,
                // as it is outside a block (evaluating it gives the same error).
                match nested_solve(other) {
                    Some(kind) => out.errors.push((i, crate::eval::not_the_whole_side(kind))),
                    None => match other {
                        Stmt::Define { name, index: None, params, .. } if params.is_empty() && block.equations.is_empty() => {
                            // The closing region already uses the unknown's ordinary definition as its starting value.
                            block.guesses.push((i, name.clone()));
                        }
                        _ => block.errors.push((i, syntax(INVALID_MEMBER))),
                    },
                }
            }
            (_, None) => {}
        }
    }
    if let Some(given) = open.and_then(|b| b.members.first().copied()) {
        out.errors.push((given, syntax(UNCLOSED)));
    }
    out
}

/// The first `Find` or `Minerr` that `stmt` uses (when it isn't a statement that ends a block, so
/// inside an expression).
fn nested_solve(stmt: &Stmt) -> Option<SolveKind> {
    crate::ast::free_names(stmt).iter().find_map(|n| match n.as_str() {
        "Find" => Some(SolveKind::Find),
        "Minerr" => Some(SolveKind::Minerr),
        _ => None,
    })
}

/// The `Find(…)` or `Minerr(…)` call that is the whole right side, or the whole expression, of
/// `stmt`: what ends a block.
fn solve_call(stmt: &mut Stmt) -> Option<&mut Expr> {
    let e = match stmt {
        Stmt::Define { body, .. } | Stmt::DefineMany { body, .. } => body,
        Stmt::Eval { expr, .. } => expr,
        Stmt::Given | Stmt::Constraint { .. } => return None,
    };
    matches!(e, Expr::Call(name, _) if name == "Find" || name == "Minerr").then_some(e)
}
