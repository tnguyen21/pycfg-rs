use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use walkdir::WalkDir;

use pycfg_rs::cfg::{self, CfgOptions, FileCfg};
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

    /// Enable verbose logging (-v info, -vv debug)
    #[arg(long, short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, clap::ValueEnum)]
enum Format {
    Text,
    Json,
    Dot,
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
    } else if p.extension().is_some_and(|ext| ext == "py") || p.exists() {
        files.push(path.to_string());
    }
    files.sort();
    files.dedup();
    files
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    };
    env_logger::Builder::new().filter_level(log_level).init();

    let options = CfgOptions {
        explicit_exceptions: cli.explicit_exceptions,
    };

    let mut all_cfgs: Vec<FileCfg> = Vec::new();

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

            let file_cfg = if let Some(ref func) = func_name {
                match cfg::build_cfg_for_function(&source, file, func, &options) {
                    Some(fc) => fc,
                    None => {
                        log::warn!("Function '{}' not found in {}", func, file);
                        continue;
                    }
                }
            } else {
                cfg::build_cfgs(&source, file, &options)
            };

            all_cfgs.push(file_cfg);
        }
    }

    if all_cfgs.is_empty() {
        bail!("No Python files or functions found to analyze");
    }

    let mut stdout = std::io::stdout().lock();

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

    Ok(())
}

fn write_output(stdout: &mut std::io::StdoutLock<'_>, content: &str) -> Result<()> {
    if let Err(e) = stdout.write_all(content.as_bytes())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Should be sorted
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
        // Cargo.toml exists but doesn't end with .py
        // However, collect_python_files has a fallback: if it exists, include it
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_collect_python_files_nonexistent_py() {
        // Catches: == to != for .py extension check (line 67)
        // A nonexistent .py file should still be included (by extension check)
        let files = collect_python_files("nonexistent_file_xyz.py");
        assert_eq!(
            files.len(),
            1,
            "nonexistent .py file should be included by extension check"
        );
        assert_eq!(files[0], "nonexistent_file_xyz.py");
    }

    #[test]
    fn test_collect_python_files_nonexistent_non_py() {
        // A nonexistent non-.py file should NOT be included
        let files = collect_python_files("nonexistent_file_xyz.txt");
        assert!(
            files.is_empty(),
            "nonexistent non-.py file should not be included"
        );
    }
}
