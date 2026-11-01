//! A password-protected ODT opens with its password (and says when one is needed or wrong), and
//! saving with a password writes it encrypted again (ODF 1.3 Part 2 §3.4).

use wordcraft_engine::io;

#[test]
fn an_odt_is_saved_and_opened_with_a_password() {
    let doc = io::open_bytes("plain.txt", b"Private notes.").unwrap();
    let enc = io::save_bytes_with("secret.odt", &doc, Some("hunter2"), &Default::default()).unwrap();
    assert!(wordcraft_formats::odt::is_encrypted(&enc));
    assert!(io::can_encrypt("secret.odt"));
    let none = io::open_bytes_with("secret.odt", &enc, None).unwrap_err();
    assert!(io::needs_password(&none), "{none}");
    let wrong = io::open_bytes_with("secret.odt", &enc, Some("nope")).unwrap_err();
    assert!(io::wrong_password(&wrong), "{wrong}");
    let (opened, encrypted) = io::open_bytes_with("secret.odt", &enc, Some("hunter2")).unwrap();
    assert!(encrypted);
    assert_eq!(opened.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Private notes."]);
}
