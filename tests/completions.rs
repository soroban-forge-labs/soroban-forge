use std::process::Command;

#[test]
fn completions_powershell_generates_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["completions", "powershell"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.trim().is_empty(), "expected PowerShell completion script");
    assert!(stdout.contains("soroban-forge"), "{stdout}");
}

#[test]
fn completions_help_lists_powershell() {
    let output = Command::new(env!("CARGO_BIN_EXE_soroban-forge"))
        .args(["completions", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("powershell"), "{stdout}");
}
