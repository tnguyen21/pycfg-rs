# pycfg-rs: Rust-based Python Control Flow Graph Analyzer

A fast, Rust-based intra-procedural CFG generator for Python. Sibling tool to [pycallgraph-rs](~/projects/pycallgraph-rs) (`pycg`). Primary consumers: LLMs and humans doing iterative software development.

Binary name: `pycfg`

## Verification (all phases)

- `cargo build`
- `cargo test`
- `cargo clippy -- -D warnings`

---

## Spec 1: Project Scaffolding + Core CFG + Text Output

### Requirements

- Initialize a Rust project (edition 2024) at `~/projects/pycfg-rs` with binary name `pycfg`
- Dependencies: `ruff_python_parser`, `ruff_python_ast`, `ruff_text_size` (git deps from astral-sh/ruff, same as pycallgraph-rs), `clap 4` (derive), `anyhow`, `walkdir`, `serde + serde_json` (for later JSON output)
- CLI accepts positional args: `pycfg <file.py>` (all functions) or `pycfg <file.py::function_name>` (single function). For methods: `file.py::ClassName.method_name`
- `--format` flag with values: `text` (default), `json`, `dot`
- Parse Python files using `ruff_python_parser::parse_unchecked` (same pattern as pycg)
- Build CFG as basic blocks for these core constructs:
  - `if`/`elif`/`else` — true/false branch edges
  - `for`/`while` — loop body, loop exit, `else` clause
  - `break`/`continue` — edges to loop exit / loop header
  - `return` — edge to function exit block
  - Sequential statements — fallthrough edges
- Each basic block contains: numeric ID, label (entry/exit/body), list of statements as source text with line numbers (format: `[L12] x = foo()`)
- Entry block (id 0) and exit block (last id) for every function
- Text output format (block-based):
  ```
  def process_request(x):

  Block 0 (entry):
    [L5] x = get_input()
    [L6] if x > 0:
    -> Block 1 [True]
    -> Block 2 [False]

  Block 1:
    [L7] result = process(x)
    [L8] return result
    -> Block 3 [return]

  Block 2:
    [L10] log_error(x)
    -> Block 3 [fallthrough]

  Block 3 (exit):
  ```
- When no `::function` is specified, print CFG for every function/method in the file, separated by blank lines
- When `::function` is specified, print only that function's CFG
- Metrics summary printed at the end of each function's CFG: `# blocks=N edges=N branches=N cyclomatic_complexity=N`
- Handle file-level mode: `pycfg file.py` with no functions defined should show the top-level code as a single CFG
- Unit tests for: if/else branching, for/while loops, break/continue, return, nested control flow, empty functions
- Handcrafted Python test fixtures under `tests/test_code/`

### Success Criteria

- All verification commands pass
- `pycfg tests/test_code/basic_if.py` produces correct block-based text output with line numbers
- `pycfg tests/test_code/loops.py::my_func` produces correct CFG for a single function
- `cargo install --path .` installs `pycfg` binary

### Ralph Command

/ralph-loop:ralph-loop "Read /Users/tau/projects/pycfg-rs/SPEC.md, implement Spec 1. Remove the existing staticfg/ and python-graphs/ directories — they are reference implementations no longer needed in the source tree. Initialize a fresh Rust project. Follow the conventions of ~/projects/pycallgraph-rs for parser integration, CLI structure, and error handling." --max-iterations 30 --completion-promise "cargo build && cargo test && cargo clippy -- -D warnings all pass"

---

## Spec 2: Advanced Control Flow Constructs

### Prerequisites

- Spec 1 complete and passing

### Requirements

- `try`/`except`/`else`/`finally` blocks:
  - Default (block-level): the try block as a whole gets edges to each except handler
  - `--explicit-exceptions` flag: every statement inside try gets an edge to each matching except handler
  - `finally` block always reachable from try, except, and else blocks
  - `else` block reachable only when try completes without exception
- `with` statements: model as entry to block, with edge to body; no `__enter__`/`__exit__` edges (flatten)
- `match`/`case` (Python 3.10+): each case arm is a separate successor from the match block, with case pattern as edge label
- `raise` statements: edge to nearest except handler or function exit
- `assert` statements: true branch continues, false branch edges to exception/exit
- Comprehensions (list/dict/set/generator): treat as inline subgraph or flatten to sequential (simpler)
- `async for`, `async with`, `await`: flatten to synchronous equivalents (no suspension edges), but statements are preserved as-is in block text
- `yield`/`yield from`: treat as regular statements (no suspension modeling), preserved in block text
- Nested functions/classes: each gets its own CFG, not merged into parent
- Update all three output formats to handle new edge labels: `[except TypeError]`, `[finally]`, `[case pattern]`, `[raise]`, `[assert-fail]`
- Tests for each construct with handcrafted fixtures

