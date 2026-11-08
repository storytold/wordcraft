use super::{MAX_CHANNELS, MAX_DEPTH, Mapping, discrete, spend};

pub(super) fn groups<'a>(mapping: &'a Mapping, work: &mut usize) -> Option<Vec<Vec<&'a Mapping>>> {
    let mut leaves = Vec::new();
    flatten(mapping, &mut leaves, work, 0)?;
    let mut groups: Vec<(u32, Vec<&Mapping>)> = Vec::new();
    for leaf in leaves {
        let mut mask =
            discrete::dependencies(leaf, work)?.iter().try_fold(0u32, |mask, &i| Some(mask | 1u32.checked_shl(u32::try_from(i).ok()?)?))?;
        if mask == 0 {
            continue;
        }
        let mut component = vec![leaf];
        let mut i = 0;
        while i < groups.len() {
            spend(work, 1)?;
            if groups.get(i)?.0 & mask != 0 {
                let (prior, maps) = groups.swap_remove(i);
                mask |= prior;
                component.extend(maps);
                i = 0;
            } else {
                i += 1;
            }
        }
        groups.push((mask, component));
    }
    Some(groups.into_iter().map(|(_, maps)| maps).collect())
}

fn flatten<'a>(mapping: &'a Mapping, out: &mut Vec<&'a Mapping>, work: &mut usize, depth: usize) -> Option<()> {
    spend(work, 1)?;
    if depth > MAX_DEPTH {
        return None;
    }
    if let Mapping::Product(maps) = mapping {
        for map in maps {
            flatten(map, out, work, depth + 1)?;
        }
    } else {
        if out.len() >= MAX_CHANNELS {
            return None;
        }
        out.push(mapping);
    }
    Some(())
}
