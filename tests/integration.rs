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
    let result = cfg::build_cfg_for_function(&source, "loops.py", "my_func", &CfgOptions::default());
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
        assert!(functions > 200, "expected >200 functions, got {}", functions);
    } else {
        eprintln!("Skipping rich corpus (not found). Run ./scripts/bootstrap-corpora.sh");
    }
}
