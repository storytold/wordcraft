//! Editing inside an equation: caret positions, typing, inserting structures, deleting,
//! moving the caret, and Word-style build-up (typing `a/b` then a space makes a fraction) and
//! Math AutoCorrect (`\alpha` then a space makes α).
//!
//! A caret position is an argument — a path of (node index, child argument) from the equation's
//! top-level argument — and an offset in *units*: each character of a run is one unit, every
//! other node is one unit. All operations take any position and clamp it; none panics.

use serde::{Deserialize, Serialize};

use crate::math::{Arg, LimLoc, MAX_DEPTH, MNode, MRun, MScr, MSty, ScriptKind, has_number_mark, merge_runs, to_linear};

/// A caret position inside an equation.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MathPos {
    /// (node index, child argument index) from the top level down to the argument.
    #[serde(default)]
    pub path: Vec<(usize, usize)>,
    /// Units into that argument.
    #[serde(default)]
    pub off: usize,
}

impl MathPos {
    pub fn new(path: Vec<(usize, usize)>, off: usize) -> MathPos {
        MathPos { path, off }
    }
}

/// How newly typed text is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeStyle {
    /// Normal (non-math) text.
    pub nor: bool,
}

/// Units in an argument.
pub fn units(a: &[MNode]) -> usize {
    a.iter().map(node_units).sum()
}

fn node_units(n: &MNode) -> usize {
    match n {
        MNode::Run(r) => r.text.chars().count(),
        _ => 1,
    }
}

/// Number of child arguments of a node.
pub fn child_count(n: &MNode) -> usize {
    match n {
        MNode::Run(_) => 0,
        MNode::Frac { .. } | MNode::Func { .. } | MNode::Lim { .. } => 2,
        MNode::Rad { .. } => 2,
        MNode::Script { .. } | MNode::Nary { .. } => 3,
        MNode::Delim { elems, .. } => elems.len(),
        MNode::EqArr { rows } => rows.len(),
        MNode::Matrix { rows, .. } => rows.iter().map(|r| r.len()).sum(),
        MNode::Acc { .. } | MNode::Bar { .. } | MNode::BorderBox { .. } | MNode::Boxed { .. } | MNode::GroupChr { .. } | MNode::Phant { .. } => 1,
    }
}

/// Child argument `i` of a node (see [`child_count`] for the numbering: fraction num 0 / den 1;
/// script base 0 / sub 1 / sup 2; radical degree 0 / base 1; n-ary sub 0 / sup 1 / operand 2;
/// function name 0 / argument 1; limit base 0 / limit 1; matrices row by row).
pub fn child(n: &MNode, i: usize) -> Option<&Arg> {
    match (n, i) {
        (MNode::Frac { num, .. }, 0) => Some(num),
        (MNode::Frac { den, .. }, 1) => Some(den),
        (MNode::Script { base, .. }, 0) => Some(base),
        (MNode::Script { sub, .. }, 1) => Some(sub),
        (MNode::Script { sup, .. }, 2) => Some(sup),
        (MNode::Rad { deg, .. }, 0) => Some(deg),
        (MNode::Rad { e, .. }, 1) => Some(e),
        (MNode::Nary { sub, .. }, 0) => Some(sub),
        (MNode::Nary { sup, .. }, 1) => Some(sup),
        (MNode::Nary { e, .. }, 2) => Some(e),
        (MNode::Delim { elems, .. }, i) => elems.get(i),
        (MNode::Func { name, .. }, 0) => Some(name),
        (MNode::Func { e, .. }, 1) => Some(e),
        (MNode::Lim { e, .. }, 0) => Some(e),
        (MNode::Lim { lim, .. }, 1) => Some(lim),
        (
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. },
            0,
        ) => Some(e),
        (MNode::EqArr { rows }, i) => rows.get(i),
        (MNode::Matrix { rows, .. }, i) => rows.iter().flatten().nth(i),
        _ => None,
    }
}

pub fn child_mut(n: &mut MNode, i: usize) -> Option<&mut Arg> {
    match (n, i) {
        (MNode::Frac { num, .. }, 0) => Some(num),
        (MNode::Frac { den, .. }, 1) => Some(den),
        (MNode::Script { base, .. }, 0) => Some(base),
        (MNode::Script { sub, .. }, 1) => Some(sub),
        (MNode::Script { sup, .. }, 2) => Some(sup),
        (MNode::Rad { deg, .. }, 0) => Some(deg),
        (MNode::Rad { e, .. }, 1) => Some(e),
        (MNode::Nary { sub, .. }, 0) => Some(sub),
        (MNode::Nary { sup, .. }, 1) => Some(sup),
        (MNode::Nary { e, .. }, 2) => Some(e),
        (MNode::Delim { elems, .. }, i) => elems.get_mut(i),
        (MNode::Func { name, .. }, 0) => Some(name),
        (MNode::Func { e, .. }, 1) => Some(e),
        (MNode::Lim { e, .. }, 0) => Some(e),
        (MNode::Lim { lim, .. }, 1) => Some(lim),
        (
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. },
            0,
        ) => Some(e),
        (MNode::EqArr { rows }, i) => rows.get_mut(i),
        (MNode::Matrix { rows, .. }, i) => rows.iter_mut().flatten().nth(i),
        _ => None,
    }
}

/// Children in reading order, without hidden ones (hidden limits, a hidden radical degree,
/// script slots the script kind doesn't use).
pub fn nav_children(n: &MNode) -> Vec<usize> {
    match n {
        MNode::Script { kind, .. } => match kind {
            ScriptKind::Sup => vec![0, 2],
            ScriptKind::Sub => vec![0, 1],
            ScriptKind::SubSup => vec![0, 1, 2],
            ScriptKind::Pre => vec![1, 2, 0],
        },
        MNode::Rad { deg_hide, .. } => {
            if *deg_hide {
                vec![1]
            } else {
                vec![0, 1]
            }
        }
        MNode::Nary { sub_hide, sup_hide, lim_loc, .. } => {
            let mut v = Vec::new();
            // Under/over limits read top to bottom as Word does: upper first only for sub/sup.
            let _ = lim_loc;
            if !sub_hide {
                v.push(0);
            }
            if !sup_hide {
                v.push(1);
            }
            v.push(2);
            v
        }
        _ => (0..child_count(n)).collect(),
    }
}

