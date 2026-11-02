use super::Placed;

/// A foreground group's indices exclude the prepended behind-text items.
#[derive(Clone)]
pub(super) struct Foreground {
    pub(super) range: std::ops::Range<usize>,
    pub(super) z: u32,
    /// Earlier paragraph-local ranks, including behind-text drawings.
    pub(super) floor: u32,
}

/// Keep each object's frame and text together until its story is complete.
pub(super) fn append_foreground(items: &mut Vec<Placed>, ranges: &mut Vec<Foreground>, behind: usize, front: Vec<(u32, u32, Vec<Placed>)>) {
    for (z, floor, group) in front {
        let start = items.len().saturating_sub(behind);
        ranges.push(Foreground { range: start..start.saturating_add(group.len()), z, floor });
        items.extend(group);
    }
}

/// Carry table-cell groups into their owning story without changing item order.
pub(super) fn extend_foreground(out: &mut Vec<Foreground>, groups: &[Foreground], offset: usize) {
    out.extend(groups.iter().map(|g| Foreground {
        range: g.range.start.saturating_add(offset)..g.range.end.saturating_add(offset),
        z: g.z,
        floor: g.floor,
    }));
}

pub(super) fn remap_foreground(groups: &[Foreground], map: &[std::ops::Range<usize>]) -> Vec<Foreground> {
    groups
        .iter()
        .filter_map(|g| {
            let mut kept = map.get(g.range.clone())?.iter().filter(|r| !r.is_empty());
            let first = kept.next()?;
            Some(Foreground { range: first.start..kept.next_back().unwrap_or(first).end, z: g.z, floor: g.floor })
        })
        .collect()
}

/// Foreground floats cover later inline content. Defer sorting until placement and vertical
/// alignment finish, keeping their indices valid for pagination and column balancing.
pub(super) fn order_foreground(items: &mut Vec<Placed>, ranges: &[Foreground], behind: usize) {
    if ranges.is_empty() {
        return;
    }
    let mut marked = vec![None; items.len()];
    for (i, group) in ranges.iter().enumerate() {
        let start = group.range.start.saturating_add(behind).min(marked.len());
        let end = group.range.end.saturating_add(behind).min(marked.len());
        if let Some(slice) = marked.get_mut(start..end) {
            slice.fill(Some(i));
        }
    }
    // Unknown ranks keep document order above ranks already encountered, as on main.
    let mut z_top = 0;
    let mut front: Vec<_> = ranges
        .iter()
        .map(|g| {
            z_top = z_top.max(g.z).max(g.floor);
            (if g.z == 0 { z_top } else { g.z }, Vec::new())
        })
        .collect();
    for (item, group) in std::mem::take(items).into_iter().zip(marked) {
        if let Some((_, group)) = group.and_then(|i| front.get_mut(i)) {
            group.push(item);
        } else {
            items.push(item);
        }
    }
    front.sort_by_key(|(z, _)| *z);
    for (_, group) in front {
        items.extend(group);
    }
}
