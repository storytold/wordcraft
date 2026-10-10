//! The Insert › Icons library: monochrome pictograms drawn in code.
//!
//! Every icon here is original WordCraft artwork: simple geometric line drawings designed on a
//! 24×24 grid for this file (no external icon set was copied, traced or adapted). The same path
//! data is drawn live by the interface's icon picker and rasterised here, by vello_cpu, into the
//! PNG picture `insert.icon` places in the document.
//!
//! Path data is a small SVG-like language in grid units (y grows downwards), absolute only:
//! - `M x y` move, `L x y` line (more pairs repeat the line), `Q cx cy x y` quadratic curve,
//!   `C c1x c1y c2x c2y x y` cubic curve, `Z` close the subpath;
//! - `A cx cy r a0 a1` circular arc from angle `a0` to `a1` in degrees (0° points right, 90°
//!   down), joined to the open subpath by a line, or starting one;
//! - `O cx cy r` a whole circle and `R x y w h r` a rounded rectangle, each its own closed subpath.
//!
//! Stroked parts are drawn with a [`STROKE`]-unit pen with round caps and joins. Filled parts are
//! convex outlines (so any front end can fill them as simple polygons).

use std::sync::OnceLock;

use vello_cpu::kurbo::{self, Affine, BezPath};
use vello_cpu::{RenderContext, Resources};
use wordcraft_doc::props::Rgb;

use crate::Rendered;

/// Side of the design grid, in grid units.
pub const GRID: f32 = 24.0;
/// Pen width of stroked parts, in grid units.
pub const STROKE: f32 = 2.0;

