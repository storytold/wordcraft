//! The syntax tree of a calculation, and what a statement defines and uses.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// One calculation statement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Stmt {
    /// `name := body`, `name(params) := body` or `name[i] := body`, optionally followed by `=` to
    /// show the value.
    Define {
        /// The defined name.
        name: String,
        /// The range variable of an indexed definition `name[i] := body`, which defines every
        /// element of `name` at once; `None` otherwise.
        index: Option<String>,
        /// Parameter names; empty for a plain variable.
        params: Vec<String>,
        /// The defining expression.
        body: Expr,
        /// Present when the statement ends with `=` (define and show).
        show: Option<Show>,
    },
    /// `expr =` or `expr = ? unit`: evaluate and show.
    Eval {
        /// The expression to evaluate.
        expr: Expr,
        /// How to show it.
        show: Show,
    },
    /// A region whose whole source is `Given`: it opens a block of equations.
    Given,
    /// `lhs == rhs` alone: an equation of a solve block.
    Constraint {
        /// The left side.
        lhs: Expr,
        /// The right side.
        rhs: Expr,
    },
    /// `[a; b; …] := body`, optionally followed by `=` to show the value: defines every name from
    /// the element of a column vector that stands at its place.
    DefineMany {
        /// The defined names, at least two, each once.
        names: Vec<String>,
        /// The defining expression: a column of as many elements as there are names.
        body: Expr,
        /// Present when the statement ends with `=` (define and show).
        show: Option<Show>,
    },
}

/// What a solve block finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SolveKind {
    /// `Find(…)`: values for the unknowns that satisfy every equation.
    Find,
    /// `Minerr(…)`: values that come as near as possible to satisfying them.
    Minerr,
}

impl SolveKind {
    /// The function name it is written as.
    pub fn name(self) -> &'static str {
        match self {
            SolveKind::Find => "Find",
            SolveKind::Minerr => "Minerr",
        }
    }
}

/// How a result is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Show {
    /// The unit text after `=` (with any leading `?` removed), verbatim and unparsed; `None` to
    /// let the evaluator choose a unit.
    pub unit: Option<String>,
}

/// An expression.
// Not `Deserialize`: the parser's depth limit is what keeps recursion over an `Expr` safe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Expr {
    /// A number.
    Num(f64),
    /// A text literal, without its quotes: `"OK"`.
    Text(String),
    /// A variable, unit or constant; the evaluator decides which.
    Name(String),
    /// A number with a unit. The parser never produces this (`5 kN` parses as `5 * kN`); it is for
    /// the evaluator and for building expressions directly.
    Quantity(f64, String),
    /// A prefix operator (only [`Op::Neg`]).
    Unary(Op, Box<Expr>),
    /// An infix operator.
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A comparison or a chain of them: `a < b` is one link, `a < b ≤ c` two (`rest` is never
    /// empty). A chain holds when every link does.
    Compare {
        /// The leftmost operand.
        first: Box<Expr>,
        /// Each further operand with the comparison that joins it to the one before.
        rest: Vec<(Cmp, Expr)>,
    },
    /// `a and b` or `a or b`.
    Logic(Logic, Box<Expr>, Box<Expr>),
    /// `not a`.
    Not(Box<Expr>),
    /// A function call; `√x` is `Call("sqrt", [x])`.
    Call(String, Vec<Expr>),
    /// Absolute value, `|x|`.
    Abs(Box<Expr>),
    /// A vector or matrix literal, `[1, 2; 3, 4]`: `rows × cols` items, row-major. A list with no
    /// `;` (`[1, 2, 3]`) is a column.
    Matrix {
        /// Number of rows.
        rows: usize,
        /// Number of columns.
        cols: usize,
        /// The elements, row by row.
        items: Vec<Expr>,
    },
    /// An element of a named vector or matrix: `D[2]` (one index) or `M[i, j]` (two).
    Index(String, Vec<Expr>),
    /// A range, `first..last` or `first, second..last` (the second value sets the step). The
    /// parser only produces one as the whole right side of a definition.
    Range {
        /// The first value.
        first: Box<Expr>,
        /// The second value, when it is given.
        second: Option<Box<Expr>>,
        /// The last value.
        last: Box<Expr>,
    },
    /// A piecewise definition, `a if c, b if d, e otherwise`: the value of the first case whose
    /// condition holds, else `otherwise`'s. The parser only produces one as the whole right side
    /// of a definition. With a vector condition each element takes its own case.
    Cases {
        /// Each case as `(value, condition)`, in order; never empty.
        cases: Vec<(Expr, Expr)>,
        /// The value when no condition holds; with none, that is an error.
        otherwise: Option<Box<Expr>>,
    },
    /// A sum with limits, `∑(i = first..last) body` or `∑(i = first, second..last) body`: `body`
    /// added up with the index `i` taking each value of the range, both ends included. The index
    /// belongs to the sum alone.
    Sum {
        /// The index, a name used only inside `body`.
        index: String,
        /// The first value of the range.
        first: Box<Expr>,
        /// The second value, when it is given (it sets the step).
        second: Option<Box<Expr>>,
        /// The last value of the range.
        last: Box<Expr>,
        /// The term added up for each index value.
        body: Box<Expr>,
    },
    /// A resolved solve block: built by recalculation from a `Find(…)` or `Minerr(…)` and the
    /// equations above it, never by the parser.
    Solve {
        /// Whether it finds or minimizes the error.
        kind: SolveKind,
        /// The unknowns, in the order they are named.
        unknowns: Vec<String>,
        /// The equations of the block, each as `(left side, right side)`, in document order.
        equations: Vec<(Expr, Expr)>,
    },
}

