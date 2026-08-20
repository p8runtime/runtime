use super::super::Parser;
use crate::parser::ast::{
    AssignOp, AttributeGroup, Expr, ExprId, ParseError, Stmt, StmtId,
};
use crate::parser::lexer::token::TokenKind;
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_closure_expr(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        is_async: bool,
        is_static: bool,
        start: usize,
    ) -> ExprId<'ast> {
        // Anonymous functions should not have a name, but allow an identifier for recovery
        if self.current_token.kind == TokenKind::Identifier {
            self.bump();
        }

        let by_ref = if matches!(
            self.current_token.kind,
            TokenKind::Ampersand
                | TokenKind::AmpersandFollowedByVarOrVararg
                | TokenKind::AmpersandNotFollowedByVarOrVararg
        ) {
            self.bump();
            true
        } else {
            false
        };

        let params = self.parse_parameter_list();
        let uses = self.parse_use_list();
        let return_type = self.parse_return_type();

        let body_stmt = self.with_function_context(is_async, |parser| parser.parse_block());
        let raw_body: &'ast [StmtId<'ast>] = match body_stmt {
            Stmt::Block { statements, .. } => statements,
            _ => self.arena.alloc_slice_copy(&[body_stmt]) as &'ast [StmtId<'ast>],
        };
        let prologue = self.take_param_destructure_prologue();
        let body = if prologue.is_empty() {
            raw_body
        } else {
            let mut merged = std::vec::Vec::with_capacity(prologue.len() + raw_body.len());
            merged.extend_from_slice(prologue);
            merged.extend_from_slice(raw_body);
            self.arena.alloc_slice_copy(&merged)
        };

        let end = self.current_token.span.end;
        self.arena.alloc(Expr::Closure {
            attributes,
            is_async,
            is_static,
            by_ref,
            params,
            uses,
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    /// Parse a DekaScript function literal: `fn(x: int) int { return x * 2 }`.
    /// The `fn` keyword has already been consumed.
    pub(in crate::parser::parser) fn parse_ds_function_literal(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        is_async: bool,
        is_static: bool,
        start: usize,
    ) -> ExprId<'ast> {
        let params = self.parse_parameter_list();
        let return_type = self.parse_return_type();

        let body_stmt = self.with_function_context(is_async, |parser| parser.parse_block());
        let raw_body: &'ast [StmtId<'ast>] = match body_stmt {
            Stmt::Block { statements, .. } => statements,
            _ => self.arena.alloc_slice_copy(&[body_stmt]) as &'ast [StmtId<'ast>],
        };
        let prologue = self.take_param_destructure_prologue();
        let body = if prologue.is_empty() {
            raw_body
        } else {
            let mut merged = std::vec::Vec::with_capacity(prologue.len() + raw_body.len());
            merged.extend_from_slice(prologue);
            merged.extend_from_slice(raw_body);
            self.arena.alloc_slice_copy(&merged)
        };

        let end = self.current_token.span.end;
        self.arena.alloc(Expr::Closure {
            attributes,
            is_async,
            is_static,
            by_ref: false,
            params,
            uses: &[],
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn parse_arrow_function(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        is_async: bool,
        is_static: bool,
        start: usize,
    ) -> ExprId<'ast> {
        let by_ref = if matches!(
            self.current_token.kind,
            TokenKind::Ampersand
                | TokenKind::AmpersandFollowedByVarOrVararg
                | TokenKind::AmpersandNotFollowedByVarOrVararg
        ) {
            self.bump();
            true
        } else {
            false
        };

        let prev_allow_optional = self.allow_optional_param_types;
        self.allow_optional_param_types = true;
        let params = self.parse_parameter_list();
        self.allow_optional_param_types = prev_allow_optional;
        let return_type = self.parse_return_type();
        if self.current_token.kind == TokenKind::DoubleArrow {
            self.bump();
        }
        let prologue = self.take_param_destructure_prologue();
        if !prologue.is_empty() {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Arrow functions do not support parameter destructuring yet",
                "Use a closure with a block body for destructuring parameters.",
            ));
        }
        let expr = self.with_function_context(is_async, |parser| parser.parse_expr(0));

        let end = expr.span().end;
        self.arena.alloc(Expr::ArrowFunction {
            attributes,
            is_async,
            is_static,
            by_ref,
            params,
            return_type,
            expr,
            span: Span::new(start, end),
        })
    }

    /// Scans ahead from an opening `(` to decide whether the parenthesised
    /// expression is actually a JS/TS-style arrow function parameter list.
    ///
    /// Returns true when the matching `)` is followed by `=>`, or by `: Type =>`.
    /// DekaScript does not allow parenthesised arrow functions.
    pub(in crate::parser::parser) fn looks_like_parenthesized_arrow_function(&self) -> bool {
        if self.is_ds() {
            return false;
        }
        let mut depth: i32 = 1;
        let mut i: usize = 1;
        while depth > 0 {
            match self.lookahead_kind(i) {
                Some(TokenKind::OpenParen) => depth += 1,
                Some(TokenKind::CloseParen) => depth -= 1,
                Some(TokenKind::Eof) | None => return false,
                _ => {}
            }
            i += 1;
        }
        // `i` now points at the first token after the matching `)`.
        let mut j = i;
        if self.lookahead_kind(j) == Some(TokenKind::Colon) {
            j += 1;
            let mut type_depth: i32 = 0;
            loop {
                match self.lookahead_kind(j) {
                    Some(TokenKind::Lt)
                    | Some(TokenKind::OpenParen)
                    | Some(TokenKind::OpenBracket) => type_depth += 1,
                    Some(TokenKind::Gt)
                    | Some(TokenKind::CloseParen)
                    | Some(TokenKind::CloseBracket) => type_depth -= 1,
                    Some(TokenKind::DoubleArrow) if type_depth == 0 => return true,
                    Some(TokenKind::SemiColon)
                    | Some(TokenKind::Comma)
                    | Some(TokenKind::CloseBrace)
                    | Some(TokenKind::Eof)
                    | None => return false,
                    _ => {}
                }
                j += 1;
                if j > i + 128 {
                    return false;
                }
            }
        } else {
            self.lookahead_kind(j) == Some(TokenKind::DoubleArrow)
        }
    }

    /// Parses a JS/TS-style parenthesised arrow function: `(a, b) => a + b`.
    pub(in crate::parser::parser) fn parse_parenthesized_arrow_function(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        start: usize,
    ) -> ExprId<'ast> {
        let prev_allow_optional = self.allow_optional_param_types;
        self.allow_optional_param_types = true;
        let params = self.parse_parameter_list();
        self.allow_optional_param_types = prev_allow_optional;
        let return_type = self.parse_return_type();
        if self.current_token.kind == TokenKind::DoubleArrow {
            self.bump();
        }
        let prologue = self.take_param_destructure_prologue();
        if !prologue.is_empty() {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Arrow functions do not support parameter destructuring yet",
                "Use a closure with a block body for destructuring parameters.",
            ));
        }
        let expr = self.with_function_context(false, |parser| parser.parse_expr(0));
        let end = expr.span().end;
        self.arena.alloc(Expr::ArrowFunction {
            attributes,
            is_async: false,
            is_static: false,
            by_ref: false,
            params,
            return_type,
            expr,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn is_assignable(&self, expr: ExprId<'ast>) -> bool {
        match expr {
            Expr::Variable { .. }
            | Expr::IndirectVariable { .. }
            | Expr::ArrayDimFetch { .. }
            | Expr::PropertyFetch { .. }
            | Expr::DotAccess { .. } => true,
            Expr::ClassConstFetch { constant, .. } => {
                if let Expr::Variable { span, .. } = constant {
                    let slice = self.lexer.slice(*span);
                    return slice.first() == Some(&b'$');
                }
                false
            }
            Expr::Array { items, .. } => {
                for item in items.iter() {
                    if let Expr::Error { .. } = item.value {
                        continue;
                    }
                    if !self.is_assignable(item.value) {
                        return false;
                    }
                }
                true
            }
            Expr::ObjectLiteral { items, .. } => {
                for item in items.iter() {
                    if !self.is_assignable(item.value) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(in crate::parser::parser) fn is_reassociable_assignment_target(
        &self,
        expr: ExprId<'ast>,
    ) -> bool {
        match expr {
            Expr::Unary { expr: inner, .. } | Expr::Cast { expr: inner, .. } => {
                self.is_assignable(inner) || self.is_reassociable_assignment_target(inner)
            }
            Expr::Binary { right, .. } => {
                self.is_assignable(right) || self.is_reassociable_assignment_target(right)
            }
            Expr::Ternary { if_false, .. } => {
                self.is_assignable(if_false) || self.is_reassociable_assignment_target(if_false)
            }
            Expr::Clone { expr: inner, .. } => {
                self.is_assignable(inner) || self.is_reassociable_assignment_target(inner)
            }
            _ => false,
        }
    }

    pub(in crate::parser::parser) fn create_assignment(
        &self,
        var: ExprId<'ast>,
        right: ExprId<'ast>,
        op: Option<AssignOp>,
    ) -> ExprId<'ast> {
        let span = Span::new(var.span().start, right.span().end);
        if let Some(assign_op) = op {
            self.arena.alloc(Expr::AssignOp {
                var,
                op: assign_op,
                expr: right,
                span,
            })
        } else {
            self.arena.alloc(Expr::Assign {
                var,
                expr: right,
                span,
            })
        }
    }

    pub(in crate::parser::parser) fn reassociate_assignment(
        &self,
        left: ExprId<'ast>,
        right: ExprId<'ast>,
        op: Option<AssignOp>,
    ) -> ExprId<'ast> {
        match left {
            Expr::Unary {
                op: unary_op,
                expr: inner,
                span,
            } => {
                let new_inner = if self.is_assignable(inner) {
                    self.create_assignment(inner, right, op)
                } else {
                    self.reassociate_assignment(inner, right, op)
                };
                let new_span = Span::new(span.start, right.span().end);
                self.arena.alloc(Expr::Unary {
                    op: *unary_op,
                    expr: new_inner,
                    span: new_span,
                })
            }
            Expr::Cast {
                kind,
                expr: inner,
                span,
            } => {
                let new_inner = if self.is_assignable(inner) {
                    self.create_assignment(inner, right, op)
                } else {
                    self.reassociate_assignment(inner, right, op)
                };
                let new_span = Span::new(span.start, right.span().end);
                self.arena.alloc(Expr::Cast {
                    kind: *kind,
                    expr: new_inner,
                    span: new_span,
                })
            }
            Expr::Binary {
                left: b_left,
                op: b_op,
                right: b_right,
                span,
            } => {
                let new_right = if self.is_assignable(b_right) {
                    self.create_assignment(b_right, right, op)
                } else {
                    self.reassociate_assignment(b_right, right, op)
                };
                let new_span = Span::new(span.start, right.span().end);
                self.arena.alloc(Expr::Binary {
                    left: b_left,
                    op: *b_op,
                    right: new_right,
                    span: new_span,
                })
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                span,
            } => {
                let new_if_false = if self.is_assignable(if_false) {
                    self.create_assignment(if_false, right, op)
                } else {
                    self.reassociate_assignment(if_false, right, op)
                };
                let new_span = Span::new(span.start, right.span().end);
                self.arena.alloc(Expr::Ternary {
                    condition,
                    if_true: *if_true,
                    if_false: new_if_false,
                    span: new_span,
                })
            }
            Expr::Clone { expr: inner, span } => {
                let new_inner = if self.is_assignable(inner) {
                    self.create_assignment(inner, right, op)
                } else {
                    self.reassociate_assignment(inner, right, op)
                };
                let new_span = Span::new(span.start, right.span().end);
                self.arena.alloc(Expr::Clone {
                    expr: new_inner,
                    span: new_span,
                })
            }
            _ => unreachable!(
                "Should only be called if is_reassociable_assignment_target returned true"
            ),
        }
    }
}
