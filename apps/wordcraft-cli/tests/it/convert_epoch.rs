//! `SOURCE_DATE_EPOCH` pins a conversion's PDF dates: the same input converts to
//! the same bytes, stamped with the epoch. The pdf crate's own tests cannot set
//! the variable (the crate forbids the `unsafe` the 2024 edition requires), so
//! the end-to-end path — environment to exported file — is covered here, in the
//! child process.
use std::process::Command;

#[test]
fn convert_is_byte_identical_under_source_date_epoch() {
    let dir = std::env::temp_dir().join(format!("wc-convert-epoch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.txt");
    std::fs::write(&input, "Hello from a reproducibility check.\n").unwrap();
    let mut outputs = Vec::new();
    for name in ["one.pdf", "two.pdf"] {
        let out = dir.join(name);
        let run = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
            .arg("convert")
            .arg(&input)
            .arg(&out)
            .env("SOURCE_DATE_EPOCH", "1700000000")
            .output()
            .unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        outputs.push(std::fs::read(&out).unwrap());
    }
    assert_eq!(outputs[0], outputs[1], "same epoch, same bytes");
    let raw = String::from_utf8_lossy(&outputs[0]);
    assert!(raw.contains("D:20231114221320"), "the epoch is the stamped date");
}
