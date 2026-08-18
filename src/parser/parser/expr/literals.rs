use super::super::Parser;
use crate::parser::ast::{
    ArrayItem, BinaryOp, Expr, ExprId, JsxAttribute, JsxChild, Name, ParseError, StructLiteralField,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_struct_literal(
        &mut self,
        name: Name<'ast>,
        start: usize,
    ) -> ExprId<'ast> {
        self.bump(); // consume {
        let mut fields = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }

            let is_field_name_token = if self.is_ds() {
                matches!(
                    self.current_token.kind,
                    TokenKind::Identifier | TokenKind::Variable
                )
            } else {
                self.current_token.kind == TokenKind::Variable
            };
            let name_token = if is_field_name_token {
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

    pub(in crate::parser::parser) fn parse_jsx_element(&mut self) -> ExprId<'ast> {
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
                let brace_start = self.current_token.span.start;
                self.bump(); // consume {
                if self.current_token.kind == TokenKind::Ellipsis {
                    let ellipsis_tok = self.arena.alloc(self.current_token);
                    let ellipsis_start = self.current_token.span.start;
                    self.bump(); // ...
                    let expr = self.parse_expr(0);
                    let spread = self.arena.alloc(Expr::Spread {
                        expr,
                        span: Span::new(ellipsis_start, expr.span().end),
                    });
                    let end = if self.current_token.kind == TokenKind::CloseBrace {
                        self.current_token.span.end
                    } else {
                        spread.span().end
                    };
                    attributes.push(JsxAttribute {
                        name: ellipsis_tok,
                        value: Some(spread),
                        span: Span::new(brace_start, end),
                    });
                } else {
                    self.errors.push(ParseError::new(
                        Span::new(brace_start, self.current_token.span.start),
                        "JSX expression attributes require a name, use `{...expr}` for spread",
                    ));
                    if self.current_token.kind != TokenKind::CloseBrace {
                        let _ = self.parse_expr(0);
                    }
                }
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

    pub(in crate::parser::parser) fn parse_jsx_attribute_value(&mut self) -> Option<ExprId<'ast>> {
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

    pub(in crate::parser::parser) fn parse_jsx_children(
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

    pub(in crate::parser::parser) fn push_jsx_text(
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

    pub(in crate::parser::parser) fn jsx_name_eq(&self, a: &Name<'ast>, b: &Name<'ast>) -> bool {
        if a.parts.len() != b.parts.len() {
            return false;
        }
        a.parts.iter().zip(b.parts.iter()).all(|(x, y)| {
            self.lexer
                .slice(x.span)
                .eq_ignore_ascii_case(self.lexer.slice(y.span))
        })
    }

    pub(in crate::parser::parser) fn jsx_is_raw_text_element(&self, name: &Name<'ast>) -> bool {
        if name.parts.len() != 1 {
            return false;
        }
        let ident = self.lexer.slice(name.parts[0].span);
        ident.eq_ignore_ascii_case(b"style") || ident.eq_ignore_ascii_case(b"script")
    }

    pub(in crate::parser::parser) fn jsx_starts_object_literal(&self) -> bool {
        matches!(
            self.current_token.kind,
            TokenKind::Identifier | TokenKind::StringLiteral
        ) && self.next_token.kind == TokenKind::Colon
    }

    pub(in crate::parser::parser) fn infix_binding_power(&self, op: BinaryOp) -> (u8, u8) {
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

    pub(in crate::parser::parser) fn parse_array_item(&mut self) -> ArrayItem<'ast> {
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

    pub(in crate::parser::parser) fn parse_interpolated_string(
        &mut self,
        end_token: TokenKind,
    ) -> ExprId<'ast> {
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

        if end_token == TokenKind::Backtick && !self.is_ds() {
            self.arena.alloc(Expr::ShellExec { parts, span })
        } else {
            self.arena.alloc(Expr::InterpolatedString { parts, span })
        }
    }
}
