use super::*;

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl<'de> Deserialize<'de> for Rgb {
    /// Reads what it writes (`[r, g, b]`) and also a string [`Rgb::parse`] reads, so the colours in
    /// `CharProps` / `ParaProps` take the `RRGGBB` that `para.borders` and `table.shading` take (#865).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, SeqAccess, Unexpected, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Rgb;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("`[r, g, b]`, `RRGGBB` or `#RRGGBB`")
            }
            fn visit_str<E: Error>(self, s: &str) -> Result<Rgb, E> {
                Rgb::parse(s).ok_or_else(|| E::invalid_value(Unexpected::Str(s), &self))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Rgb, A::Error> {
                let mut c = [0u8; 3];
                for (i, x) in c.iter_mut().enumerate() {
                    *x = a.next_element()?.ok_or_else(|| A::Error::invalid_length(i, &self))?;
                }
                if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(A::Error::invalid_length(4, &self));
                }
                Ok(Rgb(c[0], c[1], c[2]))
            }
        }
        d.deserialize_any(V)
    }
}

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);
    /// `RRGGBB` (no `#`).
    pub fn hex(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let p = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
        Some(Rgb(p(0)?, p(2)?, p(4)?))
    }
    /// This colour with the DrawingML colour modifier `name` (`lumMod`, `lumOff`, `tint`,
    /// `shade`, `satMod`, ECMA-376 §20.1.2.3) at `v` (1.0 = 100%); other names leave it
    /// unchanged.
    pub fn modified(self, name: &str, v: f64) -> Rgb {
        match name {
            "lumMod" => adjust_lum(self, |l| l * v),
            "lumOff" => adjust_lum(self, |l| l + v),
            "tint" => mix(self, 255.0, v),
            "shade" => mix(self, 0.0, v),
            "satMod" => {
                let (h, sat, l) = to_hsl(self);
                from_hsl(h, (sat * v).clamp(0.0, 1.0), l)
            }
            _ => self,
        }
    }
    /// The colour with hue `h` (0..1 of a turn), saturation `s` and lightness `l` (0..1).
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        from_hsl(h, s, l)
    }
    /// This colour with its HSL luminance changed by `f`.
    pub fn with_lum(self, f: impl Fn(f64) -> f64) -> Rgb {
        adjust_lum(self, f)
    }
    pub fn luma(self) -> f32 {
        0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32
    }
}

/// A text colour: automatic (black on light backgrounds, white on dark) or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Default)]
pub enum TextColor {
    #[default]
    Auto,
    Rgb(Rgb),
}

impl TextColor {
    /// `auto` or `automatic` (any case), or a colour [`Rgb::parse`] reads.
    pub fn parse(s: &str) -> Option<TextColor> {
        if s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("automatic") { Some(TextColor::Auto) } else { Rgb::parse(s).map(TextColor::Rgb) }
    }
}

impl<'de> Deserialize<'de> for TextColor {
    /// Reads what it writes (`"Auto"`, `{"Rgb": [r, g, b]}`) and also a string [`TextColor::parse`]
    /// reads, so `styles.modify` and `format.set` take the colours `format.color` takes.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, Unexpected};
        #[derive(Deserialize)]
        enum Written {
            Auto,
            Rgb(Rgb),
        }
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => {
                TextColor::parse(&s).ok_or_else(|| D::Error::invalid_value(Unexpected::Str(&s), &"`auto`, `RRGGBB` or `#RRGGBB`"))
            }
            v => Ok(match serde_json::from_value(v).map_err(D::Error::custom)? {
                Written::Auto => TextColor::Auto,
                Written::Rgb(c) => TextColor::Rgb(c),
            }),
        }
    }
}

/// `tint` (towards `to` = 255) and `shade` (towards 0): `v` is the share of the original colour kept.
fn mix(c: Rgb, to: f64, v: f64) -> Rgb {
    let f = |x: u8| (x as f64 * v + to * (1.0 - v)).round().clamp(0.0, 255.0) as u8;
    Rgb(f(c.0), f(c.1), f(c.2))
}

/// `c` with its HSL luminance changed by `f`.
fn adjust_lum(c: Rgb, f: impl Fn(f64) -> f64) -> Rgb {
    let (h, s, l) = to_hsl(c);
    from_hsl(h, s, f(l).clamp(0.0, 1.0))
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if s == 0.0 {
        return Rgb(byte(l), byte(l), byte(l));
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        byte(v)
    };
    Rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}
