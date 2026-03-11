use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use serde::Serialize;
use walkdir::WalkDir;

use pycfg_rs::cfg::{self, CfgOptions, FileCfg, FunctionInfo};
use pycfg_rs::writer;

#[derive(Parser)]
#[command(
    name = "pycfg",
    version,
    about = "Generate control flow graphs for Python programs"
)]
struct Cli {
    /// Python source files, directories, or file::function targets
    #[arg(required = true)]
    targets: Vec<String>,

    /// Output format
    #[arg(long, default_value = "text")]
    format: Format,

    /// Add per-statement exception edges inside try blocks
    #[arg(long)]
    explicit_exceptions: bool,

    /// List discovered functions instead of emitting CFGs
    #[arg(long, conflicts_with_all = ["summary", "diagnostics"])]
    list_functions: bool,

    /// Emit per-function metric summaries instead of full CFGs
    #[arg(long, conflicts_with_all = ["list_functions", "diagnostics"])]
    summary: bool,

    /// Emit parse diagnostics instead of CFGs
    #[arg(long, conflicts_with_all = ["list_functions", "summary"])]
    diagnostics: bool,

    /// Enable verbose logging (-v info, -vv debug)
    #[arg(long, short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Format {
    Text,
    Json,
    Dot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryMode {
    Cfg,
    ListFunctions,
    Summary,
    Diagnostics,
}

#[derive(Serialize)]
struct FunctionListFile {
    file: String,
    functions: Vec<FunctionInfo>,
}

#[derive(Serialize)]
struct FunctionListReport {
    files: Vec<FunctionListFile>,
}

#[derive(Serialize)]
struct SummaryFunction {
    name: String,
    line: usize,
    blocks: usize,
    edges: usize,
    branches: usize,
    cyclomatic_complexity: usize,
}

#[derive(Serialize, Default)]
struct SummaryTotals {
    files: usize,
    functions: usize,
    blocks: usize,
    edges: usize,
    branches: usize,
    max_cyclomatic_complexity: usize,
}

#[derive(Serialize)]
struct SummaryFile {
    file: String,
    function_count: usize,
    totals: SummaryTotals,
    functions: Vec<SummaryFunction>,
}

#[derive(Serialize)]
struct SummaryReport {
    files: Vec<SummaryFile>,
    totals: SummaryTotals,
}

#[derive(Serialize)]
struct DiagnosticsFile {
    file: String,
    diagnostics: Vec<String>,
}

#[derive(Serialize)]
struct DiagnosticsReport {
    files: Vec<DiagnosticsFile>,
}

/// Parse a target string into (file_path, optional_function_name).
fn parse_target(target: &str) -> (String, Option<String>) {
    if let Some(idx) = target.find("::") {
        let file = target[..idx].to_string();
        let func = target[idx + 2..].to_string();
        (file, Some(func))
    } else {
        (target.to_string(), None)
    }
}

fn collect_python_files(path: &str) -> Vec<String> {
    let p = PathBuf::from(path);
    let mut files = Vec::new();
    if p.is_dir() {
        for entry in WalkDir::new(&p)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().is_some_and(|ext| ext == "py")
                    && !e.path().to_string_lossy().contains("__pycache__")
            })
        {
            files.push(entry.path().to_string_lossy().to_string());
        }
    } else if p.is_file() && p.extension().is_some_and(|ext| ext == "py") {
        files.push(path.to_string());
    }
    files.sort();
    files.dedup();
    files
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let query_mode = query_mode(&cli);
    validate_query_mode(query_mode, cli.format)?;

    let log_level = log_level_for_verbosity(cli.verbose);
    env_logger::Builder::new().filter_level(log_level).init();

    let options = CfgOptions {
        explicit_exceptions: cli.explicit_exceptions,
    };

    let mut all_cfgs: Vec<FileCfg> = Vec::new();
    let mut function_reports: Vec<FunctionListFile> = Vec::new();
    let mut summary_reports: Vec<SummaryFile> = Vec::new();
    let mut diagnostics_reports: Vec<DiagnosticsFile> = Vec::new();

    for target in &cli.targets {
        let (file_path, func_name) = parse_target(target);
        let files = collect_python_files(&file_path);

        if files.is_empty() {
            log::warn!("No Python files found for: {}", file_path);
            continue;
        }

        for file in &files {
            let source = match std::fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("Failed to read {}: {}", file, e);
                    continue;
                }
            };

            match query_mode {
                QueryMode::Cfg => {
                    let file_cfg = if let Some(ref func) = func_name {
                        match cfg::try_build_cfg_for_function(&source, file, func, &options) {
                            Ok(Some(fc)) => fc,
                            Ok(None) => {
                                log::warn!("Function '{}' not found in {}", func, file);
                                continue;
                            }
                            Err(err) => {
                                log::warn!("Skipping {} due to parse errors: {}", file, err);
                                continue;
                            }
                        }
                    } else {
                        match cfg::try_build_cfgs(&source, file, &options) {
                            Ok(fc) => fc,
                            Err(err) => {
                                log::warn!("Skipping {} due to parse errors: {}", file, err);
                                continue;
                            }
                        }
                    };

                    all_cfgs.push(file_cfg);
                }
                QueryMode::ListFunctions => {
                    let mut functions = match cfg::try_list_functions(&source) {
                        Ok(functions) => functions,
                        Err(err) => {
                            log::warn!("Skipping {} due to parse errors: {}", file, err);
                            continue;
                        }
                    };
                    if let Some(ref func) = func_name {
                        functions.retain(|function| function.name == *func);
                        if functions.is_empty() {
                            log::warn!("Function '{}' not found in {}", func, file);
                            continue;
                        }
                    }
                    function_reports.push(FunctionListFile {
                        file: file.clone(),
                        functions,
                    });
                }
                QueryMode::Summary => {
                    let file_cfg = if let Some(ref func) = func_name {
                        match cfg::try_build_cfg_for_function(&source, file, func, &options) {
                            Ok(Some(fc)) => fc,
                            Ok(None) => {
                                log::warn!("Function '{}' not found in {}", func, file);
                                continue;
                            }
                            Err(err) => {
                                log::warn!("Skipping {} due to parse errors: {}", file, err);
                                continue;
                            }
                        }
                    } else {
                        match cfg::try_build_cfgs(&source, file, &options) {
                            Ok(fc) => fc,
                            Err(err) => {
                                log::warn!("Skipping {} due to parse errors: {}", file, err);
                                continue;
                            }
                        }
                    };
                    summary_reports.push(summarize_file(file_cfg));
                }
                QueryMode::Diagnostics => {
                    diagnostics_reports.push(DiagnosticsFile {
                        file: file.clone(),
                        diagnostics: cfg::parse_diagnostics(&source),
                    });
                }
            }
        }
    }

    let mut stdout = std::io::stdout().lock();

    match query_mode {
        QueryMode::Cfg => {
            if all_cfgs.is_empty() {
                bail!("No Python files or functions found to analyze");
            }

            match cli.format {
                Format::Json => {
                    let json = writer::write_json_report(&all_cfgs);
                    write_output(&mut stdout, &json)?;
                }
                Format::Text => {
                    let text = writer::write_text_report(&all_cfgs);
                    write_output(&mut stdout, &text)?;
                }
                Format::Dot => {
                    let dot = writer::write_dot_report(&all_cfgs);
                    write_output(&mut stdout, &dot)?;
                }
            }
        }
        QueryMode::ListFunctions => {
            if function_reports.is_empty() {
                bail!("No Python files or functions found to analyze");
            }

            match cli.format {
                Format::Json => {
                    let json = serde_json::to_string_pretty(&FunctionListReport {
                        files: function_reports,
                    })?;
                    write_output(&mut stdout, &json)?;
                }
                Format::Text => {
                    let text = write_function_list_report(&function_reports);
                    write_output(&mut stdout, &text)?;
                }
                Format::Dot => unreachable!("validated above"),
            }
        }
        QueryMode::Summary => {
            if summary_reports.is_empty() {
                bail!("No Python files or functions found to analyze");
            }

            let totals = summarize_totals(&summary_reports);
            match cli.format {
                Format::Json => {
                    let json = serde_json::to_string_pretty(&SummaryReport {
                        files: summary_reports,
                        totals,
                    })?;
                    write_output(&mut stdout, &json)?;
                }
                Format::Text => {
                    let text = write_summary_report(&summary_reports, &totals);
                    write_output(&mut stdout, &text)?;
                }
                Format::Dot => unreachable!("validated above"),
            }
        }
        QueryMode::Diagnostics => {
            if diagnostics_reports.is_empty() {
                bail!("No Python files found to analyze");
            }

            match cli.format {
                Format::Json => {
                    let json = serde_json::to_string_pretty(&DiagnosticsReport {
                        files: diagnostics_reports,
                    })?;
                    write_output(&mut stdout, &json)?;
                }
                Format::Text => {
                    let text = write_diagnostics_report(&diagnostics_reports);
                    write_output(&mut stdout, &text)?;
                }
                Format::Dot => unreachable!("validated above"),
            }
        }
    }

    Ok(())
}

