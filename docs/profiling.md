# Profiling pycfg-rs

How to profile, what tools to use, and how to interpret results.

## Prerequisites

```bash
cargo install samply    # CPU sampling profiler (macOS/Linux)
cargo install hyperfine # statistical benchmarking
```

The `sample` command ships with macOS and needs no installation.

## Cargo profile

The release profile strips debug symbols (`strip = true`), which makes profiled
binaries unreadable. A `[profile.profiling]` section in `Cargo.toml` inherits
release optimizations but keeps debug info:

```toml
[profile.profiling]
inherits = "release"
debug = true
strip = false
```

Build with:

```bash
cargo build --profile=profiling
# binary lands in target/profiling/pycfg
```

## Benchmark corpus

The test fixtures under `tests/test_code/` are too small (~500 lines) for
meaningful profiling. Clone real projects into `bench/` (gitignored):

```bash
mkdir -p bench
git clone --depth 1 https://github.com/psf/requests.git bench/requests
git clone --depth 1 https://github.com/pallets/flask.git bench/flask
```

This gives ~30k lines of real-world Python across ~120 files.

## Step 1: Baseline with hyperfine

Before changing anything, establish a repeatable number. `hyperfine` runs warmup
iterations, discards outliers, and reports mean/stddev/min/max.

```bash
hyperfine --warmup 3 --runs 20 \
  'target/release/pycfg bench/requests/src' \
  'target/release/pycfg bench/flask/src'
```

Compare modes to understand where time is spent:

```bash
hyperfine --warmup 3 --runs 20 \
  --command-name 'cfg'            'target/release/pycfg bench/flask/src' \
  --command-name 'list-functions'  'target/release/pycfg --list-functions bench/flask/src' \
  --command-name 'summary'         'target/release/pycfg --summary bench/flask/src' \
  --command-name 'json'            'target/release/pycfg --format json bench/flask/src'
```

The delta between `--list-functions` (parse + walk AST, no CFG build) and `cfg`
(full pipeline) isolates CFG construction cost.

For A/B comparisons after an optimization, export to JSON and compare:

```bash
hyperfine --warmup 3 --runs 30 --export-json before.json 'target/release/pycfg bench/flask/src'
# ... make changes, rebuild ...
hyperfine --warmup 3 --runs 30 --export-json after.json  'target/release/pycfg bench/flask/src'
```

## Step 2: CPU profiling with samply

`samply` is a sampling profiler that opens results in the Firefox Profiler UI.
It needs a process that runs long enough to collect samples — pycfg finishes in
~20ms, so use the profiling harness in `examples/profile_harness.rs` which loops
the analysis internally:

```bash
cargo build --profile=profiling --example profile_harness

# Record 300 iterations over both corpora
ITERATIONS=300 samply record \
  target/profiling/examples/profile_harness bench/flask/src bench/requests/src
```

This opens the Firefox Profiler in a browser. The three tabs that matter:

- **Call Tree** — top-down view. Start here to see which call paths dominate.
- **Flame Graph** — visual representation of the call tree. Wide bars = hot.
- **Bottom-Up** — flat self-time ranking. Best for finding the actual hotspot
  functions where CPU cycles are spent (not their callers).

To save a profile for later without opening the browser:

```bash
ITERATIONS=300 samply record --save-only -o bench/profile.json \
  target/profiling/examples/profile_harness bench/flask/src bench/requests/src

# Load it later:
samply load bench/profile.json
```

## Step 3: Quick flat profile with macOS sample

The built-in `sample` command gives a text-based call tree and flat profile
without needing a browser. Start the harness in the background and sample it:

```bash
ITERATIONS=500 target/profiling/examples/profile_harness \
  bench/flask/src bench/requests/src &
PID=$!
sleep 0.2
sample $PID 5 1 -file bench/sample_output.txt
wait $PID
```

The useful section is at the bottom, under "Sort by top of stack":

```
Sort by top of stack, same collapsed (when >= 5):
        CharSearcher::next_match   2453
        _platform_memcmp            543
        visit_scope                 244
        CfgBuilder::build_stmts     185
        Lexer::next_token           129
```

This is the **self-time** view — where the CPU actually sits, not including
callees. It answers: "which functions' own instructions are burning cycles?"

## Step 4: Allocation profiling with dhat

When the CPU profile points at malloc/free or you suspect allocation pressure,
use `dhat-rs` to count heap allocations.

Add to `Cargo.toml`:

```toml
[dev-dependencies]
dhat = "0.3"
```

Add to the top of your harness or binary:

```rust
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let _profiler = dhat::Profiler::new_heap();
    // ... rest of main ...
}
```

Run normally. On exit it prints a summary and writes `dhat-heap.json`, which you
can load in Firefox's DHAT viewer (`about:profiling` > Load).

## The profiling harness

`examples/profile_harness.rs` pre-reads all source files into memory, then loops
`try_build_cfgs` many times. This isolates CFG construction from filesystem I/O
and process startup. Control iteration count via `ITERATIONS` env var (default
200).

```bash
# Quick check
ITERATIONS=50 target/profiling/examples/profile_harness bench/flask/src

# Full profiling run
ITERATIONS=300 samply record target/profiling/examples/profile_harness bench/flask/src bench/requests/src
```

## Interpreting results

The pipeline for a single file is:

```
read file → ruff parse → visit AST (find functions) → build CFGs → serialize output
```

Typical time breakdown on an M4 Pro (flask + requests, ~30k lines):

| Stage | Time | Notes |
|---|---|---|
| Process startup + file I/O | ~1ms | Not worth optimizing |
| Ruff parsing | ~3ms | Already the fastest Python parser; can't improve |
| AST walking (visit_scope) | ~1ms | Linear in AST node count |
| CFG construction | ~12ms | The part we own |
| Text serialization | <1ms | Negligible |

Within CFG construction, the dominant cost (as of 2026-03) is `offset_to_line`
in `source_map.rs`, which does a linear scan from byte 0 on every call. This
shows up in the profile as `CharSearcher::next_match` because that's the
internal implementation of `str::lines()` counting newlines.

## Checklist for an optimization

1. Record baseline: `hyperfine --warmup 3 --runs 30 --export-json before.json`
2. Profile to confirm hypothesis: `samply record` or `sample`
3. Implement the change
4. Run tests: `cargo test`
5. Record after: `hyperfine --warmup 3 --runs 30 --export-json after.json`
6. Verify the profile shifted: re-run samply/sample, confirm the old hotspot shrank
