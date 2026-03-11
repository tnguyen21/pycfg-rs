# pycfg-rs

Fast, Rust-based control flow graph (CFG) generator for Python. Parses Python source files and produces intra-procedural control flow graphs for each function, without requiring a Python runtime.

Sibling tool to [pycallgraph-rs](https://github.com/tnguyen21/pycallgraph-rs) (`pycg`) — together they cover inter-procedural (call graphs) and intra-procedural (control flow) static analysis for Python.

## Installation

```bash
cargo install --path .
```

Installs the `pycfg` binary to `~/.cargo/bin/`.

## Usage

```bash
# Analyze all functions in a file (text output, default)
pycfg src/handler.py

# Analyze a specific function
pycfg src/handler.py::process_request

# Analyze a class method using its exact qualified name
pycfg src/handler.py::MyClass.handle

# Analyze all Python files in a directory
pycfg src/

# JSON output (LLM-friendly, structured)
pycfg --format json src/handler.py

# DOT output (pipe to graphviz for visualization)
pycfg --format dot src/handler.py | dot -Tsvg -o cfg.svg

# List discovered functions without building CFG text
pycfg --list-functions src/handler.py

# Emit only per-function metrics
pycfg --summary --format json src/handler.py

# Emit parse diagnostics for one or more files
pycfg --diagnostics --format json src/

# Enable per-statement exception edges inside try blocks
pycfg --explicit-exceptions src/handler.py
```

Function targets are exact. Methods must be addressed as `ClassName.method_name`, not by leaf name alone.
Only `.py` files and directories are accepted as inputs. Files with parse errors are skipped with a warning instead of aborting the entire run.

## Output Formats

### Text (default)

Block-based format with line numbers, designed for LLM consumption:

```
def check_sign:

  Block 0 (entry):
    [L2] if x > 0:
    -> Block 2 [True]
    -> Block 4 [False]

  Block 2:
    [L3] result = "positive"
    -> Block 3 [fallthrough]

  Block 4:
    [L4] elif x == 0:
    -> Block 5 [True]
    -> Block 6 [False]

  ...

  # blocks=7 edges=8 branches=2 cyclomatic_complexity=3
```

When analyzing multiple files, text output is grouped under `# file: ...` headers.

### JSON

Stable envelope with file results under `files`:

```json
{
  "files": [
    {
      "file": "src/handler.py",
      "functions": [
        {
          "name": "check_sign",
          "line": 1,
          "blocks": [
            {
              "id": 0,
              "label": "entry",
              "statements": [
                {"line": 2, "text": "if x > 0:"}
              ],
              "successors": [
                {"target": 2, "label": "True"},
                {"target": 4, "label": "False"}
              ]
            }
          ],
          "metrics": {
            "cyclomatic_complexity": 3,
            "blocks": 7,
            "edges": 8,
            "branches": 2
          }
        }
      }
    }
  ]
}
```

### DOT

GraphViz DOT format. Directory and multi-file runs emit one valid DOT document with per-file clusters. Entry/exit blocks use `Mrecord` shape, edges are color-coded:
- Green: True branch
- Red: False branch
- Blue: return
- Orange: exception/raise
- Purple: break
- Cyan: continue

## Query Modes

In addition to full CFG output, `pycfg` supports a few narrower query-style modes:

- `--list-functions`: list exact qualified function names and line numbers
- `--summary`: emit per-function metric summaries without block bodies
- `--diagnostics`: emit parse diagnostics without attempting CFG generation

`--format text` and `--format json` work for all of these modes. `--format dot` is only valid for full CFG output.

## Supported Python Constructs

- `if`/`elif`/`else`
- `for`/`while` (including `else` clauses)
- `break`/`continue`
- `return`
- `try`/`except`/`else`/`finally` (block-level or per-statement with `--explicit-exceptions`)
- `with`/`async with`
- `match`/`case` (Python 3.10+)
- `raise`
- `assert`
- `async for`, `await`, `yield`/`yield from` (flattened to synchronous)
- Nested functions and classes (each gets its own CFG)

## Metrics

Each function's CFG includes:
- **blocks**: Number of basic blocks
- **edges**: Number of control flow edges
- **branches**: Number of blocks with multiple successors
- **cyclomatic_complexity**: McCabe cyclomatic complexity (E - N + 2)

## Testing

```bash
cargo fmt --check                     # Verify formatting
cargo test                           # Run all tests
cargo clippy -- -D warnings          # Lint with warnings denied
./scripts/bootstrap-corpora.sh       # Clone test corpora (requests, flask, rich)
cargo test -- --nocapture            # See corpus test output
cargo mutants --file src/cfg/mod.rs --file src/main.rs
```

Exact CLI output contracts are pinned with golden files in `tests/golden/`. When a user-facing text, JSON, or DOT change is intentional, update the corresponding golden and review the diff as part of the change.

Mutation testing is part of the maintenance workflow for touched surfaces. Full-project runs are useful periodically, but focused runs against the files you changed are usually enough to guard against regressions during routine maintenance.

## Benchmarks

Comparative benchmarks against [staticfg](https://github.com/coetaur0/staticfg) and [python-graphs](https://github.com/google-research/python-graphs) (Google).

### Results on `rich` (100 files, 911 functions, Apple M4 Pro)

| Tool | Files parsed | Functions | Min time | Per-function | Throughput |
|------|-------------|-----------|----------|-------------|------------|
| **pycfg-rs** | **100/100 (100%)** | **984** | 65ms | 0.066 ms/func | **15,068 func/sec** |
| staticfg | 74/100 (74%) | 239 | 84ms | 0.350 ms/func | 2,860 func/sec |
| python-graphs | 13/100 (13%) | 37 | 7ms | 0.192 ms/func | 5,211 func/sec |

**Per-function**: pycfg-rs is **5.3x faster** than staticfg and **2.9x faster** than python-graphs.

The Python tools appear faster in wall-clock time only because they analyze far fewer functions — staticfg crashes on 26% of files, and python-graphs requires runtime imports so only 13% of files are importable.

### Running benchmarks

```bash
# Setup (one-time)
./scripts/bootstrap-corpora.sh                                           # clone test corpora
cd benchmark && uv venv --python 3.12 .venv && cd ..                     # create venv
source benchmark/.venv/bin/activate
uv pip install staticfg python-graphs                                    # install competitors

# Run
cargo build --release                                                    # build optimized binary
python benchmark/bench.py                                                # test fixtures (tiny)
python benchmark/bench.py --corpus benchmark/corpora/requests/src        # requests (~18 files)
python benchmark/bench.py --corpus benchmark/corpora/flask/src/flask     # flask (~24 files)
python benchmark/bench.py --corpus benchmark/corpora/rich/rich           # rich (~100 files)
```

## Performance

Uses [ruff_python_parser](https://github.com/astral-sh/ruff) for parsing — the same parser used by the Ruff linter. No Python runtime needed.
