use ruff_python_ast::{self as ast, Stmt};
use ruff_python_parser::{Mode, ParseOptions};
use serde::Serialize;
use std::error::Error;
use std::fmt;

mod builder;
mod model;
mod source_map;
mod symbols;

pub use model::{BasicBlock, BlockKind, Edge, EdgeKind, FileCfg, FunctionCfg, Metrics, Statement};

use builder::build_single_cfg;
use symbols::visit_functions;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct CfgOptions {
    pub explicit_exceptions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionInfo {
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    diagnostics: Vec<String>,
}

impl ParseError {
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostics.join(" | "))
    }
}

impl Error for ParseError {}

pub fn parse_diagnostics(source: &str) -> Vec<String> {
    match parse_module_stmts(source) {
        Ok(_) => Vec::new(),
        Err(err) => err.diagnostics,
    }
}

pub fn try_list_functions(source: &str) -> Result<Vec<FunctionInfo>, ParseError> {
    let stmts = parse_module_stmts(source)?;
    let mut functions = Vec::new();
    visit_functions(source, &stmts, &mut |function| {
        functions.push(FunctionInfo {
            name: function.qualified_name,
            line: function.line,
        });
    });
    Ok(functions)
}

pub fn list_functions(source: &str) -> Vec<FunctionInfo> {
    try_list_functions(source).unwrap_or_else(|err| panic!("failed to list functions: {err}"))
}

pub fn try_build_cfgs(
    source: &str,
    filename: &str,
    options: &CfgOptions,
) -> Result<FileCfg, ParseError> {
    let stmts = parse_module_stmts(source)?;

    let mut functions = Vec::new();
    visit_functions(source, &stmts, &mut |function| {
        functions.push(build_single_cfg(
            source,
            &function.qualified_name,
            function.line,
            function.body,
            options,
        ));
    });

    let has_top_level_code = stmts.iter().any(|s| {
        !matches!(
            s,
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) | Stmt::Import(_) | Stmt::ImportFrom(_)
        )
    });

    if has_top_level_code || functions.is_empty() {
        let top_cfg = build_single_cfg(source, "<module>", 1, &stmts, options);
        functions.insert(0, top_cfg);
    }

    Ok(FileCfg {
        file: filename.to_string(),
        functions,
    })
}

pub fn build_cfgs(source: &str, filename: &str, options: &CfgOptions) -> FileCfg {
    try_build_cfgs(source, filename, options)
        .unwrap_or_else(|err| panic!("failed to build CFG for {filename}: {err}"))
}

pub fn try_build_cfg_for_function(
    source: &str,
    filename: &str,
    function_name: &str,
    options: &CfgOptions,
) -> Result<Option<FileCfg>, ParseError> {
    let stmts = parse_module_stmts(source)?;

    let mut functions = Vec::new();
    visit_functions(source, &stmts, &mut |function| {
        if function.qualified_name == function_name {
            functions.push(build_single_cfg(
                source,
                &function.qualified_name,
                function.line,
                function.body,
                options,
            ));
        }
    });

    if functions.is_empty() {
        Ok(None)
    } else {
        Ok(Some(FileCfg {
            file: filename.to_string(),
            functions,
        }))
    }
}

pub fn build_cfg_for_function(
    source: &str,
    filename: &str,
    function_name: &str,
    options: &CfgOptions,
) -> Option<FileCfg> {
    try_build_cfg_for_function(source, filename, function_name, options)
        .unwrap_or_else(|err| panic!("failed to build CFG for {filename}::{function_name}: {err}"))
}

fn parse_module_stmts(source: &str) -> Result<Vec<Stmt>, ParseError> {
    let parsed = ruff_python_parser::parse_unchecked(source, ParseOptions::from(Mode::Module));
    let mut diagnostics: Vec<String> = parsed.errors().iter().map(ToString::to_string).collect();
    diagnostics.extend(
        parsed
            .unsupported_syntax_errors()
            .iter()
            .map(ToString::to_string),
    );
    if !diagnostics.is_empty() {
        return Err(ParseError { diagnostics });
    }

    let module = parsed.into_syntax();
    Ok(match module {
        ast::Mod::Module(m) => m.body,
        ast::Mod::Expression(e) => {
            vec![Stmt::Expr(ast::StmtExpr {
                node_index: Default::default(),
                value: e.body,
                range: e.range,
            })]
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
