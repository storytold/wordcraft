use super::*;

#[test]
fn rotation_into_an_unusable_directory_is_an_error_not_a_panic() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "a file where the directory should be").expect("block");
    assert!(rotate(&blocked).is_err());
}