fn validate_query_mode(query_mode: QueryMode, format: Format) -> Result<()> {
    if query_mode != QueryMode::Cfg && format == Format::Dot {
        bail!("--format dot is only supported for CFG output");
    }
    Ok(())
}

fn query_mode(cli: &Cli) -> QueryMode {
    if cli.list_functions {
        QueryMode::ListFunctions
    } else if cli.summary {
        QueryMode::Summary
    } else if cli.diagnostics {
        QueryMode::Diagnostics
    } else {
        QueryMode::Cfg
    }
}

fn summarize_file(file_cfg: FileCfg) -> SummaryFile {
    let functions: Vec<SummaryFunction> = file_cfg
        .functions
        .into_iter()
        .map(|function| SummaryFunction {
            name: function.name,
            line: function.line,
            blocks: function.metrics.blocks,
            edges: function.metrics.edges,
            branches: function.metrics.branches,
            cyclomatic_complexity: function.metrics.cyclomatic_complexity,
        })
        .collect();
    let totals = SummaryTotals {
        files: 1,
        functions: functions.len(),
        blocks: functions.iter().map(|function| function.blocks).sum(),
        edges: functions.iter().map(|function| function.edges).sum(),
        branches: functions.iter().map(|function| function.branches).sum(),
        max_cyclomatic_complexity: functions
            .iter()
            .map(|function| function.cyclomatic_complexity)
            .max()
            .unwrap_or(0),
    };

    SummaryFile {
        file: file_cfg.file,
        function_count: totals.functions,
        totals,
        functions,
    }
}