/// The argument at `path`.
pub fn arg_at<'a>(root: &'a Arg, path: &[(usize, usize)]) -> Option<&'a Arg> {
    let mut a = root;
    for (n, c) in path {
        a = child(a.get(*n)?, *c)?;
    }
    Some(a)
}

pub fn arg_at_mut<'a>(root: &'a mut Arg, path: &[(usize, usize)]) -> Option<&'a mut Arg> {
    let mut a = root;
    for (n, c) in path {
        a = child_mut(a.get_mut(*n)?, *c)?;
    }
    Some(a)
}

/// A valid position: the longest valid path prefix, the offset clamped.
pub fn clamp(root: &Arg, pos: &MathPos) -> MathPos {
    let mut path = Vec::new();
    let mut a = root;
    for (n, c) in pos.path.iter().take(MAX_DEPTH) {
        match a.get(*n).and_then(|node| child(node, *c)) {
            Some(next) => {
                path.push((*n, *c));
                a = next;
            }
            None => {
                // The node is gone or has no such child: put the caret before it.
                let off = node_start(a, (*n).min(a.len()));
                return MathPos { path, off };
            }
        }
    }
    MathPos { off: pos.off.min(units(a)), path }
}

/// Units before node `i`.
pub fn node_start(a: &[MNode], i: usize) -> usize {
    a.iter().take(i).map(node_units).sum()
}

/// Where an offset falls: before node `i` (`i` may be the length), or inside run `i` at char `k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loc {
    Before(usize),
    Inside(usize, usize),
}

fn locate(a: &[MNode], off: usize) -> Loc {
    let mut u = 0;
    for (i, n) in a.iter().enumerate() {
        if off == u {
            return Loc::Before(i);
        }
        let len = node_units(n);
        if off < u + len {
            return Loc::Inside(i, off - u);
        }
        u += len;
    }
    Loc::Before(a.len())
}

/// Byte index of char `k` in `s`.
fn byte_at(s: &str, k: usize) -> usize {
    s.char_indices().nth(k).map(|(b, _)| b).unwrap_or(s.len())
}

/// Split a run so that `off` falls between nodes; returns that node index.
fn split_at(a: &mut Arg, off: usize) -> usize {
    match locate(a, off) {
        Loc::Before(i) => i,
        Loc::Inside(i, k) => {
            let Some(MNode::Run(r)) = a.get_mut(i) else { return i };
            let b = byte_at(&r.text, k);
            let tail = r.text.split_off(b);
            let mut right = r.clone();
            right.text = tail;
            a.insert(i + 1, MNode::Run(right));
            i + 1
        }
    }
}

fn compatible(r: &MRun, style: TypeStyle) -> bool {
    r.nor == style.nor && !r.lit
}

/// Type `text` at `pos`. Returns the caret after it.
pub fn insert_text(root: &mut Arg, pos: &MathPos, text: &str, style: TypeStyle) -> MathPos {
    let pos = clamp(root, pos);
    let n = text.chars().count();
    if n == 0 {
        return pos;
    }
    let Some(a) = arg_at_mut(root, &pos.path) else { return pos };
    match locate(a, pos.off) {
        Loc::Inside(i, k) => {
            if let Some(MNode::Run(r)) = a.get_mut(i)
                && compatible(r, style)
            {
                let b = byte_at(&r.text, k);
                r.text.insert_str(b, text);
            } else {
                let at = split_at(a, pos.off);
                a.insert(at, MNode::Run(new_run(text, style)));
            }
        }
        Loc::Before(i) => {
            let prev_ok = i > 0 && matches!(a.get(i - 1), Some(MNode::Run(r)) if compatible(r, style));
            let next_ok = matches!(a.get(i), Some(MNode::Run(r)) if compatible(r, style));
            if prev_ok {
                if let Some(MNode::Run(r)) = a.get_mut(i - 1) {
                    r.text.push_str(text);
                }
            } else if next_ok {
                if let Some(MNode::Run(r)) = a.get_mut(i) {
                    r.text.insert_str(0, text);
                }
            } else {
                a.insert(i.min(a.len()), MNode::Run(new_run(text, style)));
            }
        }
    }
    MathPos { path: pos.path, off: pos.off + n }
}

fn new_run(text: &str, style: TypeStyle) -> MRun {
    if style.nor { MRun { text: text.to_string(), nor: true, sty: Some(MSty::Plain), ..Default::default() } } else { MRun::new(text) }
}

/// Insert structures at `pos`. The caret goes to the first empty argument inside them, or after
/// them.
pub fn insert_nodes(root: &mut Arg, pos: &MathPos, nodes: Arg) -> MathPos {
    let pos = clamp(root, pos);
    let count = nodes.len();
    let total = units(&nodes);
    let Some(a) = arg_at_mut(root, &pos.path) else { return pos };
    let at = split_at(a, pos.off).min(a.len());
    for (k, n) in nodes.into_iter().enumerate() {
        a.insert(at + k, n);
    }
    // First empty argument in the inserted nodes.
    for k in at..at + count {
        if let Some(n) = a.get(k)
            && let Some(mut inner) = first_empty(n, 0)
        {
            let mut path = pos.path.clone();
            path.push((k, inner.remove(0).1));
            path.extend(inner);
            return MathPos { path, off: 0 };
        }
    }
    let off = node_start(a, at) + total;
    tidy(root);
    clamp(root, &MathPos { path: pos.path, off })
}

/// Path (relative, starting with this node as index 0) to the first empty visible argument.
fn first_empty(n: &MNode, depth: usize) -> Option<Vec<(usize, usize)>> {
    if depth > MAX_DEPTH {
        return None;
    }
    for c in nav_children(n) {
        let a = child(n, c)?;
        if a.is_empty() {
            return Some(vec![(0, c)]);
        }
        for (k, inner) in a.iter().enumerate() {
            if let Some(mut p) = first_empty(inner, depth + 1) {
                let first = p.remove(0);
                let mut out = vec![(0, c), (k, first.1)];
                out.extend(p);
                return Some(out);
            }
        }
    }
    None
}

