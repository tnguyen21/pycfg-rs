mod common;

use common::run_pycfg;
use tempfile::tempdir;

fn assert_cli_output_matches_golden(args: &[&str], golden_path: &str) {
    let output = run_pycfg(args);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected =
        std::fs::read_to_string(golden_path).unwrap_or_else(|e| panic!("{golden_path}: {e}"));
    assert_eq!(
        stdout.trim_end_matches('\n'),
        expected.trim_end_matches('\n'),
        "golden mismatch for {:?}",
        args
    );
}

#[test]
fn test_cli_text_output() {
    assert_cli_output_matches_golden(
        &["tests/test_code/basic_if.py"],
        "tests/golden/basic_if.text",
    );
}

#[test]
fn test_cli_json_output() {
    assert_cli_output_matches_golden(
        &["--format", "json", "tests/test_code/basic_if.py"],
        "tests/golden/basic_if.json",
    );
}

#[test]
fn test_cli_dot_output() {
    assert_cli_output_matches_golden(
        &["--format", "dot", "tests/test_code/basic_if.py"],
        "tests/golden/basic_if.dot",
    );
}

#[test]
fn test_cli_function_targeting() {
    let output = run_pycfg(&["--format", "json", "tests/test_code/loops.py::my_func"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let funcs = parsed["files"][0]["functions"].as_array().unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0]["name"], "my_func");
}

#[test]
fn test_cli_function_targeting_requires_exact_name() {
    let output = run_pycfg(&["--format", "json", "tests/test_code/classes.py::validate"]);
    assert!(
        !output.status.success(),
        "leaf-name-only function target should not be accepted"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No Python files or functions found to analyze"));
}

#[test]
fn test_cli_directory_input() {
    let output = run_pycfg(&["--format", "json", "tests/test_code"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed["files"].as_array().unwrap();
    assert!(arr.len() >= 4, "should have >= 4 files, got {}", arr.len());
}

#[test]
fn test_cli_multiple_files_text() {
    let output = run_pycfg(&["tests/test_code/basic_if.py", "tests/test_code/loops.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# file: tests/test_code/basic_if.py"));
    assert!(stdout.contains("# file: tests/test_code/loops.py"));
    assert!(stdout.contains("def check_sign:"));
    assert!(stdout.contains("def my_func:"));
}

#[test]
fn test_cli_explicit_exceptions() {
    let output = run_pycfg(&[
        "--format",
        "json",
        "--explicit-exceptions",
        "tests/test_code/try_except.py",
    ]);
    assert!(output.status.success());
}

#[test]
fn test_cli_nonexistent_file() {
    let output = run_pycfg(&["nonexistent_file_xyz.py"]);
    assert!(!output.status.success(), "nonexistent file should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No Python files or functions found to analyze"));
}

#[test]
fn test_cli_skips_parse_errors_and_continues() {
    let dir = tempdir().unwrap();
    let valid = dir.path().join("valid.py");
    let invalid = dir.path().join("invalid.py");
    std::fs::write(&valid, "def ok():\n    return 1\n").unwrap();
    std::fs::write(&invalid, "def broken(:\n    return 2\n").unwrap();

    let output = run_pycfg(&["--format", "json", dir.path().to_str().unwrap()]);
    assert!(
        output.status.success(),
        "valid files should still be analyzed"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let files = parsed["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "only the valid file should be analyzed");
    assert!(files[0]["file"].as_str().unwrap().ends_with("valid.py"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Skipping"));
    assert!(stderr.contains("parse errors"));
}

#[test]
fn test_cli_multi_file_text_separator() {
    let output = run_pycfg(&["tests/test_code/basic_if.py", "tests/test_code/loops.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.starts_with('\n'),
        "output should not start with blank line"
    );
    assert!(stdout.contains("\n\n# file: tests/test_code/loops.py\n\n"));
}

#[test]
fn test_cli_json_single_vs_multi() {
    let output1 = run_pycfg(&["--format", "json", "tests/test_code/basic_if.py"]);
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let parsed1: serde_json::Value = serde_json::from_str(&stdout1).unwrap();
    assert!(
        parsed1.is_object(),
        "single file JSON should be an object envelope"
    );
    assert_eq!(parsed1["files"].as_array().unwrap().len(), 1);

    let output2 = run_pycfg(&[
        "--format",
        "json",
        "tests/test_code/basic_if.py",
        "tests/test_code/loops.py",
    ]);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let parsed2: serde_json::Value = serde_json::from_str(&stdout2).unwrap();
    assert!(
        parsed2.is_object(),
        "multi-file JSON should use the same envelope"
    );
    assert_eq!(parsed2["files"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_dot_multiple_files_single_graph() {
    let output = run_pycfg(&[
        "--format",
        "dot",
        "tests/test_code/basic_if.py",
        "tests/test_code/loops.py",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("digraph CFG {").count(), 1);
    assert!(stdout.contains("subgraph cluster_file_0"));
    assert!(stdout.contains("subgraph cluster_file_1"));
}

#[test]
fn test_cli_list_functions_text_output() {
    assert_cli_output_matches_golden(
        &["--list-functions", "tests/test_code/loops.py"],
        "tests/golden/loops.list-functions.text",
    );
}

#[test]
fn test_cli_list_functions_json_output() {
    assert_cli_output_matches_golden(
        &[
            "--list-functions",
            "--format",
            "json",
            "tests/test_code/loops.py",
        ],
        "tests/golden/loops.list-functions.json",
    );
}

#[test]
fn test_cli_list_functions_exact_target() {
    let output = run_pycfg(&[
        "--list-functions",
        "--format",
        "json",
        "tests/test_code/loops.py::my_func",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let functions = parsed["files"][0]["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["name"], "my_func");
}

#[test]
fn test_cli_summary_text_output() {
    assert_cli_output_matches_golden(
        &["--summary", "tests/test_code/basic_if.py"],
        "tests/golden/basic_if.summary.text",
    );
}

#[test]
fn test_cli_summary_json_output() {
    assert_cli_output_matches_golden(
        &[
            "--summary",
            "--format",
            "json",
            "tests/test_code/basic_if.py",
        ],
        "tests/golden/basic_if.summary.json",
    );
}

#[test]
fn test_cli_diagnostics_json_output() {
    let output = run_pycfg(&[
        "--diagnostics",
        "--format",
        "json",
        "tests/test_inputs/invalid_syntax.py",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let files = parsed["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["file"], "tests/test_inputs/invalid_syntax.py");
    let diagnostics = files[0]["diagnostics"].as_array().unwrap();
    assert!(!diagnostics.is_empty());
    assert!(diagnostics[0].as_str().unwrap().contains("Expected"));
}

#[test]
fn test_cli_diagnostics_text_output_for_valid_file() {
    let output = run_pycfg(&["--diagnostics", "tests/test_code/basic_if.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "tests/test_code/basic_if.py\nOK\n");
}

#[test]
fn test_cli_query_modes_reject_dot_output() {
    let output = run_pycfg(&[
        "--summary",
        "--format",
        "dot",
        "tests/test_code/basic_if.py",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--format dot is only supported for CFG output"));
}