/// A comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cmp {
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `≤` or `<=`
    Le,
    /// `≥` or `>=`
    Ge,
    /// `==`
    Eq,
    /// `≠` or `!=`
    Ne,
}

impl Cmp {
    /// The symbol it is shown as and named by in messages.
    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Gt => ">",
            Cmp::Le => "≤",
            Cmp::Ge => "≥",
            Cmp::Eq => "==",
            Cmp::Ne => "≠",
        }
    }
}

/// A logical connective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Logic {
    /// `and` or `∧`
    And,
    /// `or` or `∨`
    Or,
}

/// An operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`, `·`, `×` or adjacency
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
    /// Prefix `-`
    Neg,
}

/// The name a statement defines, if it is a definition.
pub fn defined_name(s: &Stmt) -> Option<&str> {
    match s {
        Stmt::Define { name, .. } => Some(name),
        Stmt::Eval { .. } | Stmt::Given | Stmt::Constraint { .. } | Stmt::DefineMany { .. } => None,
    }
}

/// Every name a statement defines: one for a [`Stmt::Define`], one per name for a
/// [`Stmt::DefineMany`], none for anything else.
pub fn defined_names(s: &Stmt) -> Vec<&str> {
    match s {
        Stmt::Define { name, .. } => vec![name],
        Stmt::DefineMany { names, .. } => names.iter().map(String::as_str).collect(),
        Stmt::Eval { .. } | Stmt::Given | Stmt::Constraint { .. } => Vec::new(),
    }
}