fn summarize_totals(files: &[SummaryFile]) -> SummaryTotals {
    SummaryTotals {
        files: files.len(),
        functions: files.iter().map(|file| file.function_count).sum(),
        blocks: files.iter().map(|file| file.totals.blocks).sum(),
        edges: files.iter().map(|file| file.totals.edges).sum(),
        branches: files.iter().map(|file| file.totals.branches).sum(),
        max_cyclomatic_complexity: files
            .iter()
            .map(|file| file.totals.max_cyclomatic_complexity)
            .max()
            .unwrap_or(0),
    }
}

fn write_function_list_report(files: &[FunctionListFile]) -> String {
    let mut out = String::new();
    let show_file_headers = files.len() > 1;

    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if show_file_headers {
            out.push_str("# file: ");
            out.push_str(&file.file);
            out.push_str("\n\n");
        }
        if file.functions.is_empty() {
            out.push_str("(no functions)\n");
            continue;
        }
        for function in &file.functions {
            out.push_str(&format!("{} [L{}]\n", function.name, function.line));
        }
    }

    out
}

fn write_summary_report(files: &[SummaryFile], totals: &SummaryTotals) -> String {
    let mut out = String::new();
    let show_file_headers = files.len() > 1;

    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if show_file_headers {
            out.push_str("# file: ");
            out.push_str(&file.file);
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "functions={} blocks={} edges={} branches={} max_cyclomatic_complexity={}\n",
            file.function_count,
            file.totals.blocks,
            file.totals.edges,
            file.totals.branches,
            file.totals.max_cyclomatic_complexity
        ));
        for function in &file.functions {
            out.push_str(&format!(
                "{} [L{}] blocks={} edges={} branches={} cyclomatic_complexity={}\n",
                function.name,
                function.line,
                function.blocks,
                function.edges,
                function.branches,
                function.cyclomatic_complexity
            ));
        }
    }

    if files.len() > 1 {
        out.push_str(&format!(
            "\nTOTAL files={} functions={} blocks={} edges={} branches={} max_cyclomatic_complexity={}\n",
            totals.files,
            totals.functions,
            totals.blocks,
            totals.edges,
            totals.branches,
            totals.max_cyclomatic_complexity
        ));
    }

    out
}

fn write_diagnostics_report(files: &[DiagnosticsFile]) -> String {
    let mut out = String::new();
    let show_file_headers = files.len() > 1;

    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if show_file_headers {
            out.push_str("# file: ");
            out.push_str(&file.file);
            out.push('\n');
        } else {
            out.push_str(&file.file);
            out.push('\n');
        }
        if file.diagnostics.is_empty() {
            out.push_str("OK\n");
            continue;
        }
        for diagnostic in &file.diagnostics {
            out.push_str("- ");
            out.push_str(diagnostic);
            out.push('\n');
        }
    }

    out
}

