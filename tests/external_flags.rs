#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// Install a `soroban-forge-envdump` script in `dir` that writes the
/// SOROBAN_FORGE_* environment and its argv to stdout.
fn install_envdump(dir: &std::path::Path) {
    let path = dir.join("soroban-forge-envdump");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         echo \"verbose=${SOROBAN_FORGE_VERBOSE}\"\n\
         echo \"quiet=${SOROBAN_FORGE_QUIET}\"\n\
         echo \"json=${SOROBAN_FORGE_JSON}\"\n\
         echo \"yes=${SOROBAN_FORGE_YES}\"\n\
         echo \"args=$*\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run_with_path(dir: &std::path::Path, args: &[&str]) -> String {
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .env("PATH", path)
        .env_remove("SOROBAN_FORGE_VERBOSE")
        .env_remove("SOROBAN_FORGE_QUIET")
        .env_remove("SOROBAN_FORGE_JSON")
        .env_remove("SOROBAN_FORGE_YES")
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn external_subcommand_receives_flags_given_before_its_name() {
    let dir = tempfile::tempdir().unwrap();
    install_envdump(dir.path());

    let stdout = run_with_path(
        dir.path(),
        &["--verbose", "--quiet", "--json", "--yes", "envdump", "extra-arg"],
    );
    assert!(stdout.contains("verbose=1"), "{stdout}");
    assert!(stdout.contains("quiet=1"), "{stdout}");
    assert!(stdout.contains("json=1"), "{stdout}");
    assert!(stdout.contains("yes=1"), "{stdout}");
    assert!(stdout.contains("args=extra-arg"), "{stdout}");
}

#[test]
fn external_subcommand_gets_no_flags_when_none_given() {
    let dir = tempfile::tempdir().unwrap();
    install_envdump(dir.path());

    let stdout = run_with_path(dir.path(), &["envdump"]);
    assert!(stdout.contains("verbose=\n"), "{stdout}");
    assert!(stdout.contains("quiet=\n"), "{stdout}");
    assert!(stdout.contains("json=\n"), "{stdout}");
}