/// The names a statement uses (variables, units, constants, called functions and indexed
/// vectors), minus a function's own parameters and, inside a sum, its index (which is itself listed
/// as used, once, where the sum starts), without duplicates, in order of first use. The
/// range variable of an indexed definition (`i` in `t[i] := …`) is used, so it comes first. The
/// names of a resolved solve block are those its equations use, then its unknowns: an unknown
/// counts as used, which ties the block to the definitions that guess it. A [`Stmt::Constraint`]
/// uses the names of both its sides; [`Stmt::Given`] uses none.
pub fn free_names(s: &Stmt) -> Vec<String> {
    let (roots, params, index): (Vec<&Expr>, &[String], Option<&String>) = match s {
        Stmt::Define { params, body, index, .. } => (vec![body], params, index.as_ref()),
        Stmt::Eval { expr, .. } => (vec![expr], &[], None),
        Stmt::DefineMany { body, .. } => (vec![body], &[], None),
        Stmt::Constraint { lhs, rhs } => (vec![lhs, rhs], &[], None),
        Stmt::Given => (Vec::new(), &[], None),
    };
    let mut out: Vec<String> = Vec::new();
    // Names already in `out` (or parameters): one lookup each, so a long body costs linear time.
    let mut seen: HashSet<&str> = params.iter().map(String::as_str).collect();
    if let Some(i) = index
        && seen.insert(i)
    {
        out.push(i.clone());
    }
    // The indexes of the sums being read, innermost last: a name that is one of them is the sum's
    // own, not a use of anything else.
    let mut bound: Vec<&str> = Vec::new();
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<Visit<'_>> = roots.into_iter().rev().map(Visit::Expr).collect();
    while let Some(visit) = stack.pop() {
        let e = match visit {
            Visit::Expr(e) => e,
            Visit::Name(n) => {
                if !bound.contains(&n) && seen.insert(n) {
                    out.push(n.to_string());
                }
                continue;
            }
            Visit::Bind(i) => {
                bound.push(i);
                continue;
            }
            Visit::Unbind => {
                bound.pop();
                continue;
            }
        };
        let name = match e {
            Expr::Num(_) | Expr::Text(_) | Expr::Quantity(..) => continue,
            Expr::Name(n) => n,
            Expr::Unary(_, a) | Expr::Abs(a) | Expr::Not(a) => {
                stack.push(Visit::Expr(a));
                continue;
            }
            Expr::Binary(_, a, b) | Expr::Logic(_, a, b) => {
                stack.push(Visit::Expr(b));
                stack.push(Visit::Expr(a));
                continue;
            }
            Expr::Compare { first, rest } => {
                stack.extend(rest.iter().rev().map(|(_, e)| Visit::Expr(e)));
                stack.push(Visit::Expr(first));
                continue;
            }
            Expr::Call(n, args) | Expr::Index(n, args) => {
                if matches!(e, Expr::Call(..)) && n == "worksheetProgram" {
                    let mut program = args.iter().collect::<Vec<_>>();
                    while let Some(item) = program.pop() {
                        if let Expr::Call(op, args) = item {
                            if matches!(op.as_str(), "worksheetSet" | "worksheetSetCell" | "worksheetFor")
                                && let Some(Expr::Text(name)) = args.first()
                            {
                                seen.insert(name);
                            }
                            program.extend(args);
                        }
                    }
                }
                stack.extend(args.iter().rev().map(Visit::Expr));
                // `if(c, a, b)` is a keyword, not a function anything defines.
                if matches!(e, Expr::Call(..)) && n == "if" {
                    continue;
                }
                n
            }
            Expr::Matrix { items, .. } => {
                stack.extend(items.iter().rev().map(Visit::Expr));
                continue;
            }
            Expr::Cases { cases, otherwise } => {
                push_cases(&mut stack, cases, otherwise.as_deref());
                continue;
            }
            Expr::Range { first, second, last } => {
                stack.push(Visit::Expr(last));
                if let Some(second) = second {
                    stack.push(Visit::Expr(second));
                }
                stack.push(Visit::Expr(first));
                continue;
            }
            Expr::Sum { index, first, second, last, body } => {
                // The index counts as used, so a definition of that name elsewhere runs first and
                // the "already defined" error doesn't depend on the order of evaluation. The ends
                // come first, outside the index; then the body, inside it.
                if seen.insert(index) {
                    out.push(index.clone());
                }
                stack.push(Visit::Unbind);
                stack.push(Visit::Expr(body));
                stack.push(Visit::Bind(index));
                stack.push(Visit::Expr(last));
                if let Some(second) = second {
                    stack.push(Visit::Expr(second));
                }
                stack.push(Visit::Expr(first));
                continue;
            }
            Expr::Solve { unknowns, equations, .. } => {
                stack.extend(unknowns.iter().rev().map(|u| Visit::Name(u)));
                for (lhs, rhs) in equations.iter().rev() {
                    stack.push(Visit::Expr(rhs));
                    stack.push(Visit::Expr(lhs));
                }
                continue;
            }
        };
        if !bound.contains(&name.as_str()) && seen.insert(name) {
            out.push(name.clone());
        }
    }
    out
}