/// Merge runs and drop empty ones everywhere (structures stay, empty or not).
pub fn tidy(root: &mut Arg) {
    fn go(a: &mut Arg, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        a.retain(|n| !matches!(n, MNode::Run(r) if r.text.is_empty()));
        let taken = std::mem::take(a);
        *a = merge_shallow(taken);
        for n in a.iter_mut() {
            for c in 0..child_count(n) {
                if let Some(ch) = child_mut(n, c) {
                    go(ch, depth + 1);
                }
            }
        }
    }
    go(root, 0);
}

fn merge_shallow(a: Arg) -> Arg {
    let mut out: Arg = Vec::with_capacity(a.len());
    for n in a {
        if let MNode::Run(r) = &n
            && let Some(MNode::Run(prev)) = out.last_mut()
            && prev.sty == r.sty
            && prev.scr == r.scr
            && prev.nor == r.nor
            && prev.lit == r.lit
            && prev.size == r.size
            && prev.color == r.color
            && prev.font == r.font
        {
            prev.text.push_str(&r.text);
            continue;
        }
        out.push(n);
    }
    out
}

/// Every visible argument of the node is empty.
fn all_empty(n: &MNode) -> bool {
    nav_children(n).iter().all(|c| child(n, *c).is_none_or(|a| a.is_empty()))
}

/// The children's contents in reading order, one after another (dissolving a structure).
fn dissolve(n: MNode) -> Arg {
    let order = nav_children(&n);
    let mut n = n;
    let mut out = Vec::new();
    for c in order {
        if let Some(a) = child_mut(&mut n, c) {
            out.append(a);
        }
    }
    out
}

/// Backspace at `pos`.
pub fn delete_back(root: &mut Arg, pos: &MathPos) -> MathPos {
    let pos = clamp(root, pos);
    if pos.off > 0 {
        let Some(a) = arg_at_mut(root, &pos.path) else { return pos };
        let (i, k) = match locate(a, pos.off - 1) {
            Loc::Before(i) => (i, 0),
            Loc::Inside(i, k) => (i, k),
        };
        match a.get_mut(i) {
            Some(MNode::Run(r)) => {
                let b = byte_at(&r.text, k);
                let e = byte_at(&r.text, k + 1);
                r.text.replace_range(b..e, "");
                let out = MathPos { path: pos.path.clone(), off: pos.off - 1 };
                tidy(root);
                return clamp(root, &out);
            }
            Some(n) => {
                if all_empty(n) {
                    a.remove(i);
                    let out = MathPos { path: pos.path.clone(), off: pos.off - 1 };
                    tidy(root);
                    return clamp(root, &out);
                }
                // Into the structure's last argument, at its end.
                if let Some(c) = nav_children(n).last().copied() {
                    let len = child(n, c).map(|a| units(a)).unwrap_or(0);
                    let mut path = pos.path.clone();
                    path.push((i, c));
                    return MathPos { path, off: len };
                }
            }
            None => {}
        }
        return pos;
    }
    // At the start of an argument: remove an empty structure, or dissolve it from its first
    // argument, or move to the previous argument.
    let Some(((j, c), parent_path)) = pos.path.split_last() else { return pos };
    let parent_path = parent_path.to_vec();
    let (j, c) = (*j, *c);
    let Some(parent) = arg_at_mut(root, &parent_path) else { return pos };
    let start = node_start(parent, j);
    let Some(node) = parent.get(j) else { return pos };
    let order = nav_children(node);
    let first = order.first() == Some(&c);
    if all_empty(node) || first {
        let n = parent.remove(j);
        let contents = if all_empty(&n) { Vec::new() } else { dissolve(n) };
        for (k, x) in contents.into_iter().enumerate() {
            parent.insert(j + k, x);
        }
        tidy(root);
        return clamp(root, &MathPos { path: parent_path, off: start });
    }
    let prev = order.iter().position(|x| *x == c).and_then(|p| p.checked_sub(1)).and_then(|p| order.get(p)).copied();
    match prev {
        Some(pc) => {
            let len = child(node, pc).map(|a| units(a)).unwrap_or(0);
            let mut path = parent_path;
            path.push((j, pc));
            MathPos { path, off: len }
        }
        None => MathPos { path: parent_path, off: start },
    }
}

/// Delete (forward) at `pos`.
pub fn delete_forward(root: &mut Arg, pos: &MathPos) -> MathPos {
    let pos = clamp(root, pos);
    let len = arg_at(root, &pos.path).map(|a| units(a)).unwrap_or(0);
    if pos.off < len {
        let Some(a) = arg_at_mut(root, &pos.path) else { return pos };
        let (i, k) = match locate(a, pos.off) {
            Loc::Before(i) => (i, 0),
            Loc::Inside(i, k) => (i, k),
        };
        match a.get_mut(i) {
            Some(MNode::Run(r)) => {
                let b = byte_at(&r.text, k);
                let e = byte_at(&r.text, k + 1);
                r.text.replace_range(b..e, "");
            }
            Some(n) => {
                if all_empty(n) {
                    a.remove(i);
                } else if let Some(c) = nav_children(n).first().copied() {
                    let mut path = pos.path.clone();
                    path.push((i, c));
                    return MathPos { path, off: 0 };
                }
            }
            None => {}
        }
        tidy(root);
        return clamp(root, &pos);
    }
    // At the end of an argument: remove an empty structure, else move on.
    let Some(((j, c), parent_path)) = pos.path.split_last() else { return pos };
    let parent_path = parent_path.to_vec();
    let (j, c) = (*j, *c);
    let Some(parent) = arg_at_mut(root, &parent_path) else { return pos };
    let start = node_start(parent, j);
    let Some(node) = parent.get(j) else { return pos };
    if all_empty(node) {
        parent.remove(j);
        tidy(root);
        return clamp(root, &MathPos { path: parent_path, off: start });
    }
    let order = nav_children(node);
    let next = order.iter().position(|x| *x == c).and_then(|p| order.get(p + 1)).copied();
    match next {
        Some(nc) => {
            let mut path = parent_path;
            path.push((j, nc));
            MathPos { path, off: 0 }
        }
        None => MathPos { path: parent_path, off: start + 1 },
    }
}

