use ruff_python_ast::{self as ast, Stmt};
use ruff_python_parser::{Mode, ParseOptions};
use ruff_text_size::Ranged;
use serde::Serialize;
use std::fmt;

// ---------------------------------------------------------------------------
// Core data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub target: usize,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Statement {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BasicBlock {
    pub id: usize,
    pub label: String,
    pub statements: Vec<Statement>,
    pub successors: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub blocks: usize,
    pub edges: usize,
    pub branches: usize,
    pub cyclomatic_complexity: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionCfg {
    pub name: String,
    pub line: usize,
    pub blocks: Vec<BasicBlock>,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCfg {
    pub file: String,
    pub functions: Vec<FunctionCfg>,
}

impl Metrics {
    fn compute(blocks: &[BasicBlock]) -> Self {
        let num_blocks = blocks.len();
        let num_edges: usize = blocks.iter().map(|b| b.successors.len()).sum();
        let branches = blocks.iter().filter(|b| b.successors.len() > 1).count();
        let cyclomatic = if num_blocks == 0 {
            1
        } else {
            num_edges.saturating_sub(num_blocks) + 2
        };
        Metrics {
            blocks: num_blocks,
            edges: num_edges,
            branches,
            cyclomatic_complexity: cyclomatic,
        }
    }
}

impl fmt::Display for FunctionCfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "def {}:", self.name)?;
        writeln!(f)?;
        for block in &self.blocks {
            if block.label == "entry" || block.label == "exit" {
                write!(f, "  Block {} ({}):", block.id, block.label)?;
            } else {
                write!(f, "  Block {}:", block.id)?;
            }
            writeln!(f)?;
            for stmt in &block.statements {
                writeln!(f, "    [L{}] {}", stmt.line, stmt.text)?;
            }
            for edge in &block.successors {
                writeln!(f, "    -> Block {} [{}]", edge.target, edge.label)?;
            }
            writeln!(f)?;
        }
        writeln!(
            f,
            "  # blocks={} edges={} branches={} cyclomatic_complexity={}",
            self.metrics.blocks, self.metrics.edges, self.metrics.branches, self.metrics.cyclomatic_complexity
        )
    }
}

// ---------------------------------------------------------------------------
// CFG Builder
// ---------------------------------------------------------------------------

struct CfgBuilder<'src> {
    source: &'src str,
    blocks: Vec<BasicBlock>,
    loop_stack: Vec<(usize, usize)>,
    except_stack: Vec<Vec<usize>>,
    explicit_exceptions: bool,
}

impl<'src> CfgBuilder<'src> {
    fn new(source: &'src str, explicit_exceptions: bool) -> Self {
        CfgBuilder {
            source,
            blocks: Vec::new(),
            loop_stack: Vec::new(),
            except_stack: Vec::new(),
            explicit_exceptions,
        }
    }

    fn new_block(&mut self, label: &str) -> usize {
        let id = self.blocks.len();
        self.blocks.push(BasicBlock {
            id,
            label: label.to_string(),
            statements: Vec::new(),
            successors: Vec::new(),
        });
        id
    }

    fn add_edge(&mut self, from: usize, to: usize, label: &str) {
        if self.blocks[from]
            .successors
            .iter()
            .any(|e| e.target == to && e.label == label)
        {
            return;
        }
        self.blocks[from].successors.push(Edge {
            target: to,
            label: label.to_string(),
        });
    }

    fn add_stmt(&mut self, block: usize, line: usize, text: &str) {
        self.blocks[block].statements.push(Statement {
            line,
            text: text.to_string(),
        });
    }

    fn offset_to_line(&self, offset: ruff_text_size::TextSize) -> usize {
        self.source[..offset.to_usize()].lines().count().max(1)
    }

