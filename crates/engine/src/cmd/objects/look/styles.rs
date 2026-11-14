//! Picture Styles (Picture Format › Picture Styles): our own gallery of looks, each a shape, the
//! frames round it and the Picture Effects it brings. A style replaces the picture's style, its
//! border and its effects; Picture Effects can then change those effects one by one.
//!
//! The first five ([`LEGACY`]) are drawn as they were before the gallery grew, so scripts that
//! name them keep drawing the same pictures.

use wordcraft_doc::props::Rgb;

use super::fx::{BevelKind, PicBevel, PicReflection, PicRotation, PicShadow, PictureEffects, Shape};

/// The styles drawn by `picture_style`, with the size of their frames a share of the picture.
pub const LEGACY: [(&str, &str); 5] =
    [("simpleFrame", "Simple Frame"), ("thickFrame", "Thick Frame"), ("rounded", "Rounded"), ("softEdge", "Soft Edge"), ("shadow", "Shadow")];

/// One line of a style's frame, from the picture outward: its colour and width (points); `metal`
/// shades it like polished metal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub color: Rgb,
    pub width: f32,
    pub metal: bool,
}

const fn ring(color: Rgb, width: f32) -> Ring {
    Ring { color, width, metal: false }
}

const fn metal(width: f32) -> Ring {
    Ring { color: Rgb(0xA0, 0xA0, 0xA4), width, metal: true }
}

const WHITE: Rgb = Rgb(255, 255, 255);
const BLACK: Rgb = Rgb(0, 0, 0);

/// A style of the gallery.
pub struct StyleSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub shape: Shape,
    pub rings: &'static [Ring],
    /// The Picture Effects it brings.
    pub effects: fn() -> PictureEffects,
}

fn shadow(blur: f32, distance: f32, angle: f32, transparency: f32) -> Option<PicShadow> {
    Some(PicShadow { blur, distance, angle, transparency, ..PicShadow::default() })
}

fn reflection(size: f32, distance: f32) -> Option<PicReflection> {
    Some(PicReflection { size, distance, ..PicReflection::default() })
}

fn bevel(kind: BevelKind, w: f32) -> Option<PicBevel> {
    Some(PicBevel { kind, width: w, height: w })
}

fn none() -> PictureEffects {
    PictureEffects::default()
}

fn drop_shadow() -> PictureEffects {
    PictureEffects { shadow: shadow(4.0, 3.0, 45.0, 60.0), ..none() }
}

fn soft_shadow() -> PictureEffects {
    PictureEffects { shadow: shadow(6.0, 2.0, 90.0, 65.0), ..none() }
}