/// One step right; `None` at the very end (the caret leaves the equation).
pub fn move_right(root: &Arg, pos: &MathPos) -> Option<MathPos> {
    let pos = clamp(root, pos);
    let a = arg_at(root, &pos.path)?;
    if pos.off < units(a) {
        match locate(a, pos.off) {
            Loc::Inside(..) => return Some(MathPos { path: pos.path, off: pos.off + 1 }),
            Loc::Before(i) => {
                let n = a.get(i)?;
                if let MNode::Run(_) = n {
                    return Some(MathPos { path: pos.path, off: pos.off + 1 });
                }
                if let Some(c) = nav_children(n).first().copied() {
                    let mut path = pos.path;
                    path.push((i, c));
                    return Some(MathPos { path, off: 0 });
                }
                return Some(MathPos { path: pos.path, off: pos.off + 1 });
            }
        }
    }
    let ((j, c), parent_path) = pos.path.split_last()?;
    let parent = arg_at(root, parent_path)?;
    let node = parent.get(*j)?;
    let order = nav_children(node);
    let next = order.iter().position(|x| x == c).and_then(|p| order.get(p + 1)).copied();
    let mut path = parent_path.to_vec();
    Some(match next {
        Some(nc) => {
            path.push((*j, nc));
            MathPos { path, off: 0 }
        }
        None => MathPos { off: node_start(parent, *j) + 1, path },
    })
}

/// One step left; `None` at the very start.
pub fn move_left(root: &Arg, pos: &MathPos) -> Option<MathPos> {
    let pos = clamp(root, pos);
    let a = arg_at(root, &pos.path)?;
    if pos.off > 0 {
        let (i, inside) = match locate(a, pos.off - 1) {
            Loc::Before(i) => (i, false),
            Loc::Inside(i, _) => (i, true),
        };
        let n = a.get(i)?;
        if inside || matches!(n, MNode::Run(_)) {
            return Some(MathPos { path: pos.path, off: pos.off - 1 });
        }
        if let Some(c) = nav_children(n).last().copied() {
            let len = child(n, c).map(|x| units(x)).unwrap_or(0);
            let mut path = pos.path;
            path.push((i, c));
            return Some(MathPos { path, off: len });
        }
        return Some(MathPos { path: pos.path, off: pos.off - 1 });
    }
    let ((j, c), parent_path) = pos.path.split_last()?;
    let parent = arg_at(root, parent_path)?;
    let node = parent.get(*j)?;
    let order = nav_children(node);
    let prev = order.iter().position(|x| x == c).and_then(|p| p.checked_sub(1)).and_then(|p| order.get(p)).copied();
    let mut path = parent_path.to_vec();
    Some(match prev {
        Some(pc) => {
            let len = child(node, pc).map(|x| units(x)).unwrap_or(0);
            path.push((*j, pc));
            MathPos { path, off: len }
        }
        None => MathPos { off: node_start(parent, *j), path },
    })
}

/// The child to move to from child `c` of `n` going up (`up`) or down.
fn vertical_target(n: &MNode, c: usize, up: bool) -> Option<usize> {
    match n {
        MNode::Frac { .. } => match (c, up) {
            (1, true) => Some(0),
            (0, false) => Some(1),
            _ => None,
        },
        MNode::Script { kind, .. } => {
            let has_sub = matches!(kind, ScriptKind::Sub | ScriptKind::SubSup | ScriptKind::Pre);
            let has_sup = matches!(kind, ScriptKind::Sup | ScriptKind::SubSup | ScriptKind::Pre);
            match (c, up) {
                (1, true) if has_sup => Some(2),
                (2, false) if has_sub => Some(1),
                (0, true) if has_sup => Some(2),
                (0, false) if has_sub => Some(1),
                _ => None,
            }
        }
        MNode::Nary { sub_hide, sup_hide, .. } => match (c, up) {
            (0, true) if !sup_hide => Some(1),
            (1, false) if !sub_hide => Some(0),
            _ => None,
        },
        MNode::Lim { upper, .. } => match (c, up, upper) {
            (0, true, true) | (0, false, false) => Some(1),
            (1, false, true) | (1, true, false) => Some(0),
            _ => None,
        },
        MNode::EqArr { rows } => {
            if up {
                c.checked_sub(1)
            } else {
                (c + 1 < rows.len()).then_some(c + 1)
            }
        }
        MNode::Matrix { rows, .. } => {
            // Row and column of flat index `c`.
            let mut k = 0;
            let mut rc = None;
            for (r, row) in rows.iter().enumerate() {
                if c < k + row.len() {
                    rc = Some((r, c - k));
                    break;
                }
                k += row.len();
            }
            let (r, col) = rc?;
            let tr = if up { r.checked_sub(1)? } else { r + 1 };
            let row = rows.get(tr)?;
            if row.is_empty() {
                return None;
            }
            let start: usize = rows.iter().take(tr).map(|x| x.len()).sum();
            Some(start + col.min(row.len() - 1))
        }
        _ => None,
    }
}

/// Up or down into the nearest stacked argument (denominator, superscript, matrix row…).
pub fn move_vertical(root: &Arg, pos: &MathPos, up: bool) -> Option<MathPos> {
    let pos = clamp(root, pos);
    for level in (0..pos.path.len()).rev() {
        let (j, c) = *pos.path.get(level)?;
        let parent = arg_at(root, pos.path.get(..level)?)?;
        let node = parent.get(j)?;
        if let Some(t) = vertical_target(node, c, up) {
            let target = child(node, t)?;
            let mut path = pos.path.get(..level)?.to_vec();
            path.push((j, t));
            let off = if level + 1 == pos.path.len() { pos.off.min(units(target)) } else { units(target) };
            return Some(MathPos { path, off });
        }
    }
    None
}

/// Start (`end` false) or end of the current argument; from there, of the whole equation.
pub fn home_end(root: &Arg, pos: &MathPos, end: bool) -> MathPos {
    let pos = clamp(root, pos);
    let len = arg_at(root, &pos.path).map(|a| units(a)).unwrap_or(0);
    let target = if end { len } else { 0 };
    if pos.off != target || pos.path.is_empty() {
        return MathPos { path: pos.path, off: target };
    }
    MathPos { path: Vec::new(), off: if end { units(root) } else { 0 } }
}