    fn range_text(&self, range: ruff_text_size::TextRange) -> String {
        let text = &self.source[range.start().to_usize()..range.end().to_usize()];
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn build_stmts(&mut self, stmts: &[Stmt], mut current: usize, exit: usize) -> Option<usize> {
        for stmt in stmts {
            match self.build_stmt(stmt, current, exit) {
                Some(next) => current = next,
                None => return None,
            }
        }
        Some(current)
    }

    fn build_stmt(&mut self, stmt: &Stmt, current: usize, exit: usize) -> Option<usize> {
        match stmt {
            Stmt::If(if_stmt) => self.build_if(if_stmt, current, exit),
            Stmt::For(for_stmt) => self.build_for(for_stmt, current, exit),
            Stmt::While(while_stmt) => self.build_while(while_stmt, current, exit),
            Stmt::Return(ret_stmt) => self.build_return(ret_stmt, current, exit),
            Stmt::Break(_) => {
                let line = self.offset_to_line(stmt.range().start());
                self.add_stmt(current, line, "break");
                if let Some(&(_, loop_exit)) = self.loop_stack.last() {
                    self.add_edge(current, loop_exit, "break");
                }
                None
            }
            Stmt::Continue(_) => {
                let line = self.offset_to_line(stmt.range().start());
                self.add_stmt(current, line, "continue");
                if let Some(&(loop_header, _)) = self.loop_stack.last() {
                    self.add_edge(current, loop_header, "continue");
                }
                None
            }
            Stmt::Try(try_stmt) => self.build_try(try_stmt, current, exit),
            Stmt::With(with_stmt) => self.build_with(with_stmt, current, exit),
            Stmt::Match(match_stmt) => self.build_match(match_stmt, current, exit),
            Stmt::Raise(raise_stmt) => self.build_raise(raise_stmt, current, exit),
            Stmt::Assert(assert_stmt) => self.build_assert(assert_stmt, current, exit),
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                let line = self.offset_to_line(stmt.range().start());
                let text = self.range_text(stmt.range());
                self.add_stmt(current, line, &text);
                Some(current)
            }
            _ => {
                let line = self.offset_to_line(stmt.range().start());
                let text = self.range_text(stmt.range());
                self.add_stmt(current, line, &text);
                if self.explicit_exceptions {
                    let handlers: Vec<usize> = self
                        .except_stack
                        .last()
                        .cloned()
                        .unwrap_or_default();
                    for handler_block in handlers {
                        self.add_edge(current, handler_block, "exception");
                    }
                }
                Some(current)
            }
        }
    }

    fn build_if(&mut self, if_stmt: &ast::StmtIf, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(if_stmt.range().start());
        let test_text = format!("if {}:", self.range_text(if_stmt.test.range()));
        self.add_stmt(current, line, &test_text);

        let true_block = self.new_block("body");
        let merge_block = self.new_block("body");

        self.add_edge(current, true_block, "True");

        let true_end = self.build_stmts(&if_stmt.body, true_block, exit);
        if let Some(te) = true_end {
            self.add_edge(te, merge_block, "fallthrough");
        }

        let mut prev_false_from = current;
        for clause in &if_stmt.elif_else_clauses {
            if let Some(ref test) = clause.test {
                let elif_test_block = self.new_block("body");
                self.add_edge(prev_false_from, elif_test_block, "False");

                let elif_line = self.offset_to_line(clause.range().start());
                let elif_text = format!("elif {}:", self.range_text(test.range()));
                self.add_stmt(elif_test_block, elif_line, &elif_text);

                let elif_body_block = self.new_block("body");
                self.add_edge(elif_test_block, elif_body_block, "True");

                let elif_end = self.build_stmts(&clause.body, elif_body_block, exit);
                if let Some(ee) = elif_end {
                    self.add_edge(ee, merge_block, "fallthrough");
                }

                prev_false_from = elif_test_block;
            } else {
                let else_block = self.new_block("body");
                self.add_edge(prev_false_from, else_block, "False");

                let else_end = self.build_stmts(&clause.body, else_block, exit);
                if let Some(ee) = else_end {
                    self.add_edge(ee, merge_block, "fallthrough");
                }
                prev_false_from = usize::MAX;
            }
        }

        if prev_false_from != usize::MAX {
            self.add_edge(prev_false_from, merge_block, "False");
        }

        Some(merge_block)
    }

    fn build_for(&mut self, for_stmt: &ast::StmtFor, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(for_stmt.range().start());
        let prefix = if for_stmt.is_async { "async for" } else { "for" };
        let iter_text = format!(
            "{} {} in {}:",
            prefix,
            self.range_text(for_stmt.target.range()),
            self.range_text(for_stmt.iter.range())
        );
        self.add_stmt(current, line, &iter_text);

        let header = current;
        let body_block = self.new_block("body");
        let exit_block = self.new_block("body");

        self.add_edge(header, body_block, "loop-body");
        self.add_edge(header, exit_block, "loop-exit");

        self.loop_stack.push((header, exit_block));
        let body_end = self.build_stmts(&for_stmt.body, body_block, exit);
        self.loop_stack.pop();

        if let Some(be) = body_end {
            self.add_edge(be, header, "loop-back");
        }

        if !for_stmt.orelse.is_empty() {
            let else_block = self.new_block("body");
            self.blocks[header]
                .successors
                .retain(|e| !(e.target == exit_block && e.label == "loop-exit"));
            self.add_edge(header, else_block, "loop-exit");

            let else_end = self.build_stmts(&for_stmt.orelse, else_block, exit);
            if let Some(ee) = else_end {
                self.add_edge(ee, exit_block, "fallthrough");
            }
        }

        Some(exit_block)
    }

