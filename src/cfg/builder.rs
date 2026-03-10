use ruff_python_ast::{self as ast, Stmt};
use ruff_text_size::Ranged;

use super::{
    BasicBlock, BlockKind, CfgOptions, Edge, EdgeKind, FunctionCfg, Metrics, Statement, source_map,
};

pub(crate) struct CfgBuilder<'src> {
    source: &'src str,
    blocks: Vec<BasicBlock>,
    loop_stack: Vec<(usize, usize)>,
    except_stack: Vec<Vec<usize>>,
    finally_stack: Vec<FinallyFrame>,
    explicit_exceptions: bool,
}

#[derive(Clone)]
pub(crate) struct PendingEdge {
    pub(crate) target: usize,
    pub(crate) label: &'static str,
}

#[derive(Clone)]
pub(crate) struct FinallyFrame {
    pub(crate) finalbody: Vec<Stmt>,
    pub(crate) local_handler_targets: Vec<usize>,
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

    fn new_block(&mut self, label: BlockKind) -> usize {
        let id = self.blocks.len();
        self.blocks.push(BasicBlock {
            id,
            label,
            statements: Vec::new(),
            successors: Vec::new(),
        });
        id
    }

    fn add_edge(&mut self, from: usize, to: usize, label: impl Into<EdgeKind>) {
        let label = label.into();
        if self.blocks[from]
            .successors
            .iter()
            .any(|e| e.target == to && e.label == label)
        {
            return;
        }
        self.blocks[from]
            .successors
            .push(Edge { target: to, label });
    }

    fn remove_edge(&mut self, from: usize, to: usize, label: impl Into<EdgeKind>) {
        let label = label.into();
        if let Some(index) = self.blocks[from]
            .successors
            .iter()
            .position(|e| e.target == to && e.label == label)
        {
            self.blocks[from].successors.remove(index);
        }
    }

    fn add_stmt(&mut self, block: usize, line: usize, text: &str) {
        self.blocks[block].statements.push(Statement {
            line,
            text: text.to_string(),
        });
    }

    fn offset_to_line(&self, offset: ruff_text_size::TextSize) -> usize {
        source_map::offset_to_line(self.source, offset)
    }

    fn range_text(&self, range: ruff_text_size::TextRange) -> String {
        source_map::range_text(self.source, range)
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

    pub(crate) fn pending_edges_are_local_handlers(
        pending: &[PendingEdge],
        frame: &FinallyFrame,
    ) -> bool {
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

            let finally_block = self.new_block(BlockKind::Body);
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

        let true_block = self.new_block(BlockKind::Body);
        let merge_block = self.new_block(BlockKind::Body);

        self.add_edge(current, true_block, "True");

        let true_end = self.build_stmts(&if_stmt.body, true_block, exit);
        if let Some(te) = true_end {
            self.add_edge(te, merge_block, "fallthrough");
        }

        let mut prev_false_from = current;
        for clause in &if_stmt.elif_else_clauses {
            if let Some(ref test) = clause.test {
                let elif_test_block = self.new_block(BlockKind::Body);
                self.add_edge(prev_false_from, elif_test_block, "False");

                let elif_line = self.offset_to_line(clause.range().start());
                let elif_text = format!("elif {}:", self.range_text(test.range()));
                self.add_stmt(elif_test_block, elif_line, &elif_text);

                let elif_body_block = self.new_block(BlockKind::Body);
                self.add_edge(elif_test_block, elif_body_block, "True");

                let elif_end = self.build_stmts(&clause.body, elif_body_block, exit);
                if let Some(ee) = elif_end {
                    self.add_edge(ee, merge_block, "fallthrough");
                }

                prev_false_from = elif_test_block;
            } else {
                let else_block = self.new_block(BlockKind::Body);
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
        let body_block = self.new_block(BlockKind::Body);
        let exit_block = self.new_block(BlockKind::Body);

        self.add_edge(header, body_block, "loop-body");
        self.add_edge(header, exit_block, "loop-exit");

        self.loop_stack.push((header, exit_block));
        let body_end = self.build_stmts(&for_stmt.body, body_block, exit);
        self.loop_stack.pop();

        if let Some(be) = body_end {
            self.add_edge(be, header, "loop-back");
        }

        if !for_stmt.orelse.is_empty() {
            let else_block = self.new_block(BlockKind::Body);
            self.remove_edge(header, exit_block, "loop-exit");
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
        let body_block = self.new_block(BlockKind::Body);
        let exit_block = self.new_block(BlockKind::Body);

        self.add_edge(header, body_block, "True");
        self.add_edge(header, exit_block, "False");

        self.loop_stack.push((header, exit_block));
        let body_end = self.build_stmts(&while_stmt.body, body_block, exit);
        self.loop_stack.pop();

        if let Some(be) = body_end {
            self.add_edge(be, header, "loop-back");
        }

        if !while_stmt.orelse.is_empty() {
            let else_block = self.new_block(BlockKind::Body);
            self.remove_edge(header, exit_block, "False");
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

        let merge_block = self.new_block(BlockKind::Body);

        let mut handler_blocks = Vec::new();
        for handler in &try_stmt.handlers {
            let handler_block = self.new_block(BlockKind::Body);
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
        let finally_depth = self.finally_stack.len();
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

        let try_body_block = self.new_block(BlockKind::Body);
        self.add_edge(current, try_body_block, "try");
        let try_end = self.build_stmts(&try_stmt.body, try_body_block, exit);

        self.except_stack.pop();
        self.finally_stack.truncate(finally_depth);

        if !self.explicit_exceptions {
            for &handler_block in &handler_blocks {
                self.add_edge(try_body_block, handler_block, "exception");
            }
        }

        if !try_stmt.orelse.is_empty() {
            let else_block = self.new_block(BlockKind::Body);
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
            let finally_block = self.new_block(BlockKind::Body);
            let finally_line = self.offset_to_line(try_stmt.finalbody[0].range().start());
            self.add_stmt(
                finally_block,
                finally_line.saturating_sub(1).max(1),
                "finally:",
            );

            let new_merge = self.new_block(BlockKind::Body);
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

        let merge_block = self.new_block(BlockKind::Body);

        for case in &match_stmt.cases {
            let case_block = self.new_block(BlockKind::Body);
            let pattern_text = self.range_text(case.pattern.range());
            let label = format!("case {}", pattern_text);
            self.add_edge(current, case_block, label.as_str());

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

pub(crate) fn build_single_cfg(
    source: &str,
    name: &str,
    line: usize,
    body: &[Stmt],
    options: &CfgOptions,
) -> FunctionCfg {
    let mut builder = CfgBuilder::new(source, options.explicit_exceptions);

    let entry = builder.new_block(BlockKind::Entry);
    let exit = builder.new_block(BlockKind::Exit);

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