/// The library's categories, in the order the picker shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Arrows,
    Communication,
    Office,
    People,
    Nature,
    Weather,
    Shapes,
    Symbols,
    Technology,
    Transport,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Arrows,
        Category::Communication,
        Category::Office,
        Category::People,
        Category::Nature,
        Category::Weather,
        Category::Shapes,
        Category::Symbols,
        Category::Technology,
        Category::Transport,
    ];

    /// English name (also the interface catalogs' source text).
    pub fn name(self) -> &'static str {
        match self {
            Category::Arrows => "Arrows",
            Category::Communication => "Communication",
            Category::Office => "Office",
            Category::People => "People",
            Category::Nature => "Nature",
            Category::Weather => "Weather",
            Category::Shapes => "Shapes",
            Category::Symbols => "Symbols",
            Category::Technology => "Technology",
            Category::Transport => "Transport",
        }
    }

    /// The category named `name` (case-insensitive).
    pub fn parse(name: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// One drawing step of an icon.
#[derive(Clone, Copy, Debug)]
pub enum Part {
    /// Path data stroked with the pen.
    Stroke(&'static str),
    /// Path data filled; every subpath is convex.
    Fill(&'static str),
}

/// A library icon.
#[derive(Debug)]
pub struct Icon {
    /// Stable id (`insert.icon`'s `id`).
    pub id: &'static str,
    /// English name; also the inserted picture's alt text.
    pub name: &'static str,
    pub category: Category,
    /// Extra search words.
    pub keywords: &'static [&'static str],
    pub parts: &'static [Part],
}

/// A flattened subpath in grid units.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
    pub fill: bool,
}

use Category::*;
use Part::{Fill as F, Stroke as S};

macro_rules! icon {
    ($id:literal, $name:literal, $cat:expr, [$($k:literal),*], [$($p:expr),* $(,)?]) => {
        Icon { id: $id, name: $name, category: $cat, keywords: &[$($k),*], parts: &[$($p),*] }
    };
}

/// The library, grouped by category.
pub static ICONS: &[Icon] = &[
    // Arrows
    icon!("arrow-right", "Arrow Right", Arrows, ["next", "forward", "east"], [S("M3.5 12 L20 12 M14 6 L20 12 L14 18")]),
    icon!("arrow-left", "Arrow Left", Arrows, ["back", "previous", "west"], [S("M20.5 12 L4 12 M10 6 L4 12 L10 18")]),
    icon!("arrow-up", "Arrow Up", Arrows, ["north", "increase", "top"], [S("M12 20.5 L12 4 M6 10 L12 4 L18 10")]),
    icon!("arrow-down", "Arrow Down", Arrows, ["south", "decrease", "bottom", "download"], [S("M12 3.5 L12 20 M6 14 L12 20 L18 14")]),
    icon!("refresh", "Refresh", Arrows, ["reload", "repeat", "cycle", "rotate"], [S("A12 12 7.5 -45 225"), S("M2.7 6.7 L6.7 6.7 L6.7 10.7"),]),
    icon!("swap", "Swap", Arrows, ["exchange", "transfer", "trade"], [S("M4 8 L19 8 M15 4 L19 8 L15 12 M20 16 L5 16 M9 12 L5 16 L9 20")]),
    // Communication
    icon!("mail", "Mail", Communication, ["email", "envelope", "letter", "message"], [S("R2.5 5 19 14 2 M3.5 6.5 L12 13 L20.5 6.5")]),
    icon!(
        "chat",
        "Chat",
        Communication,
        ["speech", "bubble", "talk", "comment", "message"],
        [S(
            "M5 3.5 L19 3.5 Q21.5 3.5 21.5 6 L21.5 13.5 Q21.5 16 19 16 L11 16 L6 20.5 L6 16 L5 16 Q2.5 16 2.5 13.5 L2.5 6 Q2.5 3.5 5 3.5 Z M7 8 L17 8 M7 11.5 L14 11.5"
        )]
    ),
    icon!(
        "megaphone",
        "Megaphone",
        Communication,
        ["announce", "loud", "marketing", "news"],
        [S("M3 9.5 L3 14.5 L7 14.5 L17 19.5 L17 4.5 L7 9.5 Z M7 9.5 L7 14.5 M7 14.5 L8.5 20.5 L11.5 20.5 L10.5 16 M20 9.5 Q21.5 12 20 14.5")]
    ),
    icon!(
        "bell",
        "Bell",
        Communication,
        ["notification", "alert", "ring", "reminder"],
        [S("M5 17 L19 17 M7 17 L7 11 Q7 5.5 12 5.5 Q17 5.5 17 11 L17 17 M12 3 L12 5.5 M10 20.5 L14 20.5")]
    ),
    icon!(
        "inbox",
        "Inbox",
        Communication,
        ["tray", "receive", "messages"],
        [S("M2.5 13 L2.5 19.5 L21.5 19.5 L21.5 13 L16 13 L15 15.5 L9 15.5 L8 13 Z M5.5 13 L7.5 4.5 L16.5 4.5 L18.5 13")]
    ),
    // Office
    icon!(
        "document",
        "Document",
        Office,
        ["page", "file", "paper", "text"],
        [S("M6 2.5 L14 2.5 L19 7.5 L19 21.5 L6 21.5 Z M14 2.5 L14 7.5 L19 7.5 M9 12 L16 12 M9 15 L16 15 M9 18 L13 18")]
    ),
    icon!(
        "folder",
        "Folder",
        Office,
        ["directory", "files", "archive"],
        [S("M2.5 5 L9 5 L11 7.5 L21.5 7.5 L21.5 19.5 L2.5 19.5 Z M2.5 10.5 L21.5 10.5")]
    ),
    icon!(
        "calendar",
        "Calendar",
        Office,
        ["date", "schedule", "month", "event"],
        [S("R3 5 18 16 2 M3 10 L21 10 M8 3 L8 7 M16 3 L16 7"), F("O8 13.8 1.1 O12 13.8 1.1 O16 13.8 1.1 O8 17.4 1.1 O12 17.4 1.1"),]
    ),
    icon!(
        "clipboard",
        "Clipboard",
        Office,
        ["list", "tasks", "notes", "checklist"],
        [S("R5 4.5 14 17 2 R9 2.5 6 4 1 M9 11 L15 11 M9 14.5 L15 14.5 M9 18 L13 18")]
    ),
    icon!(
        "pencil",
        "Pencil",
        Office,
        ["edit", "write", "draw", "pen"],
        [S("M4 20 L5 15 L15.5 4.5 Q16.5 3.5 17.5 4.5 L19.5 6.5 Q20.5 7.5 19.5 8.5 L9 19 Z M13.5 6.5 L17.5 10.5 M5 15 L9 19")]
    ),
    icon!(
        "briefcase",
        "Briefcase",
        Office,
        ["work", "business", "job", "case"],
        [S("R2.5 7 19 13 2 M9 7 L9 4.5 L15 4.5 L15 7 M2.5 13 L21.5 13"), F("R10.5 11.5 3 3 0.5"),]
    ),
    icon!(
        "bar-chart",
        "Bar Chart",
        Office,
        ["graph", "statistics", "report", "data"],
        [S("M3 21 L21 21"), F("R5.5 12 3 7 0.5 R10.5 6 3 13 0.5 R15.5 9 3 10 0.5"),]
    ),
    // People
    icon!("person", "Person", People, ["user", "account", "profile", "human"], [S("O12 7.5 4 M4 21 Q4 13.5 12 13.5 Q20 13.5 20 21")]),
    icon!(
        "group",
        "Group",
        People,
        ["team", "users", "people", "friends"],
        [S("O9 8 3.5 M2.5 20 Q2.5 14 9 14 Q15.5 14 15.5 20 O16.5 7 3 M17 12.8 Q21.5 12.8 21.5 18")]
    ),
    icon!(
        "smiley",
        "Smiley",
        People,
        ["face", "happy", "smile", "emotion"],
        [S("O12 12 9.5 M8.2 14.2 Q12 18.5 15.8 14.2"), F("O9 9.8 1.3 O15 9.8 1.3"),]
    ),
    icon!(
        "person-add",
        "Add Person",
        People,
        ["invite", "new user", "join"],
        [S("O10 7.5 4 M3 20.5 Q3 13.5 10 13.5 Q12.8 13.5 14.6 14.6 M19 13 L19 19 M16 16 L22 16")]
    ),
    icon!(
        "badge",
        "Name Badge",
        People,
        ["id", "identity", "card", "employee"],
        [S("R4 2.5 16 19 2 O12 10 3 M8 17.5 Q12 13.5 16 17.5 M10 5.5 L14 5.5")]
    ),
    // Nature
    icon!(
        "leaf",
        "Leaf",
        Nature,
        ["plant", "eco", "green", "growth"],
        [S("M4.5 19.5 Q4.5 5 19.5 4.5 Q19.5 19.5 4.5 19.5 Z M4.5 19.5 L13 11 M3 21 L4.5 19.5")]
    ),
    icon!(
        "tree",
        "Pine Tree",
        Nature,
        ["forest", "fir", "evergreen", "winter"],
        [S("M12 2.5 L18 10 L15 10 L19.5 16.5 L4.5 16.5 L9 10 L6 10 Z M12 16.5 L12 21.5")]
    ),
    icon!(
        "flower",
        "Tulip",
        Nature,
        ["flower", "spring", "garden", "bloom"],
        [S(
            "M7 5 L9.5 8 L12 4 L14.5 8 L17 5 L17 10 Q17 14.5 12 14.5 Q7 14.5 7 10 Z M12 14.5 L12 21.5 M12 18.5 Q8 18.5 6.5 15.5 M12 19.5 Q15.5 19.5 17 17"
        )]
    ),
    icon!("mountain", "Mountain", Nature, ["landscape", "peak", "hiking", "outdoors"], [S("M2 20 L9 8 L13 14 L16 10 L22 20 Z"), F("O18.5 5 2"),]),
    icon!("paw", "Paw Print", Nature, ["pet", "animal", "dog", "cat"], [F("O12 16.5 3.6 O6.5 11 1.9 O10 7 1.9 O14 7 1.9 O17.5 11 1.9")]),
    // Weather
    icon!(
        "sun",
        "Sun",
        Weather,
        ["sunny", "bright", "day", "summer"],
        [S(
            "O12 12 4 M12 3 L12 5.5 M12 18.5 L12 21 M3 12 L5.5 12 M18.5 12 L21 12 M5.64 5.64 L7.4 7.4 M16.6 16.6 L18.36 18.36 M5.64 18.36 L7.4 16.6 M16.6 7.4 L18.36 5.64"
        )]
    ),
    icon!("cloud", "Cloud", Weather, ["cloudy", "overcast", "storage"], [S("M7 18 A7 14.5 3.5 90 270 A12 11 5 180 360 A17 14.5 3.5 270 450 Z")]),
    icon!(
        "rain",
        "Rain",
        Weather,
        ["rainy", "shower", "storm", "wet"],
        [S("M7 15 A7 11.5 3.5 90 270 A12 8 5 180 360 A17 11.5 3.5 270 450 Z M8 18 L7 21 M12 18 L11 21 M16 18 L15 21")]
    ),
    icon!(
        "lightning",
        "Lightning",
        Weather,
        ["bolt", "thunder", "energy", "flash"],
        [S("M13.5 2.5 L5 13.5 L11.5 13.5 L10.5 21.5 L19 10 L12.5 10 Z")]
    ),
    icon!("moon", "Moon", Weather, ["night", "crescent", "sleep", "dark"], [S("M18 5.07 A14 12 8 -60 -300 A18 12 6.93 90 270 Z")]),
    icon!(
        "snowflake",
        "Snowflake",
        Weather,
        ["snow", "cold", "winter", "frozen"],
        [S(
            "M12 2.5 L12 21.5 M20.23 7.25 L3.77 16.75 M20.23 16.75 L3.77 7.25 M14.4 3.9 L12 6.5 L9.6 3.9 M20.21 10.03 L16.76 9.25 L17.81 5.87 M17.81 18.13 L16.76 14.75 L20.21 13.97 M9.6 20.1 L12 17.5 L14.4 20.1 M3.79 13.97 L7.24 14.75 L6.19 18.13 M6.19 5.87 L7.24 9.25 L3.79 10.03"
        )]
    ),
    icon!(
        "droplet",
        "Droplet",
        Weather,
        ["water", "drop", "liquid", "humidity"],
        [S("M12 2.5 C12 2.5 18.5 10 18.5 14.5 C18.5 18.5 15.5 21.5 12 21.5 C8.5 21.5 5.5 18.5 5.5 14.5 C5.5 10 12 2.5 12 2.5 Z")]
    ),
    // Shapes
    icon!("circle", "Circle", Shapes, ["round", "ring", "dot"], [S("O12 12 9")]),
    icon!("square", "Square", Shapes, ["box", "rectangle"], [S("R3.5 3.5 17 17 2.5")]),
    icon!("triangle", "Triangle", Shapes, ["delta", "pyramid"], [S("M12 3.5 L21 19.5 L3 19.5 Z")]),
    icon!(
        "star",
        "Star",
        Shapes,
        ["favorite", "rating", "award"],
        [S("M12 3 L14.29 9.35 L21.04 9.56 L15.71 13.71 L17.58 20.19 L12 16.4 L6.42 20.19 L8.29 13.71 L2.96 9.56 L9.71 9.35 Z")]
    ),
    icon!("hexagon", "Hexagon", Shapes, ["polygon", "honeycomb", "cell"], [S("M12 3 L19.79 7.5 L19.79 16.5 L12 21 L4.21 16.5 L4.21 7.5 Z")]),
    // Symbols
    icon!(
        "heart",
        "Heart",
        Symbols,
        ["love", "like", "favorite", "health"],
        [S(
            "M12 20 C5 15 2.5 11.5 2.5 8.5 C2.5 5.8 4.7 3.8 7.3 3.8 C9.4 3.8 11 5 12 6.8 C13 5 14.6 3.8 16.7 3.8 C19.3 3.8 21.5 5.8 21.5 8.5 C21.5 11.5 19 15 12 20 Z"
        )]
    ),
    icon!("check", "Check Mark", Symbols, ["done", "ok", "yes", "complete", "tick"], [S("O12 12 9.5 M7.8 12.3 L10.8 15.3 L16.3 9.3")]),
    icon!("plus", "Plus", Symbols, ["add", "new", "more", "positive"], [S("M12 4.5 L12 19.5 M4.5 12 L19.5 12")]),
    icon!("info", "Information", Symbols, ["info", "about", "help", "details"], [S("O12 12 9.5 M12 10.5 L12 16.5"), F("O12 7.5 1.25")]),
    icon!(
        "warning",
        "Warning",
        Symbols,
        ["caution", "alert", "danger", "attention"],
        [S("M12 3 L21.5 20 L2.5 20 Z M12 9.5 L12 14"), F("O12 17 1.2"),]
    ),
    icon!(
        "question",
        "Question",
        Symbols,
        ["help", "ask", "faq", "unknown"],
        [S("O12 12 9.5 M9.3 9.3 Q9.3 6.8 12 6.8 Q14.7 6.8 14.7 9.3 Q14.7 11 12 12.3 L12 13.6"), F("O12 16.8 1.2"),]
    ),
    icon!("flag", "Flag", Symbols, ["goal", "milestone", "report", "country"], [S("M5 21.5 L5 3 M5 4 L19 4 L15.5 8.5 L19 13 L5 13")]),
    icon!(
        "lightbulb",
        "Light Bulb",
        Symbols,
        ["idea", "tip", "insight", "innovation"],
        [S("M9 17 L9 15.2 Q5.8 13.2 5.8 9.3 Q5.8 3 12 3 Q18.2 3 18.2 9.3 Q18.2 13.2 15 15.2 L15 17 Z M9.5 20.5 L14.5 20.5")]
    ),
    // Technology
    icon!(
        "laptop",
        "Laptop",
        Technology,
        ["computer", "notebook", "device"],
        [S("R4.5 5 15 10.5 1.5 M2.5 19 L21.5 19 M4.5 15.5 L2.5 19 M19.5 15.5 L21.5 19")]
    ),
    icon!("mobile", "Mobile Phone", Technology, ["phone", "smartphone", "cell", "device"], [S("R6.5 2.5 11 19 2 M11 18.5 L13 18.5")]),
    icon!(
        "wifi",
        "Wireless",
        Technology,
        ["wifi", "signal", "network", "internet"],
        [S("A12 19.5 4.5 225 315"), S("A12 19.5 9 225 315"), S("A12 19.5 13.5 225 315"), F("O12 19.5 1.4"),]
    ),
    icon!("monitor", "Monitor", Technology, ["screen", "display", "desktop", "computer"], [S("R2.5 3.5 19 13 1.5 M12 16.5 L12 20 M8 20.5 L16 20.5")]),
    icon!("battery", "Battery", Technology, ["power", "charge", "energy"], [S("R2.5 7 17 10 2 M21.5 10.5 L21.5 13.5"), F("R5 9.5 8 5 0.6")]),
    icon!("code", "Code", Technology, ["programming", "brackets", "developer", "html"], [S("M8 7 L3 12 L8 17 M16 7 L21 12 L16 17 M14 4.5 L10 19.5")]),
    icon!("power", "Power", Technology, ["on", "off", "switch", "shutdown"], [S("A12 13 7.5 -60 240"), S("M12 3 L12 11")]),
    // Transport
    icon!(
        "car",
        "Car",
        Transport,
        ["auto", "vehicle", "drive", "road"],
        [S("M2.5 16 L2.5 11.5 L5.5 6 L18.5 6 L21.5 11.5 L21.5 16 Z M2.5 11.5 L21.5 11.5 M12 6 L12 11.5"), F("O7 17.5 2.3 O17 17.5 2.3"),]
    ),
    icon!(
        "bicycle",
        "Bicycle",
        Transport,
        ["bike", "cycling", "ride"],
        [S("O5.5 16 3.5 O18.5 16 3.5 M5.5 16 L9.5 9 L12 16 Z M9.5 9 L16 9 L12 16 M16 9 L18.5 16 M16 9 L15 6 L17.5 6 M9.5 9 L9 7 M7.5 7 L10.5 7")]
    ),
    icon!(
        "airplane",
        "Airplane",
        Transport,
        ["plane", "flight", "travel", "airport"],
        [S(
            "M12 2.5 Q13.5 2.5 13.5 5 L13.5 9.5 L21 14 L21 16 L13.5 13.5 L13.5 18 L16 20 L16 21.5 L12 20.5 L8 21.5 L8 20 L10.5 18 L10.5 13.5 L3 16 L3 14 L10.5 9.5 L10.5 5 Q10.5 2.5 12 2.5 Z"
        )]
    ),
    icon!(
        "bus",
        "Bus",
        Transport,
        ["coach", "public transport", "transit"],
        [S("R4 2.5 16 16.5 2.5 M4 11 L20 11 M7 19 L7 21.5 M17 19 L17 21.5 M8 5.5 L16 5.5"), F("O8 15 1.3 O16 15 1.3"),]
    ),
    icon!(
        "sailboat",
        "Sailboat",
        Transport,
        ["boat", "ship", "sea", "sailing"],
        [S("M3 16 L21 16 L18 20.5 L6 20.5 Z M12 2.5 L12 16 M12 4 L18.5 13.5 L12 13.5 M10 6 L5.5 13.5 L10 13.5 Z")]
    ),
    icon!(
        "map-pin",
        "Map Pin",
        Transport,
        ["location", "place", "marker", "map", "travel"],
        [S("M12 21.5 Q5 14 5 9.5 A12 9.5 7 180 360 Q19 14 12 21.5 Z O12 9.5 2.5")]
    ),
];

/// The icon with this id.
pub fn find(id: &str) -> Option<&'static Icon> {
    ICONS.iter().find(|i| i.id == id.trim())
}