### Success Criteria

- All verification commands pass
- `pycfg tests/test_code/try_except.py::func` shows correct try/except/finally edges
- `pycfg --explicit-exceptions tests/test_code/try_except.py::func` shows per-statement exception edges
- `pycfg tests/test_code/match_case.py::func` shows case arms as separate branches
- Nested function definitions produce separate CFGs

### Ralph Command

/ralph-loop:ralph-loop "Read /Users/tau/projects/pycfg-rs/SPEC.md, implement Spec 2. Spec 1 is already complete." --max-iterations 30 --completion-promise "cargo build && cargo test && cargo clippy -- -D warnings all pass"

---

## Spec 3: JSON + DOT Output + CLI Polish

### Prerequisites

- Spec 2 complete and passing

### Requirements

- JSON output (`--format json`): graph-native format with successors inline per block:
  ```json
  {
    "file": "src/handler.py",
    "functions": [
      {
        "name": "process_request",
        "line": 5,
        "blocks": [
          {
            "id": 0,
            "label": "entry",
            "statements": [
              {"line": 5, "text": "x = get_input()"},
              {"line": 6, "text": "if x > 0:"}
            ],
            "successors": [
              {"target": 1, "label": "True"},
              {"target": 2, "label": "False"}
            ]
          }
        ],
        "metrics": {
          "cyclomatic_complexity": 4,
          "blocks": 7,
          "edges": 9,
          "branches": 3
        }
      }
    ]
  }
  ```
- DOT output (`--format dot`):
  - Each basic block is a node with label showing statements
  - Edges labeled with branch conditions
  - Entry/exit blocks visually distinct (double border or different shape)
  - One subgraph per function when analyzing whole file
  - Pipeable to `dot -Tsvg -o graph.svg`
- CLI enhancements:
  - Accept directories as input (recursively find `.py` files, same as pycg)
  - `--root` flag for module name resolution (display `pkg.module::func` instead of `src/pkg/module.py::func`)
  - `-v` / `-vv` for log verbosity (using `log` + `env_logger`, same as pycg)
  - Graceful handling of parse errors: warn and skip unparseable files, don't crash
  - `--version` flag
- Serialize/deserialize tests for JSON output
- DOT output validation (well-formed DOT syntax)

### Success Criteria

- All verification commands pass
- `pycfg --format json tests/test_code/basic_if.py` produces valid, parseable JSON matching the specified schema
- `pycfg --format dot tests/test_code/basic_if.py | dot -Tsvg` produces valid SVG (if graphviz installed; otherwise just validate DOT syntax)
- `pycfg src/` recursively analyzes all `.py` files in a directory
- Parse errors in one file don't prevent analysis of other files

### Ralph Command

/ralph-loop:ralph-loop "Read /Users/tau/projects/pycfg-rs/SPEC.md, implement Spec 3. Specs 1-2 are already complete." --max-iterations 25 --completion-promise "cargo build && cargo test && cargo clippy -- -D warnings all pass"

---

## Spec 4: Real-World Corpus Testing + Benchmarks

### Prerequisites

- Spec 3 complete and passing

### Requirements

- Add `scripts/bootstrap-corpora.sh` that clones test corpora (same pattern as pycallgraph-rs):
  - `requests` (small, clean codebase)
  - `flask` (medium, good mix of control flow)
  - `rich` (large, complex rendering logic)
- Corpus smoke tests (skip if corpora not present):
  - Parse all `.py` files without panicking
  - Produce non-degenerate CFGs (at least N blocks for files with control flow)
  - JSON output is valid and parseable
  - Metrics are plausible (cyclomatic complexity >= 1 for every function)
- Benchmark suite using `std::time::Instant` (not criterion, keep it simple):
  - Time to analyze all functions in a corpus
  - Print results as: `analyzed N functions in Xms (Y functions/sec)`
  - Run via `cargo run --release -- --format json <corpus_dir> > /dev/null` or a dedicated `--bench` flag
- Add a README.md with:
  - Installation instructions (`cargo install --path .`)
  - Usage examples for all three formats
  - Comparison with staticfg / python-graphs (speed, features)
  - Description of the block-based text format for LLM consumption

### Success Criteria

- All verification commands pass
- `./scripts/bootstrap-corpora.sh` downloads corpora successfully
- `pycfg flask/ --format json > /dev/null` completes without errors
- `pycfg requests/ --format text` produces plausible output for all functions
- README.md exists and covers installation, usage, and output formats

### Ralph Command

/ralph-loop:ralph-loop "Read /Users/tau/projects/pycfg-rs/SPEC.md, implement Spec 4. Specs 1-3 are already complete." --max-iterations 20 --completion-promise "cargo build && cargo test && cargo clippy -- -D warnings all pass, corpus smoke tests pass"
