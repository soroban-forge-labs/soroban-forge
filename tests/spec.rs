//! End-to-end behaviour of `soroban-forge spec` that does not need a
//! `stellar` binary: everything up to the point the interface would be read
//! out of a built wasm.

use std::process::Command;

fn forge() -> Command {
    Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
}

/// Scaffold a project with `new` and return its path.
fn scaffold(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let output = forge()
        .args([
            "--quiet",
            "new",
            name,
            "--template",
            "hello-world",
            "--author",
            "Test Author",
            "--no-git",
            "--output-dir",
            parent.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    parent.join(name)
}

#[test]
fn spec_is_listed_as_a_subcommand() {
    let output = forge().arg("--list").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("spec"), "{stdout}");
}

#[test]
fn spec_help_mentions_entrypoints_and_types() {
    let output = forge().args(["spec", "--help"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("entrypoint"), "{stdout}");
    assert!(stdout.contains("--wasm"), "{stdout}");
    assert!(stdout.contains("--path"), "{stdout}");
}

#[test]
fn spec_without_a_build_points_at_stellar_contract_build() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold(temp.path(), "spec-demo");

    let output = forge()
        .args(["spec", "--path", project.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("stellar contract build"), "{stderr}");
    assert!(stderr.contains("spec_demo.wasm"), "{stderr}");
}

#[test]
fn spec_outside_a_cargo_project_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let output = forge()
        .args(["spec", "--path", temp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not a cargo project"), "{stderr}");
}

#[test]
fn quiet_spec_keeps_stdout_empty_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold(temp.path(), "quiet-spec-demo");

    let output = forge()
        .args(["--quiet", "spec", "--path", project.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn json_spec_reports_errors_as_json() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold(temp.path(), "json-spec-demo");

    let output = forge()
        .args(["--json", "spec", "--path", project.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stderr).expect("stderr must be JSON");
    assert_eq!(parsed["exit_code"], 1);
    assert!(parsed["error"]
        .as_str()
        .unwrap()
        .contains("stellar contract build"));
}

#[test]
fn spec_help_mentions_contract_id_and_format() {
    let output = forge().args(["spec", "--help"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("CONTRACT_ID"), "{stdout}");
    assert!(stdout.contains("--format"), "{stdout}");
    assert!(stdout.contains("--network"), "{stdout}");
}

#[test]
fn spec_rejects_malformed_contract_id_before_network() {
    let output = forge()
        .args([
            "spec",
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not a valid contract ID"), "{stderr}");
}

#[test]
fn spec_with_contract_id_refuses_offline() {
    let output = forge()
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
}