/// Icons matching every word of `query` (in the id, name, keywords or category; case-insensitive)
/// and, when given, in `category`. An empty query matches everything.
pub fn search(query: &str, category: Option<Category>) -> Vec<&'static Icon> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    ICONS
        .iter()
        .filter(|i| category.is_none_or(|c| i.category == c))
        .filter(|i| {
            words.iter().all(|w| {
                i.id.contains(w.as_str())
                    || i.name.to_lowercase().contains(w.as_str())
                    || i.category.name().to_lowercase().contains(w.as_str())
                    || i.keywords.iter().any(|k| k.contains(w.as_str()))
            })
        })
        .collect()
}

impl Icon {
    /// Parse and flatten the icon's parts.
    pub fn parse(&self) -> Result<Vec<Piece>, String> {
        let mut out = Vec::new();
        for part in self.parts {
            let (d, fill) = match part {
                Part::Stroke(d) => (d, false),
                Part::Fill(d) => (d, true),
            };
            for (points, closed) in flatten(d).map_err(|e| format!("{}: {e}", self.id))? {
                out.push(Piece { points, closed: closed || fill, fill });
            }
        }
        Ok(out)
    }

    /// The flattened geometry, parsed once (empty for an icon whose data doesn't parse, logged).
    pub fn pieces(&self) -> &'static [Piece] {
        static ALL: OnceLock<Vec<Vec<Piece>>> = OnceLock::new();
        let all = ALL.get_or_init(|| {
            ICONS
                .iter()
                .map(|i| {
                    i.parse().unwrap_or_else(|e| {
                        log::error!("icon library: {e}");
                        Vec::new()
                    })
                })
                .collect()
        });
        ICONS.iter().position(|i| i.id == self.id).and_then(|n| all.get(n)).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[derive(Clone, Copy, Debug)]
enum Tok {
    Cmd(char),
    Num(f32),
}

fn tokens(d: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = d.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() || c == ',' {
            continue;
        }
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c));
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, n)) = chars.peek() {
            if n.is_ascii_digit() || n == '.' {
                end = j + n.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let text = d.get(i..end).unwrap_or("");
        let v: f32 = text.parse().map_err(|_| format!("bad number {text:?}"))?;
        if !v.is_finite() {
            return Err(format!("bad number {text:?}"));
        }
        out.push(Tok::Num(v));
    }
    Ok(out)
}

