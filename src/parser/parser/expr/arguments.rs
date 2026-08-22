use super::super::Parser;
use crate::parser::ast::{
    Arg, BinaryOp, ClosureUse, Expr, ExprId, ObjectItem, ObjectKey,
    Param, ParseError, Stmt, Type,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_call_arguments(&mut self) -> (&'ast [Arg<'ast>], Span) {
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

    pub(in crate::parser::parser) fn parse_phpx_param_pattern(&mut self) -> Option<ExprId<'ast>> {
        match self.current_token.kind {
            TokenKind::OpenBracket | TokenKind::List => Some(self.parse_expr(0)),
            TokenKind::OpenBrace => Some(self.parse_phpx_object_pattern()),
            _ => None,
        }
    }

    pub(in crate::parser::parser) fn pattern_last_binding(
        &self,
        pattern: ExprId<'ast>,
    ) -> Option<&'ast Token> {
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

    pub(in crate::parser::parser) fn push_param_pattern_prologue(
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
                    "Use variable, array, or object patterns in DekaScript destructuring.",
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

            if self.current_token.kind == TokenKind::Variable
                || (self.is_ds() && self.current_token.kind == TokenKind::Identifier)
            {
                // Shorthand: { $name } -> { name: $name } in PHPX,
                //            { name }  -> { name: name }  in DekaScript.
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
        if self.is_ds() {
            // DekaScript function return types are written without a colon.
            // Accept an optional colon for backward compatibility with existing
            // DekaScript sources, then parse the type if present.
            if self.current_token.kind == TokenKind::Colon {
                self.bump();
            }
            if let Some(t) = self.parse_type() {
                Some(self.arena.alloc(t) as &'ast Type<'ast>)
            } else {
                None
            }
        } else if self.current_token.kind == TokenKind::Colon {
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
}