/// What [`free_names`] still has to do: read an expression, or enter or leave a sum's body, where
/// its index is bound.
enum Visit<'a> {
    Expr(&'a Expr),
    /// A name that is used, with no expression around it (an unknown of a solve block).
    Name(&'a str),
    Bind(&'a str),
    Unbind,
}

impl<'a> From<&'a Expr> for Visit<'a> {
    fn from(e: &'a Expr) -> Self {
        Visit::Expr(e)
    }
}

/// Push the parts of a piecewise onto a stack of expressions to visit, so they come off it in
/// source order: each value then its condition, then `otherwise`.
fn push_cases<'a, T: From<&'a Expr>>(stack: &mut Vec<T>, cases: &'a [(Expr, Expr)], otherwise: Option<&'a Expr>) {
    stack.extend(otherwise.map(T::from));
    for (value, condition) in cases.iter().rev() {
        stack.push(T::from(condition));
        stack.push(T::from(value));
    }
}

/// The keywords a statement uses as operators, without duplicates, in order of first use: `and`,
/// `or`, `not`, `if` (a piecewise's cases, and the `if(c, a, b)` function) and `otherwise`. A document that also defines one of these words as a name (written
/// before the word became a keyword) has changed meaning, which [`crate::recalc`] reports.
pub(crate) fn keywords_used(s: &Stmt) -> Vec<&'static str> {
    let roots: Vec<&Expr> = match s {
        Stmt::Define { body, .. } | Stmt::DefineMany { body, .. } => vec![body],
        Stmt::Eval { expr, .. } => vec![expr],
        Stmt::Constraint { lhs, rhs } => vec![lhs, rhs],
        Stmt::Given => Vec::new(),
    };
    let mut out: Vec<&'static str> = Vec::new();
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<&Expr> = roots.into_iter().rev().collect();
    while let Some(e) = stack.pop() {
        let word = match e {
            Expr::Num(_) | Expr::Text(_) | Expr::Name(_) | Expr::Quantity(..) => None,
            Expr::Unary(_, a) | Expr::Abs(a) => {
                stack.push(a);
                None
            }
            Expr::Not(a) => {
                stack.push(a);
                Some("not")
            }
            Expr::Binary(_, a, b) => {
                stack.push(b);
                stack.push(a);
                None
            }
            Expr::Logic(logic, a, b) => {
                stack.push(b);
                stack.push(a);
                Some(if *logic == Logic::And { "and" } else { "or" })
            }
            Expr::Compare { first, rest } => {
                stack.extend(rest.iter().rev().map(|(_, e)| e));
                stack.push(first);
                None
            }
            Expr::Call(n, args) | Expr::Index(n, args) => {
                stack.extend(args.iter().rev());
                (matches!(e, Expr::Call(..)) && n == "if").then_some("if")
            }
            Expr::Matrix { items, .. } => {
                stack.extend(items.iter().rev());
                None
            }
            Expr::Cases { cases, otherwise } => {
                push_cases(&mut stack, cases, otherwise.as_deref());
                for word in ["if", "otherwise"].into_iter().take(if otherwise.is_some() { 2 } else { 1 }) {
                    if !out.contains(&word) {
                        out.push(word);
                    }
                }
                None
            }
            Expr::Range { first, second, last } => {
                stack.push(last);
                if let Some(second) = second {
                    stack.push(second);
                }
                stack.push(first);
                None
            }
            Expr::Sum { first, second, last, body, .. } => {
                stack.push(body);
                stack.push(last);
                if let Some(second) = second {
                    stack.push(second);
                }
                stack.push(first);
                None
            }
            Expr::Solve { equations, .. } => {
                stack.extend(equations.iter().rev().flat_map(|(lhs, rhs)| [rhs, lhs]));
                None
            }
        };
        if let Some(word) = word
            && !out.contains(&word)
        {
            out.push(word);
        }
    }
    out
}