/// Flatten path data into polylines (`(points, closed)`).
fn flatten(d: &str) -> Result<Vec<(Vec<[f32; 2]>, bool)>, String> {
    let toks = tokens(d)?;
    let mut subs: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let mut i = 0;
    let mut last = None;
    let flush = |cur: &mut Vec<[f32; 2]>, subs: &mut Vec<(Vec<[f32; 2]>, bool)>, closed: bool| {
        if cur.len() >= 2 {
            subs.push((std::mem::take(cur), closed));
        }
        cur.clear();
    };
    while i < toks.len() {
        let cmd = match toks.get(i) {
            Some(Tok::Cmd(c)) => {
                i += 1;
                *c
            }
            // A number repeats the previous command (a repeated move is a line).
            Some(Tok::Num(_)) => match last {
                Some('M') => 'L',
                Some(c @ ('L' | 'Q' | 'C')) => c,
                _ => return Err("number without a command".into()),
            },
            None => break,
        };
        let argc = match cmd {
            'M' | 'L' => 2,
            'Q' => 4,
            'C' => 6,
            'A' | 'R' => 5,
            'O' => 3,
            'Z' => 0,
            c => return Err(format!("unknown command {c:?}")),
        };
        let mut a = [0f32; 6];
        for slot in a.iter_mut().take(argc) {
            match toks.get(i) {
                Some(Tok::Num(v)) => *slot = *v,
                _ => return Err(format!("{cmd} needs {argc} numbers")),
            }
            i += 1;
        }
        let from = cur.last().copied();
        match cmd {
            'M' => {
                flush(&mut cur, &mut subs, false);
                cur.push([a[0], a[1]]);
            }
            'L' => {
                from.ok_or("L without a current point")?;
                cur.push([a[0], a[1]]);
            }
            'Q' => {
                let p0 = from.ok_or("Q without a current point")?;
                for k in 1..=12 {
                    let t = k as f32 / 12.0;
                    let u = 1.0 - t;
                    cur.push([u * u * p0[0] + 2.0 * u * t * a[0] + t * t * a[2], u * u * p0[1] + 2.0 * u * t * a[1] + t * t * a[3]]);
                }
            }
            'C' => {
                let p0 = from.ok_or("C without a current point")?;
                for k in 1..=16 {
                    let t = k as f32 / 16.0;
                    let u = 1.0 - t;
                    let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push([b0 * p0[0] + b1 * a[0] + b2 * a[2] + b3 * a[4], b0 * p0[1] + b1 * a[1] + b2 * a[3] + b3 * a[5]]);
                }
            }
            'A' => {
                let (cx, cy, r, a0, a1) = (a[0], a[1], a[2], a[3], a[4]);
                if r <= 0.0 || (a1 - a0).abs() > 720.0 {
                    return Err("bad arc".into());
                }
                let n = ((a1 - a0).abs() / 7.5).ceil().max(2.0) as usize;
                for k in 0..=n {
                    let t = (a0 + (a1 - a0) * k as f32 / n as f32).to_radians();
                    cur.push([cx + r * t.cos(), cy + r * t.sin()]);
                }
            }
            'O' => {
                flush(&mut cur, &mut subs, false);
                let (cx, cy, r) = (a[0], a[1], a[2]);
                if r <= 0.0 {
                    return Err("bad circle".into());
                }
                let pts = (0..48).map(|k| (k as f32 * 7.5).to_radians()).map(|t| [cx + r * t.cos(), cy + r * t.sin()]).collect();
                subs.push((pts, true));
            }
            'R' => {
                flush(&mut cur, &mut subs, false);
                let (x, y, w, h) = (a[0], a[1], a[2], a[3]);
                if w <= 0.0 || h <= 0.0 {
                    return Err("bad rectangle".into());
                }
                let r = a[4].clamp(0.0, w.min(h) / 2.0);
                let mut pts = Vec::new();
                for (cx, cy, start) in [(x + w - r, y + r, -90.0f32), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)] {
                    for k in 0..=6 {
                        let t = (start + 15.0 * k as f32).to_radians();
                        pts.push([cx + r * t.cos(), cy + r * t.sin()]);
                    }
                }
                pts.dedup();
                subs.push((pts, true));
            }
            _ => flush(&mut cur, &mut subs, true),
        }
        last = Some(cmd);
    }
    flush(&mut cur, &mut subs, false);
    Ok(subs)
}

