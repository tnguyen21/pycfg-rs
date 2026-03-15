// Standalone profiling harness. Compile and run under samply.
// Usage: cargo build --profile=profiling --example profile_harness
//        samply record target/profiling/examples/profile_harness bench/flask/src bench/requests/src

use std::env;
use std::path::PathBuf;
use walkdir::WalkDir;

use pycfg_rs::cfg::{self, CfgOptions};

fn collect_python_files(path: &str) -> Vec<String> {
    let p = PathBuf::from(path);
    let mut files = Vec::new();
    if p.is_dir() {
        for entry in WalkDir::new(&p)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "py"))
        {
            files.push(entry.path().to_string_lossy().to_string());
        }
    } else if p.is_file() {
        files.push(path.to_string());
    }
    files.sort();
    files
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: profile_harness <dir1> [dir2] ...");
        std::process::exit(1);
    }

    let iterations = env::var("ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    // Pre-read all files into memory so we're profiling CFG construction, not I/O
    let mut sources: Vec<(String, String)> = Vec::new();
    for dir in &args {
        for file in collect_python_files(dir) {
            if let Ok(source) = std::fs::read_to_string(&file) {
                sources.push((file, source));
            }
        }
    }

    eprintln!(
        "Profiling {} files x {} iterations = {} total parses",
        sources.len(),
        iterations,
        sources.len() * iterations
    );

    let options = CfgOptions {
        explicit_exceptions: false,
    };

    for i in 0..iterations {
        for (file, source) in &sources {
            let _cfg = cfg::try_build_cfgs(source, file, &options);
        }
        if i % 50 == 0 {
            eprintln!("  iteration {}/{}", i, iterations);
        }
    }

    eprintln!("Done.");
}
