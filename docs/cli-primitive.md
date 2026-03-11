# pycfg-rs as a CLI Primitive

## Positioning

`pycfg-rs` should be treated as a focused CFG-analysis primitive inside a
larger engineering workflow, not as a broad end-user application and not as a
general static-analysis platform.

That framing changes what matters:

- Fast enough to invoke repeatedly during code reading and refactoring.
- Deterministic enough that scripts, tools, and agents can trust the output.
- Explicit about parse failures and partial coverage.
- Useful in both machine-readable and inspection-friendly forms.
- Narrow in scope: intra-procedural control flow, not every possible graph or query abstraction.

From this point of view, the job of `pycfg-rs` is not just "serialize a CFG."
The job is to provide a stable control-flow primitive that helps humans and
LLMs answer concrete questions about function structure quickly.

## Product Boundary

The project should be optimized as:

- A CLI-first analysis tool.
- Personal and team infrastructure for code understanding.
- A dependable primitive for downstream tooling that needs function-level control-flow structure.

It should not be optimized first for:

- Broad library ergonomics.
- Large UI or visualization layers.
- Expanding into unrelated analyses.
- Premature workspace or crate decomposition while the codebase remains small.

## What "Good" Looks Like

If the tool is succeeding in its intended role, a higher-level system should be
able to call it and reliably answer questions like:

- What are the blocks and branch edges inside this function?
- Where are the exits, loop back-edges, and exceptional paths?
- Does this edit materially change the function's structure?
- Which block should I inspect next when debugging behavior?
- Is this parseable and analyzable, or should I fall back to raw source?

That leads to five practical requirements.

## 1. Stable Machine Interface

The JSON report should be treated as the primary product contract.

Important properties:

- Stable `files -> functions -> blocks -> successors` structure.
- Stable field names and edge labels.
- Deterministic ordering where practical.
- Well-defined block and edge semantics.
- Predictable handling of parse failures and skipped files.

The text and DOT outputs matter, but they are primarily inspection surfaces.
The strongest downstream leverage comes from a stable machine-readable report.

## 2. Useful Narrow Scoping

`pycfg-rs` becomes much more useful when callers can cheaply constrain work.

Today that mostly means:

- Exact `file.py::FunctionName` targeting.
- Directory traversal over Python files.
- Whole-file analysis when function targeting is not needed.
- Optional `--explicit-exceptions` when a caller wants more detailed
  exceptional edges.

If the CLI grows, it should keep favoring narrow, cheap scopes over broad new
subcommands.

## 3. Explainable Structure

Control-flow output is only useful if a downstream consumer can relate it back
to source.

Results should keep carrying:

- Fully qualified function names.
- File paths.
- Line numbers on statements.
- Explicit edge labels such as `True`, `False`, `return`, `loop-back`,
  `finally`, and `case ...`.

That is usually enough provenance for a downstream tool to answer "why does
this edge exist?" by inspecting the corresponding block text and source line.

## 4. Honest Failure and Partial Analysis

Python parsing and CFG construction will always have limits. The tool should be
explicit about them.

Important behaviors:

- Warn and skip unparseable files instead of silently pretending analysis succeeded.
- Preserve exact function-targeting rules instead of guessing by leaf name.
- Keep unsupported or flattened constructs documented, especially around async, comprehensions, and exception modeling.

This improves downstream trust much more than pretending the analysis is more complete than it really is.

## 5. Output Contracts that Age Well

This repo already has three public surfaces:

- Text for quick reading.
- JSON for downstream tools.
- DOT for graph rendering and visual inspection.

Those contracts should stay stable unless there is a clear user-facing reason to change them.
When they do change, the change should be intentional and reviewable, not accidental.

That is why exact golden tests and focused mutation testing are a good fit for this project.

## Strategic Priorities

Given the intended role of `pycfg-rs`, the main investment areas should be:

1. Correctness on realistic Python control-flow constructs.
2. Stable output contracts.
3. Speed on medium and large repositories.
4. Clear, honest documentation of limits and behavior.

## What Not to Overinvest In

These may matter later, but they should not drive the roadmap now:

- A large query-command surface modeled after call-graph tools.
- Many extra export formats.
- Heavy visualization work.
- Broad library abstraction layers.
- Structural refactors without a concrete maintenance payoff.

## Working Definition of Done

The project is "done enough" for its intended role when it is:

- Fast enough to call repeatedly during code understanding.
- Accurate enough on the Python constructs it claims to support.
- Stable enough that downstream tooling can trust its JSON contract.
- Honest enough about skipped files and simplified modeling.
- Small and boring enough to maintain without continuous architectural churn.

At that point, `pycfg-rs` should live in maintenance-first mode with targeted improvements rather than open-ended feature expansion.
