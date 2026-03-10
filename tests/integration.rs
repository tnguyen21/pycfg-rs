use pycfg_rs::cfg::{self, CfgOptions};
use std::path::Path;

fn analyze_file(path: &str) -> cfg::FileCfg {
    let source = std::fs::read_to_string(path).unwrap();
    cfg::build_cfgs(&source, path, &CfgOptions::default())
}

#[test]
fn test_basic_if_fixture() {
    let result = analyze_file("tests/test_code/basic_if.py");
    assert!(!result.functions.is_empty());
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "check_sign")
        .expect("should find check_sign");
    assert_eq!(func.metrics.cyclomatic_complexity, 3);
}

#[test]
fn test_loops_fixture() {
    let result = analyze_file("tests/test_code/loops.py");
    assert!(result.functions.iter().any(|f| f.name == "my_func"));
    assert!(result.functions.iter().any(|f| f.name == "while_loop"));
    assert!(result.functions.iter().any(|f| f.name == "break_loop"));
}

#[test]
fn test_loops_targeting() {
    let source = std::fs::read_to_string("tests/test_code/loops.py").unwrap();
    let result =
        cfg::build_cfg_for_function(&source, "loops.py", "my_func", &CfgOptions::default());
    assert!(result.is_some());
    let file_cfg = result.unwrap();
    assert_eq!(file_cfg.functions.len(), 1);
    assert_eq!(file_cfg.functions[0].name, "my_func");
}

#[test]
fn test_try_except_fixture() {
    let result = analyze_file("tests/test_code/try_except.py");
    let func = result.functions.iter().find(|f| f.name == "func").unwrap();
    let has_exception = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "exception");
    assert!(has_exception);
    let has_finally = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "finally");
    assert!(has_finally);
}

#[test]
fn test_match_case_fixture() {
    let result = analyze_file("tests/test_code/match_case.py");
    let func = result.functions.iter().find(|f| f.name == "func").unwrap();
    let case_edges: Vec<_> = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.label.starts_with("case "))
        .collect();
    assert_eq!(case_edges.len(), 4);
}

#[test]
fn test_json_roundtrip() {
    let result = analyze_file("tests/test_code/basic_if.py");
    let json = serde_json::to_string(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["functions"].is_array());
    assert!(parsed["functions"][0]["metrics"]["cyclomatic_complexity"].is_number());
}

#[test]
fn test_dot_wellformed() {
    let result = analyze_file("tests/test_code/basic_if.py");
    let dot = pycfg_rs::writer::write_dot(&result);
    assert!(dot.starts_with("digraph CFG {"));
    assert!(dot.contains("subgraph cluster_"));
    assert!(dot.ends_with("}\n"));
    assert!(dot.contains("->"));
}

#[test]
fn test_text_output_format() {
    let result = analyze_file("tests/test_code/basic_if.py");
    let text = pycfg_rs::writer::write_text(&result);
    assert!(text.contains("def check_sign:"));
    assert!(text.contains("Block 0 (entry):"));
    assert!(text.contains("[L"));
    assert!(text.contains("-> Block"));
    assert!(text.contains("# blocks="));
}

// ---------------------------------------------------------------------------
// Edge-case fixture tests
// ---------------------------------------------------------------------------

#[test]
fn test_nested_loops_fixture() {
    let result = analyze_file("tests/test_code/nested_loops.py");
    let funcs: Vec<&str> = result.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(
        funcs.contains(&"nested_for_while"),
        "missing nested_for_while"
    );
    assert!(
        funcs.contains(&"nested_while_while"),
        "missing nested_while_while"
    );
    assert!(funcs.contains(&"triple_nested"), "missing triple_nested");
    assert!(
        funcs.contains(&"break_outer_via_flag"),
        "missing break_outer_via_flag"
    );

    // nested_for_while should have break and continue edges
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "nested_for_while")
        .unwrap();
    let edge_labels: Vec<&str> = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .map(|e| e.label.as_str())
        .collect();
    assert!(edge_labels.contains(&"break"));
    assert!(edge_labels.contains(&"continue"));
}

