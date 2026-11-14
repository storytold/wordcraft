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

#[test]
fn an_odt_template_opens_and_saves_as_a_template_with_a_password_too() {
    let doc = io::open_bytes("plain.txt", b"Letterhead.").unwrap();
    let ott = io::save_bytes("letter.ott", &doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&ott)).unwrap();
    let mut mime = String::new();
    std::io::Read::read_to_string(&mut z.by_name("mimetype").unwrap(), &mut mime).unwrap();
    assert_eq!(mime, wordcraft_formats::odt::TEMPLATE_MIME);
    let opened = io::open_bytes("letter.ott", &ott).unwrap();
    assert_eq!(opened.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Letterhead."]);
    // With a password.
    assert!(io::can_encrypt("letter.ott"));
    let enc = io::save_bytes_with("letter.ott", &doc, Some("pw"), &Default::default()).unwrap();
    assert!(wordcraft_formats::odt::is_encrypted(&enc));
    let (back, encrypted) = io::open_bytes_with("letter.ott", &enc, Some("pw")).unwrap();
    assert!(encrypted && back.body.iter().filter_map(|b| b.as_para()).any(|p| p.text == "Letterhead."));
    assert!(io::OPEN_EXTS.contains(&"ott") && io::SAVE_EXTS.contains(&"ott"));
}
