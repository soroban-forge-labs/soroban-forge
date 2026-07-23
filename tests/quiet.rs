use std::process::Command;

#[test]
fn quiet_new_is_silent_and_still_creates_project() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args([
            "--quiet",
            "new",
            "silent-demo",
            "--template",
            "hello-world",
            "--author",
            "Test Author",
            "--output-dir",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(temp.path().join("silent-demo/Cargo.toml").is_file());
    assert!(temp.path().join("silent-demo/forge.toml").is_file());
}

#[test]
fn quiet_mode_keeps_errors_visible() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["--quiet", "new", "INVALID-NAME"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error:"), "{stderr}");
    assert!(stderr.contains("not a valid project name"), "{stderr}");
}
