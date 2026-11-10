//! Separate process: a session for a document that embeds no fonts, opened and closed, leaves the
//! process-wide font database unstarted, so it neither loads the system fonts nor decides which
//! fonts a test process gets (#3687).
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_fonts::FontDb;

#[test]
fn a_session_without_embedded_fonts_leaves_the_font_database_unstarted() {
    let session = Session::new(Document::from_text("Plain text"));
    assert!(FontDb::global_if_started().is_none(), "opening it started the font database");
    drop(session);
    assert!(FontDb::global_if_started().is_none(), "closing it started the font database");
}