/// Rasterise `icon` into a transparent `px`×`px` image in `ink` (premultiplied RGBA).
pub fn render(icon: &Icon, px: u32, ink: Rgb) -> Rendered {
    let side = px.clamp(8, 4096) as u16;
    let mut ctx = RenderContext::new_with(side, side, vello_cpu::RenderSettings { num_threads: crate::default_threads(), ..Default::default() });
    ctx.set_transform(Affine::scale(side as f64 / GRID as f64));
    ctx.set_paint(crate::color(ink, 1.0));
    ctx.set_stroke(kurbo::Stroke::new(STROKE as f64).with_caps(kurbo::Cap::Round).with_join(kurbo::Join::Round));
    for piece in icon.pieces() {
        let mut path = BezPath::new();
        for (k, p) in piece.points.iter().enumerate() {
            let p = (p[0] as f64, p[1] as f64);
            if k == 0 { path.move_to(p) } else { path.line_to(p) }
        }
        if piece.closed {
            path.close_path();
        }
        if piece.fill {
            ctx.fill_path(&path);
        } else {
            ctx.stroke_path(&path);
        }
    }
    ctx.flush();
    let mut pixels = vec![0u8; side as usize * side as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(side, side, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    Rendered { width: side as u32, height: side as u32, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twice the signed area's sign of each corner; a convex outline never changes turning sign.
    fn convex(points: &[[f32; 2]]) -> bool {
        let n = points.len();
        let mut sign = 0.0f32;
        for k in 0..n {
            let (a, b, c) = (points[k], points[(k + 1) % n], points[(k + 2) % n]);
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            if cross.abs() < 1e-4 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    #[test]
    fn every_icon_is_unique_parses_fits_the_grid_and_renders() {
        assert!((40..=80).contains(&ICONS.len()), "{} icons", ICONS.len());
        let mut ids = std::collections::HashSet::new();
        for icon in ICONS {
            assert!(ids.insert(icon.id), "duplicate id {}", icon.id);
            assert!(!icon.name.is_empty() && !icon.keywords.is_empty(), "{}", icon.id);
            let pieces = icon.parse().unwrap_or_else(|e| panic!("{e}"));
            assert!(!pieces.is_empty(), "{} draws nothing", icon.id);
            for p in &pieces {
                assert!(p.points.len() >= 2, "{}: degenerate subpath", icon.id);
                // Strokes keep half the pen inside the grid.
                let m = if p.fill { 0.0 } else { STROKE / 2.0 - 0.01 };
                for &[x, y] in &p.points {
                    assert!((m..=GRID - m).contains(&x) && (m..=GRID - m).contains(&y), "{}: ({x}, {y}) leaves the grid", icon.id);
                }
                if p.fill {
                    assert!(convex(&p.points), "{}: filled outlines must be convex", icon.id);
                }
            }
            let img = render(icon, 48, Rgb::BLACK);
            let inked = img.pixels.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count();
            assert!(inked > 48, "{} renders blank", icon.id);
            // Nothing touches the image's edge (margins are part of the design).
            let edge = |x: usize, y: usize| img.pixels.get((y * 48 + x) * 4 + 3).copied().unwrap_or(0);
            assert!((0..48).all(|k| edge(k, 0) == 0 && edge(k, 47) == 0 && edge(0, k) == 0 && edge(47, k) == 0), "{} is clipped", icon.id);
        }
        assert!(Category::ALL.iter().all(|c| ICONS.iter().any(|i| i.category == *c)), "every category has icons");
    }

    #[test]
    fn search_matches_words_names_keywords_and_categories() {
        let ids = |q: &str, c: Option<Category>| search(q, c).iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(search("", None).len(), ICONS.len());
        assert_eq!(ids("Envelope", None), ["mail"]);
        assert!(ids("weather", None).contains(&"rain"));
        assert_eq!(ids("arrow up", None), ["arrow-up"]);
        assert!(ids("help", Some(Category::Symbols)).contains(&"question"));
        assert!(ids("help", Some(Category::Arrows)).is_empty());
        assert!(ids("zzz-nothing", None).is_empty());
        assert_eq!(Category::parse(" weather "), Some(Category::Weather));
        assert!(find("heart").is_some() && find("nope").is_none());
        assert!(flatten("M1 1 L").is_err() && flatten("X 1").is_err() && flatten("L 2 2").is_err() && flatten("5").is_err());
    }
}
