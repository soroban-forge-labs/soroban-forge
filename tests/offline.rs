use std::process::Command;

#[test]
fn offline_rejects_remote_templates_before_git_clone() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .env("PATH", "")
        .args([
            "--offline",
            "new",
            "demo",
            "--from",
            "https://example.invalid/template",
            "--output-dir",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("offline mode"), "{stderr}");
    assert!(
        !stderr.contains("git"),
        "git must not be launched: {stderr}"
    );
}

#[test]
fn offline_rejects_verify_before_stellar_is_launched() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .env("PATH", "")
        .args([
            "--offline",
            "verify",
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("offline mode"), "{stderr}");
    assert!(
        !stderr.contains("not found on PATH"),
        "stellar must not be launched: {stderr}"
    );
}

#[test]
fn offline_rejects_spec_with_contract_id_before_stellar_is_launched() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .env("PATH", "")
        .args([
            "--offline",
            "spec",
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("offline mode"), "{stderr}");
    assert!(
        !stderr.contains("not found on PATH"),
        "stellar must not be launched: {stderr}"
    );
}

#[test]
fn offline_rejects_deploy_before_stellar_is_launched() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .env("PATH", "")
        .args(["--offline", "deploy", "--source", "alice"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("offline mode"), "{stderr}");
    assert!(
        !stderr.contains("not found on PATH"),
        "stellar must not be launched: {stderr}"
    );
}