    fn build_while(&mut self, while_stmt: &ast::StmtWhile, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(while_stmt.range().start());
        let test_text = format!("while {}:", self.range_text(while_stmt.test.range()));
        self.add_stmt(current, line, &test_text);

        let header = current;
        let body_block = self.new_block("body");
        let exit_block = self.new_block("body");

        self.add_edge(header, body_block, "True");
        self.add_edge(header, exit_block, "False");

        self.loop_stack.push((header, exit_block));
        let body_end = self.build_stmts(&while_stmt.body, body_block, exit);
        self.loop_stack.pop();

        if let Some(be) = body_end {
            self.add_edge(be, header, "loop-back");
        }

        if !while_stmt.orelse.is_empty() {
            let else_block = self.new_block("body");
            self.blocks[header]
                .successors
                .retain(|e| !(e.target == exit_block && e.label == "False"));
            self.add_edge(header, else_block, "False");

            let else_end = self.build_stmts(&while_stmt.orelse, else_block, exit);
            if let Some(ee) = else_end {
                self.add_edge(ee, exit_block, "fallthrough");
            }
        }

        Some(exit_block)
    }

    fn build_return(&mut self, ret_stmt: &ast::StmtReturn, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(ret_stmt.range().start());
        let text = if let Some(ref value) = ret_stmt.value {
            format!("return {}", self.range_text(value.range()))
        } else {
            "return".to_string()
        };
        self.add_stmt(current, line, &text);
        self.add_edge(current, exit, "return");
        None
    }

    fn build_try(&mut self, try_stmt: &ast::StmtTry, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(try_stmt.range().start());
        self.add_stmt(current, line, "try:");

        let merge_block = self.new_block("body");

        let mut handler_blocks = Vec::new();
        for handler in &try_stmt.handlers {
            let handler_block = self.new_block("body");
            let ast::ExceptHandler::ExceptHandler(h) = handler;
            let handler_line = self.offset_to_line(h.range().start());
            let handler_text = if let Some(ref ty) = h.type_ {
                let ty_text = self.range_text(ast::Expr::range(ty));
                if let Some(ref name) = h.name {
                    format!("except {} as {}:", ty_text, name)
                } else {
                    format!("except {}:", ty_text)
                }
            } else {
                "except:".to_string()
            };
            self.add_stmt(handler_block, handler_line, &handler_text);
            handler_blocks.push(handler_block);
        }

        if self.explicit_exceptions {
            let exc_targets = if handler_blocks.is_empty() {
                vec![exit]
            } else {
                handler_blocks.clone()
            };
            self.except_stack.push(exc_targets);
        }

        let try_body_block = self.new_block("body");
        self.add_edge(current, try_body_block, "try");
        let try_end = self.build_stmts(&try_stmt.body, try_body_block, exit);

        if self.explicit_exceptions {
            self.except_stack.pop();
        }

        if !self.explicit_exceptions {
            for &handler_block in &handler_blocks {
                self.add_edge(try_body_block, handler_block, "exception");
            }
        }

        if !try_stmt.orelse.is_empty() {
            let else_block = self.new_block("body");
            let else_line = self.offset_to_line(try_stmt.orelse[0].range().start());
            self.add_stmt(else_block, else_line.saturating_sub(1).max(1), "else:");
            if let Some(te) = try_end {
                self.add_edge(te, else_block, "try-else");
            }
            let else_end = self.build_stmts(&try_stmt.orelse, else_block, exit);
            if let Some(ee) = else_end {
                self.add_edge(ee, merge_block, "fallthrough");
            }
        } else if let Some(te) = try_end {
            self.add_edge(te, merge_block, "fallthrough");
        }

        for (i, handler) in try_stmt.handlers.iter().enumerate() {
            let ast::ExceptHandler::ExceptHandler(h) = handler;
            let handler_block = handler_blocks[i];
            let handler_end = self.build_stmts(&h.body, handler_block, exit);
            if let Some(he) = handler_end {
                self.add_edge(he, merge_block, "fallthrough");
            }
        }

        if !try_stmt.finalbody.is_empty() {
            let finally_block = self.new_block("body");
            let finally_line = self.offset_to_line(try_stmt.finalbody[0].range().start());
            self.add_stmt(finally_block, finally_line.saturating_sub(1).max(1), "finally:");

            let new_merge = self.new_block("body");
            self.add_edge(merge_block, finally_block, "finally");
            let finally_end = self.build_stmts(&try_stmt.finalbody, finally_block, exit);
            if let Some(fe) = finally_end {
                self.add_edge(fe, new_merge, "fallthrough");
            }
            Some(new_merge)
        } else {
            Some(merge_block)
        }
    }

