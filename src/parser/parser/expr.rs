use super::Parser;
use crate::parser::ast::{
    Arg, ArrayItem, AssignOp, AttributeGroup, BinaryOp, CastKind, ClosureUse, Expr, ExprId,
    IncludeKind, JsxAttribute, JsxChild, MagicConstKind, MatchArm, Name, ObjectItem, ObjectKey,
    Param, ParseError, Stmt, StmtId, StructLiteralField, Type, UnaryOp,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(super) fn parse_call_arguments(&mut self) -> (&'ast [Arg<'ast>], Span) {
        let start = self.current_token.span.start;
        if self.current_token.kind != TokenKind::OpenParen {
            return (&[], Span::default());
        }
        self.bump(); // consume (

        let mut args = bumpalo::collections::Vec::new_in(self.arena);
        let mut has_named = false;
        while self.current_token.kind != TokenKind::CloseParen
            && self.current_token.kind != TokenKind::Eof
        {
            let mut name: Option<&'ast Token> = None;
            let mut unpack = false;
            let start = self.current_token.span.start;

            // Named argument: identifier-like token followed by :
            if (self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved())
                && self.next_token.kind == TokenKind::Colon
            {
                name = Some(self.arena.alloc(self.current_token));
                self.bump(); // Identifier
                self.bump(); // Colon
                has_named = true;
            } else if self.current_token.kind == TokenKind::Ellipsis {
                if self.next_token.kind == TokenKind::CloseParen {
                    let span = self.current_token.span;
                    self.bump(); // Eat ...
                    let value = self.arena.alloc(Expr::VariadicPlaceholder { span });
                    args.push(Arg {
                        name: None,
                        value,
                        unpack: false,
                        span,
                    });
                    continue;
                }
                unpack = true;
                self.bump();
            } else if has_named {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Cannot use positional argument after named argument",
                ));
            }

            let value = self.parse_expr(0);

            args.push(Arg {
                name,
                value,
                unpack,
                span: Span {
                    start,
                    end: value.span().end,
                },
            });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                // Allow trailing comma in argument list
                if self.current_token.kind == TokenKind::CloseParen {
                    break;
                }
            } else if self.current_token.kind != TokenKind::CloseParen {
                break;
            }
        }
        let end = self.current_token.span.end;
        if self.current_token.kind == TokenKind::CloseParen {
            self.bump();
        }
        (args.into_bump_slice(), Span::new(start, end))
    }

    pub(crate) fn parse_parameter_list(&mut self) -> &'ast [Param<'ast>] {
        self.param_destructure_prologue.clear();
        if self.current_token.kind == TokenKind::OpenParen {
            self.bump();
        }
        let mut params = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseParen
            && self.current_token.kind != TokenKind::Eof
        {
            params.push(self.parse_param());
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            }
        }
        if self.current_token.kind == TokenKind::CloseParen {
            self.bump();
        }
        params.into_bump_slice()
    }

    pub(super) fn parse_phpx_param_pattern(&mut self) -> Option<ExprId<'ast>> {
        match self.current_token.kind {
            TokenKind::OpenBracket | TokenKind::List => Some(self.parse_expr(0)),
            TokenKind::OpenBrace => Some(self.parse_phpx_object_pattern()),
            _ => None,
        }
    }

    pub(super) fn pattern_last_binding(&self, pattern: ExprId<'ast>) -> Option<&'ast Token> {
        match pattern {
            Expr::Variable { span, .. } => Some(self.arena.alloc(Token {
                kind: TokenKind::Variable,
                span: *span,
            })),
            Expr::Assign { var, .. } => self.pattern_last_binding(var),
            Expr::Array { items, .. } => {
                let mut out = None;
                for item in items.iter() {
                    if let Some(found) = self.pattern_last_binding(item.value) {
                        out = Some(found);
                    }
                }
                out
            }
            Expr::ObjectLiteral { items, .. } => {
                let mut out = None;
                for item in items.iter() {
                    if let Some(found) = self.pattern_last_binding(item.value) {
                        out = Some(found);
                    }
                }
                out
            }
            _ => None,
        }
    }

    pub(super) fn push_param_pattern_prologue(
        &mut self,
        pattern: ExprId<'ast>,
        source_var: &'ast Token,
    ) {
        let source_expr = self.arena.alloc(Expr::Variable {
            name: source_var.span,
            span: source_var.span,
        });
        self.push_pattern_binding(pattern, source_expr, source_var.span);
    }

    fn push_pattern_binding(
        &mut self,
        pattern: ExprId<'ast>,
        source_expr: ExprId<'ast>,
        fallback_span: Span,
    ) {
        match pattern {
            Expr::Variable { .. } => {
                self.push_binding_assign(pattern, source_expr, fallback_span);
            }
            Expr::Assign {
                var, expr: default, ..
            } => {
                let rhs = self.arena.alloc(Expr::Binary {
                    left: source_expr,
                    op: BinaryOp::Coalesce,
                    right: default,
                    span: Span::new(source_expr.span().start, default.span().end),
                });
                self.push_pattern_binding(var, rhs, fallback_span);
            }
            Expr::Array { items, span } => {
                for (idx, item) in items.iter().enumerate() {
                    if matches!(item.value, Expr::Error { .. }) {
                        continue;
                    }
                    let dim_expr = if let Some(key) = item.key {
                        key
                    } else {
                        let idx_bytes = idx.to_string();
                        let idx_value = self.arena.alloc_slice_copy(idx_bytes.as_bytes());
                        self.arena.alloc(Expr::Integer {
                            value: idx_value,
                            span: *span,
                        })
                    };
                    let access = self.arena.alloc(Expr::ArrayDimFetch {
                        array: source_expr,
                        dim: Some(dim_expr),
                        span: Span::new(source_expr.span().start, dim_expr.span().end),
                    });
                    self.push_pattern_binding(item.value, access, *span);
                }
            }
            Expr::ObjectLiteral { items, span } => {
                for item in items.iter() {
                    let access = match item.key {
                        ObjectKey::Ident(token) => {
                            let raw = self.lexer.slice(token.span);
                            let normalized = if raw.starts_with(b"$") && raw.len() > 1 {
                                &raw[1..]
                            } else {
                                raw
                            };
                            let key_expr = self.arena.alloc(Expr::String {
                                value: self.arena.alloc_slice_copy(normalized),
                                span: token.span,
                            });
                            self.arena.alloc(Expr::PropertyFetch {
                                target: source_expr,
                                property: key_expr,
                                span: Span::new(source_expr.span().start, token.span.end),
                            })
                        }
                        ObjectKey::String(token) => {
                            let raw = self.lexer.slice(token.span);
                            let mut value = if raw.len() >= 2
                                && ((raw[0] == b'"' && raw[raw.len() - 1] == b'"')
                                    || (raw[0] == b'\'' && raw[raw.len() - 1] == b'\''))
                            {
                                &raw[1..raw.len() - 1]
                            } else {
                                raw
                            };
                            if value.starts_with(b"$") && value.len() > 1 {
                                value = &value[1..];
                            }
                            let key_expr = self.arena.alloc(Expr::String {
                                value: self.arena.alloc_slice_copy(value),
                                span: token.span,
                            });
                            self.arena.alloc(Expr::PropertyFetch {
                                target: source_expr,
                                property: key_expr,
                                span: Span::new(source_expr.span().start, token.span.end),
                            })
                        }
                    };
                    self.push_pattern_binding(item.value, access, *span);
                }
            }
            _ => {
                self.errors.push(ParseError::with_help(
                    fallback_span,
                    "Unsupported destructuring pattern",
                    "Use variable, array, or object patterns in PHPX destructuring.",
                ));
            }
        }
    }

    fn push_binding_assign(
        &mut self,
        target: ExprId<'ast>,
        rhs: ExprId<'ast>,
        fallback_span: Span,
    ) {
        let span = Span::new(target.span().start, rhs.span().end);
        let assign = self.arena.alloc(Expr::Assign {
            var: target,
            expr: rhs,
            span,
        });
        self.param_destructure_prologue
            .push(self.arena.alloc(Stmt::Expression { expr: assign, span }));
        if !matches!(target, Expr::Variable { .. }) {
            self.errors.push(ParseError::with_help(
                fallback_span,
                "Destructuring target must resolve to variables",
                "Use variable bindings inside the destructuring pattern.",
            ));
        }
    }

    fn parse_phpx_object_pattern(&mut self) -> ExprId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // consume {
        let mut items = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }

            let item_start = self.current_token.span.start;

            if self.current_token.kind == TokenKind::Variable {
                // Shorthand: { $name } -> { name: $name }
                let value_tok = self.arena.alloc(self.current_token);
                self.bump();
                let key = ObjectKey::Ident(value_tok);
                let mut value = self.arena.alloc(Expr::Variable {
                    name: value_tok.span,
                    span: value_tok.span,
                });
                if self.current_token.kind == TokenKind::Eq {
                    self.bump();
                    let default = self.parse_expr(0);
                    value = self.arena.alloc(Expr::Assign {
                        var: value,
                        expr: default,
                        span: Span::new(item_start, default.span().end),
                    });
                }
                let span = Span::new(item_start, value.span().end);
                items.push(ObjectItem { key, value, span });
            } else {
                let (key, _) = match self.current_token.kind {
                    TokenKind::Identifier => {
                        let tok = self.arena.alloc(self.current_token);
                        self.bump();
                        (ObjectKey::Ident(tok), tok.span.start)
                    }
                    TokenKind::StringLiteral => {
                        let tok = self.arena.alloc(self.current_token);
                        self.bump();
                        (ObjectKey::String(tok), tok.span.start)
                    }
                    _ if self.current_token.kind.is_semi_reserved() => {
                        let tok = self.arena.alloc(self.current_token);
                        self.bump();
                        (ObjectKey::Ident(tok), tok.span.start)
                    }
                    _ => {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected key or variable in object pattern",
                        ));
                        let tok = self.arena.alloc(Token {
                            kind: TokenKind::Error,
                            span: self.current_token.span,
                        });
                        self.bump();
                        (ObjectKey::Ident(tok), tok.span.start)
                    }
                };

                if self.current_token.kind == TokenKind::Colon {
                    self.bump();
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected ':' after object pattern key",
                    ));
                }

                let mut value = if matches!(
                    self.current_token.kind,
                    TokenKind::OpenBrace | TokenKind::OpenBracket | TokenKind::List
                ) {
                    self.parse_phpx_param_pattern()
                        .unwrap_or_else(|| self.parse_expr(0))
                } else {
                    self.parse_expr(0)
                };

                if self.current_token.kind == TokenKind::Eq {
                    self.bump();
                    let default = self.parse_expr(0);
                    value = self.arena.alloc(Expr::Assign {
                        var: value,
                        expr: default,
                        span: Span::new(item_start, default.span().end),
                    });
                }

                let span = Span::new(item_start, value.span().end);
                items.push(ObjectItem { key, value, span });
            }

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            }
        }

        let end = if self.current_token.kind == TokenKind::CloseBrace {
            let end = self.current_token.span.end;
            self.bump();
            end
        } else {
            self.current_token.span.end
        };

        self.arena.alloc(Expr::ObjectLiteral {
            items: items.into_bump_slice(),
            span: Span::new(start, end),
        })
    }

    pub(crate) fn parse_use_list(&mut self) -> &'ast [ClosureUse<'ast>] {
        if self.current_token.kind == TokenKind::Use {
            self.bump();
            if self.current_token.kind == TokenKind::OpenParen {
                self.bump();
            }

            let mut uses = bumpalo::collections::Vec::new_in(self.arena);
            while self.current_token.kind != TokenKind::CloseParen
                && self.current_token.kind != TokenKind::Eof
            {
                let by_ref = if matches!(
                    self.current_token.kind,
                    TokenKind::Ampersand | TokenKind::AmpersandFollowedByVarOrVararg
                ) {
                    self.bump();
                    true
                } else {
                    false
                };

                let var = if self.current_token.kind == TokenKind::Variable {
                    let t = self.arena.alloc(self.current_token);
                    self.bump();
                    t
                } else {
                    self.arena.alloc(Token {
                        kind: TokenKind::Error,
                        span: Span::default(),
                    })
                };

                uses.push(ClosureUse {
                    var,
                    by_ref,
                    span: var.span,
                });

                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                }
            }
            if self.current_token.kind == TokenKind::CloseParen {
                self.bump();
                uses.into_bump_slice()
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected ')' after closure use list",
                ));
                &[]
            }
        } else {
            &[]
        }
    }

    pub(crate) fn parse_return_type(&mut self) -> Option<&'ast Type<'ast>> {
        if self.current_token.kind == TokenKind::Colon {
            self.bump();
            if let Some(t) = self.parse_type() {
                Some(self.arena.alloc(t) as &'ast Type<'ast>)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub(super) fn parse_closure_expr(
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

    pub(super) fn parse_arrow_function(
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

        let params = self.parse_parameter_list();
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

    fn is_assignable(&self, expr: ExprId<'ast>) -> bool {
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

    fn is_reassociable_assignment_target(&self, expr: ExprId<'ast>) -> bool {
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

    fn create_assignment(
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

    fn reassociate_assignment(
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

    pub(super) fn parse_expr(&mut self, min_bp: u8) -> ExprId<'ast> {
        let mut left = self.parse_nud();
        let mut just_parsed_ternary = false;
        let mut just_parsed_elvis = false;

        loop {
            // PHPX ASI guard: a line break before `(` should terminate the expression
            // instead of continuing as a call chain on the next line.
            if self.is_phpx()
                && self.current_token.kind == TokenKind::OpenParen
                && self.has_line_terminator_between(self.prev_token.span, self.current_token.span)
            {
                break;
            }

            if self.current_token.kind == TokenKind::Dot && self.is_phpx() {
                let dot_span = self.current_token.span;
                let next = self.next_token;
                let left_span = left.span();
                let tight = left_span.end == dot_span.start && dot_span.end == next.span.start;

                if tight && (next.kind == TokenKind::Identifier || next.kind.is_semi_reserved()) {
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump(); // consume .
                    let property = self.arena.alloc(self.current_token);
                    self.bump();
                    let span = Span::new(left_span.start, property.span.end);
                    left = self.arena.alloc(Expr::DotAccess {
                        target: left,
                        property,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
            }

            let op = match self.current_token.kind {
                TokenKind::Plus => BinaryOp::Plus,
                TokenKind::Minus => BinaryOp::Minus,
                TokenKind::Asterisk => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                TokenKind::Dot => BinaryOp::Concat,
                TokenKind::EqEq => BinaryOp::EqEq,
                TokenKind::EqEqEq => BinaryOp::EqEqEq,
                TokenKind::BangEq => BinaryOp::NotEq,
                TokenKind::BangEqEq => BinaryOp::NotEqEq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                TokenKind::AmpersandAmpersand => BinaryOp::And,
                TokenKind::PipePipe => BinaryOp::Or,
                TokenKind::Ampersand
                | TokenKind::AmpersandFollowedByVarOrVararg
                | TokenKind::AmpersandNotFollowedByVarOrVararg => BinaryOp::BitAnd,
                TokenKind::Pipe => BinaryOp::BitOr,
                TokenKind::PipeGt => BinaryOp::Pipe,
                TokenKind::Caret => BinaryOp::BitXor,
                TokenKind::LogicalAnd => BinaryOp::LogicalAnd,
                TokenKind::LogicalOr | TokenKind::Insteadof => BinaryOp::LogicalOr,
                TokenKind::LogicalXor => BinaryOp::LogicalXor,
                TokenKind::Coalesce => BinaryOp::Coalesce,
                TokenKind::Spaceship => BinaryOp::Spaceship,
                TokenKind::Pow => BinaryOp::Pow,
                TokenKind::Sl => BinaryOp::ShiftLeft,
                TokenKind::Sr => BinaryOp::ShiftRight,
                TokenKind::InstanceOf => BinaryOp::Instanceof,
                TokenKind::Question => {
                    // Ternary: a ? b : c
                    // PHP allows any expression in both branches, including low-precedence ones
                    let l_bp = 40;
                    if l_bp < min_bp {
                        break;
                    }

                    let current_is_elvis = self.next_token.kind == TokenKind::Colon;

                    if just_parsed_ternary && (!just_parsed_elvis || !current_is_elvis) {
                        self.errors.push(ParseError::new(self.current_token.span, "Unparenthesized `a ? b : c ? d : e` is not supported. Use either `(a ? b : c) ? d : e` or `a ? b : (c ? d : e)`"));
                    }

                    self.bump();

                    let if_true = if self.current_token.kind != TokenKind::Colon {
                        Some(self.parse_expr(0))
                    } else {
                        None
                    };

                    if self.current_token.kind == TokenKind::Colon {
                        self.bump();
                    }

                    // Use l_bp + 1 to enforce left-associativity for the else branch,
                    // which allows us to detect the unparenthesized nesting in the next iteration.
                    let if_false = self.parse_expr(l_bp + 1);

                    let span = Span::new(left.span().start, if_false.span().end);
                    left = self.arena.alloc(Expr::Ternary {
                        condition: left,
                        if_true,
                        if_false,
                        span,
                    });
                    just_parsed_ternary = true;
                    just_parsed_elvis = current_is_elvis;
                    continue;
                }
                TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::MulEq
                | TokenKind::DivEq
                | TokenKind::ModEq
                | TokenKind::ConcatEq
                | TokenKind::AndEq
                | TokenKind::OrEq
                | TokenKind::XorEq
                | TokenKind::SlEq
                | TokenKind::SrEq
                | TokenKind::PowEq
                | TokenKind::CoalesceEq => {
                    let op = match self.current_token.kind {
                        TokenKind::PlusEq => AssignOp::Plus,
                        TokenKind::MinusEq => AssignOp::Minus,
                        TokenKind::MulEq => AssignOp::Mul,
                        TokenKind::DivEq => AssignOp::Div,
                        TokenKind::ModEq => AssignOp::Mod,
                        TokenKind::ConcatEq => AssignOp::Concat,
                        TokenKind::AndEq => AssignOp::BitAnd,
                        TokenKind::OrEq => AssignOp::BitOr,
                        TokenKind::XorEq => AssignOp::BitXor,
                        TokenKind::SlEq => AssignOp::ShiftLeft,
                        TokenKind::SrEq => AssignOp::ShiftRight,
                        TokenKind::PowEq => AssignOp::Pow,
                        TokenKind::CoalesceEq => AssignOp::Coalesce,
                        _ => unreachable!(),
                    };

                    let l_bp = 35; // Same as Assignment
                    if l_bp < min_bp && (min_bp >= 80 || !self.is_assignable(left)) {
                        break;
                    }

                    if !self.is_assignable(left) {
                        if self.is_reassociable_assignment_target(left) {
                            self.bump();
                            let right = self.parse_expr(l_bp - 1);
                            left = self.reassociate_assignment(left, right, Some(op));
                            continue;
                        }

                        self.errors.push(ParseError::new(
                            left.span(),
                            "Assignments can only happen to writable values",
                        ));
                    }

                    self.bump();
                    let right = self.parse_expr(l_bp - 1);
                    let span = Span::new(left.span().start, right.span().end);
                    left = self.arena.alloc(Expr::AssignOp {
                        var: left,
                        op,
                        expr: right,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Eq => {
                    // Assignment: $a = 1
                    let l_bp = 35; // Higher than 'and' (30), lower than 'ternary' (40)
                    if l_bp < min_bp {
                        // Special check for PHP grammar quirk:
                        // If LHS is assignable, assignment binds tighter than anything (effectively),
                        // because "expr = ..." is invalid, only "var = ..." is valid.
                        // However, this only applies to lower precedence operators (<= &&).
                        // Higher precedence operators (like &, |, +, ++, $) do not allow assignment on RHS.
                        if min_bp >= 80 || !self.is_assignable(left) {
                            break;
                        }
                    }

                    if !self.is_assignable(left) {
                        if self.is_reassociable_assignment_target(left) {
                            self.bump();
                            let right = self.parse_expr(l_bp - 1);
                            left = self.reassociate_assignment(left, right, None);
                            continue;
                        }

                        self.errors.push(ParseError::new(
                            left.span(),
                            "Assignments can only happen to writable values",
                        ));
                    }

                    self.bump();

                    // Assignment by reference: $a =& $b
                    if matches!(
                        self.current_token.kind,
                        TokenKind::Ampersand
                            | TokenKind::AmpersandFollowedByVarOrVararg
                            | TokenKind::AmpersandNotFollowedByVarOrVararg
                    ) {
                        self.bump();
                        let right = self.parse_expr(l_bp - 1);
                        let span = Span::new(left.span().start, right.span().end);
                        left = self.arena.alloc(Expr::AssignRef {
                            var: left,
                            expr: right,
                            span,
                        });
                        continue;
                    }

                    // Right associative
                    let right = self.parse_expr(l_bp - 1);

                    let span = Span::new(left.span().start, right.span().end);
                    left = self.arena.alloc(Expr::Assign {
                        var: left,
                        expr: right,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::OpenBracket => {
                    // Array Dimension Fetch: $a[1]
                    let l_bp = 210; // Very high
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let dim = if self.current_token.kind == TokenKind::CloseBracket {
                        None
                    } else {
                        Some(self.parse_expr(0))
                    };

                    let end = if self.current_token.kind == TokenKind::CloseBracket {
                        let end = self.current_token.span.end;
                        self.bump();
                        end
                    } else {
                        self.current_token.span.start
                    };

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::ArrayDimFetch {
                        array: left,
                        dim,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }

                TokenKind::NullSafeArrow => {
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let prop_or_method = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    if self.current_token.kind == TokenKind::OpenParen {
                        let (args, args_span) = self.parse_call_arguments();
                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::NullsafeMethodCall {
                            target: left,
                            method: prop_or_method,
                            args,
                            span,
                        });
                    } else {
                        let span = Span::new(left.span().start, prop_or_method.span().end);
                        left = self.arena.alloc(Expr::NullsafePropertyFetch {
                            target: left,
                            property: prop_or_method,
                            span,
                        });
                    }
                    continue;
                }
                TokenKind::Arrow => {
                    // Property Fetch or Method Call: $a->b or $a->b()
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    // Expect identifier or variable (for dynamic property)
                    // For now assume identifier
                    let prop_or_method = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        // We need to wrap this token in an Expr
                        // Reusing Variable/Identifier logic from parse_nud would be good but we need to call it explicitly or just handle it here
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            // Using Variable for now, should be Identifier if it's a name
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        // Error
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    // Check for method call
                    if self.current_token.kind == TokenKind::OpenParen {
                        let (args, args_span) = self.parse_call_arguments();

                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::MethodCall {
                            target: left,
                            method: prop_or_method,
                            args,
                            span,
                        });
                    } else {
                        // Property Fetch
                        let span = Span::new(left.span().start, prop_or_method.span().end);
                        left = self.arena.alloc(Expr::PropertyFetch {
                            target: left,
                            property: prop_or_method,
                            span,
                        });
                    }
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::DoubleColon => {
                    // Static Property/Method/Const: A::b, A::b(), A::CONST
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let member = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    if self.current_token.kind == TokenKind::OpenParen {
                        // Static Method Call
                        let (args, args_span) = self.parse_call_arguments();
                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::StaticCall {
                            class: left,
                            method: member,
                            args,
                            span,
                        });
                    } else {
                        // Class Const Fetch (or static property if it starts with $)
                        // For now assume const fetch if identifier
                        let span = Span::new(left.span().start, member.span().end);
                        left = self.arena.alloc(Expr::ClassConstFetch {
                            class: left,
                            constant: member,
                            span,
                        });
                    }
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::OpenParen => {
                    // Function Call
                    let l_bp = 190;
                    if l_bp < min_bp {
                        break;
                    }

                    let (args, args_span) = self.parse_call_arguments();

                    let span = Span::new(left.span().start, args_span.end);
                    left = self.arena.alloc(Expr::Call {
                        func: left,
                        args,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Inc => {
                    let l_bp = 180;
                    if l_bp < min_bp {
                        break;
                    }
                    let end = self.current_token.span.end;
                    self.bump();

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::PostInc { var: left, span });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Dec => {
                    let l_bp = 180;
                    if l_bp < min_bp {
                        break;
                    }
                    let end = self.current_token.span.end;
                    self.bump();

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::PostDec { var: left, span });
                    just_parsed_ternary = false;
                    continue;
                }
                _ => break,
            };

            let (l_bp, r_bp) = self.infix_binding_power(op);
            if l_bp < min_bp {
                break;
            }

            self.bump();
            let right = self.parse_expr(r_bp);

            let span = Span::new(left.span().start, right.span().end);
            left = self.arena.alloc(Expr::Binary {
                left,
                op,
                right,
                span,
            });
            just_parsed_ternary = false;
        }

        left
    }

    fn parse_nud(&mut self) -> ExprId<'ast> {
        let mut attributes = &[] as &'ast [AttributeGroup<'ast>];
        if self.current_token.kind == TokenKind::Attribute {
            attributes = self.parse_attributes();
        }

        let token = self.current_token;
        match token.kind {
            TokenKind::Lt => {
                if self.is_phpx() {
                    return self.parse_jsx_element();
                }
                self.errors
                    .push(ParseError::new(token.span, "Unexpected '<' in expression"));
                self.bump();
                self.arena.alloc(Expr::Error { span: token.span })
            }
            TokenKind::Empty => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Empty {
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Isset
            | TokenKind::LogicalOr
            | TokenKind::Insteadof
            | TokenKind::LogicalAnd
            | TokenKind::LogicalXor => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut vars = bumpalo::collections::Vec::new_in(self.arena);
                vars.push(self.parse_expr(0));
                while self.current_token.kind == TokenKind::Comma {
                    self.bump();
                    if self.current_token.kind == TokenKind::CloseParen {
                        break;
                    }
                    vars.push(self.parse_expr(0));
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Isset {
                    vars: vars.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::Eval => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Eval {
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Die | TokenKind::Exit => {
                let start = token.span.start;
                let is_die = token.kind == TokenKind::Die;
                self.bump();
                let expr = if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                    let e = if self.current_token.kind == TokenKind::CloseParen {
                        None
                    } else {
                        Some(self.parse_expr(0))
                    };
                    if self.current_token.kind == TokenKind::CloseParen {
                        self.bump();
                    }
                    e
                } else {
                    None
                };
                let end = self.current_token.span.end;
                let span = Span::new(start, end);
                if is_die {
                    self.arena.alloc(Expr::Die { expr, span })
                } else {
                    self.arena.alloc(Expr::Exit { expr, span })
                }
            }
            TokenKind::Dir
            | TokenKind::File
            | TokenKind::Line
            | TokenKind::FuncC
            | TokenKind::ClassC
            | TokenKind::TraitC
            | TokenKind::MethodC
            | TokenKind::NsC
            | TokenKind::PropertyC => {
                let span = token.span;
                self.bump();
                self.arena.alloc(Expr::MagicConst {
                    kind: match token.kind {
                        TokenKind::Dir => MagicConstKind::Dir,
                        TokenKind::File => MagicConstKind::File,
                        TokenKind::Line => MagicConstKind::Line,
                        TokenKind::FuncC => MagicConstKind::Function,
                        TokenKind::ClassC => MagicConstKind::Class,
                        TokenKind::TraitC => MagicConstKind::Trait,
                        TokenKind::MethodC => MagicConstKind::Method,
                        TokenKind::NsC => MagicConstKind::Namespace,
                        TokenKind::PropertyC => MagicConstKind::Property,
                        _ => unreachable!(),
                    },
                    span,
                })
            }
            TokenKind::Include
            | TokenKind::IncludeOnce
            | TokenKind::Require
            | TokenKind::RequireOnce => {
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(0);
                let end = expr.span().end;
                self.arena.alloc(Expr::Include {
                    kind: match token.kind {
                        TokenKind::Include => IncludeKind::Include,
                        TokenKind::IncludeOnce => IncludeKind::IncludeOnce,
                        TokenKind::Require => IncludeKind::Require,
                        TokenKind::RequireOnce => IncludeKind::RequireOnce,
                        _ => unreachable!(),
                    },
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Print => {
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(31);
                let span = Span::new(start, expr.span().end);
                self.arena.alloc(Expr::Print { expr, span })
            }
            TokenKind::Yield | TokenKind::YieldFrom => {
                let start = token.span.start;
                self.bump();

                let mut is_from = token.kind == TokenKind::YieldFrom;
                if !is_from && self.current_token.kind == TokenKind::Identifier {
                    let text = self.lexer.slice(self.current_token.span);
                    let mut lowered = text.to_vec();
                    lowered.make_ascii_lowercase();
                    if lowered == b"from" {
                        is_from = true;
                        self.bump(); // consume 'from'
                    }
                }

                if is_from {
                    let value = self.parse_expr(31);
                    let span = Span::new(start, value.span().end);
                    return self.arena.alloc(Expr::Yield {
                        key: None,
                        value: Some(value),
                        from: true,
                        span,
                    });
                }

                if matches!(
                    self.current_token.kind,
                    TokenKind::SemiColon
                        | TokenKind::CloseTag
                        | TokenKind::Eof
                        | TokenKind::CloseBrace
                        | TokenKind::Comma
                ) {
                    let span = Span::new(start, self.current_token.span.start);
                    return self.arena.alloc(Expr::Yield {
                        key: None,
                        value: None,
                        from: false,
                        span,
                    });
                }

                let first = self.parse_expr(31);
                let (key, value) = if self.current_token.kind == TokenKind::DoubleArrow {
                    self.bump();
                    let val = self.parse_expr(31);
                    (Some(first), val)
                } else {
                    (None, first)
                };
                let span = Span::new(start, value.span().end);
                self.arena.alloc(Expr::Yield {
                    key,
                    value: Some(value),
                    from: false,
                    span,
                })
            }

            TokenKind::Throw => {
                // Throw expression (PHP 8+): reuse error node to avoid a new variant
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(0);
                let span = Span::new(start, expr.span().end);
                self.arena.alloc(Expr::Error { span })
            }

            TokenKind::Function => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                self.parse_closure_expr(attributes, false, false, start)
            }
            TokenKind::Fn => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                self.parse_arrow_function(attributes, false, false, start)
            }
            TokenKind::Static => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                match self.current_token.kind {
                    TokenKind::Function => {
                        self.bump();
                        self.parse_closure_expr(attributes, false, true, start)
                    }
                    TokenKind::Fn => {
                        self.bump();
                        self.parse_arrow_function(attributes, false, true, start)
                    }
                    TokenKind::DoubleColon => {
                        // static scope resolution (e.g., static::CONST)
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    }
                    _ => self.arena.alloc(Expr::Variable {
                        name: token.span,
                        span: token.span,
                    }),
                }
            }
            TokenKind::Identifier if self.token_eq_ident(&token, b"await") => {
                if !self.is_phpx() {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "await is only available in PHPX mode",
                        "Use a .phpx file for async/await support.",
                    ));
                } else if self.fn_depth > 0 && self.async_fn_depth == 0 {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "await is only allowed in async functions",
                        "Mark the function as async or move await to module top level.",
                    ));
                }
                self.bump();
                let awaited = self.parse_expr(180);
                let span = Span::new(token.span.start, awaited.span().end);
                self.arena.alloc(Expr::Await {
                    expr: awaited,
                    span,
                })
            }
            TokenKind::Identifier
                if self.is_phpx()
                    && self.token_eq_ident(&token, b"async")
                    && self.next_token.kind == TokenKind::Function =>
            {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump(); // async
                self.bump(); // function
                self.parse_closure_expr(attributes, true, false, start)
            }
            TokenKind::Identifier
                if self.is_phpx()
                    && self.token_eq_ident(&token, b"async")
                    && self.next_token.kind == TokenKind::Fn =>
            {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump(); // async
                self.bump(); // fn
                self.parse_arrow_function(attributes, true, false, start)
            }
            TokenKind::New => {
                if self.is_phpx() {
                    self.errors.push(ParseError::new(
                        token.span,
                        "new is not allowed in PHPX; use struct literals instead",
                    ));
                }
                self.bump();

                let attributes = if self.current_token.kind == TokenKind::Attribute {
                    self.parse_attributes()
                } else {
                    &[]
                };

                // Parse optional modifiers for anonymous class
                let mut modifiers = std::vec::Vec::new();
                while matches!(
                    self.current_token.kind,
                    TokenKind::Abstract | TokenKind::Final | TokenKind::Readonly
                ) {
                    modifiers.push(self.current_token);
                    self.bump();
                }

                if self.current_token.kind == TokenKind::Class {
                    let (class, args) = self
                        .parse_anonymous_class(attributes, self.arena.alloc_slice_copy(&modifiers));
                    let span = Span::new(token.span.start, class.span().end);
                    self.arena.alloc(Expr::New { class, args, span })
                } else {
                    if !attributes.is_empty() || !modifiers.is_empty() {
                        let start = if let Some(attr) = attributes.first() {
                            attr.span.start
                        } else {
                            modifiers.first().unwrap().span.start
                        };
                        let end = if let Some(attr) = attributes.last() {
                            attr.span.end
                        } else {
                            modifiers.last().unwrap().span.end
                        };
                        self.errors.push(ParseError::new(Span::new(start, end), "Attributes and modifiers are only allowed on anonymous classes in new expression"));
                    }

                    let class = self.parse_expr(200); // High binding power to grab the class name

                    let (args, end_pos) = if self.current_token.kind == TokenKind::OpenParen {
                        let (a, s) = self.parse_call_arguments();
                        (a, s.end)
                    } else {
                        (&[] as &[Arg], class.span().end)
                    };

                    let span = Span::new(token.span.start, end_pos);
                    self.arena.alloc(Expr::New { class, args, span })
                }
            }
            TokenKind::Clone => {
                self.bump();
                let expr = self.parse_expr(200);
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Clone { expr, span })
            }
            TokenKind::Match => {
                let start = token.span.start;
                self.bump(); // Eat match

                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let condition = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }

                if self.current_token.kind == TokenKind::OpenBrace {
                    self.bump();
                }

                let mut arms = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::SemiColon {
                        self.errors
                            .push(ParseError::new(self.current_token.span, "Unexpected ';'"));
                        self.bump();
                        continue;
                    }

                    let arm_start = self.current_token.span.start;

                    let conditions = if self.current_token.kind == TokenKind::Default {
                        self.bump();
                        None
                    } else {
                        let mut conds = bumpalo::collections::Vec::new_in(self.arena);
                        conds.push(self.parse_expr(0));
                        while self.current_token.kind == TokenKind::Comma {
                            self.bump();
                            if self.current_token.kind == TokenKind::DoubleArrow {
                                break;
                            }
                            conds.push(self.parse_expr(0));
                        }
                        Some(conds.into_bump_slice() as &'ast [ExprId<'ast>])
                    };

                    if self.current_token.kind == TokenKind::DoubleArrow {
                        self.bump();
                    }

                    let body = self.parse_expr(0);

                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }

                    let arm_end = body.span().end;

                    arms.push(MatchArm {
                        conditions,
                        body,
                        span: Span::new(arm_start, arm_end),
                    });
                }

                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                }

                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Match {
                    condition,
                    arms: arms.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::Dollar => {
                let start = self.current_token.span.start;
                self.bump();

                if self.current_token.kind == TokenKind::OpenBrace {
                    self.bump();
                    let expr = self.parse_expr(0);
                    let end = if self.current_token.kind == TokenKind::CloseBrace {
                        let end = self.current_token.span.end;
                        self.bump();
                        end
                    } else {
                        self.current_token.span.start
                    };

                    let span = Span::new(start, end);
                    self.arena
                        .alloc(Expr::IndirectVariable { name: expr, span })
                } else {
                    let expr = self.parse_expr(200);
                    let span = Span::new(start, expr.span().end);
                    self.arena
                        .alloc(Expr::IndirectVariable { name: expr, span })
                }
            }
            TokenKind::StringVarname => {
                self.bump();
                self.arena.alloc(Expr::Variable {
                    name: token.span,
                    span: token.span,
                })
            }
            TokenKind::Variable => {
                self.bump();
                self.arena.alloc(Expr::Variable {
                    name: token.span,
                    span: token.span,
                })
            }
            TokenKind::LNumber => {
                self.bump();
                self.arena.alloc(Expr::Integer {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::DNumber => {
                self.bump();
                self.arena.alloc(Expr::Float {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::StringLiteral => {
                self.bump();
                self.arena.alloc(Expr::String {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::DoubleQuote => self.parse_interpolated_string(TokenKind::DoubleQuote),
            TokenKind::StartHeredoc => self.parse_interpolated_string(TokenKind::EndHeredoc),
            TokenKind::Backtick => self.parse_interpolated_string(TokenKind::Backtick),
            TokenKind::TypeTrue => {
                self.bump();
                self.arena.alloc(Expr::Boolean {
                    value: true,
                    span: token.span,
                })
            }
            TokenKind::TypeFalse => {
                self.bump();
                self.arena.alloc(Expr::Boolean {
                    value: false,
                    span: token.span,
                })
            }
            TokenKind::TypeNull => {
                self.bump();
                self.arena.alloc(Expr::Null { span: token.span })
            }
            TokenKind::Identifier
            | TokenKind::Namespace
            | TokenKind::NsSeparator
            | TokenKind::Enum
            | TokenKind::TypeInt
            | TokenKind::TypeFloat
            | TokenKind::TypeBool
            | TokenKind::TypeString
            | TokenKind::TypeVoid
            | TokenKind::TypeNever
            | TokenKind::TypeMixed
            | TokenKind::TypeIterable
            | TokenKind::TypeObject
            | TokenKind::TypeCallable
            | TokenKind::Readonly => {
                let name = self.parse_name();
                if self.is_phpx() && self.current_token.kind == TokenKind::OpenBrace {
                    return self.parse_struct_literal(name, name.span.start);
                }
                self.arena.alloc(Expr::Variable {
                    name: name.span,
                    span: name.span,
                })
            }
            TokenKind::Bang => {
                self.bump();
                let expr = self.parse_expr(160); // BP for !
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Unary {
                    op: UnaryOp::Not,
                    expr,
                    span,
                })
            }
            TokenKind::Minus
            | TokenKind::Plus
            | TokenKind::BitNot
            | TokenKind::At
            | TokenKind::Inc
            | TokenKind::Dec
            | TokenKind::Ampersand
            | TokenKind::AmpersandFollowedByVarOrVararg
            | TokenKind::AmpersandNotFollowedByVarOrVararg => {
                let op = match token.kind {
                    TokenKind::Minus => UnaryOp::Minus,
                    TokenKind::Plus => UnaryOp::Plus,
                    TokenKind::BitNot => UnaryOp::BitNot,
                    TokenKind::At => UnaryOp::ErrorSuppress,
                    TokenKind::Inc => UnaryOp::PreInc,
                    TokenKind::Dec => UnaryOp::PreDec,
                    TokenKind::Ampersand
                    | TokenKind::AmpersandFollowedByVarOrVararg
                    | TokenKind::AmpersandNotFollowedByVarOrVararg => UnaryOp::Reference,
                    _ => unreachable!(),
                };
                self.bump();
                let expr = self.parse_expr(180); // BP for unary +, -, ~, ++, --
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Unary { op, expr, span })
            }
            TokenKind::IntCast
            | TokenKind::BoolCast
            | TokenKind::FloatCast
            | TokenKind::StringCast
            | TokenKind::ArrayCast
            | TokenKind::ObjectCast
            | TokenKind::UnsetCast
            | TokenKind::VoidCast => {
                let kind = match token.kind {
                    TokenKind::IntCast => CastKind::Int,
                    TokenKind::BoolCast => CastKind::Bool,
                    TokenKind::FloatCast => CastKind::Float,
                    TokenKind::StringCast => CastKind::String,
                    TokenKind::ArrayCast => CastKind::Array,
                    TokenKind::ObjectCast => CastKind::Object,
                    TokenKind::UnsetCast => CastKind::Unset,
                    TokenKind::VoidCast => CastKind::Void,
                    _ => unreachable!(),
                };
                self.bump();
                let expr = self.parse_expr(180); // BP for casts (same as unary)
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Cast { kind, expr, span })
            }
            TokenKind::Array => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseParen
                    && self.current_token.kind != TokenKind::Eof
                {
                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::List => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseParen
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::Comma {
                        // Empty slot in list()
                        items.push(ArrayItem {
                            key: None,
                            value: self.arena.alloc(Expr::Error {
                                span: self.current_token.span,
                            }),
                            by_ref: false,
                            unpack: false,
                            span: self.current_token.span,
                        });
                        self.bump();
                        continue;
                    }
                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                        // allow trailing comma
                        if self.current_token.kind == TokenKind::CloseParen {
                            break;
                        }
                    }
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenBracket => {
                // Short array syntax [1, 2, 3]
                let start = token.span.start;
                self.bump();
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBracket
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::Comma {
                        // Empty slot in short array destructuring [a, , b]
                        items.push(ArrayItem {
                            key: None,
                            value: self.arena.alloc(Expr::Error {
                                span: self.current_token.span,
                            }),
                            by_ref: false,
                            unpack: false,
                            span: self.current_token.span,
                        });
                        self.bump();
                        continue;
                    }

                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }
                }
                if self.current_token.kind == TokenKind::CloseBracket {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenBrace => {
                if !self.is_phpx() {
                    self.bump();
                    return self.arena.alloc(Expr::Error { span: token.span });
                }

                let start = token.span.start;
                self.bump(); // consume {
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    let (key, key_start) = match self.current_token.kind {
                        TokenKind::Identifier => {
                            let tok = self.arena.alloc(self.current_token);
                            self.bump();
                            (ObjectKey::Ident(tok), tok.span.start)
                        }
                        TokenKind::StringLiteral => {
                            let tok = self.arena.alloc(self.current_token);
                            self.bump();
                            (ObjectKey::String(tok), tok.span.start)
                        }
                        _ if self.current_token.kind.is_semi_reserved() => {
                            let tok = self.arena.alloc(self.current_token);
                            self.bump();
                            (ObjectKey::Ident(tok), tok.span.start)
                        }
                        _ => {
                            self.errors.push(ParseError::new(
                                self.current_token.span,
                                "Expected identifier or string literal in object literal",
                            ));
                            let tok = self.arena.alloc(Token {
                                kind: TokenKind::Error,
                                span: self.current_token.span,
                            });
                            self.bump();
                            (ObjectKey::Ident(tok), tok.span.start)
                        }
                    };

                    if self.current_token.kind == TokenKind::Colon {
                        self.bump();
                    } else {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected ':' after object key",
                        ));
                    }

                    let value = self.parse_expr(0);
                    let span = Span::new(key_start, value.span().end);
                    items.push(ObjectItem { key, value, span });

                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                        if self.current_token.kind == TokenKind::CloseBrace {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                let end = if self.current_token.kind == TokenKind::CloseBrace {
                    let end = self.current_token.span.end;
                    self.bump();
                    end
                } else {
                    self.current_token.span.end
                };

                self.arena.alloc(Expr::ObjectLiteral {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenParen => {
                self.bump();
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                expr
            }
            TokenKind::Error => {
                self.errors
                    .push(ParseError::new(token.span, "Unexpected token"));
                self.bump();
                self.arena.alloc(Expr::Error { span: token.span })
            }
            _ => {
                // Error recovery
                let is_terminator = matches!(
                    token.kind,
                    TokenKind::SemiColon
                        | TokenKind::CloseBrace
                        | TokenKind::CloseTag
                        | TokenKind::Eof
                );

                self.errors
                    .push(ParseError::new(token.span, "Syntax error"));

                if is_terminator {
                    // Do not consume terminator, let the statement parser handle it
                    self.arena.alloc(Expr::Error {
                        span: Span::new(token.span.start, token.span.start),
                    })
                } else {
                    self.bump();
                    self.arena.alloc(Expr::Error { span: token.span })
                }
            }
        }
    }

    fn parse_struct_literal(&mut self, name: Name<'ast>, start: usize) -> ExprId<'ast> {
        self.bump(); // consume {
        let mut fields = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }

            let name_token = if self.current_token.kind == TokenKind::Variable {
                let tok = self.arena.alloc(self.current_token);
                self.bump();
                tok
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected field name in struct literal",
                ));
                let tok = self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: self.current_token.span,
                });
                self.bump();
                tok
            };

            let value = if self.current_token.kind == TokenKind::Colon
                || self.current_token.kind == TokenKind::Eq
            {
                if self.current_token.kind == TokenKind::Eq {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected ':' after struct field name",
                    ));
                }
                self.bump();
                self.parse_expr(0)
            } else {
                self.arena.alloc(Expr::Variable {
                    name: name_token.span,
                    span: name_token.span,
                })
            };

            let span = Span::new(name_token.span.start, value.span().end);
            fields.push(StructLiteralField {
                name: name_token,
                value,
                span,
            });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                if self.current_token.kind == TokenKind::CloseBrace {
                    break;
                }
            } else {
                break;
            }
        }

        let end = if self.current_token.kind == TokenKind::CloseBrace {
            let end = self.current_token.span.end;
            self.bump();
            end
        } else {
            self.current_token.span.end
        };

        self.arena.alloc(Expr::StructLiteral {
            name,
            fields: fields.into_bump_slice(),
            span: Span::new(start, end),
        })
    }

    fn parse_jsx_element(&mut self) -> ExprId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // consume '<'

        if self.current_token.kind == TokenKind::Gt {
            self.bump(); // consume '>'
            let (children, end) = self.parse_jsx_children(None);
            return self.arena.alloc(Expr::JsxFragment {
                children,
                span: Span::new(start, end),
            });
        }

        let name = self.parse_name();
        let mut attributes = bumpalo::collections::Vec::new_in(self.arena);

        loop {
            if self.current_token.kind == TokenKind::Slash && self.next_token.kind == TokenKind::Gt
            {
                let end = self.next_token.span.end;
                self.bump(); // consume '/'
                self.bump(); // consume '>'
                return self.arena.alloc(Expr::JsxElement {
                    name,
                    attributes: attributes.into_bump_slice(),
                    children: &[],
                    span: Span::new(start, end),
                });
            }

            if self.current_token.kind == TokenKind::Gt {
                self.bump(); // consume '>'
                let (children, end) = self.parse_jsx_children(Some(name));
                return self.arena.alloc(Expr::JsxElement {
                    name,
                    attributes: attributes.into_bump_slice(),
                    children,
                    span: Span::new(start, end),
                });
            }

            if self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved()
            {
                let attr_start = self.current_token.span.start;
                let mut attr_end = self.current_token.span.end;
                self.bump();

                if self.current_token.kind == TokenKind::Colon {
                    let colon_span = self.current_token.span;
                    self.bump();
                    if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind.is_semi_reserved()
                    {
                        attr_end = self.current_token.span.end;
                        self.bump();
                    } else {
                        self.errors.push(ParseError::new(
                            colon_span,
                            "Expected identifier after ':' in JSX attribute",
                        ));
                    }
                }

                let attr_name = self.arena.alloc(Token {
                    kind: TokenKind::Identifier,
                    span: Span::new(attr_start, attr_end),
                });

                let mut value = None;
                let mut end = attr_name.span.end;
                if self.current_token.kind == TokenKind::Eq {
                    self.bump();
                    value = self.parse_jsx_attribute_value();
                    if let Some(val) = value {
                        end = val.span().end;
                    }
                }
                attributes.push(JsxAttribute {
                    name: attr_name,
                    value,
                    span: Span::new(attr_name.span.start, end),
                });
                continue;
            }

            if self.current_token.kind == TokenKind::OpenBrace {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "JSX spread attributes are not supported",
                ));
                // Attempt recovery: skip until closing brace
                self.bump();
                let _ = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                }
                continue;
            }

            if self.current_token.kind == TokenKind::Eof {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Unterminated JSX element",
                ));
                return self.arena.alloc(Expr::Error {
                    span: Span::new(start, self.current_token.span.end),
                });
            }

            self.errors.push(ParseError::new(
                self.current_token.span,
                "Unexpected token in JSX attributes",
            ));
            self.bump();
        }
    }

    fn parse_jsx_attribute_value(&mut self) -> Option<ExprId<'ast>> {
        match self.current_token.kind {
            TokenKind::StringLiteral => {
                let tok = self.current_token;
                self.bump();
                Some(self.arena.alloc(Expr::String {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(tok.span)),
                    span: tok.span,
                }))
            }
            TokenKind::OpenBrace => {
                self.bump(); // consume '{'
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Empty JSX expression is not allowed",
                    ));
                    self.bump();
                    return None;
                }
                if self.jsx_starts_object_literal() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Object literal requires double braces in JSX",
                    ));
                }
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected '}' after JSX expression",
                    ));
                }
                Some(expr)
            }
            _ => {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected JSX attribute value",
                ));
                None
            }
        }
    }

    fn parse_jsx_children(
        &mut self,
        closing_name: Option<Name<'ast>>,
    ) -> (&'ast [JsxChild<'ast>], usize) {
        let mut children = bumpalo::collections::Vec::new_in(self.arena);
        let mut text_start = self.current_token.span.start;
        let raw_text_mode = closing_name
            .as_ref()
            .map(|name| self.jsx_is_raw_text_element(name))
            .unwrap_or(false);

        loop {
            if self.current_token.kind == TokenKind::Lt && self.next_token.kind == TokenKind::Slash
            {
                let end = self.current_token.span.start;
                self.push_jsx_text(text_start, end, &mut children);

                self.bump(); // consume '<'
                self.bump(); // consume '/'

                if closing_name.is_none() {
                    if self.current_token.kind == TokenKind::Gt {
                        let end = self.current_token.span.end;
                        self.bump();
                        return (children.into_bump_slice(), end);
                    }
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected '>' to close JSX fragment",
                    ));
                    return (children.into_bump_slice(), self.current_token.span.end);
                }

                let close_name = self.parse_name();
                if let Some(expected) = closing_name.as_ref() {
                    if !self.jsx_name_eq(expected, &close_name) {
                        self.errors.push(ParseError::new(
                            close_name.span,
                            "Mismatched JSX closing tag",
                        ));
                    }
                }

                if self.current_token.kind == TokenKind::Gt {
                    let end = self.current_token.span.end;
                    self.bump();
                    return (children.into_bump_slice(), end);
                }

                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected '>' to close JSX tag",
                ));
                return (children.into_bump_slice(), self.current_token.span.end);
            }

            if self.current_token.kind == TokenKind::Lt {
                if raw_text_mode {
                    self.bump();
                    continue;
                }
                let end = self.current_token.span.start;
                self.push_jsx_text(text_start, end, &mut children);
                let child = self.parse_jsx_element();
                children.push(JsxChild::Expr(child));
                text_start = self.current_token.span.start;
                continue;
            }

            if self.current_token.kind == TokenKind::OpenBrace {
                if raw_text_mode {
                    self.bump();
                    continue;
                }
                let end = self.current_token.span.start;
                self.push_jsx_text(text_start, end, &mut children);

                self.bump(); // consume '{'
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Empty JSX expression is not allowed",
                    ));
                    self.bump();
                } else {
                    if self.jsx_starts_object_literal() {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Object literal requires double braces in JSX",
                        ));
                    }
                    let expr = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    } else {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected '}' after JSX expression",
                        ));
                    }
                    children.push(JsxChild::Expr(expr));
                }

                text_start = self.current_token.span.start;
                continue;
            }

            if self.current_token.kind == TokenKind::Eof {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Unterminated JSX children",
                ));
                return (children.into_bump_slice(), self.current_token.span.end);
            }

            self.bump();
        }
    }

    fn push_jsx_text(
        &mut self,
        start: usize,
        end: usize,
        children: &mut bumpalo::collections::Vec<JsxChild<'ast>>,
    ) {
        if end <= start {
            return;
        }

        // Lexer tokenization skips spaces/tabs between tokens; include that
        // boundary whitespace so inline JSX text spacing is preserved.
        let mut actual_end = end;
        while actual_end < self.lexer.input_len() {
            let byte = self
                .lexer
                .input_slice(Span::new(actual_end, actual_end + 1))[0];
            if byte == b' ' || byte == b'\t' {
                actual_end += 1;
                continue;
            }
            break;
        }

        let span = Span::new(start, actual_end);
        let raw = self.lexer.input_slice(span);
        if raw.iter().all(|b| b.is_ascii_whitespace()) {
            return;
        }
        children.push(JsxChild::Text(span));
    }

    fn jsx_name_eq(&self, a: &Name<'ast>, b: &Name<'ast>) -> bool {
        if a.parts.len() != b.parts.len() {
            return false;
        }
        a.parts.iter().zip(b.parts.iter()).all(|(x, y)| {
            self.lexer
                .slice(x.span)
                .eq_ignore_ascii_case(self.lexer.slice(y.span))
        })
    }

    fn jsx_is_raw_text_element(&self, name: &Name<'ast>) -> bool {
        if name.parts.len() != 1 {
            return false;
        }
        let ident = self.lexer.slice(name.parts[0].span);
        ident.eq_ignore_ascii_case(b"style") || ident.eq_ignore_ascii_case(b"script")
    }

    fn jsx_starts_object_literal(&self) -> bool {
        matches!(
            self.current_token.kind,
            TokenKind::Identifier | TokenKind::StringLiteral
        ) && self.next_token.kind == TokenKind::Colon
    }

    fn infix_binding_power(&self, op: BinaryOp) -> (u8, u8) {
        match op {
            BinaryOp::LogicalOr => (10, 11),
            BinaryOp::LogicalXor => (20, 21),
            BinaryOp::LogicalAnd => (30, 31),

            BinaryOp::Coalesce => (51, 50), // Right associative

            BinaryOp::Or => (60, 61),  // ||
            BinaryOp::And => (70, 71), // &&

            BinaryOp::BitOr => (80, 81),
            BinaryOp::BitXor => (90, 91),
            BinaryOp::BitAnd => (100, 101),

            BinaryOp::EqEq
            | BinaryOp::NotEq
            | BinaryOp::EqEqEq
            | BinaryOp::NotEqEq
            | BinaryOp::Spaceship => (110, 111),
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => (120, 121),

            BinaryOp::Pipe => (125, 126),

            BinaryOp::ShiftLeft | BinaryOp::ShiftRight => (130, 131),

            BinaryOp::Plus | BinaryOp::Minus | BinaryOp::Concat => (140, 141),
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => (150, 151),

            BinaryOp::Instanceof => (170, 171), // Non-associative usually, but let's say left for now

            BinaryOp::Pow => (191, 190), // Right associative

            _ => (0, 0),
        }
    }

    fn parse_array_item(&mut self) -> ArrayItem<'ast> {
        let unpack = if self.current_token.kind == TokenKind::Ellipsis {
            self.bump();
            true
        } else {
            false
        };

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

        let expr1 = self.parse_expr(0);

        if self.current_token.kind == TokenKind::DoubleArrow {
            self.bump();
            let value_by_ref = if matches!(
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
            let value = self.parse_expr(0);
            ArrayItem {
                key: Some(expr1),
                value,
                by_ref: value_by_ref,
                unpack,
                span: Span::new(expr1.span().start, value.span().end),
            }
        } else {
            ArrayItem {
                key: None,
                value: expr1,
                by_ref,
                unpack,
                span: expr1.span(),
            }
        }
    }

    fn parse_interpolated_string(&mut self, end_token: TokenKind) -> ExprId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat opening token

        let mut parts: bumpalo::collections::Vec<&'ast Expr<'ast>> =
            bumpalo::collections::Vec::new_in(self.arena);

        while self.current_token.kind != end_token && self.current_token.kind != TokenKind::Eof {
            match self.current_token.kind {
                TokenKind::EncapsedAndWhitespace => {
                    let token = self.current_token;
                    self.bump();
                    parts.push(self.arena.alloc(Expr::String {
                        value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                        span: token.span,
                    }));
                }
                TokenKind::Variable => {
                    let token = self.current_token;
                    self.bump();
                    let var_expr = self.arena.alloc(Expr::Variable {
                        name: token.span,
                        span: token.span,
                    }) as &'ast Expr<'ast>;

                    // Check for array offset
                    if self.current_token.kind == TokenKind::OpenBracket {
                        self.bump(); // [

                        // Key
                        let key = match self.current_token.kind {
                            TokenKind::Identifier => {
                                let t = self.current_token;
                                self.bump();
                                self.arena.alloc(Expr::String {
                                    value: self.arena.alloc_slice_copy(self.lexer.slice(t.span)),
                                    span: t.span,
                                }) as &'ast Expr<'ast>
                            }
                            TokenKind::NumString => {
                                let t = self.current_token;
                                self.bump();
                                self.arena.alloc(Expr::Integer {
                                    value: self.arena.alloc_slice_copy(self.lexer.slice(t.span)),
                                    span: t.span,
                                }) as &'ast Expr<'ast>
                            }
                            TokenKind::Variable => {
                                let t = self.current_token;
                                self.bump();
                                self.arena.alloc(Expr::Variable {
                                    name: t.span,
                                    span: t.span,
                                }) as &'ast Expr<'ast>
                            }
                            TokenKind::Minus => {
                                // Handle negative number?
                                let minus = self.current_token;
                                self.bump();
                                if self.current_token.kind == TokenKind::NumString {
                                    let t = self.current_token;
                                    self.bump();

                                    let mut value = bumpalo::collections::Vec::with_capacity_in(
                                        (minus.span.end - minus.span.start)
                                            + (t.span.end - t.span.start),
                                        self.arena,
                                    );
                                    value.extend_from_slice(self.lexer.slice(minus.span));
                                    value.extend_from_slice(self.lexer.slice(t.span));

                                    self.arena.alloc(Expr::Integer {
                                        value: value.into_bump_slice(),
                                        span: Span::new(minus.span.start, t.span.end),
                                    }) as &'ast Expr<'ast>
                                } else {
                                    self.arena.alloc(Expr::Error {
                                        span: self.current_token.span,
                                    }) as &'ast Expr<'ast>
                                }
                            }
                            _ => {
                                // Error
                                self.arena.alloc(Expr::Error {
                                    span: self.current_token.span,
                                }) as &'ast Expr<'ast>
                            }
                        };

                        if self.current_token.kind == TokenKind::CloseBracket {
                            self.bump();
                        }

                        parts.push(self.arena.alloc(Expr::ArrayDimFetch {
                            array: var_expr,
                            dim: Some(key),
                            span: Span::new(token.span.start, self.current_token.span.end),
                        }));
                    } else if self.current_token.kind == TokenKind::Arrow {
                        // Property fetch $foo->bar
                        self.bump();
                        if self.current_token.kind == TokenKind::Identifier {
                            let prop_name = self.current_token;
                            self.bump();

                            parts.push(self.arena.alloc(Expr::PropertyFetch {
                                target: var_expr,
                                property: self.arena.alloc(Expr::Variable {
                                    name: prop_name.span,
                                    span: prop_name.span,
                                }),
                                span: Span::new(token.span.start, prop_name.span.end),
                            }));
                        } else {
                            parts.push(var_expr);
                        }
                    } else {
                        parts.push(var_expr);
                    }
                }
                TokenKind::CurlyOpen => {
                    self.bump();
                    let expr = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    }
                    parts.push(expr);
                }
                TokenKind::DollarOpenCurlyBraces => {
                    self.bump();
                    // ${expr}
                    let expr = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    }
                    parts.push(expr);
                }
                _ => {
                    // Unexpected token inside string
                    let token = self.current_token;
                    self.bump();
                    parts.push(self.arena.alloc(Expr::Error { span: token.span }));
                }
            }
        }

        let end = if self.current_token.kind == end_token {
            let end = self.current_token.span.end;
            self.bump();
            end
        } else {
            self.current_token.span.start
        };

        let span = Span::new(start, end);
        let parts = parts.into_bump_slice();

        if end_token == TokenKind::Backtick {
            self.arena.alloc(Expr::ShellExec { parts, span })
        } else {
            self.arena.alloc(Expr::InterpolatedString { parts, span })
        }
    }
}
