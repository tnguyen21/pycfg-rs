# Roadmap

## Intent

This roadmap assumes `pycfg-rs` is primarily a CLI-first CFG analyzer used as a
small static-analysis primitive in broader engineering workflows.

The project should not optimize for feature breadth by default. It should optimize for a narrower claim:

- Fast intra-procedural control-flow analysis for Python.
- Better reliability and coverage than stale alternatives on realistic code.
- Stable enough output contracts that humans, scripts, and agents can depend on it.

## Current Phase

`pycfg-rs` is past the "prove the core idea" stage and is close to a true maintenance-first posture.

It already has:

- A working CFG analyzer with support for the major control-flow constructs this project cares about.
- Text, JSON, and DOT outputs.
- Directory traversal, exact function targeting, and parse-error skipping.
- Narrow query-oriented modes for function discovery, summaries, and diagnostics.
- CI, corpus smoke tests, and focused mutation-testing discipline.
- Golden tests for stable CLI output contracts.

That means the default posture should now be:

- Maintenance for the existing analyzer and output contracts.
- Small, user-driven improvements when they clearly raise usefulness.
- Minimal appetite for broad new feature families.

## Priority Order

## 1. Correctness and Regression Resistance

This remains the most important area because trust in CFG structure is the entire value of the tool.

Recommended work:

- Add targeted regression tests when real bugs are found.
- Keep mutation testing focused on touched files.
- Strengthen tricky-language fixtures only where the current suite is weak.
- Preserve exact-output goldens for text, JSON, and DOT when contracts change.

Questions to answer:

- On which classes of Python control flow does `pycfg-rs` clearly outperform alternatives?
- What simplifications are still present and should stay documented?
- Which regressions are most likely when editing loop or `try`/`finally` routing?

## 2. Output Contract Stability

The public product is not just "the code works." The public product is the set of output contracts other tooling can safely consume.

Recommended work:

- Keep JSON structure stable unless a clear downstream need justifies change.
- Review golden diffs carefully for text, JSON, and DOT.
- Document contract changes when they are intentional.
- Prefer additive evolution over churn in field names or edge labels.

## 3. Performance on Real Workloads

Performance matters because this tool is useful precisely when it is cheap to invoke repeatedly.

Recommended work:

- Profile only when a real workload feels slow.
- Preserve fast single-file and function-targeted workflows.
- Re-run benchmarks on representative corpora before claiming performance wins.
- Avoid speculative optimization without measurement.

The target is not just a good benchmark table. The target is a tool that feels cheap enough to call constantly.

## 4. Documentation and Operational Clarity

Maintenance mode still requires honest docs.

Recommended work:

- Keep README and docs aligned with real CLI behavior.
- Document known modeling simplifications rather than hiding them.
- Make the maintenance workflow clear: standard checks, targeted mutation testing, and golden updates.
- Keep examples anchored in current output, not aspirational designs.

## 5. Small Structural Cleanup Only When It Pays Off

Refactoring still matters, but only when it reduces maintenance cost.

Recommended work:

- Keep oversized files from regrowing without a reason.
- Prefer file-level cleanup over major architecture changes.
- Consider crate or workspace splits only if a boundary becomes genuinely reusable or independently complex.

## Things Not Worth Heavy Investment Right Now

- A large query-command surface modeled after call-graph tools.
- Broad library API design beyond what current callers need.
- Many new export formats.
- Heavy visual presentation work.
- Premature decomposition into many crates.

## Suggested Allocation

For future work, a reasonable effort split is:

- 45% correctness and regression resistance.
- 25% output contract stability and tests.
- 20% measured performance work.
- 10% docs and small maintenance refactors.

## Exit Criteria for Full Maintenance Mode

The project can be treated as fully in maintenance mode once these conditions stay true over time:

- The current feature set is sufficient for the real workflows that use it.
- Output contracts are stable and guarded by exact tests.
- Mutation testing on touched surfaces stays clean.
- Benchmarks remain competitive enough for the intended workloads.
- New work is mostly bug fixing, dependency updates, and narrow user-driven improvements.

At that point, the default answer to "what should we build next?" should be "nothing broad unless a real workflow demands it."
