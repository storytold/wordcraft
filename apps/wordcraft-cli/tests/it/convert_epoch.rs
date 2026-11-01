//! `SOURCE_DATE_EPOCH` pins a conversion's PDF dates: the same input converts to
//! the same bytes, stamped with the epoch. The pdf crate's own tests cannot set
//! the variable (the crate forbids the `unsafe` the 2024 edition requires), so
//! the end-to-end path — environment to exported file — is covered here, in the
//! child process.
use std::process::Command;

fn assert_dates(bytes: &[u8], info: &str, xmp: &str) {
    let raw = String::from_utf8_lossy(bytes);
    for key in ["CreationDate", "ModDate"] {
        assert!(raw.contains(&format!("/{key}(D:{info}Z)")), "{key} has the UTC date");
    }
    for key in ["CreateDate", "ModifyDate"] {
        assert!(raw.contains(&format!("<xmp:{key}>{xmp}+00:00</xmp:{key}>")), "XMP {key} has the UTC date");
    }
}

fn convert(input: &std::path::Path, out: &std::path::Path, epoch: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cmd.arg("convert").arg(input).arg(out).env("TZ", "Pacific/Honolulu").env_remove("SOURCE_DATE_EPOCH");
    if let Some(epoch) = epoch {
        cmd.env("SOURCE_DATE_EPOCH", epoch);
    }
    cmd.output().unwrap()
}

fn xmp_value(bytes: &[u8], name: &str) -> String {
    String::from_utf8_lossy(bytes).split_once(&format!("<xmpMM:{name}>")).unwrap().1.split_once('<').unwrap().0.to_owned()
}

#[test]
fn convert_is_byte_identical_under_source_date_epoch() {
    let dir = tempfile::Builder::new().prefix("wc-convert-epoch-").tempdir().unwrap();
    let input = dir.path().join("in.txt");
    std::fs::write(&input, "Hello from a reproducibility check.\n").unwrap();
    let mut outputs = Vec::new();
    for name in ["one.pdf", "two.pdf"] {
        let out = dir.path().join(name);
        let run = convert(&input, &out, Some("1700000000"));
        assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
        assert!(!String::from_utf8_lossy(&run.stderr).contains("invalid SOURCE_DATE_EPOCH"));
        outputs.push(std::fs::read(&out).unwrap());
    }
    assert_eq!(outputs[0], outputs[1], "same epoch, same bytes");
    assert_dates(&outputs[0], "20231114221320", "2023-11-14T22:13:20");
    assert_eq!(xmp_value(&outputs[0], "DocumentID"), xmp_value(&outputs[1], "DocumentID"));
    assert_eq!(xmp_value(&outputs[0], "InstanceID"), xmp_value(&outputs[1], "InstanceID"));

    std::fs::write(&input, "Different document content.\n").unwrap();
    let changed = dir.path().join("changed.pdf");
    assert_eq!(convert(&input, &changed, Some("1700000000")).status.code(), Some(0));
    let changed = std::fs::read(changed).unwrap();
    assert_ne!(xmp_value(&outputs[0], "DocumentID"), xmp_value(&changed, "DocumentID"));
    assert_ne!(xmp_value(&outputs[0], "InstanceID"), xmp_value(&changed, "InstanceID"));
}

#[test]
fn invalid_source_date_epoch_warns_and_keeps_the_document_date() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.json");
    let mut doc = wordcraft_doc::Document::new();
    doc.core.created = "2001-02-03T04:05:06Z".into();
    std::fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    for (n, epoch) in ["private\nvalue", "-1", "253402300800", "9223372036854775807", ""].iter().enumerate() {
        let out = dir.path().join(format!("invalid-{n}.pdf"));
        let run = convert(&input, &out, Some(epoch));
        assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(stderr.contains("warning: Ignoring invalid SOURCE_DATE_EPOCH"), "{stderr}");
        assert!(!stderr.contains("private"), "the untrusted value is not echoed");
        assert_dates(&std::fs::read(out).unwrap(), "20010203040506", "2001-02-03T04:05:06");
    }
}

#[test]
fn an_unset_source_date_epoch_uses_current_utc_time_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.txt");
    let out = dir.path().join("out.pdf");
    std::fs::write(&input, "A document without a creation date.").unwrap();
    let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let run = convert(&input, &out, None);
    let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(!String::from_utf8_lossy(&run.stderr).contains("SOURCE_DATE_EPOCH"));
    let bytes = std::fs::read(out).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    let stamp = raw.split_once("/CreationDate(D:").unwrap().1.get(..14).unwrap();
    let date = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M%S").unwrap().and_utc();
    assert!((before..=after).contains(&date.timestamp()), "{stamp} is the current UTC time");
    assert_dates(&bytes, stamp, &date.format("%Y-%m-%dT%H:%M:%S").to_string());
}
