//! Bounded token decoding for regular and intermittent InkML channels.

use super::{MAX_POINTS, bounded, format::Format, number, spend};

/// One raw sample; an explicitly unknown intermittent channel has no value.
pub(super) type Sample = Vec<Option<f64>>;

/// Decode all samples with independent integration state per channel and per trace fragment.
pub(super) fn read(text: &str, format: &Format, work: &mut usize, count: &mut usize) -> Option<Vec<Sample>> {
    let mut tokens = Tokens(text.trim());
    let mut state: Vec<_> = format
        .channels
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if i < format.regular {
                ChannelState::default()
            } else {
                ChannelState { value: Some(c.default), last: Some(c.default), ..Default::default() }
            }
        })
        .collect();
    let mut samples = Vec::new();
    while !tokens.0.is_empty() {
        spend(work, format.channels.len() + 1)?;
        if *count >= MAX_POINTS {
            return None;
        }
        let mut values = Vec::with_capacity(state.len());
        for (i, (channel, s)) in format.channels.iter().zip(&mut state).enumerate() {
            tokens.space();
            let value = if i >= format.regular && tokens.end_point() {
                s.repeat()?
            } else if i >= format.regular && tokens.0.starts_with('?') {
                tokens.0 = tokens.0.get(1..)?;
                None
            } else {
                s.next(&mut tokens, channel.boolean)?;
                s.value
            };
            if value.is_some_and(|v| !channel.accepts(v)) {
                return None;
            }
            values.push(value);
        }
        samples.push(values);
        *count += 1;
        tokens.space();
        if !tokens.0.is_empty() {
            tokens.0 = tokens.0.strip_prefix(',')?;
            tokens.space();
        }
    }
    (!samples.is_empty()).then_some(samples)
}

/// Accumulated explicit, first-difference and second-difference state for one channel.
#[derive(Clone, Default)]
struct ChannelState {
    /// Current reconstructed value.
    value: Option<f64>,
    /// Current velocity, needed before a second difference is legal.
    delta: Option<f64>,
    /// Most recently encoded number (not the integrated value).
    last: Option<f64>,
    /// Active difference order, initially explicit.
    order: u8,
}

impl ChannelState {
    /// Integrate the last encoded value for an omitted intermittent sample.
    fn repeat(&mut self) -> Option<Option<f64>> {
        self.integrate(self.last?)?;
        Some(self.value)
    }

    /// Integrate a finite token using the active order.
    fn integrate(&mut self, value: f64) -> Option<()> {
        let next = match self.order {
            0 => {
                self.delta = None;
                value
            }
            1 => {
                self.delta = Some(value);
                self.value? + value
            }
            2 => {
                let delta = self.delta? + value;
                self.delta = Some(delta);
                self.value? + delta
            }
            _ => return None,
        };
        if !bounded(next) {
            return None;
        }
        self.value = Some(next);
        self.last = Some(value);
        Some(())
    }

    /// Consume one numeric, boolean, or wildcard value without reading past the point boundary.
    fn next(&mut self, tokens: &mut Tokens<'_>, boolean: bool) -> Option<()> {
        tokens.space();
        let prefix = match tokens.0.as_bytes().first()? {
            b'!' => Some(0),
            b'\'' => Some(1),
            b'"' => Some(2),
            _ => None,
        };
        if let Some(order) = prefix {
            if boolean {
                return None;
            }
            self.order = order;
            tokens.0 = tokens.0.get(1..)?;
            tokens.space();
        }
        let value = if let Some(rest) = tokens.0.strip_prefix('*') {
            if prefix.is_some() {
                return None;
            }
            tokens.0 = rest;
            self.last?
        } else if boolean {
            let v = match tokens.0.as_bytes().first()? {
                b'T' => 1.0,
                b'F' => 0.0,
                _ => return None,
            };
            tokens.0 = tokens.0.get(1..)?;
            v
        } else {
            tokens.number()?
        };
        self.integrate(value)
    }
}

/// Cursor over compact comma-separated trace or affine/table data.
pub(super) struct Tokens<'a>(pub &'a str);

impl Tokens<'_> {
    /// Skip XML whitespace.
    pub fn space(&mut self) {
        self.0 = self.0.trim_start_matches([' ', '\t', '\r', '\n']);
    }

    /// Whether the next channel would cross a comma or end of trace.
    fn end_point(&self) -> bool {
        self.0.is_empty() || self.0.starts_with(',')
    }

    /// Consume ASCII digits while slicing only at ASCII boundaries.
    fn digits(&mut self) -> usize {
        let len = self.0.bytes().take_while(u8::is_ascii_digit).count();
        self.0 = self.0.get(len..).unwrap_or("");
        len
    }

    /// Read signed decimal, exponential or hexadecimal syntax with bounded magnitude.
    pub fn number(&mut self) -> Option<f64> {
        self.space();
        let sign = if let Some(rest) = self.0.strip_prefix('-') {
            self.0 = rest;
            self.space();
            -1.0
        } else {
            1.0
        };
        if let Some(rest) = self.0.strip_prefix('#') {
            let len = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
            if len == 0 || len > 8 {
                return None;
            }
            let value = u32::from_str_radix(rest.get(..len)?, 16).ok()? as f64 * sign;
            self.0 = rest.get(len..)?;
            return bounded(value).then_some(value);
        }
        let start = self.0;
        let mut digits = self.digits();
        if let Some(rest) = self.0.strip_prefix('.') {
            self.0 = rest;
            digits += self.digits();
        }
        if digits == 0 {
            return None;
        }
        if self.0.starts_with(['e', 'E']) {
            self.0 = self.0.get(1..)?;
            if self.0.starts_with(['+', '-']) {
                self.0 = self.0.get(1..)?;
            }
            if self.digits() == 0 {
                return None;
            }
        }
        Some(number(start.get(..start.len().saturating_sub(self.0.len()))?)? * sign)
    }
}
