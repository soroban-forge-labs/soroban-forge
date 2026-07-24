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
