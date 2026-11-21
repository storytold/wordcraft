//! Word keeps touching inline pictures and the word right after them on one line: the whole run
//! moves down together. Checked in Word 16 (M365) on 2026-11-09. Synthetic shapes only.
use super::picture_glue::lines;

#[test]
fn touching_pictures_move_down_together_with_their_word() {
    // PR #6100 review. Word 16 (checked 2026-11-09): two touching pictures right before a word go to the
    // next line together with it, even when the first would fit on this line. Here a 150 pt picture, a
    // space, then two touching 20 pt pictures glued to "Glued" in a 200 pt column: the first 20 pt picture
    // fits on line 1 (150 + space + 20), the rest doesn't.
    let got = lines(" Glued", &[(0, 150.0, 20.0), (1, 20.0, 20.0), (1, 20.0, 20.0)], Some(200.0));
    assert_eq!(got, ["O_", "OOTTTTT"], "{got:?}");
}

#[test]
fn touching_pictures_split_at_a_space() {
    // With a space between the two small pictures, the first stays on line 1.
    let got = lines("  Glued", &[(0, 150.0, 20.0), (1, 20.0, 20.0), (2, 20.0, 20.0)], Some(200.0));
    assert_eq!(got, ["O_O_", "OTTTTT"], "{got:?}");
}