#[test]
fn test_loop_else_fixture() {
    let result = analyze_file("tests/test_code/loop_else.py");

    // for_else_no_break: else clause should exist
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "for_else_no_break")
        .unwrap();
    assert!(
        func.blocks.len() >= 4,
        "for-else needs extra block for else body"
    );

    // while_else: should have False edge
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "while_else")
        .unwrap();
    let edge_labels: Vec<&str> = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .map(|e| e.label.as_str())
        .collect();
    assert!(
        edge_labels.contains(&"loop-back"),
        "while-else should have loop-back"
    );

    // for_else_with_break: break should skip else
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "for_else_with_break")
        .unwrap();
    let has_break = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "break");
    assert!(has_break);
}

#[test]
fn test_try_complex_fixture() {
    let result = analyze_file("tests/test_code/try_complex.py");

    // multiple_excepts: 3 exception edges
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "multiple_excepts")
        .unwrap();
    let exc_edges = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.label == "exception")
        .count();
    assert_eq!(
        exc_edges, 3,
        "multiple_excepts should have 3 exception edges"
    );

    // bare except should produce "except:" text
    let has_bare = func
        .blocks
        .iter()
        .any(|b| b.statements.iter().any(|s| s.text == "except:"));
    assert!(has_bare);

    // nested_try: should have exception edges for both levels
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "nested_try")
        .unwrap();
    let exc_edges = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.label == "exception")
        .count();
    assert!(exc_edges >= 2);

    // try_except_else: has try-else edge
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "try_except_else")
        .unwrap();
    let has_try_else = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "try-else");
    assert!(has_try_else);

    // try_except_else_finally: all edge types present
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "try_except_else_finally")
        .unwrap();
    let all_labels: Vec<&str> = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .map(|e| e.label.as_str())
        .collect();
    assert!(all_labels.contains(&"finally"));
    assert!(all_labels.contains(&"try-else"));
    assert!(all_labels.contains(&"exception"));

    // bare_raise: has raise edge
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "bare_raise")
        .unwrap();
    let has_raise = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "raise");
    assert!(has_raise);
}

#[test]
fn test_async_constructs_fixture() {
    let result = analyze_file("tests/test_code/async_constructs.py");

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "async_for_loop")
        .unwrap();
    let has_async_for = func
        .blocks
        .iter()
        .any(|b| b.statements.iter().any(|s| s.text.starts_with("async for")));
    assert!(has_async_for);

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "async_with_statement")
        .unwrap();
    let has_async_with = func.blocks.iter().any(|b| {
        b.statements
            .iter()
            .any(|s| s.text.starts_with("async with"))
    });
    assert!(has_async_with);
}

#[test]
fn test_generators_fixture() {
    let result = analyze_file("tests/test_code/generators.py");

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "simple_generator")
        .unwrap();
    let yields = func
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .filter(|s| s.text.starts_with("yield"))
        .count();
    assert_eq!(yields, 3);

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "yield_from_example")
        .unwrap();
    let yield_froms = func
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .filter(|s| s.text.starts_with("yield from"))
        .count();
    assert_eq!(yield_froms, 2);

    // generator_with_return: should have return edge
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "generator_with_return")
        .unwrap();
    let has_return = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .any(|e| e.label == "return");
    assert!(has_return);
}

#[test]
fn test_straight_line_fixture() {
    let result = analyze_file("tests/test_code/straight_line.py");

    for func_name in &[
        "straight_line",
        "single_statement",
        "pass_only",
        "assignments_only",
    ] {
        let func = result
            .functions
            .iter()
            .find(|f| f.name == *func_name)
            .unwrap_or_else(|| panic!("missing {}", func_name));
        assert_eq!(
            func.metrics.cyclomatic_complexity, 1,
            "{} should have complexity 1, got {}",
            func_name, func.metrics.cyclomatic_complexity
        );
        assert_eq!(
            func.metrics.branches, 0,
            "{} should have 0 branches",
            func_name
        );
    }
}

#[test]
fn test_complex_nesting_fixture() {
    let result = analyze_file("tests/test_code/complex_nesting.py");

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "if_in_loop_in_try")
        .unwrap();
    assert!(func.metrics.cyclomatic_complexity >= 3);

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "deeply_nested_returns")
        .unwrap();
    let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
    let return_edges = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.target == exit_id && e.label == "return")
        .count();
    assert!(
        return_edges >= 3,
        "deeply_nested_returns should have >= 3 return edges"
    );
}

