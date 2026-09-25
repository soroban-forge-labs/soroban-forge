//! Integration coverage for workspace scaffolding (`new --workspace`) and
//! `test-init` at a workspace root.
//!
//! These exercise only environment-independent behaviour — file layout and
//! CLI output — so they never depend on a live toolchain. The generation
//! logic itself is unit-tested in the scaffold and testgen crates.

use std::process::Command;

/// `new --workspace --contract` lays out a Cargo workspace with one crate per
/// contract under `contracts/`, a `[workspace]` root manifest, and a root
/// `forge.toml`.
#[test]
fn new_workspace_scaffolds_multiple_members() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args([
            "new",
            "ws-demo",
            "--workspace",
            "--contract",
            "token:token",
            "--contract",
            "pool:amm",
            "--author",
            "Test Author",
            "--output-dir",
            temp.path().to_str().unwrap(),
            "--no-git",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let root = temp.path().join("ws-demo");

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[workspace]"), "{manifest}");
    assert!(manifest.contains("contracts/token"), "{manifest}");
    assert!(manifest.contains("contracts/pool"), "{manifest}");
    assert!(manifest.contains("[workspace.dependencies]"), "{manifest}");

    assert!(root.join("forge.toml").is_file());

    for member in ["token", "pool"] {
        let member_manifest =
            std::fs::read_to_string(root.join("contracts").join(member).join("Cargo.toml"))
                .unwrap();
        assert!(
            member_manifest.contains("soroban-sdk.workspace = true"),
            "member {member}: {member_manifest}"
        );
        assert!(
            !member_manifest.contains("[profile."),
            "member {member} should not carry its own profile"
        );
        assert!(root
            .join("contracts")
            .join(member)
            .join("src/lib.rs")
            .is_file());
    }
}

/// `--workspace` without any `--contract` is a usage error, not a panic.
#[test]
fn workspace_without_contracts_errors() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args([
            "new",
            "empty-ws",
            "--workspace",
            "--output-dir",
            temp.path().to_str().unwrap(),
            "--no-git",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--contract"), "{stderr}");
}

/// `test-init` at a multi-contract workspace root requires `--contract <name>`.
#[test]
fn test_init_harnesses_each_workspace_member() {
    let temp = tempfile::tempdir().unwrap();

    let scaffold = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args([
            "new",
            "ws-demo",
            "--workspace",
            "--contract",
            "a:hello-world",
            "--contract",
            "b:hello-world",
            "--author",
            "Test Author",
            "--output-dir",
            temp.path().to_str().unwrap(),
            "--no-git",
        ])
        .output()
        .unwrap();
    assert!(scaffold.status.success(), "{scaffold:?}");
    let root = temp.path().join("ws-demo");

    // Bare `test-init` without `--contract` in multi-contract workspace fails with candidate list
    let init_bare = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["test-init", "--path", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!init_bare.status.success(), "{init_bare:?}");
    let stderr = String::from_utf8(init_bare.stderr).unwrap();
    assert!(stderr.contains("--contract"), "{stderr}");
    assert!(stderr.contains("a (contracts/a)"), "{stderr}");
    assert!(stderr.contains("b (contracts/b)"), "{stderr}");

    // Providing `--contract` generates harness for each targeted member
    for member in ["a", "b"] {
        let init = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
            .args([
                "test-init",
                "--path",
                root.to_str().unwrap(),
                "--contract",
                member,
            ])
            .output()
            .unwrap();
        assert!(init.status.success(), "{init:?}");

        let tests = root.join("contracts").join(member).join("tests");
        assert!(
            tests.join("forge_smoke.rs").is_file(),
            "missing smoke test for {member}"
        );
        assert!(
            tests.join("forge_snapshots.rs").is_file(),
            "missing snapshot test for {member}"
        );
        assert!(
            tests.join("common/mod.rs").is_file(),
            "missing fixtures for {member}"
        );
    }
}
