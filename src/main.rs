use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use walkdir::WalkDir;

use pycfg_rs::cfg::{self, CfgOptions, FileCfg};
use pycfg_rs::writer;

#[derive(Parser)]
#[command(name = "pycfg", version, about = "Generate control flow graphs for Python programs")]
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

    /// Root directory for module name resolution
    #[arg(long, short = 'r')]
    root: Option<String>,

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
            if all_cfgs.len() == 1 {
                let json = writer::write_json(&all_cfgs[0]);
                write_output(&mut stdout, &json)?;
            } else {
                let json =
                    serde_json::to_string_pretty(&all_cfgs).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e));
                write_output(&mut stdout, &json)?;
            }
        }
        Format::Text => {
            for (i, file_cfg) in all_cfgs.iter().enumerate() {
                if i > 0 {
                    write_output(&mut stdout, "\n")?;
                }
                let text = writer::write_text(file_cfg);
                write_output(&mut stdout, &text)?;
            }
        }
        Format::Dot => {
            for file_cfg in &all_cfgs {
                let dot = writer::write_dot(file_cfg);
                write_output(&mut stdout, &dot)?;
            }
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
