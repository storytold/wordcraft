//! Trace channel declarations, physical units, and device-resolution calibration.

use super::{El, MAX_CHANNELS, MAX_DEPTH, number, spend};

/// One typed channel in the exact order used by the trace tokens.
#[derive(Clone, Debug)]
pub(super) struct Channel {
    /// Reserved or application-defined channel name.
    pub name: String,
    /// Boolean channels use `T`/`F` rather than numeric integration.
    pub boolean: bool,
    /// Integer channels reject fractional samples.
    pub integer: bool,
    /// Initial value for an omitted intermittent channel.
    pub default: f64,
    /// Optional inclusive range declared by the device or canvas.
    pub min: Option<f64>,
    /// Optional inclusive upper bound.
    pub max: Option<f64>,
    /// Unit and orientation conversion; `None` means uncalibrated device units.
    pub scale: Option<f64>,
    /// Axis orientation retained while a device-unit channel awaits calibration.
    pub orientation: f64,
    /// Whether force/transparency/color samples are already normalized fractions.
    pub fraction: bool,
}

impl Channel {
    /// Reject nonfinite, mistyped, or out-of-range samples before any rendering.
    pub fn accepts(&self, value: f64) -> bool {
        super::bounded(value)
            && (!self.boolean || value == 0.0 || value == 1.0)
            && (!self.integer || value.fract() == 0.0)
            && self.min.is_none_or(|v| value >= v)
            && self.max.is_none_or(|v| value <= v)
    }

    /// Normalize a visual channel with an explicit range or fractional units.
    pub fn fraction(&self, value: f64) -> Option<f64> {
        let v = if self.fraction {
            value
        } else {
            let min = self.min.unwrap_or(0.0);
            let max = self.max?;
            if max <= min {
                return None;
            }
            (value - min) / (max - min)
        };
        (v.is_finite() && (0.0..=1.0).contains(&v)).then_some(v)
    }
}

/// Regular channels followed by optional intermittent channels.
#[derive(Clone, Debug)]
pub(super) struct Format {
    /// Ordered declarations, bounded by `MAX_CHANNELS`.
    pub channels: Vec<Channel>,
    /// Number of mandatory values at each sample.
    pub regular: usize,
}

impl Default for Format {
    /// The application's default canvas uses point-like X/Y coordinates.
    fn default() -> Self {
        Self {
            channels: ["X", "Y"]
                .into_iter()
                .map(|name| Channel {
                    name: name.into(),
                    boolean: false,
                    integer: false,
                    default: 0.0,
                    min: None,
                    max: None,
                    scale: Some(1.0),
                    orientation: 1.0,
                    fraction: false,
                })
                .collect(),
            regular: 2,
        }
    }
}

impl Format {
    /// Find a named channel; declarations cannot contain duplicate names.
    pub fn index(&self, name: &str) -> Option<usize> {
        self.channels.iter().position(|c| c.name == name)
    }

    /// Read a value in the channel's physical units, including its orientation.
    pub fn value(&self, values: &[Option<f64>], name: &str) -> Option<f64> {
        let i = self.index(name)?;
        Some(values.get(i).copied().flatten()? * self.channels.get(i)?.scale?)
    }

    /// Get a normalized force, alpha, or individual color component.
    pub fn fraction(&self, values: &[Option<f64>], name: &str) -> Option<f64> {
        let i = self.index(name)?;
        self.channels.get(i)?.fraction(values.get(i).copied().flatten()?)
    }
}

/// Read a complete format, allowing custom channels without interpreting them as visual data.
pub(super) fn read(el: &El, work: &mut usize) -> Option<Format> {
    charge(el, work, 0)?;
    let mut out = Format { channels: Vec::new(), regular: 0 };
    let mut intermittent = false;
    for child in el.els() {
        match child.name.as_str() {
            "ink:channel" if !intermittent => {
                add(&mut out, child)?;
                out.regular += 1;
            }
            "ink:intermittentChannels" if !intermittent => {
                intermittent = true;
                for channel in child.els() {
                    add(&mut out, channel)?;
                }
            }
            _ => return None,
        }
    }
    (!out.channels.is_empty()).then_some(out)
}

