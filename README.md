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

# Analyze a class method
pycfg src/handler.py::MyClass.handle

# Analyze all Python files in a directory
pycfg src/

# JSON output (LLM-friendly, structured)
pycfg --format json src/handler.py

# DOT output (pipe to graphviz for visualization)
pycfg --format dot src/handler.py | dot -Tsvg -o cfg.svg

# Enable per-statement exception edges inside try blocks
pycfg --explicit-exceptions src/handler.py
```

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

### JSON

Graph-native format with successors inline per block:

```json
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
  ]
}
```

### DOT

GraphViz DOT format. Entry/exit blocks use `Mrecord` shape, edges are color-coded:
- Green: True branch
- Red: False branch
- Blue: return
- Orange: exception/raise
- Purple: break
- Cyan: continue

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
cargo test                           # Run all tests
./scripts/bootstrap-corpora.sh       # Clone test corpora (requests, flask, rich)
cargo test -- --nocapture            # See corpus test output
```

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