/// The gallery's styles after [`LEGACY`].
pub static STYLES: [StyleSpec; 23] = [
    StyleSpec { id: "whiteFrameShadow", label: "White Frame, Shadow", shape: Shape::Rect, rings: &[ring(WHITE, 7.0)], effects: drop_shadow },
    StyleSpec { id: "thinBlackFrame", label: "Thin Black Frame", shape: Shape::Rect, rings: &[ring(BLACK, 3.0)], effects: none },
    StyleSpec {
        id: "doubleBlackFrame",
        label: "Double Black Frame",
        shape: Shape::Rect,
        rings: &[ring(BLACK, 1.5), ring(WHITE, 2.5), ring(BLACK, 1.5)],
        effects: none,
    },
    StyleSpec {
        id: "thickThinBlackFrame",
        label: "Thick and Thin Black Frame",
        shape: Shape::Rect,
        rings: &[ring(BLACK, 1.0), ring(WHITE, 1.5), ring(BLACK, 5.0)],
        effects: soft_shadow,
    },
    StyleSpec {
        id: "whiteMat",
        label: "White Mat",
        shape: Shape::Rect,
        rings: &[ring(Rgb(0x8C, 0x8C, 0x8C), 0.75), ring(WHITE, 18.0)],
        effects: drop_shadow,
    },
    StyleSpec {
        id: "blackMat",
        label: "Black Mat",
        shape: Shape::Rect,
        rings: &[ring(Rgb(0x40, 0x40, 0x40), 0.75), ring(BLACK, 18.0)],
        effects: soft_shadow,
    },
    StyleSpec { id: "metalFrame", label: "Metal Frame", shape: Shape::Rect, rings: &[metal(8.0)], effects: drop_shadow },
    StyleSpec { id: "metalRounded", label: "Metal, Rounded", shape: Shape::Rounded(0.12), rings: &[metal(7.0)], effects: soft_shadow },
    StyleSpec { id: "metalOval", label: "Metal Oval", shape: Shape::Oval, rings: &[metal(7.0)], effects: soft_shadow },
    StyleSpec {
        id: "dropShadow",
        label: "Drop Shadow",
        shape: Shape::Rect,
        rings: &[],
        effects: || PictureEffects { shadow: shadow(8.0, 6.0, 45.0, 55.0), ..none() },
    },
    StyleSpec {
        id: "shadowAllRound",
        label: "Shadow All Round",
        shape: Shape::Rect,
        rings: &[],
        effects: || PictureEffects { shadow: shadow(12.0, 0.0, 0.0, 50.0), ..none() },
    },
    StyleSpec {
        id: "roundedReflection",
        label: "Rounded, Reflection",
        shape: Shape::Rounded(0.15),
        rings: &[],
        effects: || PictureEffects { reflection: reflection(35.0, 0.0), ..none() },
    },
    StyleSpec { id: "softOval", label: "Soft Oval", shape: Shape::Oval, rings: &[], effects: || PictureEffects { soft_edge: Some(10.0), ..none() } },
    StyleSpec {
        id: "bevelledOval",
        label: "Bevelled Oval",
        shape: Shape::Oval,
        rings: &[ring(BLACK, 4.0)],
        effects: || PictureEffects { bevel: bevel(BevelKind::Circle, 5.0), shadow: shadow(6.0, 3.0, 90.0, 55.0), ..none() },
    },
    StyleSpec {
        id: "roundedDiagonal",
        label: "Rounded Diagonal Corners",
        shape: Shape::RoundDiagonal(0.2),
        rings: &[ring(WHITE, 6.0)],
        effects: drop_shadow,
    },
    StyleSpec {
        id: "clippedDiagonal",
        label: "Clipped Diagonal Corners",
        shape: Shape::SnipDiagonal(0.16),
        rings: &[ring(WHITE, 6.0)],
        effects: drop_shadow,
    },
    StyleSpec {
        id: "tiltedWhite",
        label: "Tilted, White Frame",
        shape: Shape::Rect,
        rings: &[ring(WHITE, 8.0)],
        effects: || PictureEffects {
            rotation: Some(PicRotation { z: 356.0, ..PicRotation::default() }),
            shadow: shadow(6.0, 4.0, 60.0, 60.0),
            ..none()
        },
    },
    StyleSpec {
        id: "floorShadow",
        label: "Floor Shadow, White Frame",
        shape: Shape::Rect,
        rings: &[ring(WHITE, 6.0)],
        effects: || PictureEffects {
            shadow: Some(PicShadow { blur: 6.0, distance: 0.0, transparency: 75.0, perspective: Some((-0.3, 0.0)), ..PicShadow::default() }),
            ..none()
        },
    },
    StyleSpec {
        id: "leaningBack",
        label: "Leaning Back, White Frame",
        shape: Shape::Rect,
        rings: &[ring(WHITE, 6.0)],
        effects: || PictureEffects {
            rotation: Some(PicRotation { y: 25.0, perspective: 45.0, ..PicRotation::default() }),
            shadow: shadow(6.0, 3.0, 90.0, 60.0),
            ..none()
        },
    },
    StyleSpec {
        id: "bevelled",
        label: "Bevelled",
        shape: Shape::Rect,
        rings: &[],
        effects: || PictureEffects { bevel: bevel(BevelKind::Circle, 8.0), ..none() },
    },
    StyleSpec {
        id: "bevelledTurnedLeft",
        label: "Bevelled, Turned Left",
        shape: Shape::Rect,
        rings: &[],
        effects: || PictureEffects {
            bevel: bevel(BevelKind::SoftRound, 6.0),
            rotation: Some(PicRotation { x: 25.0, perspective: 45.0, ..PicRotation::default() }),
            ..none()
        },
    },
    StyleSpec {
        id: "turnedRightReflection",
        label: "Turned Right, Reflection",
        shape: Shape::Rect,
        rings: &[],
        effects: || PictureEffects {
            rotation: Some(PicRotation { x: -25.0, perspective: 45.0, ..PicRotation::default() }),
            reflection: reflection(40.0, 2.0),
            ..none()
        },
    },
    StyleSpec {
        id: "blackBevelReflection",
        label: "Black Bevel, Reflection",
        shape: Shape::Rect,
        rings: &[ring(BLACK, 5.0)],
        effects: || PictureEffects { bevel: bevel(BevelKind::Circle, 4.0), reflection: reflection(30.0, 1.0), ..none() },
    },
];

/// The style `id` of the gallery after [`LEGACY`].
pub fn spec(id: &str) -> Option<&'static StyleSpec> {
    STYLES.iter().find(|s| s.id == id)
}

/// Every style `picture.style` takes, with its label: [`LEGACY`] then [`STYLES`].
pub fn gallery() -> impl Iterator<Item = (&'static str, &'static str)> {
    LEGACY.into_iter().chain(STYLES.iter().map(|s| (s.id, s.label)))
}
