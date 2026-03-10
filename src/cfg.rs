use ruff_python_ast::{self as ast, Stmt};
use ruff_python_parser::{Mode, ParseOptions};
use ruff_text_size::{Ranged, TextSize};
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
            // E - N + 2 (McCabe formula). Use signed arithmetic to handle E < N.
            (num_edges as isize - num_blocks as isize + 2).max(1) as usize
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
            self.metrics.blocks,
            self.metrics.edges,
            self.metrics.branches,
            self.metrics.cyclomatic_complexity
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
    finally_stack: Vec<FinallyFrame>,
    explicit_exceptions: bool,
}

#[derive(Clone)]
struct PendingEdge {
    target: usize,
    label: &'static str,
}

#[derive(Clone)]
struct FinallyFrame {
    finalbody: Vec<Stmt>,
    local_handler_targets: Vec<usize>,
}

impl<'src> CfgBuilder<'src> {
    fn new(source: &'src str, explicit_exceptions: bool) -> Self {
        CfgBuilder {
            source,
            blocks: Vec::new(),
            loop_stack: Vec::new(),
            except_stack: Vec::new(),
            finally_stack: Vec::new(),
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
                    self.emit_pending_edges(
                        current,
                        &[PendingEdge {
                            target: loop_exit,
                            label: "break",
                        }],
                        exit,
                    );
                }
                None
            }
            Stmt::Continue(_) => {
                let line = self.offset_to_line(stmt.range().start());
                self.add_stmt(current, line, "continue");
                if let Some(&(loop_header, _)) = self.loop_stack.last() {
                    self.emit_pending_edges(
                        current,
                        &[PendingEdge {
                            target: loop_header,
                            label: "continue",
                        }],
                        exit,
                    );
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
                    let handlers: Vec<PendingEdge> = self
                        .except_stack
                        .last()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|target| PendingEdge {
                            target,
                            label: "exception",
                        })
                        .collect();
                    self.emit_pending_edges(current, &handlers, exit);
                }
                Some(current)
            }
        }
    }

    fn add_pending_edges(&mut self, from: usize, pending: &[PendingEdge]) {
        for edge in pending {
            self.add_edge(from, edge.target, edge.label);
        }
    }

    fn pending_edges_are_local_handlers(pending: &[PendingEdge], frame: &FinallyFrame) -> bool {
        !frame.local_handler_targets.is_empty()
            && pending.iter().all(|edge| {
                matches!(edge.label, "exception" | "raise" | "assert-fail")
                    && frame.local_handler_targets.contains(&edge.target)
            })
    }

    fn emit_pending_edges(&mut self, from: usize, pending: &[PendingEdge], exit: usize) {
        if pending.is_empty() {
            return;
        }

        if let Some(frame) = self.finally_stack.pop() {
            if Self::pending_edges_are_local_handlers(pending, &frame) {
                self.finally_stack.push(frame);
                self.add_pending_edges(from, pending);
                return;
            }

            let finally_block = self.new_block("body");
            let finally_line = self.offset_to_line(frame.finalbody[0].range().start());
            self.add_stmt(
                finally_block,
                finally_line.saturating_sub(1).max(1),
                "finally:",
            );
            self.add_edge(from, finally_block, "finally");

            let finally_end = self.build_stmts(&frame.finalbody, finally_block, exit);
            if let Some(fe) = finally_end {
                self.emit_pending_edges(fe, pending, exit);
            }

            self.finally_stack.push(frame);
            return;
        }

        self.add_pending_edges(from, pending);
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
        let prefix = if for_stmt.is_async {
            "async for"
        } else {
            "for"
        };
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

    fn build_while(
        &mut self,
        while_stmt: &ast::StmtWhile,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
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

    fn build_return(
        &mut self,
        ret_stmt: &ast::StmtReturn,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
        let line = self.offset_to_line(ret_stmt.range().start());
        let text = if let Some(ref value) = ret_stmt.value {
            format!("return {}", self.range_text(value.range()))
        } else {
            "return".to_string()
        };
        self.add_stmt(current, line, &text);
        self.emit_pending_edges(
            current,
            &[PendingEdge {
                target: exit,
                label: "return",
            }],
            exit,
        );
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

        let finalbody = try_stmt.finalbody.clone();
        if !finalbody.is_empty() {
            self.finally_stack.push(FinallyFrame {
                finalbody,
                local_handler_targets: handler_blocks.clone(),
            });
        }

        let exc_targets = if handler_blocks.is_empty() {
            self.except_stack
                .last()
                .cloned()
                .unwrap_or_else(|| vec![exit])
        } else {
            handler_blocks.clone()
        };
        self.except_stack.push(exc_targets);

        let try_body_block = self.new_block("body");
        self.add_edge(current, try_body_block, "try");
        let try_end = self.build_stmts(&try_stmt.body, try_body_block, exit);

        self.except_stack.pop();

        if !try_stmt.finalbody.is_empty() {
            self.finally_stack.pop();
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
            self.add_stmt(
                finally_block,
                finally_line.saturating_sub(1).max(1),
                "finally:",
            );

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

    fn build_with(
        &mut self,
        with_stmt: &ast::StmtWith,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
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
        let prefix = if with_stmt.is_async {
            "async with"
        } else {
            "with"
        };
        let text = format!("{} {}:", prefix, items_text.join(", "));
        self.add_stmt(current, line, &text);

        self.build_stmts(&with_stmt.body, current, exit)
    }

    fn build_match(
        &mut self,
        match_stmt: &ast::StmtMatch,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
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

    fn build_raise(
        &mut self,
        raise_stmt: &ast::StmtRaise,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
        let line = self.offset_to_line(raise_stmt.range().start());
        let text = if let Some(ref exc) = raise_stmt.exc {
            format!("raise {}", self.range_text(exc.range()))
        } else {
            "raise".to_string()
        };
        self.add_stmt(current, line, &text);

        let handlers: Vec<PendingEdge> = self
            .except_stack
            .last()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|target| PendingEdge {
                target,
                label: "raise",
            })
            .collect();
        if handlers.is_empty() {
            self.emit_pending_edges(
                current,
                &[PendingEdge {
                    target: exit,
                    label: "raise",
                }],
                exit,
            );
        } else {
            self.emit_pending_edges(current, &handlers, exit);
        }
        None
    }

    fn build_assert(
        &mut self,
        assert_stmt: &ast::StmtAssert,
        current: usize,
        exit: usize,
    ) -> Option<usize> {
        let line = self.offset_to_line(assert_stmt.range().start());
        let text = format!("assert {}", self.range_text(assert_stmt.test.range()));
        self.add_stmt(current, line, &text);

        let handlers: Vec<PendingEdge> = self
            .except_stack
            .last()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|target| PendingEdge {
                target,
                label: "assert-fail",
            })
            .collect();
        if handlers.is_empty() {
            self.emit_pending_edges(
                current,
                &[PendingEdge {
                    target: exit,
                    label: "assert-fail",
                }],
                exit,
            );
        } else {
            self.emit_pending_edges(current, &handlers, exit);
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

struct FunctionVisit<'a> {
    qualified_name: String,
    leaf_name: String,
    line: usize,
    body: &'a [Stmt],
}

fn visit_functions<'a, F>(source: &str, stmts: &'a [Stmt], visit: &mut F)
where
    F: FnMut(FunctionVisit<'a>),
{
    visit_scope(source, stmts, "", visit);
}

fn visit_scope<'a, F>(source: &str, stmts: &'a [Stmt], prefix: &str, visit: &mut F)
where
    F: FnMut(FunctionVisit<'a>),
{
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(func_def) => {
                let qualified_name = qualify_name(prefix, func_def.name.as_str());
                visit(FunctionVisit {
                    qualified_name: qualified_name.clone(),
                    leaf_name: func_def.name.to_string(),
                    line: line_from_offset(source, func_def.range().start()),
                    body: &func_def.body,
                });
                visit_scope(source, &func_def.body, &qualified_name, visit);
            }
            Stmt::ClassDef(class_def) => {
                let class_prefix = qualify_name(prefix, class_def.name.as_str());
                visit_scope(source, &class_def.body, &class_prefix, visit);
            }
            _ => {}
        }
    }
}

fn qualify_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn line_from_offset(source: &str, offset: TextSize) -> usize {
    source[..offset.to_usize()].lines().count().max(1)
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

    let mut functions = Vec::new();
    visit_functions(source, &stmts, &mut |function| {
        if function.qualified_name == function_name || function.leaf_name == function_name {
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
        None
    } else {
        Some(FileCfg {
            file: filename.to_string(),
            functions,
        })
    }
}

fn build_single_cfg(
    source: &str,
    name: &str,
    line: usize,
    body: &[Stmt],
    options: &CfgOptions,
) -> FunctionCfg {
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
        let source =
            "def foo(x):\n    if x > 0:\n        y = 1\n    else:\n        y = 2\n    return y\n";
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
        let source = "def foo():\n    for i in range(10):\n        if i == 5:\n            break\n        if i % 2 == 0:\n            continue\n        print(i)\n";
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
        let source = "def foo(x, y):\n    if x > 0:\n        for i in range(y):\n            if i > x:\n                break\n    return 0\n";
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
        assert!(
            result
                .functions
                .iter()
                .any(|f| f.name == "MyClass.my_method")
        );
    }

    #[test]
    fn test_try_except() {
        let source = "def foo():\n    try:\n        x = risky()\n    except ValueError:\n        x = 0\n    return x\n";
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
        assert!(
            exception_edges >= 2,
            "expected >= 2 exception edges, got {}",
            exception_edges
        );
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
        let source =
            "def foo():\n    with open('f') as f:\n        data = f.read()\n    return data\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_with = func
            .blocks
            .iter()
            .any(|b| b.statements.iter().any(|s| s.text.starts_with("with ")));
        assert!(has_with);
    }

    // -----------------------------------------------------------------------
    // Edge cases from staticfg and python-graphs test suites
    // -----------------------------------------------------------------------

    #[test]
    fn test_straight_line_complexity() {
        let source = "def foo():\n    x = 1\n    y = 2\n    z = x + y\n    return z\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert_eq!(func.metrics.cyclomatic_complexity, 1);
        assert_eq!(func.metrics.branches, 0);
    }

    #[test]
    fn test_for_else() {
        let source = "def foo():\n    for i in range(10):\n        pass\n    else:\n        x = 1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_loop_exit = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "loop-exit");
        assert!(has_loop_exit);
        // else block should exist: loop-exit goes to else, not directly to merge
        assert!(func.blocks.len() >= 4);
    }

    #[test]
    fn test_while_else() {
        let source = "def foo():\n    x = 10\n    while x > 0:\n        x -= 1\n    else:\n        y = 1\n    return y\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // While-else: False branch goes to else block, not directly to exit
        let has_false = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "False");
        assert!(has_false);
    }

    #[test]
    fn test_for_else_with_break() {
        let source = "def foo():\n    for i in range(10):\n        if i == 5:\n            break\n    else:\n        x = 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_break = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "break");
        assert!(has_break);
        // Break should skip the else block and go to the loop exit
    }

    #[test]
    fn test_nested_loops() {
        let source = "def foo():\n    for i in range(10):\n        j = 0\n        while j < i:\n            if j == 3:\n                break\n            j += 1\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // Nested loops: for + while + if = at least 3 branches
        assert!(func.metrics.cyclomatic_complexity >= 3);
        let break_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label == "break")
            .count();
        assert_eq!(break_edges, 1, "break should only target inner loop exit");
    }

    #[test]
    fn test_triple_nested_loops() {
        let source = "def foo():\n    for i in range(3):\n        for j in range(3):\n            for k in range(3):\n                if i == j:\n                    break\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // 3 loops + 1 if = at least complexity 4
        assert!(
            func.metrics.cyclomatic_complexity >= 4,
            "expected >= 4, got {}",
            func.metrics.cyclomatic_complexity
        );
    }

    #[test]
    fn test_multiple_excepts() {
        let source = "def foo():\n    try:\n        x = risky()\n    except ValueError:\n        x = 0\n    except TypeError as e:\n        x = str(e)\n    except:\n        x = None\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exception_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label == "exception")
            .count();
        assert_eq!(
            exception_edges, 3,
            "should have 3 exception edges (one per handler)"
        );
    }

    #[test]
    fn test_bare_except() {
        let source = "def foo():\n    try:\n        x = 1\n    except:\n        x = 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // Bare except should produce a handler with text "except:"
        let has_bare = func
            .blocks
            .iter()
            .any(|b| b.statements.iter().any(|s| s.text == "except:"));
        assert!(has_bare);
    }

    #[test]
    fn test_nested_try() {
        let source = "def foo():\n    try:\n        try:\n            x = inner()\n        except ValueError:\n            x = 0\n    except Exception:\n        x = -1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exception_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label == "exception")
            .count();
        // Inner try has 1 exception edge, outer try has 1 = 2 total
        assert!(
            exception_edges >= 2,
            "expected >= 2, got {}",
            exception_edges
        );
    }

    #[test]
    fn test_try_else() {
        let source = "def foo():\n    try:\n        x = compute()\n    except ValueError:\n        x = 0\n    else:\n        x = x + 1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_try_else = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "try-else");
        assert!(has_try_else);
    }

    #[test]
    fn test_try_except_else_finally() {
        let source = "def foo():\n    try:\n        x = 1\n    except ValueError:\n        x = 0\n    else:\n        x = 2\n    finally:\n        cleanup()\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_finally = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "finally");
        let has_try_else = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "try-else");
        let has_exception = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "exception");
        assert!(has_finally, "should have finally edge");
        assert!(has_try_else, "should have try-else edge");
        assert!(has_exception, "should have exception edge");
    }

    #[test]
    fn test_return_in_try_finally_runs_finally() {
        let source = "def foo(x):\n    try:\n        if x:\n            return 1\n    finally:\n        cleanup()\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let finally_blocks: Vec<_> = func
            .blocks
            .iter()
            .filter(|b| b.statements.iter().any(|s| s.text == "finally:"))
            .collect();
        assert!(
            finally_blocks.len() >= 2,
            "expected normal and abrupt finally paths, got {}",
            finally_blocks.len()
        );
        assert!(finally_blocks.iter().any(|block| {
            block
                .successors
                .iter()
                .any(|edge| edge.target == exit_id && edge.label == "return")
        }));
    }

    #[test]
    fn test_break_in_try_finally_runs_finally() {
        let source = "def foo():\n    for i in range(10):\n        try:\n            break\n        finally:\n            cleanup()\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.blocks.iter().any(|block| {
            block.statements.iter().any(|s| s.text == "finally:")
                && block.successors.iter().any(|edge| edge.label == "break")
        }));
    }

    #[test]
    fn test_continue_in_try_finally_runs_finally() {
        let source = "def foo():\n    for i in range(3):\n        try:\n            continue\n        finally:\n            cleanup()\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(func.blocks.iter().any(|block| {
            block.statements.iter().any(|s| s.text == "finally:")
                && block.successors.iter().any(|edge| edge.label == "continue")
        }));
    }

    #[test]
    fn test_raise_in_try_finally_runs_finally_before_outer_handler() {
        let source = "def foo():\n    try:\n        try:\n            raise ValueError()\n        finally:\n            cleanup()\n    except ValueError:\n        handle()\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let outer_handler = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text == "except ValueError:"))
            .unwrap()
            .id;
        assert!(func.blocks.iter().any(|block| {
            block.statements.iter().any(|s| s.text == "finally:")
                && block
                    .successors
                    .iter()
                    .any(|edge| edge.target == outer_handler && edge.label == "raise")
        }));
    }

    #[test]
    fn test_local_except_does_not_create_abrupt_finally_path() {
        let source = "def foo():\n    try:\n        raise ValueError()\n    except ValueError:\n        handle()\n    finally:\n        cleanup()\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];

        let raise_block = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text == "raise ValueError()"))
            .unwrap();
        let handler_id = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text == "except ValueError:"))
            .unwrap()
            .id;
        let finally_blocks: Vec<_> = func
            .blocks
            .iter()
            .filter(|b| b.statements.iter().any(|s| s.text == "finally:"))
            .collect();

        assert_eq!(
            finally_blocks.len(),
            1,
            "handled exceptions should only traverse the normal finally block once"
        );
        assert!(
            raise_block
                .successors
                .iter()
                .any(|e| e.target == handler_id && e.label == "raise")
        );
        assert!(!raise_block.successors.iter().any(|e| e.label == "finally"));
    }

    #[test]
    fn test_return_in_try_except_finally_still_runs_finally() {
        let source = "def foo():\n    try:\n        return 1\n    except ValueError:\n        handle()\n    finally:\n        cleanup()\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let return_block = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text == "return 1"))
            .unwrap();

        assert!(return_block.successors.iter().any(|e| e.label == "finally"));
        assert!(!return_block.successors.iter().any(|e| e.label == "return"));
    }

    #[test]
    fn test_pending_edges_are_local_handlers_precise() {
        let frame = FinallyFrame {
            finalbody: Vec::new(),
            local_handler_targets: vec![2, 4],
        };

        assert!(CfgBuilder::pending_edges_are_local_handlers(
            &[PendingEdge {
                target: 2,
                label: "raise",
            }],
            &frame,
        ));
        assert!(!CfgBuilder::pending_edges_are_local_handlers(
            &[PendingEdge {
                target: 2,
                label: "return",
            }],
            &frame,
        ));
        assert!(!CfgBuilder::pending_edges_are_local_handlers(
            &[PendingEdge {
                target: 3,
                label: "raise",
            }],
            &frame,
        ));
        assert!(!CfgBuilder::pending_edges_are_local_handlers(
            &[PendingEdge {
                target: 2,
                label: "raise",
            }],
            &FinallyFrame {
                finalbody: Vec::new(),
                local_handler_targets: Vec::new(),
            },
        ));
    }

    #[test]
    fn test_break_in_try() {
        let source = "def foo():\n    for i in range(10):\n        try:\n            if i == 5:\n                break\n        except Exception:\n            pass\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_break = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "break");
        assert!(has_break);
    }

    #[test]
    fn test_continue_in_except() {
        let source = "def foo():\n    for i in range(10):\n        try:\n            risky(i)\n        except Exception:\n            continue\n        process(i)\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_continue = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "continue");
        assert!(has_continue);
    }

    #[test]
    fn test_raise_in_except() {
        let source = "def foo():\n    try:\n        risky()\n    except ValueError as e:\n        raise RuntimeError(\"wrapped\") from e\n";
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
    fn test_bare_raise() {
        let source = "def foo():\n    try:\n        risky()\n    except Exception:\n        log()\n        raise\n";
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
    fn test_guard_clauses() {
        let source = "def foo(x, y, z):\n    if x is None:\n        return -1\n    if y < 0:\n        return -2\n    if z == 0:\n        return -3\n    return x + y + z\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let return_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.target == exit_id && e.label == "return")
            .count();
        assert_eq!(return_edges, 4, "3 guard returns + 1 final return");
    }

    #[test]
    fn test_return_in_all_branches() {
        let source = "def foo(x):\n    if x > 0:\n        return \"pos\"\n    elif x < 0:\n        return \"neg\"\n    else:\n        return \"zero\"\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let return_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.target == exit_id && e.label == "return")
            .count();
        assert_eq!(return_edges, 3, "all 3 branches return");
    }

    #[test]
    fn test_return_in_loop() {
        let source = "def foo(items):\n    for item in items:\n        if is_valid(item):\n            return item\n    return None\n";
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
    fn test_async_def() {
        let source = "async def foo():\n    result = await fetch()\n    return result\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert_eq!(func.name, "foo");
        assert!(func.blocks.len() >= 2);
    }

    #[test]
    fn test_async_for() {
        let source = "async def foo():\n    results = []\n    async for item in aiter:\n        results.append(item)\n    return results\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_async_for = func
            .blocks
            .iter()
            .any(|b| b.statements.iter().any(|s| s.text.starts_with("async for")));
        assert!(has_async_for);
    }

    #[test]
    fn test_async_with() {
        let source = "async def foo():\n    async with session() as s:\n        data = await s.fetch()\n    return data\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_async_with = func.blocks.iter().any(|b| {
            b.statements
                .iter()
                .any(|s| s.text.starts_with("async with"))
        });
        assert!(has_async_with);
    }

    #[test]
    fn test_yield() {
        let source = "def gen():\n    yield 1\n    yield 2\n    yield 3\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert_eq!(func.name, "gen");
        let yield_stmts = func
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter(|s| s.text.starts_with("yield"))
            .count();
        assert_eq!(yield_stmts, 3);
    }

    #[test]
    fn test_yield_in_loop() {
        let source = "def gen():\n    for i in range(10):\n        yield i * 2\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_yield = func
            .blocks
            .iter()
            .any(|b| b.statements.iter().any(|s| s.text.starts_with("yield")));
        assert!(has_yield);
    }

    #[test]
    fn test_yield_from() {
        let source = "def gen():\n    yield from range(5)\n    yield from range(10, 15)\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let yield_from_stmts = func
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter(|s| s.text.starts_with("yield from"))
            .count();
        assert_eq!(yield_from_stmts, 2);
    }

    #[test]
    fn test_nested_class_methods() {
        let source = "class Outer:\n    class Inner:\n        def method(self):\n            return 42\n    def outer_method(self):\n        return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(
            result
                .functions
                .iter()
                .any(|f| f.name == "Outer.Inner.method"),
            "should find nested class method"
        );
        assert!(
            result
                .functions
                .iter()
                .any(|f| f.name == "Outer.outer_method"),
            "should find outer class method"
        );
    }

    #[test]
    fn test_class_with_decorators() {
        let source = "class Foo:\n    @classmethod\n    def create(cls, value):\n        return cls(value)\n    @staticmethod\n    def validate(value):\n        if value < 0:\n            raise ValueError\n        return True\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(result.functions.iter().any(|f| f.name == "Foo.create"));
        assert!(result.functions.iter().any(|f| f.name == "Foo.validate"));
        let validate = result
            .functions
            .iter()
            .find(|f| f.name == "Foo.validate")
            .unwrap();
        assert!(validate.metrics.cyclomatic_complexity >= 2);
    }

    #[test]
    fn test_multiple_with_items() {
        let source = "def foo():\n    with open('a') as a, open('b') as b:\n        data = a.read() + b.read()\n    return data\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let with_stmt = func
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .find(|s| s.text.starts_with("with "))
            .unwrap();
        assert!(
            with_stmt.text.contains(", "),
            "should show both context managers"
        );
    }

    #[test]
    fn test_while_true_with_break() {
        let source = "def foo():\n    while True:\n        x = read()\n        if x == 'quit':\n            break\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let has_break = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .any(|e| e.label == "break");
        assert!(has_break);
    }

    #[test]
    fn test_match_with_guard() {
        // match case with guard clause (case X if condition)
        let source = "def foo(p):\n    match p:\n        case (x, y) if x > 0:\n            return 'pos'\n        case (x, y):\n            return 'other'\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let case_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label.starts_with("case "))
            .count();
        assert_eq!(case_edges, 2);
    }

    #[test]
    fn test_if_in_loop_in_try() {
        let source = "def foo():\n    try:\n        for i in range(10):\n            if i % 2 == 0:\n                process(i)\n            else:\n                skip(i)\n    except Exception:\n        handle()\n    return True\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert!(
            func.metrics.cyclomatic_complexity >= 3,
            "expected >= 3, got {}",
            func.metrics.cyclomatic_complexity
        );
    }

    #[test]
    fn test_match_in_loop() {
        let source = "def foo(events):\n    for event in events:\n        match event:\n            case \"click\":\n                handle_click()\n            case \"key\":\n                handle_key()\n            case _:\n                pass\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let case_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.label.starts_with("case "))
            .count();
        assert_eq!(case_edges, 3);
    }

    #[test]
    fn test_deeply_nested_returns() {
        let source = "def foo():\n    if a():\n        if b():\n            return 'ab'\n        else:\n            for i in range(10):\n                if check(i):\n                    return i\n    elif c():\n        try:\n            return compute()\n        except Exception:\n            return None\n    return 'default'\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let return_edges = func
            .blocks
            .iter()
            .flat_map(|b| &b.successors)
            .filter(|e| e.target == exit_id && e.label == "return")
            .count();
        assert!(
            return_edges >= 4,
            "expected >= 4 return paths, got {}",
            return_edges
        );
    }

    #[test]
    fn test_while_complex_body() {
        let source = "def foo():\n    x = 100\n    while x > 0:\n        if x > 50:\n            x -= 10\n        elif x > 20:\n            x -= 5\n        else:\n            x -= 1\n        if x == 42:\n            break\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // while + if/elif/else (2 branches) + if (1 branch) = at least 4
        assert!(
            func.metrics.cyclomatic_complexity >= 4,
            "expected >= 4, got {}",
            func.metrics.cyclomatic_complexity
        );
    }

    #[test]
    fn test_module_level_code() {
        let source = "x = 1\ny = 2\nif x > 0:\n    z = 3\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(
            result.functions.iter().any(|f| f.name == "<module>"),
            "should create <module> CFG for top-level code"
        );
    }

    #[test]
    fn test_function_targeting_class_method() {
        let source = "class Foo:\n    def bar(self):\n        return 1\n    def baz(self):\n        return 2\n";
        let result = build_cfg_for_function(source, "test.py", "Foo.bar", &CfgOptions::default());
        assert!(result.is_some());
        let file_cfg = result.unwrap();
        assert_eq!(file_cfg.functions.len(), 1);
        assert_eq!(file_cfg.functions[0].name, "Foo.bar");
    }

    #[test]
    fn test_function_targeting_not_found() {
        let source = "def foo():\n    return 1\n";
        let result =
            build_cfg_for_function(source, "test.py", "nonexistent", &CfgOptions::default());
        assert!(result.is_none());
    }

    #[test]
    fn test_edge_dedup() {
        // Ensure add_edge deduplicates identical edges
        let source = "def foo():\n    pass\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        for block in &func.blocks {
            let mut seen = std::collections::HashSet::new();
            for edge in &block.successors {
                let key = (edge.target, &edge.label);
                assert!(
                    seen.insert(key),
                    "duplicate edge to {} with label '{}' in block {}",
                    edge.target,
                    edge.label,
                    block.id
                );
            }
        }
    }

    #[test]
    fn test_metrics_edge_count() {
        // Verify E - N + 2 formula
        let source = "def foo(x):\n    if x:\n        return 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let expected_cc = func.metrics.edges - func.metrics.blocks + 2;
        assert_eq!(func.metrics.cyclomatic_complexity, expected_cc);
    }

    #[test]
    fn test_entry_exit_blocks_present() {
        let source = "def foo():\n    return 1\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        assert_eq!(func.blocks[0].label, "entry");
        assert_eq!(func.blocks[1].label, "exit");
    }

    #[test]
    fn test_exit_block_has_no_successors() {
        let source = "def foo():\n    if x:\n        return 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_block = func.blocks.iter().find(|b| b.label == "exit").unwrap();
        assert!(
            exit_block.successors.is_empty(),
            "exit block should have no successors"
        );
    }

    #[test]
    fn test_display_format() {
        let source = "def foo(x):\n    if x > 0:\n        return 1\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let display = func.to_string();
        assert!(display.contains("def foo:"));
        assert!(display.contains("Block 0 (entry):"));
        assert!(display.contains("Block 1 (exit):"));
        assert!(display.contains("# blocks="));
        assert!(display.contains("cyclomatic_complexity="));
    }

    #[test]
    fn test_line_numbers_monotonic() {
        let source = "def foo():\n    x = 1\n    y = 2\n    z = 3\n    return z\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let lines: Vec<usize> = func
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .map(|s| s.line)
            .collect();
        for window in lines.windows(2) {
            assert!(
                window[0] <= window[1],
                "line numbers should be non-decreasing: {} > {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_assert_in_try() {
        let source = "def foo(x):\n    try:\n        assert x > 0\n        return x\n    except AssertionError:\n        return -1\n";
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
    fn test_assert_then_return_both_edges_to_exit() {
        // Catches: add_edge dedup label == to != (line 138)
        // When a block has both assert-fail and return edges to exit,
        // the dedup must not confuse them (same target, different labels)
        let source = "def foo(x):\n    assert x > 0\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let exit_id = func.blocks.iter().find(|b| b.label == "exit").unwrap().id;
        let entry = &func.blocks[0];
        let has_assert_fail = entry
            .successors
            .iter()
            .any(|e| e.target == exit_id && e.label == "assert-fail");
        let has_return = entry
            .successors
            .iter()
            .any(|e| e.target == exit_id && e.label == "return");
        assert!(
            has_assert_fail,
            "entry should have assert-fail edge to exit"
        );
        assert!(
            has_return,
            "entry should also have return edge to exit (same target, different label)"
        );
    }

    #[test]
    fn test_expression_mode() {
        // Expression mode should still produce a FileCfg
        let parsed =
            ruff_python_parser::parse_unchecked("1 + 2", ParseOptions::from(Mode::Expression));
        let module = parsed.into_syntax();
        assert!(matches!(module, ast::Mod::Expression(_)));
    }

    // -----------------------------------------------------------------------
    // Mutation-test-targeted tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_edge_dedup_same_label_different_targets() {
        // Catches: add_edge == to != mutation (line 138)
        // Multiple exception edges from same block with label "exception" to different handler blocks
        let source = "def foo():\n    try:\n        x = risky()\n    except ValueError:\n        x = 0\n    except TypeError:\n        x = 1\n    except:\n        x = 2\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // Find the try body block — it should have 3 exception edges to 3 different handlers
        let try_body = func
            .blocks
            .iter()
            .find(|b| b.successors.iter().any(|e| e.label == "exception"))
            .expect("should have a block with exception edges");
        let exc_targets: Vec<usize> = try_body
            .successors
            .iter()
            .filter(|e| e.label == "exception")
            .map(|e| e.target)
            .collect();
        assert_eq!(
            exc_targets.len(),
            3,
            "need 3 exception edges with same label to different targets"
        );
        // All targets should be different
        let mut unique = exc_targets.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            3,
            "all 3 exception targets should be different blocks"
        );
    }

    #[test]
    fn test_line_numbers_beyond_first_line() {
        // Catches: offset_to_line returning 0 or 1 (line 156)
        let source = "def foo():\n    x = 1\n    y = 2\n    z = 3\n    return z\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        let lines: Vec<usize> = func
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .map(|s| s.line)
            .collect();
        // Statements are on lines 2, 3, 4, 5
        assert!(
            lines.iter().any(|&l| l >= 3),
            "should have lines >= 3, got {:?}",
            lines
        );
        assert!(
            lines.iter().any(|&l| l >= 4),
            "should have lines >= 4, got {:?}",
            lines
        );
    }

    #[test]
    fn test_line_from_offset_precise() {
        let source = "alpha\nbeta\ngamma\n";
        assert_eq!(line_from_offset(source, TextSize::from(0)), 1);
        assert_eq!(line_from_offset(source, TextSize::from(7)), 2);
        assert_eq!(line_from_offset(source, TextSize::from(12)), 3);
    }

    #[test]
    fn test_with_body_is_processed() {
        // Catches: delete match arm With (line 197)
        // If With arm is deleted, the body wouldn't be processed
        let source = "def foo():\n    with open('f') as f:\n        if f:\n            x = 1\n        else:\n            x = 2\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];
        // The if/else inside with should create branches
        assert!(
            func.metrics.branches >= 1,
            "with body should be processed, creating branches"
        );
        assert!(
            func.metrics.cyclomatic_complexity >= 2,
            "with body if/else should increase CC"
        );
    }

    #[test]
    fn test_funcdef_in_try_no_exception_edges() {
        // Catches: delete FunctionDef|ClassDef match arm (line 201)
        // With explicit_exceptions, func/class defs should NOT get exception edges
        // (but the fallback _ arm would add them)
        // Put ONLY a def inside the try body — no other statements — so the def
        // is the only thing that could add exception edges.
        let source = "def foo():\n    try:\n        def inner():\n            pass\n    except ValueError:\n        pass\n";
        let opts = CfgOptions {
            explicit_exceptions: true,
        };
        let result = build_cfgs(source, "test.py", &opts);
        let func = &result.functions[0];
        // Find the try body block (the one after try: with the def inner statement)
        let def_block = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text.starts_with("def inner")))
            .expect("should have block with def inner");
        // With the FunctionDef arm: no exception edges (def is not risky)
        // With the _ fallback: exception edges would be added
        let exc_edges = def_block
            .successors
            .iter()
            .filter(|e| e.label == "exception")
            .count();
        assert_eq!(
            exc_edges, 0,
            "def/class defs should not get exception edges even with explicit_exceptions"
        );
    }

    #[test]
    fn test_for_else_edge_targets() {
        // Catches: for-else retain logic mutations (lines 305-309)
        // Verify that loop-exit goes to else block, NOT directly to exit_block
        let source = "def foo():\n    for i in range(10):\n        pass\n    else:\n        x = 'else'\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];

        // Find the header block (has the "for" statement)
        let header = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text.starts_with("for ")))
            .expect("should have for header");

        // loop-exit edge should go to the else block (which contains x = 'else')
        let loop_exit_edge = header
            .successors
            .iter()
            .find(|e| e.label == "loop-exit")
            .expect("header should have loop-exit edge");

        let target_block = &func.blocks[loop_exit_edge.target];
        let has_else_stmt = target_block
            .statements
            .iter()
            .any(|s| s.text.contains("'else'") || s.text.contains("\"else\""));
        assert!(
            has_else_stmt,
            "loop-exit should target else block with x = 'else', but targets block {} with stmts: {:?}",
            target_block.id, target_block.statements
        );

        // Also verify there's a fallthrough from else block to the merge block
        assert!(
            target_block
                .successors
                .iter()
                .any(|e| e.label == "fallthrough"),
            "else block should have fallthrough to merge"
        );
    }

    #[test]
    fn test_for_no_else_exits_directly() {
        // Counterpart: for without else, loop-exit should go to merge block directly
        let source = "def foo():\n    for i in range(10):\n        pass\n    return 0\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];

        let header = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text.starts_with("for ")))
            .unwrap();

        // Should have exactly loop-body and loop-exit edges
        assert_eq!(
            header
                .successors
                .iter()
                .filter(|e| e.label == "loop-exit")
                .count(),
            1,
            "for without else should have exactly 1 loop-exit edge"
        );
        assert_eq!(
            header
                .successors
                .iter()
                .filter(|e| e.label == "loop-body")
                .count(),
            1,
            "for should have exactly 1 loop-body edge"
        );
    }

    #[test]
    fn test_while_else_edge_targets() {
        // Catches: while-else retain logic mutations (lines 341-345)
        let source = "def foo():\n    x = 10\n    while x > 0:\n        x -= 1\n    else:\n        y = 'else'\n    return y\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];

        let header = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text.starts_with("while ")))
            .expect("should have while header");

        // False edge should go to else block
        let false_edge = header
            .successors
            .iter()
            .find(|e| e.label == "False")
            .expect("while should have False edge");

        let target_block = &func.blocks[false_edge.target];
        let has_else_stmt = target_block
            .statements
            .iter()
            .any(|s| s.text.contains("'else'") || s.text.contains("\"else\""));
        assert!(
            has_else_stmt,
            "False edge should target else block, but targets block {} with stmts: {:?}",
            target_block.id, target_block.statements
        );

        // Header should have exactly True and False edges (no duplicate False)
        let false_count = header
            .successors
            .iter()
            .filter(|e| e.label == "False")
            .count();
        assert_eq!(
            false_count, 1,
            "should have exactly 1 False edge after retain"
        );
    }

    #[test]
    fn test_while_no_else_false_exits_directly() {
        // Counterpart: while without else
        let source = "def foo():\n    x = 10\n    while x > 0:\n        x -= 1\n    return x\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        let func = &result.functions[0];

        let header = func
            .blocks
            .iter()
            .find(|b| b.statements.iter().any(|s| s.text.starts_with("while ")))
            .unwrap();

        assert_eq!(
            header
                .successors
                .iter()
                .filter(|e| e.label == "True")
                .count(),
            1
        );
        assert_eq!(
            header
                .successors
                .iter()
                .filter(|e| e.label == "False")
                .count(),
            1
        );
    }

    #[test]
    fn test_top_level_code_with_functions() {
        // Catches: || to && in build_cfgs (line 572)
        // has_top_level_code=true, functions.is_empty()=false
        let source = "x = 1\ndef foo():\n    return 2\ny = x + 1\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(
            result.functions.iter().any(|f| f.name == "<module>"),
            "<module> should exist when there's top-level code alongside functions"
        );
        assert!(
            result.functions.iter().any(|f| f.name == "foo"),
            "foo should also exist"
        );
    }

    #[test]
    fn test_class_inside_function() {
        // Catches: delete ClassDef arm in collect_nested_functions (line 718)
        let source = "def outer():\n    class Inner:\n        def method(self):\n            return 42\n    return Inner()\n";
        let result = build_cfgs(source, "test.py", &CfgOptions::default());
        assert!(
            result
                .functions
                .iter()
                .any(|f| f.name == "outer.Inner.method"),
            "should find class method nested inside a function; found: {:?}",
            result.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }
}
