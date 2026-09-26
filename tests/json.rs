use std::process::Command;

#[test]
fn json_mode_formats_errors_as_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["--json", "new", "INVALID-NAME"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();

    // Parse the stderr as JSON
    let parsed: serde_json::Value = serde_json::from_str(&stderr)
        .expect("Expected stderr to be valid JSON");
    assert_eq!(parsed["exit_code"], 1);
    assert!(parsed["error"].as_str().unwrap().contains("not a valid project name"));
}

#[test]
fn list_json_mode_is_machine_readable() {
    let path = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["--list", "--json"])
        .env("PATH", path.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Expected stdout to be valid JSON");
    let builtin = parsed["builtin"]
        .as_array()
        .expect("builtin should be a JSON array");

    assert!(builtin.iter().any(|name| name.as_str() == Some("new")));
    assert!(builtin.iter().any(|name| name.as_str() == Some("doctor")));
    assert_eq!(parsed["external"], serde_json::json!([]));
}

#[test]
fn list_without_json_keeps_human_readable_output() {
    let path = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .arg("--list")
        .env("PATH", path.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("Installed subcommands:\n\n  Built-in:\n"));
    assert!(stdout.contains("\n    new\n"));
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).is_err());
}