fn write_output(stdout: &mut std::io::StdoutLock<'_>, content: &str) -> Result<()> {
    write_output_to(stdout, content)
}

fn log_level_for_verbosity(verbose: u8) -> log::LevelFilter {
    match verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    }
}

fn write_output_to<W: Write>(writer: &mut W, content: &str) -> Result<()> {
    if let Err(e) = writer.write_all(content.as_bytes())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct StubWriter {
        error: Option<io::ErrorKind>,
        writes: Vec<u8>,
    }

    impl StubWriter {
        fn ok() -> Self {
            Self {
                error: None,
                writes: Vec::new(),
            }
        }

        fn failing(kind: io::ErrorKind) -> Self {
            Self {
                error: Some(kind),
                writes: Vec::new(),
            }
        }
    }

    impl Write for StubWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.error {
                return Err(io::Error::from(kind));
            }
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_parse_target_file_only() {
        let (file, func) = parse_target("src/handler.py");
        assert_eq!(file, "src/handler.py");
        assert_eq!(func, None);
    }

    #[test]
    fn test_parse_target_with_function() {
        let (file, func) = parse_target("src/handler.py::process_request");
        assert_eq!(file, "src/handler.py");
        assert_eq!(func, Some("process_request".to_string()));
    }

    #[test]
    fn test_parse_target_with_class_method() {
        let (file, func) = parse_target("src/handler.py::MyClass.handle");
        assert_eq!(file, "src/handler.py");
        assert_eq!(func, Some("MyClass.handle".to_string()));
    }

    #[test]
    fn test_parse_target_no_separator() {
        let (file, func) = parse_target("just_a_file.py");
        assert_eq!(file, "just_a_file.py");
        assert_eq!(func, None);
    }

    #[test]
    fn test_collect_python_files_single_file() {
        let files = collect_python_files("tests/test_code/basic_if.py");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "tests/test_code/basic_if.py");
    }

    #[test]
    fn test_collect_python_files_directory() {
        let files = collect_python_files("tests/test_code");
        assert!(
            files.len() >= 4,
            "should find multiple .py files, got {}",
            files.len()
        );
        assert!(files.iter().all(|f| f.ends_with(".py")));
        for window in files.windows(2) {
            assert!(
                window[0] <= window[1],
                "files should be sorted: {} > {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_collect_python_files_excludes_pycache() {
        let files = collect_python_files("tests/test_code");
        assert!(!files.iter().any(|f| f.contains("__pycache__")));
    }

    #[test]
    fn test_collect_python_files_nonexistent() {
        let files = collect_python_files("this_does_not_exist_xyz");
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_python_files_non_python() {
        let files = collect_python_files("Cargo.toml");
        assert!(files.is_empty(), "non-.py files should be rejected");
    }

    #[test]
    fn test_collect_python_files_nonexistent_py_rejected() {
        let files = collect_python_files("nonexistent_file_xyz.py");
        assert!(
            files.is_empty(),
            "nonexistent .py files should not be accepted"
        );
    }

    #[test]
    fn test_collect_python_files_nonexistent_non_py() {
        let files = collect_python_files("nonexistent_file_xyz.txt");
        assert!(
            files.is_empty(),
            "nonexistent non-.py file should not be included"
        );
    }

    #[test]
    fn test_log_level_for_verbosity() {
        assert_eq!(log_level_for_verbosity(0), log::LevelFilter::Warn);
        assert_eq!(log_level_for_verbosity(1), log::LevelFilter::Info);
        assert_eq!(log_level_for_verbosity(2), log::LevelFilter::Debug);
        assert_eq!(log_level_for_verbosity(9), log::LevelFilter::Debug);
    }

    #[test]
    fn test_query_mode_for_flags() {
        let mut cli = Cli {
            targets: vec!["test.py".to_string()],
            format: Format::Text,
            explicit_exceptions: false,
            list_functions: false,
            summary: false,
            diagnostics: false,
            verbose: 0,
        };
        assert_eq!(query_mode(&cli), QueryMode::Cfg);

        cli.list_functions = true;
        assert_eq!(query_mode(&cli), QueryMode::ListFunctions);

        cli.list_functions = false;
        cli.summary = true;
        assert_eq!(query_mode(&cli), QueryMode::Summary);

        cli.summary = false;
        cli.diagnostics = true;
        assert_eq!(query_mode(&cli), QueryMode::Diagnostics);
    }

    #[test]
    fn test_validate_query_mode_rejects_dot_for_non_cfg() {
        let err = validate_query_mode(QueryMode::Summary, Format::Dot).unwrap_err();
        assert!(err.to_string().contains("only supported for CFG output"));
    }

    #[test]
    fn test_write_function_list_report_single_file() {
        let text = write_function_list_report(&[FunctionListFile {
            file: "test.py".to_string(),
            functions: vec![FunctionInfo {
                name: "foo".to_string(),
                line: 3,
            }],
        }]);
        assert_eq!(text, "foo [L3]\n");
    }

    #[test]
    fn test_write_function_list_report_multiple_files() {
        let text = write_function_list_report(&[
            FunctionListFile {
                file: "a.py".to_string(),
                functions: vec![FunctionInfo {
                    name: "foo".to_string(),
                    line: 1,
                }],
            },
            FunctionListFile {
                file: "b.py".to_string(),
                functions: vec![FunctionInfo {
                    name: "bar".to_string(),
                    line: 2,
                }],
            },
        ]);
        assert_eq!(
            text,
            "# file: a.py\n\nfoo [L1]\n\n# file: b.py\n\nbar [L2]\n"
        );
    }

    #[test]
    fn test_write_summary_report_multiple_files_includes_total() {
        let files = vec![
            SummaryFile {
                file: "a.py".to_string(),
                function_count: 1,
                totals: SummaryTotals {
                    files: 1,
                    functions: 1,
                    blocks: 2,
                    edges: 1,
                    branches: 0,
                    max_cyclomatic_complexity: 1,
                },
                functions: vec![SummaryFunction {
                    name: "foo".to_string(),
                    line: 1,
                    blocks: 2,
                    edges: 1,
                    branches: 0,
                    cyclomatic_complexity: 1,
                }],
            },
            SummaryFile {
                file: "b.py".to_string(),
                function_count: 1,
                totals: SummaryTotals {
                    files: 1,
                    functions: 1,
                    blocks: 3,
                    edges: 2,
                    branches: 1,
                    max_cyclomatic_complexity: 2,
                },
                functions: vec![SummaryFunction {
                    name: "bar".to_string(),
                    line: 4,
                    blocks: 3,
                    edges: 2,
                    branches: 1,
                    cyclomatic_complexity: 2,
                }],
            },
        ];
        let totals = summarize_totals(&files);
        let text = write_summary_report(&files, &totals);
        assert_eq!(
            text,
            "# file: a.py\n\nfunctions=1 blocks=2 edges=1 branches=0 max_cyclomatic_complexity=1\nfoo [L1] blocks=2 edges=1 branches=0 cyclomatic_complexity=1\n\n# file: b.py\n\nfunctions=1 blocks=3 edges=2 branches=1 max_cyclomatic_complexity=2\nbar [L4] blocks=3 edges=2 branches=1 cyclomatic_complexity=2\n\nTOTAL files=2 functions=2 blocks=5 edges=3 branches=1 max_cyclomatic_complexity=2\n"
        );
    }

    #[test]
    fn test_write_diagnostics_report_multiple_files() {
        let text = write_diagnostics_report(&[
            DiagnosticsFile {
                file: "a.py".to_string(),
                diagnostics: Vec::new(),
            },
            DiagnosticsFile {
                file: "b.py".to_string(),
                diagnostics: vec!["Expected ')'".to_string()],
            },
        ]);
        assert_eq!(text, "# file: a.py\nOK\n\n# file: b.py\n- Expected ')'\n");
    }

    #[test]
    fn test_write_diagnostics_report_ok() {
        let text = write_diagnostics_report(&[DiagnosticsFile {
            file: "test.py".to_string(),
            diagnostics: Vec::new(),
        }]);
        assert_eq!(text, "test.py\nOK\n");
    }

    #[test]
    fn test_write_output_to_writes_content() {
        let mut writer = StubWriter::ok();
        write_output_to(&mut writer, "hello").unwrap();
        assert_eq!(writer.writes, b"hello");
    }

    #[test]
    fn test_write_output_to_ignores_broken_pipe() {
        let mut writer = StubWriter::failing(io::ErrorKind::BrokenPipe);
        write_output_to(&mut writer, "hello").unwrap();
    }

    #[test]
    fn test_write_output_to_returns_other_errors() {
        let mut writer = StubWriter::failing(io::ErrorKind::WriteZero);
        let err = write_output_to(&mut writer, "hello").unwrap_err();
        assert!(
            err.to_string().contains("write zero"),
            "unexpected error: {err}"
        );
    }
}