/// All argument paths in reading order (for Tab between placeholders).
fn arg_paths(a: &[MNode], prefix: &mut Vec<(usize, usize)>, out: &mut Vec<Vec<(usize, usize)>>, depth: usize) {
    if depth > MAX_DEPTH || out.len() > 10_000 {
        return;
    }
    out.push(prefix.clone());
    for (i, n) in a.iter().enumerate() {
        for c in nav_children(n) {
            if let Some(ch) = child(n, c) {
                prefix.push((i, c));
                arg_paths(ch, prefix, out, depth + 1);
                prefix.pop();
            }
        }
    }
}

/// The next (or previous) empty argument after `pos`, wrapping around; `None` when there is none.
pub fn next_placeholder(root: &Arg, pos: &MathPos, back: bool) -> Option<MathPos> {
    let mut paths = Vec::new();
    arg_paths(root, &mut Vec::new(), &mut paths, 0);
    let empties: Vec<&Vec<(usize, usize)>> = paths.iter().filter(|p| !p.is_empty() && arg_at(root, p).is_some_and(|a| a.is_empty())).collect();
    if empties.is_empty() {
        return None;
    }
    let cur = paths.iter().position(|p| *p == pos.path).unwrap_or(0);
    let idx = |p: &Vec<(usize, usize)>| paths.iter().position(|q| q == p).unwrap_or(0);
    let pick = if back {
        empties.iter().rev().find(|p| idx(p) < cur).or_else(|| empties.last())
    } else {
        empties.iter().find(|p| idx(p) > cur).or_else(|| empties.first())
    };
    pick.map(|p| MathPos { path: (*p).clone(), off: 0 })
}

/// Math AutoCorrect: replace `\name` just before the caret with its character. Returns the new
/// caret when something changed.
pub fn autocorrect(root: &mut Arg, pos: &MathPos) -> Option<MathPos> {
    let pos = clamp(root, pos);
    let a = arg_at_mut(root, &pos.path)?;
    let (i, k) = match locate(a, pos.off) {
        Loc::Inside(i, k) => (i, k),
        Loc::Before(i) => {
            let i = i.checked_sub(1)?;
            (i, node_units(a.get(i)?))
        }
    };
    let MNode::Run(r) = a.get_mut(i)? else { return None };
    if r.nor || r.lit {
        return None;
    }
    let before: Vec<char> = r.text.chars().take(k).collect();
    let slash = before.iter().rposition(|c| *c == '\\')?;
    let name: String = before.get(slash + 1..)?.iter().collect();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let c = crate::math_symbols::symbol(&name)?;
    let b0 = byte_at(&r.text, slash);
    let b1 = byte_at(&r.text, k);
    r.text.replace_range(b0..b1, &c.to_string());
    Some(MathPos { path: pos.path, off: pos.off - name.chars().count() })
}

/// Characters that make typed linear text build up into structures.
fn builds(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            '/' | '⁄'
                | '∕'
                | '¦'
                | '^'
                | '_'
                | '√'
                | '∛'
                | '∜'
                | '▒'
                | '┬'
                | '┴'
                | '█'
                | '■'
                | '▭'
                | '□'
                | '¯'
                | '▁'
                | '⟡'
                | '⏞'
                | '⏟'
                | '('
                | '['
                | '{'
                | '|'
                | '‖'
                | '⟨'
                | '⌈'
                | '⌊'
                | '〖'
                | '├'
                | '\u{2061}'
        ) || crate::math_linear::is_combining(c)
            || crate::math::is_nary_char(c)
    }) || crate::math_linear::FUNCTION_NAMES.iter().any(|f| s.contains(f))
}

/// A run is plain math text that build-up may rewrite.
fn plain(n: &MNode) -> bool {
    match n {
        MNode::Run(r) => !r.nor && !r.lit && r.scr == MScr::Roman && r.color.is_none() && r.size.is_none() && r.font.is_none(),
        // Arrays and matrices are finished pieces; typing after them builds up on its own.
        MNode::EqArr { .. } | MNode::Matrix { .. } => false,
        _ => true,
    }
}

/// Unclosed brackets (still being typed) or LaTeX braces.
fn unbalanced(nodes: &[MNode], depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    nodes.iter().any(|n| match n {
        MNode::Delim { beg: Some(_), end: None, .. } => true,
        MNode::Run(r) => r.text.contains('\\'),
        other => (0..child_count(other)).any(|c| child(other, c).is_some_and(|a| unbalanced(a, depth + 1))),
    })
}

/// Build up the linear text before the caret in its argument (`latex`: read it as LaTeX).
/// Returns the new caret when the structure changed.
pub fn build_up(root: &mut Arg, pos: &MathPos, latex: bool) -> Option<MathPos> {
    let pos = clamp(root, pos);
    let a = arg_at_mut(root, &pos.path)?;
    let at = split_at(a, pos.off);
    // Rewrite from after the last formatted run (its formatting would be lost) to the caret.
    let start = a.get(..at)?.iter().rposition(|n| !plain(n)).map(|k| k + 1).unwrap_or(0);
    let region: Arg = a.get(start..at)?.to_vec();
    if region.is_empty() {
        tidy(root);
        return None;
    }
    let text = to_linear(&region);
    if !latex && !builds(&text) {
        tidy(root);
        return None;
    }
    let mut built = if latex { crate::math_latex::parse_latex(&text) } else { crate::math_linear::parse(&text) };
    // `#` numbering wraps the whole equation, not a piece of it.
    if let [MNode::EqArr { rows }] = built.as_slice()
        && rows.len() == 1
        && rows.first().is_some_and(|r| has_number_mark(r))
    {
        built = rows.first().cloned().unwrap_or_default();
    }
    let built = merge_runs(built);
    let unchanged = merge_runs(region.clone()) == built;
    if unchanged || (!latex && unbalanced(&built, 0)) || (latex && text.matches('{').count() != text.matches('}').count()) {
        tidy(root);
        return None;
    }
    let new_units = units(&built);
    let before = node_start(a, start);
    a.splice(start..at, built);
    let out = MathPos { path: pos.path, off: before + new_units };
    tidy(root);
    Some(clamp(root, &out))
}

