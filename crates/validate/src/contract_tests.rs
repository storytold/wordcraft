//! The generated register and its rejection paths run with the ordinary validator tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn observation_contracts_are_current_and_reject_undeclared_consumption() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("observations/generate.py");
    let mut result = std::process::Command::new("python3").arg(&script).args(["--check", "--self-test"]).output();
    if result.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        result = std::process::Command::new("python").arg(&script).args(["--check", "--self-test"]).output();
    }
    let output = result.expect("Python 3 is required for the observation-contract and model tooling gates");
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}
