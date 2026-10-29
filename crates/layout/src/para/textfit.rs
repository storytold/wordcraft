//! Manual run width, after shaping and the document grid, before line breaking.

use super::Arc;
use super::builder::Builder;
use super::model::ClKind;

/// Contiguous formatted runs sharing a fit ID occupy the width once, even when font
/// fallback or a change of formatting splits them into several drawing styles.
pub(super) fn fit_text(b: &mut Builder) {
    let mut start = 0;
    while start < b.clusters.len() {
        let Some(first) = b.clusters.get(start) else { break };
        let fit = b.styles.get(first.style as usize).and_then(|s| s.rc.fit_text);
        let Some(fit) = fit.filter(|f| f.width.is_finite() && f.width > 0.0) else {
            start += 1;
            continue;
        };
        if !matches!(first.kind, ClKind::Text | ClKind::Space) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while let Some(c) = b.clusters.get(end) {
            if !matches!(c.kind, ClKind::Text | ClKind::Space) {
                break;
            }
            let next = b.styles.get(c.style as usize).and_then(|s| s.rc.fit_text);
            // Word treats a missing ID as zero ([MS-OI29500] fitText implementation note).
            if !next.is_some_and(|n| n.width > 0.0 && n.id.unwrap_or(0) == fit.id.unwrap_or(0) && n.width == fit.width) {
                break;
            }
            end += 1;
        }
        let width: f32 = b.clusters.get(start..end).unwrap_or_default().iter().map(|c| c.adv).sum();
        let factor = fit.width.clamp(0.1, 1584.0) / width;
        // The font dialog and resolved formatting accept horizontal scales from 1% to
        // 600%. A hostile tiny advance or huge requested width must not escape that
        // drawing budget. Keep the natural text if fitting would exceed it.
        let bounded = b.clusters.get(start..end).unwrap_or_default().iter().all(|c| {
            b.styles.get(c.style as usize).is_some_and(|st| {
                let scale = st.rc.scale * factor;
                scale.is_finite() && (1.0..=600.0).contains(&scale)
            })
        });
        if width > 0.0 && factor.is_finite() && factor > 0.0 && bounded {
            // A style may also be used outside this group: clone it only for these clusters.
            let mut styles = std::collections::HashMap::new();
            for c in b.clusters.get_mut(start..end).unwrap_or_default() {
                let style = *styles.entry(c.style).or_insert_with(|| {
                    let Some(mut st) = b.styles.get(c.style as usize).cloned() else { return c.style };
                    let mut rc = (*st.rc).clone();
                    rc.scale *= factor;
                    st.rc = Arc::new(rc);
                    let Ok(index) = u32::try_from(b.styles.len()) else { return c.style };
                    b.styles.push(st);
                    index
                });
                c.style = style;
                c.adv *= factor;
                for g in b.glyphs.get_mut(c.g0 as usize..c.g1 as usize).into_iter().flatten() {
                    g.dx *= factor;
                }
            }
        }
        start = end;
    }
}