/// A top-level `#` numbers the equation: wrap it as a one-row equation array.
pub fn number_equation(root: &mut Arg) -> bool {
    if has_number_mark(root) && !matches!(root.as_slice(), [MNode::EqArr { .. }]) {
        let row = std::mem::take(root);
        *root = vec![MNode::EqArr { rows: vec![row] }];
        return true;
    }
    false
}

/// Word's right-click actions for the structures around the caret: (level, action id, label),
/// innermost first. `level` indexes `pos.path` (the structure is the node at that step).
pub fn structure_actions(root: &Arg, pos: &MathPos) -> Vec<(usize, &'static str, &'static str)> {
    let pos = clamp(root, pos);
    let mut out = Vec::new();
    for level in (0..pos.path.len()).rev() {
        let Some(&(j, _)) = pos.path.get(level) else { continue };
        let Some(node) = pos.path.get(..level).and_then(|p| arg_at(root, p)).and_then(|a| a.get(j)) else { continue };
        let mut add = |a: &'static str, l: &'static str| out.push((level, a, l));
        match node {
            MNode::Frac { kind, .. } => {
                use crate::math::FracKind::*;
                for (k, a, l) in [
                    (Bar, "frac.bar", "Change to Stacked Fraction"),
                    (Skewed, "frac.skewed", "Change to Skewed Fraction"),
                    (Linear, "frac.linear", "Change to Linear Fraction"),
                    (NoBar, "frac.noBar", "Remove Fraction Bar"),
                ] {
                    if *kind != k {
                        add(a, l);
                    }
                }
            }
            MNode::Rad { deg_hide, .. } => add("rad.toggleDegree", if *deg_hide { "Show Degree" } else { "Hide Degree" }),
            MNode::Nary { lim_loc, sub_hide, sup_hide, grow, chr, .. } => {
                let under = lim_loc.unwrap_or(if crate::math::is_integral(*chr) { LimLoc::SubSup } else { LimLoc::UndOvr }) == LimLoc::UndOvr;
                add(
                    if under { "nary.limitsSide" } else { "nary.limitsUnder" },
                    if under { "Change Limit Location to Subscript-Superscript" } else { "Change Limit Location to Under-Over" },
                );
                add("nary.toggleLower", if *sub_hide { "Show Lower Limit" } else { "Hide Lower Limit" });
                add("nary.toggleUpper", if *sup_hide { "Show Upper Limit" } else { "Hide Upper Limit" });
                add("nary.toggleGrow", if *grow { "Don't Grow with Argument" } else { "Grow with Argument" });
            }
            MNode::Delim { grow, elems, .. } => {
                add("delim.addSeparator", "Insert Argument After");
                if elems.len() > 1 {
                    add("delim.deleteArgument", "Delete Argument");
                }
                add("delim.toggleGrow", if *grow { "Don't Stretch Brackets" } else { "Stretch Brackets" });
            }
            MNode::Script { kind: ScriptKind::SubSup, .. } => {
                add("script.removeSub", "Remove Subscript");
                add("script.removeSup", "Remove Superscript");
            }
            MNode::Script { kind: ScriptKind::Sub, .. } => add("script.addSup", "Add Superscript"),
            MNode::Script { kind: ScriptKind::Sup, .. } => add("script.addSub", "Add Subscript"),
            MNode::Matrix { .. } => {
                add("matrix.rowAbove", "Insert Row Above");
                add("matrix.rowBelow", "Insert Row Below");
                add("matrix.colBefore", "Insert Column Before");
                add("matrix.colAfter", "Insert Column After");
                add("matrix.deleteRow", "Delete Row");
                add("matrix.deleteCol", "Delete Column");
            }
            MNode::EqArr { .. } => {
                add("eqarr.rowAbove", "Insert Equation Before");
                add("eqarr.rowBelow", "Insert Equation After");
                add("eqarr.deleteRow", "Delete Equation");
            }
            MNode::GroupChr { top, .. } => add("group.flip", if *top { "Move Character Below" } else { "Move Character Above" }),
            MNode::Bar { top, .. } => add("bar.flip", if *top { "Change to Underbar" } else { "Change to Overbar" }),
            _ => {}
        }
        add(
            "remove",
            match node {
                MNode::Delim { .. } => "Remove Brackets",
                MNode::Rad { .. } => "Remove Radical",
                MNode::Frac { .. } => "Remove Fraction",
                MNode::Script { .. } => "Remove Scripts",
                MNode::Nary { .. } => "Remove Operator",
                MNode::Acc { .. } => "Remove Accent",
                MNode::Matrix { .. } => "Remove Matrix",
                _ => "Remove Structure",
            },
        );
    }
    out
}