/// Parse and append a uniquely named channel with bounded defaults and units.
fn add(format: &mut Format, el: &El) -> Option<()> {
    if el.name != "ink:channel" || format.channels.len() >= MAX_CHANNELS {
        return None;
    }
    let name = el.attr("name")?;
    if name.is_empty() || name.len() > 128 || format.index(name).is_some() {
        return None;
    }
    let kind = el.attr("type").unwrap_or("decimal");
    if !matches!(kind, "decimal" | "double" | "integer" | "boolean") {
        return None;
    }
    let boolean = kind == "boolean";
    let default = match el.attr("default") {
        Some("T" | "true") if boolean => 1.0,
        Some("F" | "false") if boolean => 0.0,
        Some(v) => number(v)?,
        None => 0.0,
    };
    let min = match el.attr("min") {
        Some(v) => Some(number(v)?),
        None => None,
    };
    let max = match el.attr("max") {
        Some(v) => Some(number(v)?),
        None => None,
    };
    if min.zip(max).is_some_and(|(a, b)| a > b) {
        return None;
    }
    let sign = match el.attr("orientation") {
        None | Some("+ve") => 1.0,
        Some("-ve") => -1.0,
        _ => return None,
    };
    let units = el.attr("units");
    let scale = if matches!(name, "X" | "Y" | "W" | "BW" | "BH") {
        if el.attr("respectTo").is_some() {
            return None;
        }
        match units {
            Some("dev") => None,
            _ => Some(unit(units)? * sign),
        }
    } else if name == "OR" {
        Some(
            match units {
                None | Some("deg") => std::f64::consts::PI / 180.0,
                Some("rad") => 1.0,
                _ => return None,
            } * sign,
        )
    } else {
        Some(sign)
    };
    format.channels.push(Channel {
        name: name.into(),
        boolean,
        integer: kind == "integer",
        default,
        min,
        max,
        scale,
        orientation: sign,
        fraction: units == Some("%"),
    });
    Some(())
}

/// Read the source format and calibrate `dev` units from physical channel resolutions.
pub(super) fn source(el: &El, work: &mut usize) -> Option<Format> {
    let mut format = read(el.child("ink:traceFormat")?, work)?;
    calibrate(&mut format, el, work)?;
    Some(format)
}

/// Apply source resolutions to a trace format, including an explicit format overriding the source's.
pub(super) fn calibrate(format: &mut Format, el: &El, work: &mut usize) -> Option<()> {
    charge(el, work, 0)?;
    if let Some(properties) = el.child("ink:channelProperties") {
        for p in properties.els() {
            if p.name != "ink:channelProperty" || p.attr("name") != Some("resolution") {
                continue;
            }
            let Some(i) = format.index(p.attr("channel")?) else {
                continue;
            };
            let c = format.channels.get_mut(i)?;
            if c.scale.is_some() {
                continue;
            }
            let v = number(p.attr("value")?)?;
            if v <= 0.0 {
                return None;
            }
            let units = p.attr("units")?;
            let scale = if let Some(u) = units.strip_prefix("1/") { unit(Some(u))? / v } else { unit(Some(units))? * v };
            c.scale = Some(scale * c.orientation);
        }
    }
    Some(())
}

/// Length units in application points; nominal em/ex sizes are application-defined.
pub(super) fn unit(value: Option<&str>) -> Option<f64> {
    match value {
        None | Some("em" | "pt") => Some(1.0),
        Some("ex") => Some(0.5),
        Some("in") => Some(72.0),
        Some("cm") => Some(72.0 / 2.54),
        Some("mm") => Some(72.0 / 25.4),
        Some("m") => Some(72.0 / 0.0254),
        Some("pc") => Some(12.0),
        _ => None,
    }
}

/// Charge repeated format/source scans before resolving attributes; references cannot amplify
/// a declaration with thousands of attributes into unaccounted quadratic work.
pub(super) fn charge(el: &El, work: &mut usize, depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let bytes = el.attrs.iter().fold(0usize, |n, (key, value)| n.saturating_add(key.len()).saturating_add(value.len()));
    spend(work, el.attrs.len().saturating_add(el.kids.len()).saturating_mul(16).saturating_add(bytes.div_ceil(16)).max(1))?;
    for child in el.els() {
        charge(child, work, depth + 1)?;
    }
    Some(())
}
