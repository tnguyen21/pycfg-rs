//! Fast, Rust-based control flow graph generator for Python.
//!
//! Parses Python source and produces intra-procedural control flow graphs
//! for each function, without requiring a Python runtime.
//!
//! Uses [`ruff_python_parser`] for parsing — the same parser used by the Ruff linter.

pub mod cfg;
pub mod writer;
