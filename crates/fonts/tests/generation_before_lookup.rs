//! Separate process: reading the global database's generation, as a layout's cache key does,
//! leaves a test process free to choose its fonts afterwards (#3687).
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use wordcraft_fonts::FontDb;

#[test]
fn reading_the_generation_first_leaves_the_database_unstarted() {
    assert_eq!(FontDb::global_generation(), 0, "nothing has been added or removed yet");
    assert!(FontDb::init_test_fonts(), "the generation was read without starting the database");
    assert_eq!(FontDb::global_generation(), FontDb::global().generation());
}