    fn build_with(&mut self, with_stmt: &ast::StmtWith, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(with_stmt.range().start());
        let items_text: Vec<String> = with_stmt
            .items
            .iter()
            .map(|item| {
                let ctx = self.range_text(item.context_expr.range());
                if let Some(ref var) = item.optional_vars {
                    format!("{} as {}", ctx, self.range_text(var.range()))
                } else {
                    ctx
                }
            })
            .collect();
        let prefix = if with_stmt.is_async { "async with" } else { "with" };
        let text = format!("{} {}:", prefix, items_text.join(", "));
        self.add_stmt(current, line, &text);

        self.build_stmts(&with_stmt.body, current, exit)
    }

    fn build_match(&mut self, match_stmt: &ast::StmtMatch, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(match_stmt.range().start());
        let subject_text = format!("match {}:", self.range_text(match_stmt.subject.range()));
        self.add_stmt(current, line, &subject_text);

        let merge_block = self.new_block("body");

        for case in &match_stmt.cases {
            let case_block = self.new_block("body");
            let pattern_text = self.range_text(case.pattern.range());
            let label = format!("case {}", pattern_text);
            self.add_edge(current, case_block, &label);

            let case_end = self.build_stmts(&case.body, case_block, exit);
            if let Some(ce) = case_end {
                self.add_edge(ce, merge_block, "fallthrough");
            }
        }

        Some(merge_block)
    }

    fn build_raise(&mut self, raise_stmt: &ast::StmtRaise, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(raise_stmt.range().start());
        let text = if let Some(ref exc) = raise_stmt.exc {
            format!("raise {}", self.range_text(exc.range()))
        } else {
            "raise".to_string()
        };
        self.add_stmt(current, line, &text);

        let handlers: Vec<usize> = self.except_stack.last().cloned().unwrap_or_default();
        if handlers.is_empty() {
            self.add_edge(current, exit, "raise");
        } else {
            for handler_block in handlers {
                self.add_edge(current, handler_block, "raise");
            }
        }
        None
    }

    fn build_assert(&mut self, assert_stmt: &ast::StmtAssert, current: usize, exit: usize) -> Option<usize> {
        let line = self.offset_to_line(assert_stmt.range().start());
        let text = format!("assert {}", self.range_text(assert_stmt.test.range()));
        self.add_stmt(current, line, &text);

        let handlers: Vec<usize> = self.except_stack.last().cloned().unwrap_or_default();
        if handlers.is_empty() {
            self.add_edge(current, exit, "assert-fail");
        } else {
            for handler_block in handlers {
                self.add_edge(current, handler_block, "assert-fail");
            }
        }
        Some(current)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct CfgOptions {
    pub explicit_exceptions: bool,
}

pub fn build_cfgs(source: &str, filename: &str, options: &CfgOptions) -> FileCfg {
    let parsed = ruff_python_parser::parse_unchecked(source, ParseOptions::from(Mode::Module));
    let module = parsed.into_syntax();

    let stmts = match module {
        ast::Mod::Module(m) => m.body,
        ast::Mod::Expression(e) => {
            vec![Stmt::Expr(ast::StmtExpr {
                node_index: Default::default(),
                value: e.body,
                range: e.range,
            })]
        }
    };

    let mut functions = Vec::new();
    collect_functions(source, &stmts, &mut functions, options);

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

    FileCfg {
        file: filename.to_string(),
        functions,
    }
}

pub fn build_cfg_for_function(
    source: &str,
    filename: &str,
    function_name: &str,
    options: &CfgOptions,
) -> Option<FileCfg> {
    let parsed = ruff_python_parser::parse_unchecked(source, ParseOptions::from(Mode::Module));
    let module = parsed.into_syntax();

    let stmts = match module {
        ast::Mod::Module(m) => m.body,
        ast::Mod::Expression(_) => return None,
    };

    let mut result = Vec::new();
    find_function(source, &stmts, function_name, &mut result, options, "");

    if result.is_empty() {
        None
    } else {
        Some(FileCfg {
            file: filename.to_string(),
            functions: result,
        })
    }
}

fn find_function(
    source: &str,
    stmts: &[Stmt],
    target: &str,
    result: &mut Vec<FunctionCfg>,
    options: &CfgOptions,
    prefix: &str,
) {
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let qualified = if prefix.is_empty() {
                    func_def.name.to_string()
                } else {
                    format!("{}.{}", prefix, func_def.name)
                };
                if qualified == target || func_def.name.as_str() == target {
                    let line = source[..func_def.range().start().to_usize()]
                        .lines()
                        .count()
                        .max(1);
                    let cfg = build_single_cfg(source, &qualified, line, &func_def.body, options);
                    result.push(cfg);
                }
                find_function(source, &func_def.body, target, result, options, &qualified);
            }
            Stmt::ClassDef(class_def) => {
                let class_prefix = if prefix.is_empty() {
                    class_def.name.to_string()
                } else {
                    format!("{}.{}", prefix, class_def.name)
                };
                find_function(source, &class_def.body, target, result, options, &class_prefix);
            }
            _ => {}
        }
    }
}