/// Apply a structure action (from [`structure_actions`]) at `level`. Returns the new caret.
pub fn apply_structure(root: &mut Arg, pos: &MathPos, level: usize, action: &str) -> Option<MathPos> {
    let pos = clamp(root, pos);
    let &(j, c) = pos.path.get(level)?;
    let parent_path = pos.path.get(..level)?.to_vec();
    let inner = MathPos { path: pos.path.clone(), off: pos.off };
    let parent = arg_at_mut(root, &parent_path)?;
    let start = node_start(parent, j);
    let node = parent.get_mut(j)?;
    let here = |child: usize| {
        let mut p = parent_path.clone();
        p.push((j, child));
        MathPos { path: p, off: 0 }
    };
    let out = match (node, action) {
        (_, "remove") => {
            let n = parent.remove(j);
            for (k, x) in dissolve(n).into_iter().enumerate() {
                parent.insert(j + k, x);
            }
            MathPos { path: parent_path.clone(), off: start }
        }
        (MNode::Frac { kind, .. }, a) if a.starts_with("frac.") => {
            use crate::math::FracKind::*;
            *kind = match a {
                "frac.skewed" => Skewed,
                "frac.linear" => Linear,
                "frac.noBar" => NoBar,
                _ => Bar,
            };
            inner
        }
        (MNode::Rad { deg_hide, deg, .. }, "rad.toggleDegree") => {
            *deg_hide = !*deg_hide;
            if *deg_hide {
                deg.clear();
                here(1)
            } else {
                here(0)
            }
        }
        (MNode::Nary { lim_loc, .. }, "nary.limitsSide") => {
            *lim_loc = Some(LimLoc::SubSup);
            inner
        }
        (MNode::Nary { lim_loc, .. }, "nary.limitsUnder") => {
            *lim_loc = Some(LimLoc::UndOvr);
            inner
        }
        (MNode::Nary { sub_hide, sub, .. }, "nary.toggleLower") => {
            *sub_hide = !*sub_hide;
            if *sub_hide {
                sub.clear();
                here(2)
            } else {
                here(0)
            }
        }
        (MNode::Nary { sup_hide, sup, .. }, "nary.toggleUpper") => {
            *sup_hide = !*sup_hide;
            if *sup_hide {
                sup.clear();
                here(2)
            } else {
                here(1)
            }
        }
        (MNode::Nary { grow, .. }, "nary.toggleGrow") | (MNode::Delim { grow, .. }, "delim.toggleGrow") => {
            *grow = !*grow;
            inner
        }
        (MNode::Delim { elems, sep, .. }, "delim.addSeparator") => {
            let at = (c + 1).min(elems.len());
            elems.insert(at, Vec::new());
            if sep.is_none() {
                *sep = Some('|');
            }
            here(at)
        }
        (MNode::Delim { elems, .. }, "delim.deleteArgument") if elems.len() > 1 => {
            let k = c.min(elems.len() - 1);
            elems.remove(k);
            here(k.min(elems.len() - 1))
        }
        (MNode::Script { kind, sub, .. }, "script.removeSub") => {
            *kind = ScriptKind::Sup;
            sub.clear();
            here(2)
        }
        (MNode::Script { kind, sup, .. }, "script.removeSup") => {
            *kind = ScriptKind::Sub;
            sup.clear();
            here(1)
        }
        (MNode::Script { kind, .. }, "script.addSup") | (MNode::Script { kind, .. }, "script.addSub") => {
            *kind = ScriptKind::SubSup;
            here(if action == "script.addSup" { 2 } else { 1 })
        }
        (MNode::Matrix { rows, .. }, a) if a.starts_with("matrix.") => {
            let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
            // Row and column of the caret's cell.
            let mut k = 0;
            let mut rc = (0, 0);
            for (r, row) in rows.iter().enumerate() {
                if c < k + row.len() {
                    rc = (r, c - k);
                    break;
                }
                k += row.len();
            }
            let (r, col) = rc;
            let (mut tr, mut tc) = (r, col);
            match a {
                "matrix.rowAbove" | "matrix.rowBelow" if rows.len() < 1000 => {
                    let at = if a == "matrix.rowAbove" { r } else { r + 1 };
                    rows.insert(at.min(rows.len()), vec![Vec::new(); ncols]);
                    tr = at;
                }
                "matrix.colBefore" | "matrix.colAfter" if ncols < 64 => {
                    let at = if a == "matrix.colBefore" { col } else { col + 1 };
                    for row in rows.iter_mut() {
                        row.insert(at.min(row.len()), Vec::new());
                    }
                    tc = at;
                }
                "matrix.deleteRow" if rows.len() > 1 => {
                    rows.remove(r.min(rows.len() - 1));
                    tr = r.min(rows.len() - 1);
                }
                "matrix.deleteCol" if ncols > 1 => {
                    for row in rows.iter_mut() {
                        if col < row.len() {
                            row.remove(col);
                        }
                    }
                    tc = col.min(ncols - 2);
                }
                _ => {}
            }
            let flat: usize = rows.iter().take(tr).map(|x| x.len()).sum::<usize>() + tc;
            here(flat)
        }
        (MNode::EqArr { rows }, a) if a.starts_with("eqarr.") => match a {
            "eqarr.rowAbove" | "eqarr.rowBelow" if rows.len() < 1000 => {
                let at = if a == "eqarr.rowAbove" { c } else { c + 1 }.min(rows.len());
                rows.insert(at, Vec::new());
                here(at)
            }
            "eqarr.deleteRow" if rows.len() > 1 => {
                let k = c.min(rows.len() - 1);
                rows.remove(k);
                here(k.min(rows.len() - 1))
            }
            _ => inner,
        },
        (MNode::GroupChr { top, chr, .. }, "group.flip") => {
            *top = !*top;
            *chr = match *chr {
                '⏞' => '⏟',
                '⏟' => '⏞',
                '⏜' => '⏝',
                '⏝' => '⏜',
                '⎴' => '⎵',
                '⎵' => '⎴',
                other => other,
            };
            inner
        }
        (MNode::Bar { top, .. }, "bar.flip") => {
            *top = !*top;
            inner
        }
        _ => return None,
    };
    tidy(root);
    Some(clamp(root, &out))
}

