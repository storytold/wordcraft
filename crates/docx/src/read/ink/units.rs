use super::{MAX_DEPTH, bounded};

pub(super) const LENGTH: [i8; 4] = [1, 0, 0, 0];
pub(super) const ANGLE: [i8; 4] = [0, 0, 0, 1];

#[derive(Clone, Copy)]
pub(super) struct Quantity {
    pub scale: f64,
    pub dimension: [i8; 4],
}

/// Convert supported length units to points, retaining application defaults for omitted units.
pub(super) fn length(value: Option<&str>) -> Option<f64> {
    match value {
        None | Some("pt" | "em") => Some(1.0),
        Some("ex") => Some(0.5),
        Some(value) => {
            let quantity = parse(value)?;
            (quantity.dimension == LENGTH).then_some(quantity.scale)
        }
    }
}

/// Convert a positive device resolution with matching direct or reciprocal dimensions.
pub(super) fn resolution(value: f64, units: &str, dimension: [i8; 4]) -> Option<f64> {
    if value <= 0.0 || !bounded(value) {
        return None;
    }
    let q = parse(units)?;
    let mut reciprocal = dimension;
    for d in &mut reciprocal {
        *d = d.checked_neg()?;
    }
    let scale = if q.dimension == dimension {
        q.scale * value
    } else if q.dimension == reciprocal {
        1.0 / (q.scale * value)
    } else {
        return None;
    };
    (scale > 0.0 && bounded(scale)).then_some(scale)
}

/// Parse a bounded compound unit expression into a scale and physical dimensions.
pub(super) fn parse(text: &str) -> Option<Quantity> {
    if text.len() > 256 {
        return None;
    }
    let mut parser = Parser { text, tokens: 0 };
    let quantity = parser.expression(0)?;
    parser.space();
    parser.text.is_empty().then_some(quantity)
}

struct Parser<'a> {
    text: &'a str,
    tokens: usize,
}

impl Parser<'_> {
    /// Consume leading XML whitespace between unit tokens.
    fn space(&mut self) {
        self.text = self.text.trim_start_matches([' ', '\t', '\r', '\n']);
    }

    /// Combine unit products and quotients with checked dimension arithmetic and bounded scales.
    fn expression(&mut self, depth: usize) -> Option<Quantity> {
        let mut left = self.primitive(depth)?;
        loop {
            self.space();
            let divide = match self.text.as_bytes().first() {
                Some(b'*') => false,
                Some(b'/') => true,
                _ => return Some(left),
            };
            self.text = self.text.get(1..)?;
            let right = self.primitive(depth)?;
            left.scale = if divide { left.scale / right.scale } else { left.scale * right.scale };
            if !bounded(left.scale) || left.scale <= 0.0 {
                return None;
            }
            for (a, b) in left.dimension.iter_mut().zip(right.dimension) {
                *a = a.checked_add(if divide { b.checked_neg()? } else { b })?;
            }
        }
    }

    /// Parse a unit symbol, dimensionless one or parenthesized expression within token/depth limits.
    fn primitive(&mut self, depth: usize) -> Option<Quantity> {
        self.space();
        self.tokens += 1;
        if self.tokens > 64 || depth > MAX_DEPTH {
            return None;
        }
        if let Some(rest) = self.text.strip_prefix('(') {
            self.text = rest;
            let quantity = self.expression(depth + 1)?;
            self.space();
            self.text = self.text.strip_prefix(')')?;
            return Some(quantity);
        }
        if let Some(rest) = self.text.strip_prefix('1') {
            self.text = rest;
            return Some(Quantity { scale: 1.0, dimension: [0; 4] });
        }
        let len = self.text.bytes().take_while(u8::is_ascii_alphabetic).count();
        let name = self.text.get(..len)?;
        self.text = self.text.get(len..)?;
        let (scale, dimension) = match name {
            "m" => (72.0 / 0.0254, LENGTH),
            "cm" => (72.0 / 2.54, LENGTH),
            "mm" => (72.0 / 25.4, LENGTH),
            "in" => (72.0, LENGTH),
            "pt" => (1.0, LENGTH),
            "pc" => (12.0, LENGTH),
            "kg" => (1.0, [0, 1, 0, 0]),
            "g" => (0.001, [0, 1, 0, 0]),
            "mg" => (0.000001, [0, 1, 0, 0]),
            "s" => (1.0, [0, 0, 1, 0]),
            "ms" => (0.001, [0, 0, 1, 0]),
            "N" => (72.0 / 0.0254, [1, 1, -2, 0]),
            "deg" => (std::f64::consts::PI / 180.0, ANGLE),
            "rad" => (1.0, ANGLE),
            _ => return None,
        };
        Some(Quantity { scale, dimension })
    }
}