fn collect_functions(source: &str, stmts: &[Stmt], functions: &mut Vec<FunctionCfg>, options: &CfgOptions) {
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let line = source[..func_def.range().start().to_usize()]
                    .lines()
                    .count()
                    .max(1);
                let name = func_def.name.to_string();
                let cfg = build_single_cfg(source, &name, line, &func_def.body, options);
                functions.push(cfg);
                collect_nested_functions(source, &func_def.body, functions, options, &name);
            }
            Stmt::ClassDef(class_def) => {
                let class_name = class_def.name.to_string();
                collect_class_methods(source, &class_def.body, functions, options, &class_name);
            }
            _ => {}
        }
    }
}

fn collect_class_methods(
    source: &str,
    stmts: &[Stmt],
    functions: &mut Vec<FunctionCfg>,
    options: &CfgOptions,
    class_name: &str,
) {
    for stmt in stmts {
        if let Stmt::FunctionDef(func_def) = stmt {
            let line = source[..func_def.range().start().to_usize()]
                .lines()
                .count()
                .max(1);
            let qualified = format!("{}.{}", class_name, func_def.name);
            let cfg = build_single_cfg(source, &qualified, line, &func_def.body, options);
            functions.push(cfg);
            collect_nested_functions(source, &func_def.body, functions, options, &qualified);
        }
    }
}

fn collect_nested_functions(
    source: &str,
    stmts: &[Stmt],
    functions: &mut Vec<FunctionCfg>,
    options: &CfgOptions,
    prefix: &str,
) {
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let line = source[..func_def.range().start().to_usize()]
                    .lines()
                    .count()
                    .max(1);
                let qualified = format!("{}.{}", prefix, func_def.name);
                let cfg = build_single_cfg(source, &qualified, line, &func_def.body, options);
                functions.push(cfg);
                collect_nested_functions(source, &func_def.body, functions, options, &qualified);
            }
            Stmt::ClassDef(class_def) => {
                let class_qualified = format!("{}.{}", prefix, class_def.name);
                collect_class_methods(source, &class_def.body, functions, options, &class_qualified);
            }
            _ => {}
        }
    }
}

