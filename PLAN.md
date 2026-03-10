# Plan

Date: 2026-03-10

This file is temporary and exists to track the current execution sequence.

## Goals

- Kill the remaining surviving mutants with focused tests.
- Reduce the refactor risk around `src/cfg.rs`.
- Follow through on the structural cleanup from `REPO_STATUS.md`.

## Sequence

1. Add narrow regression tests for the known survivors in `src/main.rs`.
   - Lock down verbosity mapping.
   - Lock down broken-pipe handling.
2. Add narrow regression tests for the known survivors in `src/cfg.rs`.
   - `for ... else` retain logic.
   - `while ... else` retain logic.
   - `build_try` finalbody stack handling.
3. Split `src/cfg.rs` into smaller modules without changing behavior.
4. Split `tests/integration.rs` by concern.
5. Replace stringly-typed block and edge semantics with enums while keeping output stable.
6. Tighten frontend contracts.
   - Exact function targeting.
   - Non-`.py` rejection policy.
   - Better parse diagnostics.
7. Update CI and docs.
   - `cargo fmt --check`
   - Release smoke coverage
   - Mutation-regression coverage
   - Reconcile `SPEC.md` with actual CLI behavior

## Acceptance Checks

- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