/// Set the limit placement of n-ary operators (`None`: Word's default by operator).
pub fn set_limits(nodes: &mut Arg, loc: Option<LimLoc>) {
    for n in nodes.iter_mut() {
        if let MNode::Nary { lim_loc, .. } = n {
            *lim_loc = loc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::parse_linear;

    fn lin(a: &Arg) -> String {
        to_linear(a)
    }

    #[test]
    fn typing_and_structures() {
        let mut eq: Arg = Vec::new();
        let mut p = MathPos::default();
        p = insert_text(&mut eq, &p, "x=", TypeStyle::default());
        p = insert_nodes(&mut eq, &p, parse_linear("⬚/⬚"));
        assert_eq!(p.path, vec![(1, 0)], "caret in the numerator");
        p = insert_text(&mut eq, &p, "a", TypeStyle::default());
        p = move_right(&eq, &p).unwrap();
        assert_eq!(p.path, vec![(1, 1)], "then the denominator");
        p = insert_text(&mut eq, &p, "b", TypeStyle::default());
        p = move_right(&eq, &p).unwrap();
        assert_eq!(p, MathPos::new(vec![], 3), "then after the fraction");
        assert_eq!(move_right(&eq, &p), None);
        assert_eq!(lin(&eq), "x=a/b");
        // Up from the denominator reaches the numerator.
        let den = MathPos::new(vec![(1, 1)], 1);
        assert_eq!(move_vertical(&eq, &den, true).unwrap().path, vec![(1, 0)]);
    }

    #[test]
    fn build_up_and_autocorrect() {
        let mut eq: Arg = Vec::new();
        let p = insert_text(&mut eq, &MathPos::default(), "x^2+\\alpha", TypeStyle::default());
        let p = autocorrect(&mut eq, &p).unwrap();
        assert_eq!(lin(&eq), "x^2+α");
        let p = build_up(&mut eq, &p, false).unwrap();
        assert!(matches!(eq[0], MNode::Script { kind: ScriptKind::Sup, .. }), "{eq:#?}");
        assert_eq!(p.off, units(&eq));
        // An unclosed bracket waits.
        let mut eq: Arg = Vec::new();
        let p = insert_text(&mut eq, &MathPos::default(), "(a+b", TypeStyle::default());
        assert!(build_up(&mut eq, &p, false).is_none());
        // LaTeX input.
        let mut eq: Arg = Vec::new();
        let p = insert_text(&mut eq, &MathPos::default(), "\\frac{a}{b}", TypeStyle::default());
        assert!(build_up(&mut eq, &p, true).is_some());
        assert!(matches!(eq[0], MNode::Frac { .. }));
    }

    #[test]
    fn backspace_dissolves_and_removes() {
        let mut eq = parse_linear("√x");
        // At the start of the radicand: the radical goes, x stays.
        let p = delete_back(&mut eq, &MathPos::new(vec![(0, 1)], 0));
        assert_eq!(lin(&eq), "x");
        assert_eq!(p, MathPos::new(vec![], 0));
        // An empty structure is removed by backspace after it.
        let mut eq = parse_linear("a+⬚/⬚");
        let p = delete_back(&mut eq, &MathPos::new(vec![], 3));
        assert_eq!(lin(&eq), "a+");
        assert_eq!(p.off, 2);
    }

    #[test]
    fn placeholders_and_numbering() {
        let eq = parse_linear("∑_⬚^⬚▒⬚");
        let p = next_placeholder(&eq, &MathPos::default(), false).unwrap();
        assert_eq!(p.path, vec![(0, 0)]);
        let p = next_placeholder(&eq, &p, false).unwrap();
        assert_eq!(p.path, vec![(0, 1)]);
        let mut eq = parse_linear("E=mc^2");
        let end = MathPos::new(vec![], units(&eq));
        let p = insert_text(&mut eq, &end, "#(1)", TypeStyle::default());
        let _ = build_up(&mut eq, &p, false);
        assert!(number_equation(&mut eq));
        assert_eq!(lin(&eq), "E=mc^2#(1)");
    }

    #[test]
    fn structure_actions_work() {
        let mut eq = parse_linear("■(a&b@c&d)");
        let p = MathPos::new(vec![(0, 3)], 0);
        let acts = structure_actions(&eq, &p);
        assert!(acts.iter().any(|a| a.1 == "matrix.rowBelow"));
        let p = apply_structure(&mut eq, &p, 0, "matrix.rowBelow").unwrap();
        let p = apply_structure(&mut eq, &p, 0, "matrix.colAfter").unwrap();
        assert_eq!(lin(&eq), "■(a&b&@c&d&@&&)");
        let _ = apply_structure(&mut eq, &p, 0, "matrix.deleteRow").unwrap();
        let mut eq = parse_linear("√x");
        let p = apply_structure(&mut eq, &MathPos::new(vec![(0, 1)], 1), 0, "rad.toggleDegree").unwrap();
        assert_eq!(p.path, vec![(0, 0)], "caret into the degree");
        let mut eq = parse_linear("a+(b)");
        apply_structure(&mut eq, &MathPos::new(vec![(1, 0)], 0), 0, "remove").unwrap();
        assert_eq!(lin(&eq), "a+b");
        // Every action on every structure, at hostile positions, never panics.
        let all = parse_linear("x=(-b±√(3&b^2-4ac))/2a+∑_(i=1)^n▒i+■(a&b@c&d)+█(x@y)+⏞(a)+¯(b)+x_1^2+(a│b)");
        let mut paths = Vec::new();
        arg_paths(&all, &mut Vec::new(), &mut paths, 0);
        for path in paths {
            let p = MathPos::new(path, 0);
            for (level, act, _) in structure_actions(&all, &p) {
                let mut e = all.clone();
                let _ = apply_structure(&mut e, &p, level, act);
                let _ = apply_structure(&mut e, &p, level + 5, act);
            }
        }
    }

    #[test]
    fn hostile_positions_never_panic() {
        let mut eq = parse_linear("x=(-b±√(b^2-4ac))/2a+■(a&b@c&d)");
        let bad = [MathPos::new(vec![(99, 99)], 5), MathPos::new(vec![(1, 9)], 999), MathPos::new(vec![(0, 0); 300], 0), MathPos::default()];
        for p in &bad {
            let _ = move_left(&eq, p);
            let _ = move_right(&eq, p);
            let _ = move_vertical(&eq, p, true);
            let _ = home_end(&eq, p, true);
            let _ = next_placeholder(&eq, p, false);
            let _ = insert_text(&mut eq.clone(), p, "é", TypeStyle { nor: true });
            let _ = insert_nodes(&mut eq.clone(), p, parse_linear("⬚/⬚"));
            let _ = delete_back(&mut eq.clone(), p);
            let _ = delete_forward(&mut eq.clone(), p);
            let _ = build_up(&mut eq, p, false);
            let _ = autocorrect(&mut eq, p);
        }
        // Walk the whole equation both ways: always terminates.
        let mut p = MathPos::default();
        for _ in 0..500 {
            match move_right(&eq, &p) {
                Some(n) => p = n,
                None => break,
            }
        }
        assert_eq!(p, MathPos::new(vec![], units(&eq)));
        for _ in 0..500 {
            match move_left(&eq, &p) {
                Some(n) => p = n,
                None => break,
            }
        }
        assert_eq!(p, MathPos::default());
    }
}
