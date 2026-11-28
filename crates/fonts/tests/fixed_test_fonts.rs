//! Separate process: verifies the global test fixture before any other lookup initializes it.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use wordcraft_fonts::FontDb;

#[test]
fn fixed_fonts_exclude_system_and_optional_faces() {
    let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(FontDb::init_test_fonts)).collect();
    assert!(threads.into_iter().all(|t| t.join().unwrap()));
    let db = FontDb::global();
    let families = [
        "Inter",
        "Inter Medium",
        "Inter SemiBold",
        "JetBrains Mono",
        "Source Sans 3",
        "Source Sans 3 Semibold",
        "Source Serif 4",
        "Source Serif 4 Semibold",
    ];
    assert_eq!(db.families(), families);
    assert_eq!(db.load_system_fonts(), 0);
    assert_eq!(db.families(), families);
    for family in ["Arial", "Calibri", "Carlito", "Noto Sans Arabic", "BIZ UDPGothic"] {
        assert!(!db.has_family(family), "external family {family} leaked into the test fixture");
    }
    assert!(db.fallback_for('你', 0).is_none(), "missing glyph lookup must not consult system fonts");
    assert_eq!(db.loaded_font_bytes(), 0);
    assert!(FontDb::init_test_fonts());
}
