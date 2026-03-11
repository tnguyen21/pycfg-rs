# Using `pycfg-rs` in LLM Workflows

## What It Is

`pycfg-rs` is a fast CLI for building intra-procedural control-flow graphs for
Python.

It is useful when a model needs function-level execution structure:

- branches
- loop exits and back-edges
- return paths
- `try`/`except`/`finally` routing
- `match`/`case` fanout

It is not a call-graph tool and it is not a full semantic interpreter.

## When to Use It

Reach for `pycfg-rs` when the task is about:

- understanding a complicated function before editing it
- checking whether all branches return or converge
- reasoning about loop `break` / `continue` behavior
- inspecting exception routing through `try` / `finally`
- getting a machine-readable structural summary for a Python function or file

Do not reach for it first when the task is about:

- cross-function dependencies
- import relationships
- dynamic runtime behavior
- type inference
- exact value flow

## High-Value Commands

Use the narrowest command that answers the question.

### Full CFG

```bash
pycfg path/to/file.py
pycfg path/to/file.py::ExactQualifiedName
pycfg --format json path/to/file.py::ExactQualifiedName
pycfg --format dot path/to/file.py::ExactQualifiedName
```

Use this when you need block bodies and edge labels.

### Function Discovery

```bash
pycfg --list-functions path/to/file.py
pycfg --list-functions --format json path/to/dir/
```

Use this before targeted analysis if you do not know the exact qualified name.

### Lightweight Metrics

```bash
pycfg --summary path/to/file.py
pycfg --summary --format json path/to/file.py::ExactQualifiedName
```

Use this for quick sizing and routing decisions without full block dumps.

### Parse Diagnostics

```bash
pycfg --diagnostics path/to/file.py
pycfg --diagnostics --format json path/to/dir/
```

Use this when analysis may fail because of syntax errors or unsupported parse
state.

### More Detailed Exceptional Paths

```bash
pycfg --explicit-exceptions path/to/file.py::ExactQualifiedName
```

Use this only when exception routing detail matters. The default CFG is often
easier to read.

## Recommended Dev Flow

For an LLM editing Python code, a good sequence is:

1. Find the exact function name with `--list-functions` if needed.
2. Run `--summary` to size the function and decide whether full CFG inspection
   is worth it.
3. Run full CFG output, usually `--format json`, on the exact target.
4. Make the code change.
5. Re-run the same command and compare structure if the change was control-flow
   sensitive.

Use text output for quick reading. Use JSON output when the result will be
post-processed or compared programmatically.

## Important Behavioral Facts

- Function targets are exact qualified names.
- Directory inputs recurse over `.py` files.
- Non-`.py` files are ignored.
- Files with parse errors are skipped during CFG generation, but diagnostics can
  be queried explicitly with `--diagnostics`.
- Async constructs are flattened to synchronous structure.
- `yield` and `yield from` are preserved as statements, not modeled as
  suspension points.

## Prompt Snippet

If you want to hand this tool to another model, this is a good starting prompt:

```text
Use `pycfg-rs` when you need function-level Python control-flow structure.
Prefer the narrowest command that answers the question:
- `--list-functions` to discover exact qualified names
- `--summary` for metric-only inspection
- full CFG output for block/edge structure
- `--diagnostics` if parsing may be the problem

Use exact `file.py::Qualified.name` targets whenever possible.
Prefer `--format json` if you will reason over the result programmatically.
Do not use `pycfg-rs` for call-graph or whole-program dependency questions.
```