fn build_single_cfg(source: &str, name: &str, line: usize, body: &[Stmt], options: &CfgOptions) -> FunctionCfg {
    let mut builder = CfgBuilder::new(source, options.explicit_exceptions);

    let entry = builder.new_block("entry");
    let exit = builder.new_block("exit");

    let last = builder.build_stmts(body, entry, exit);
    if let Some(last_block) = last {
        builder.add_edge(last_block, exit, "fallthrough");
    }

    let blocks = builder.blocks;
    let metrics = Metrics::compute(&blocks);

    FunctionCfg {
        name: name.to_string(),
        line,
        blocks,
        metrics,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function() {
        let source = "def foo():\n    x = 1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert_eq!(result.functions.len(), 1);
        let func = &result.functions[0];
        assert_eq!(func.name, "foo");
        assert!(func.blocks.len() >= 2);
    }

    #[test]
    fn test_if_else() {
        let source = "def foo(x):\n    if x > 0:\n        y = 1\n    else:\n        y = 2\n    return y\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.metrics.branches > 0);
        assert!(func.metrics.cyclomatic_complexity >= 2);
    }

    #[test]
    fn test_while_loop() {
        let source = "def foo():\n    x = 0\n    while x < 10:\n        x += 1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.metrics.branches > 0);
    }

    #[test]
    fn test_for_loop() {
        let source = "def foo():\n    total = 0\n    for i in range(10):\n        total += i\n    return total\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.blocks.len() >= 3);
    }

    #[test]
    fn test_break_continue() {
        let source =
            "def foo():\n    for i in range(10):\n        if i == 5:\n            break\n        if i % 2 == 0:\n            continue\n        print(i)\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.metrics.branches >= 2);
    }

    #[test]
    fn test_return_mid_function() {
        let source = "def foo(x):\n    if x < 0:\n        return -1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let return_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.target == exit_id && e.label == "return")
            .count();
        assert!(return_edges >= 1);
    }

    #[test]
    fn test_nested_control_flow() {
        let source =
            "def foo(x, y):\n    if x > 0:\n        for i in range(y):\n            if i > x:\n                break\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.metrics.cyclomatic_complexity >= 3);
    }

    #[test]
    fn test_empty_function() {
        let source = "def foo():\n    pass\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.blocks.len() >= 2);
    }

    #[test]
    fn test_function_targeting() {
        let source = "def foo():\n    return 1\n\ndef bar():\n    return 2\n";
        let result = build_cfg_for_function(source, "test.py", "bar", &CfgOptions::default());
        assert!(result.is_some());
        let file_cfg = result.unwrap();
        assert_eq!(file_cfg.functions.len(), 1);
        assert_eq!(file_cfg.functions[0].name, "bar");
    }

    #[test]
    fn test_class_method() {
        let source = "class MyClass:\n    def my_method(self):\n        return self.x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(result.functions.iter().any(|f| f.name == "MyClass.my_method"));
    }

    #[test]
    fn test_try_except() {
        let source =
            "def foo():\n    try:\n        x = risky()\n    except ValueError:\n        x = 0\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_exception_edge = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "exception");
        assert!(has_exception_edge);
    }

    #[test]
    fn test_match_case() {
        let source = "def foo(cmd):\n    match cmd:\n        case \"start\":\n            run()\n        case \"stop\":\n            halt()\n        case _:\n            pass\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let case_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label.starts_with("case "))
            .count();
        assert!(case_edges >= 2);
    }

    #[test]
    fn test_text_output() {
        let source = "def foo(x):\n    if x > 0:\n        return 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let output = func.to_string();
        assert!(output.contains("Block 0 (entry):"));
        assert!(output.contains("[L"));
        assert!(output.contains("-> Block"));
    }

    #[test]
    fn test_metrics() {
        let source = "def foo(x):\n    if x > 0:\n        return 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.metrics.blocks >= 2);
        assert!(func.metrics.edges >= 1);
        assert!(func.metrics.cyclomatic_complexity >= 2);
    }

    #[test]
    fn test_explicit_exceptions() {
        // Use control flow in the try body to create multiple blocks
        let source = "def foo():\n    try:\n        a = 1\n        if a:\n            b = 2\n    except ValueError:\n        c = 3\n";
        let opts = CfgOptions {
            explicit_exceptions: true,
        };
        let result = build_cfgs(source, "test.py", &opts);
        let func = &result.functions[0];
        let exception_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label == "exception")
            .count();
        // Multiple blocks inside try, each should have exception edges
        assert!(exception_edges >= 2, "expected >= 2 exception edges, got {}", exception_edges);
    }

    #[test]
    fn test_raise() {
        let source = "def foo():\n    raise ValueError(\"bad\")\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_raise = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "raise");
        assert!(has_raise);
    }

    #[test]
    fn test_assert() {
        let source = "def foo(x):\n    assert x > 0\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_assert_fail = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "assert-fail");
        assert!(has_assert_fail);
    }

    #[test]
    fn test_json_output() {
        let source = "def foo(x):\n    return x + 1\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let json = serde_json::to_string(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["functions"].is_array());
    }

    #[test]
    fn test_nested_function() {
        let source = "def outer():\n    def inner():\n        return 1\n    return inner()\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(result.functions.iter().any(|f| f.name == "outer"));
        assert!(result.functions.iter().any(|f| f.name == "outer.inner"));
    }

    #[test]
    fn test_with_statement() {
        let source = "def foo():\n    with open('f') as f:\n        data = f.read()\n    return data\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_with = func
            .blocks
            .iter()
            .any(|b| b.statements.iter().any(|s| s.text.starts_with("with ")));
        assert!(has_with);
    }
}