#[test]
fn test_multiple_returns_fixture() {
    let result = analyze_file("tests/test_code/multiple_returns.py");

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "guard_clauses")
        .unwrap();
    let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
    let return_edges = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.target == exit_id && e.label == "return")
        .count();
    assert_eq!(
        return_edges, 4,
        "guard_clauses: 3 guard returns + 1 final return"
    );

    let func = result
        .functions
        .iter()
        .find(|f| f.name == "return_in_branches")
        .unwrap();
    let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
    let return_edges = func
        .blocks
        .iter()
        .flat_map(|b| &b.successors)
        .filter(|e| e.target == exit_id && e.label == "return")
        .count();
    assert_eq!(return_edges, 3, "return_in_branches has 3 returns");
}

#[test]
fn test_classes_fixture() {
    let result = analyze_file("tests/test_code/classes.py");

    let names: Vec<&str> = result.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Simple.__init__"));
    assert!(names.contains(&"Simple.get_x"));
    assert!(names.contains(&"WithClassMethod.create"));
    assert!(names.contains(&"WithClassMethod.validate"));
    assert!(names.contains(&"Nested.Inner.inner_method"));
    assert!(names.contains(&"Nested.outer_method"));
    assert!(names.contains(&"WithProperties.__init__"));
    assert!(names.contains(&"WithProperties.value"));

    // validate method should have complexity >= 2 (if branch)
    let func = result
        .functions
        .iter()
        .find(|f| f.name == "WithClassMethod.validate")
        .unwrap();
    assert!(func.metrics.cyclomatic_complexity >= 2);
}

#[test]
fn test_all_fixtures_json_roundtrip() {
    // Every fixture file should produce valid JSON
    let fixtures = [
        "tests/test_code/basic_if.py",
        "tests/test_code/loops.py",
        "tests/test_code/try_except.py",
        "tests/test_code/match_case.py",
        "tests/test_code/nested_loops.py",
        "tests/test_code/loop_else.py",
        "tests/test_code/try_complex.py",
        "tests/test_code/async_constructs.py",
        "tests/test_code/generators.py",
        "tests/test_code/straight_line.py",
        "tests/test_code/complex_nesting.py",
        "tests/test_code/multiple_returns.py",
        "tests/test_code/classes.py",
    ];
    for fixture in &fixtures {
        let result = analyze_file(fixture);
        for func in &result.functions {
            let json = serde_json::to_string(func).unwrap();
            let _: serde_json::Value = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("invalid JSON for {} in {}: {}", func.name, fixture, e));
            // All functions should have valid metrics
            assert!(
                func.metrics.cyclomatic_complexity >= 1,
                "cc < 1 for {} in {}",
                func.name,
                fixture
            );
            assert!(
                func.blocks.len() >= 2,
                "< 2 blocks for {} in {}",
                func.name,
                fixture
            );
        }
    }
}

#[test]
fn test_all_fixtures_dot_output() {
    let fixtures = [
        "tests/test_code/basic_if.py",
        "tests/test_code/nested_loops.py",
        "tests/test_code/try_complex.py",
        "tests/test_code/complex_nesting.py",
    ];
    for fixture in &fixtures {
        let result = analyze_file(fixture);
        let dot = pycfg_rs::writer::write_dot(&result);
        assert!(
            dot.starts_with("digraph CFG {"),
            "bad DOT start for {}",
            fixture
        );
        assert!(dot.ends_with("}\n"), "bad DOT end for {}", fixture);
        assert!(dot.contains("->"), "no edges in DOT for {}", fixture);
    }
}

// ---------------------------------------------------------------------------
// Writer tests (mutation-targeted)
// ---------------------------------------------------------------------------

#[test]
fn test_text_multi_function_separator() {
    // Catches: write_text `i > 0` mutations (line 8)
    let source = "def foo():\n    return 1\n\ndef bar():\n    return 2\n";
    let result = analyze_file_source(source);
    let text = pycfg_rs::writer::write_text(&result);
    // Multiple functions should be separated by blank lines
    assert!(text.contains("def foo:"));
    assert!(text.contains("def bar:"));
    // There should be a double newline between functions (separator)
    let foo_end = text.find("def bar:").unwrap();
    let before_bar = &text[..foo_end];
    assert!(
        before_bar.ends_with("\n\n"),
        "functions should be separated by blank line"
    );
}

