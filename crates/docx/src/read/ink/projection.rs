//! Per-trace compiled source-to-canvas projection, including mapped visual channels.

use super::{Context, El, Format, mapping::Mapping, spend, trace::Sample};
use std::collections::HashMap;

/// Source samples overlaid with the regular canvas channels after transformation.
pub(super) struct Projection {
    /// Effective format used for coordinates, brush channels and pen contact state.
    pub format: Format,
    /// Regular output canvas channels; intermittent canvas declarations have no effect.
    canvas: Format,
    /// Canvas channel positions in the effective format, empty without an explicit canvas/map.
    destinations: Vec<usize>,
    /// Source-to-canvas mapping; missing means the identity.
    mapping: Option<Mapping>,
    /// Source indices used by the implicit identity mapping.
    sources: Vec<Option<usize>>,
}

impl Projection {
    /// Compile the forward map once and determine the precedence of mapped channels over source data.
    pub fn new(ctx: &Context<'_>, ids: &HashMap<&str, &El>, work: &mut usize) -> Option<Self> {
        let mut canvas = ctx.canvas.as_deref().cloned().unwrap_or_default();
        canvas.channels.truncate(canvas.regular);
        let mapping = match ctx.transform {
            Some(transform) => Some(Mapping::read(transform.child("ink:mapping")?, &ctx.format, &canvas, ids, work, 0)?),
            None => None,
        };
        let mut format = (*ctx.format).clone();
        spend(work, canvas.channels.len() + format.channels.len())?;
        let mut destinations = Vec::new();
        if mapping.is_some() || ctx.canvas.is_some() {
            for c in &canvas.channels {
                let i = format.index(&c.name).unwrap_or(format.channels.len());
                if let Some(existing) = format.channels.get_mut(i) {
                    *existing = c.clone();
                } else {
                    format.channels.push(c.clone());
                }
                destinations.push(i);
            }
        }
        let sources = canvas.channels.iter().map(|c| ctx.format.index(&c.name)).collect();
        Some(Self { format, canvas, destinations, mapping, sources })
    }

    /// Map one bounded sample, enforce declared canvas ranges and round integer outputs.
    pub fn apply(&self, values: &Sample, work: &mut usize) -> Option<Sample> {
        spend(work, self.format.channels.len() + self.canvas.channels.len())?;
        let mut resolved = values.clone();
        resolved.resize(self.format.channels.len(), None);
        if !self.destinations.is_empty() {
            let mut mapped = vec![None; self.canvas.channels.len()];
            if let Some(mapping) = &self.mapping {
                mapping.apply(values, &mut mapped, work)?;
            } else {
                for (out, source) in mapped.iter_mut().zip(&self.sources) {
                    if let Some(i) = source {
                        *out = *values.get(*i)?;
                    }
                }
            }
            for ((c, value), destination) in self.canvas.channels.iter().zip(&mut mapped).zip(&self.destinations) {
                if let Some(v) = value {
                    if c.integer {
                        *v = v.round();
                    }
                    if !c.accepts(*v) {
                        return None;
                    }
                }
                *resolved.get_mut(*destination)? = *value;
            }
        }
        Some(resolved)
    }
}
