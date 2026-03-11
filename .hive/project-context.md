# Project Context — pycfg-rs

## Overview
Fast Rust CLI that parses Python source files and generates intra-procedural control flow graphs (basic blocks, edges, metrics) for each function, without requiring a Python runtime.

## Architecture
- **`src/cfg/`** — Core CFG construction: parses Python via `ruff_python_parser`, walks AST statements, builds basic blocks with edges for all Python control flow constructs (if/for/while/try/match/raise/assert/with/async). Submodules: `builder.rs` (CFG construction), `model.rs` (data types), `symbols.rs` (function/class visitor), `source_map.rs` (offset-to-line mapping).
- **`src/writer/`** — Output serialization: text (block-based, LLM-friendly), JSON (structured envelope), DOT (GraphViz with color-coded edges and per-function subgraphs).
- **`src/main.rs`** — CLI entry point (clap-derive). Four query modes: CFG (default), `--list-functions`, `--summary`, `--diagnostics`. Supports `file.py::FunctionName` targeting, directory recursion, and three output formats.
- **Data flow**: Python source -> `ruff_python_parser` AST -> `symbols::visit_functions` discovers functions -> `builder::build_single_cfg` constructs `FunctionCfg` per function -> `writer` serializes to chosen format -> stdout.

## Key Files
- `src/main.rs` — CLI definition, query modes, output dispatch, target parsing
- `src/cfg/mod.rs` — Public API: `try_build_cfgs`, `try_build_cfg_for_function`, `try_list_functions`, `parse_diagnostics`
- `src/cfg/builder.rs` — `CfgBuilder` struct: statement-by-statement CFG construction with loop/except/finally stacks
- `src/cfg/model.rs` — Core types: `BasicBlock`, `Edge`, `EdgeKind`, `BlockKind`, `FunctionCfg`, `FileCfg`, `Metrics`
- `src/cfg/symbols.rs` — `visit_functions`: walks AST to discover functions/methods with qualified names (e.g., `Class.method`)
- `src/cfg/source_map.rs` — `offset_to_line` and `range_text` for mapping AST ranges to source lines/text
- `src/cfg/tests.rs` — ~60 unit tests covering all control flow constructs, edge cases, explicit exceptions
- `src/writer/mod.rs` — `write_text`, `write_json`, `write_dot` and their `_report` multi-file variants
- `tests/cli.rs` — Integration tests: golden file comparisons, multi-file, directory input, error handling
- `tests/fixtures.rs` — Fixture-based tests per Python construct (loops, try/except, match, classes, etc.)
- `tests/writer.rs` — Output format tests: JSON roundtrip, DOT well-formedness, edge colors, escaping
- `tests/corpus.rs` — Corpus smoke tests (requests, flask, rich) — skipped if corpora not cloned
- `tests/common/mod.rs` — Test helpers: `analyze_file`, `run_pycfg`, `corpus_dir`, `analyze_corpus`
- `tests/golden/` — Golden files for CLI output pinning (text, JSON, DOT, summary, list-functions)

## Build & Test
- **Language**: Rust, edition 2024
- **Package manager**: Cargo (with git dependencies for ruff crates pinned to rev `b0617e8a9c`)
- **Build**: `cargo build` (release: `cargo build --release` with LTO, single codegen unit, stripped)
- **Test**: `cargo test`
- **Lint**: `cargo clippy -- -D warnings`
- **Format**: `cargo fmt --check`
- **Type check**: N/A (Rust compiler handles this)
- **Pre-commit**: N/A
- **CI**: GitHub Actions on push to master and PRs — runs build, fmt check, test, clippy, release smoke test (sequential, single job)
- **Quirks**: Corpus tests (`test_corpus_*`) are skipped gracefully if `benchmark/corpora/` not populated — run `./scripts/bootstrap-corpora.sh` first. Golden file tests in `tests/cli.rs` compare exact CLI output; update golden files when changing user-facing output. The binary name is `pycfg` (not `pycfg-rs`).

## Conventions
- All types derive `Serialize` for JSON output; `EdgeKind` and `BlockKind` serialize as lowercase strings
- `EdgeKind` is an enum with named variants (True, False, Return, Exception, etc.) plus `Case(String)` and `Other(String)` for extensibility
- Error handling: `try_*` functions return `Result<T, ParseError>`; panicking wrappers (`build_cfgs`, `list_functions`) exist for convenience
- Parse errors are collected (not fatal) — files with parse errors are skipped with `log::warn!`
- Test fixtures are handcrafted Python files under `tests/test_code/`; each covers a specific construct
- Integration tests use `env!("CARGO_BIN_EXE_pycfg")` to invoke the compiled binary
- Golden files live in `tests/golden/` — when output format changes, update golden files and review the diff
- Function targeting uses `file.py::QualifiedName` syntax (exact match, `Class.method` not just `method`)
- `CfgBuilder` uses stacks for loop context (`loop_stack`), exception handlers (`except_stack`), and finally blocks (`finally_stack`)
- Multi-file text output uses `# file: path` headers; JSON wraps in `{"files": [...]}` envelope; DOT uses file-level `subgraph cluster_file_N`

## Dependencies & Integration
- **ruff_python_parser / ruff_python_ast / ruff_text_size**: Pinned git deps from astral-sh/ruff — provides Python parsing without a Python runtime. This is the same parser the Ruff linter uses.
- **clap 4** (derive): CLI argument parsing
- **serde / serde_json**: JSON serialization of all output types
- **walkdir**: Recursive directory traversal for `.py` file discovery
- **anyhow**: Error handling in CLI main
- **log / env_logger**: Logging with `-v`/`-vv` verbosity levels
- **tempfile** (dev): Temporary directories in integration tests
- No external services, databases, or network calls at runtime
- Sibling tool: [pycallgraph-rs](https://github.com/tnguyen21/pycallgraph-rs) (`pycg`) for inter-procedural call graphs

## Gotchas
- The ruff parser crates are **git dependencies** pinned to a specific rev — `cargo update` won't bump them. To update, change the `rev` in `Cargo.toml` and verify nothing broke in the AST API.
- `BlockKind::Entry` is always block 0, `BlockKind::Exit` is always block 1. The builder creates them first in `build_single_cfg`.
- `range_text()` only returns the **first line** of a multi-line statement (trimmed). Multi-line expressions appear truncated in CFG output.
- `finally` handling is complex: `emit_pending_edges` intercepts control flow transfers (return/break/raise) to route through finally blocks. The `FinallyFrame` also tracks `local_handler_targets` to avoid double-routing exception edges that already target a local handler.
- Mutation testing (`cargo mutants`) is part of the maintenance workflow for touched files — see README.
- The `benchmark/` directory contains cloned Python projects (requests, flask, rich, etc.) — these are large and not committed. The `staticfg/` and `python-graphs/` directories are vendored reference implementations kept for comparison benchmarks.
- Edition 2024 Rust — requires recent stable toolchain. Uses `let-else` chains and `if let ... &&` syntax.