#[test]
fn test_text_single_function_no_leading_blank() {
    let source = "def foo():\n    return 1\n";
    let result = analyze_file_source(source);
    let text = pycfg_rs::writer::write_text(&result);
    assert!(
        !text.starts_with('\n'),
        "single function should not start with blank line"
    );
}

#[test]
fn test_json_output_valid() {
    // Catches: write_json returning empty/garbage (line 18)
    let source = "def foo(x):\n    if x > 0:\n        return 1\n    return 0\n";
    let result = analyze_file_source(source);
    let json = pycfg_rs::writer::write_json(&result);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should be valid");
    assert!(parsed["functions"].is_array());
    let funcs = parsed["functions"].as_array().unwrap();
    assert!(!funcs.is_empty());
    assert_eq!(funcs[0]["name"], "foo");
    assert!(
        funcs[0]["metrics"]["cyclomatic_complexity"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert!(funcs[0]["blocks"].is_array());
    let blocks = funcs[0]["blocks"].as_array().unwrap();
    assert!(blocks.len() >= 2);
}

#[test]
fn test_json_report_output_stable_envelope() {
    let source = "def foo():\n    return 1\n";
    let result = analyze_file_source(source);
    let json = pycfg_rs::writer::write_json_report(&[result]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should be valid");
    assert!(parsed["files"].is_array());
    let files = parsed["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["functions"][0]["name"], "foo");
}

#[test]
fn test_dot_entry_exit_mrecord() {
    // Catches: entry/exit shape mutations (line 45)
    let source = "def foo():\n    return 1\n";
    let result = analyze_file_source(source);
    let dot = pycfg_rs::writer::write_dot(&result);
    // Entry and exit blocks should use Mrecord shape
    assert!(
        dot.contains("shape=Mrecord"),
        "entry/exit blocks should use Mrecord shape"
    );
}

#[test]
fn test_dot_body_block_record() {
    // Catches: entry/exit == mutations (line 45)
    // Verify that entry (block 0) and exit (block 1) have Mrecord,
    // but body blocks (block 2+) have plain record.
    let source = "def foo(x):\n    if x:\n        return 1\n    return 0\n";
    let result = analyze_file_source(source);
    let dot = pycfg_rs::writer::write_dot(&result);

    // Entry block (id=0) should have Mrecord
    assert!(
        dot.contains("foo_0 [shape=Mrecord"),
        "entry block should use Mrecord"
    );
    // Exit block (id=1) should have Mrecord
    assert!(
        dot.contains("foo_1 [shape=Mrecord"),
        "exit block should use Mrecord"
    );
    // Body block (id=2) should have plain record, not Mrecord
    assert!(
        dot.contains("foo_2 [shape=record,"),
        "body blocks should use record (not Mrecord)"
    );
    // Verify we don't have all Mrecords (body blocks must be different)
    let mrecord_count = dot.matches("shape=Mrecord").count();
    let record_count = dot.matches("shape=record,").count();
    assert!(mrecord_count >= 2, "need at least 2 Mrecord (entry+exit)");
    assert!(record_count >= 1, "need at least 1 plain record (body)");
}

#[test]
fn test_dot_edge_colors() {
    // Catches: edge color match arm deletions (lines 81-86)
    let source = "def foo(x):\n    while x > 0:\n        if x == 5:\n            break\n        if x == 3:\n            continue\n        x -= 1\n    return x\n";
    let result = analyze_file_source(source);
    let dot = pycfg_rs::writer::write_dot(&result);
    assert!(dot.contains("color=green"), "True edges should be green");
    assert!(dot.contains("color=red"), "False edges should be red");
    assert!(dot.contains("color=purple"), "break edges should be purple");
    assert!(dot.contains("color=cyan"), "continue edges should be cyan");
}

#[test]
fn test_dot_return_edge_color() {
    let source = "def foo():\n    return 1\n";
    let result = analyze_file_source(source);
    let dot = pycfg_rs::writer::write_dot(&result);
    assert!(dot.contains("color=blue"), "return edges should be blue");
}

#[test]
fn test_dot_exception_edge_color() {
    let source = "def foo():\n    raise ValueError()\n";
    let result = analyze_file_source(source);
    let dot = pycfg_rs::writer::write_dot(&result);
    assert!(dot.contains("color=orange"), "raise edges should be orange");
}

#[test]
fn test_dot_report_multi_file_single_graph() {
    let foo = analyze_file_source("def foo():\n    return 1\n");
    let bar = cfg::build_cfgs(
        "def bar():\n    return 2\n",
        "other.py",
        &CfgOptions::default(),
    );
    let dot = pycfg_rs::writer::write_dot_report(&[foo, bar]);
    assert_eq!(dot.matches("digraph CFG {").count(), 1);
    assert!(dot.contains("subgraph cluster_file_0"));
    assert!(dot.contains("subgraph cluster_file_1"));
}

fn analyze_file_source(source: &str) -> cfg::FileCfg {
    cfg::build_cfgs(source, "test.py", &CfgOptions::default())
}

// ---------------------------------------------------------------------------
// CLI binary tests
// ---------------------------------------------------------------------------

fn run_pycfg(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pycfg"))
        .args(args)
        .output()
        .expect("failed to execute pycfg")
}

#[test]
fn test_cli_text_output() {
    let output = run_pycfg(&["tests/test_code/basic_if.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("def check_sign:"));
    assert!(stdout.contains("Block 0 (entry):"));
}

#[test]
fn test_cli_json_output() {
    let output = run_pycfg(&["--format", "json", "tests/test_code/basic_if.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["files"].is_array());
}

#[test]
fn test_cli_dot_output() {
    let output = run_pycfg(&["--format", "dot", "tests/test_code/basic_if.py"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("digraph CFG {"));
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
    // The file doesn't exist, so read will fail, but it's still collected.
    // Depending on the code path, it may error or produce empty output.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // It should either fail or produce no functions
    assert!(
        !output.status.success() || stdout.is_empty() || stderr.contains("Failed"),
        "nonexistent file should produce error or warning"
    );
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

// ---------------------------------------------------------------------------
// Corpus smoke tests (skip if corpora not present)
// ---------------------------------------------------------------------------

fn corpus_dir(name: &str) -> Option<String> {
    let path = format!("benchmark/corpora/{}/", name);
    if Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

fn analyze_corpus(dir: &str) -> (usize, usize) {
    let mut total_functions = 0;
    let mut total_files = 0;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().is_some_and(|ext| ext == "py")
                && !e.path().to_string_lossy().contains("__pycache__")
        })
    {
        let path = entry.path().to_string_lossy().to_string();
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let result = cfg::build_cfgs(&source, &path, &CfgOptions::default());
        total_functions += result.functions.len();
        total_files += 1;

        // Validate each function's CFG
        for func in &result.functions {
            assert!(
                func.metrics.cyclomatic_complexity >= 1,
                "cyclomatic complexity < 1 for {} in {}",
                func.name,
                path
            );
            assert!(
                func.blocks.len() >= 2,
                "fewer than 2 blocks for {} in {}",
                func.name,
                path
            );

            // JSON should be valid
            let json = serde_json::to_string(&func).unwrap();
            let _: serde_json::Value = serde_json::from_str(&json).unwrap();
        }
    }
    (total_files, total_functions)
}

#[test]
fn test_corpus_requests() {
    if let Some(dir) = corpus_dir("requests") {
        let (files, functions) = analyze_corpus(&dir);
        eprintln!("requests: {} files, {} functions", files, functions);
        assert!(files > 10, "expected >10 files, got {}", files);
        assert!(functions > 50, "expected >50 functions, got {}", functions);
    } else {
        eprintln!("Skipping requests corpus (not found). Run ./scripts/bootstrap-corpora.sh");
    }
}

#[test]
fn test_corpus_flask() {
    if let Some(dir) = corpus_dir("flask") {
        let (files, functions) = analyze_corpus(&dir);
        eprintln!("flask: {} files, {} functions", files, functions);
        assert!(files > 10, "expected >10 files, got {}", files);
        assert!(functions > 50, "expected >50 functions, got {}", functions);
    } else {
        eprintln!("Skipping flask corpus (not found). Run ./scripts/bootstrap-corpora.sh");
    }
}

#[test]
fn test_corpus_rich() {
    if let Some(dir) = corpus_dir("rich") {
        let (files, functions) = analyze_corpus(&dir);
        eprintln!("rich: {} files, {} functions", files, functions);
        assert!(files > 20, "expected >20 files, got {}", files);
        assert!(
            functions > 200,
            "expected >200 functions, got {}",
            functions
        );
    } else {
        eprintln!("Skipping rich corpus (not found). Run ./scripts/bootstrap-corpora.sh");
    }
}
