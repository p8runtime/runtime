use super::super::{ParseError, Parser};
use crate::parser::ast::{
    AttributeGroup, Expr, ExprId,
    Param, PropertyHook, Stmt, StmtId, Type,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;


impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_function(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
        is_async: bool,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        self.bump(); // Eat function

        // By-reference return (returns_ref)
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

        // Name (function_name: T_STRING | T_READONLY)
        let name = if self.current_token.kind == TokenKind::Identifier
            || self.current_token.kind == TokenKind::Readonly
        {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            // Error: expected identifier
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        };

        if self.is_ds() && self.lexer.slice(name.span) == b"array" {
            self.ds_array_callable_declared = true;
        }

        let type_params = if self.is_ds_scripting() && self.current_token.kind == TokenKind::Lt {
            self.parse_type_params()
        } else {
            &[]
        };

        // Params
        let params = self.parse_parameter_list();

        let return_type = self.parse_return_type();

        // Body
        let body_stmt = self.with_function_context(is_async, |parser| parser.parse_stmt()); // Should be a block
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

        self.arena.alloc(Stmt::Function {
            attributes,
            name,
            is_async,
            by_ref,
            type_params,
            params,
            return_type,
            body,
            doc_comment,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn parse_param(&mut self) -> Param<'ast> {
        let mut attributes = &[] as &'ast [AttributeGroup<'ast>];
        if self.current_token.kind == TokenKind::Attribute {
            attributes = self.parse_attributes();
        }

        let start = if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };

        let mut modifiers = std::vec::Vec::new();
        while matches!(
            self.current_token.kind,
            TokenKind::Public | TokenKind::Protected | TokenKind::Private | TokenKind::Readonly
        ) {
            modifiers.push(self.current_token);
            self.bump();
        }

        let mut ty = None;
        let mut old_style_phpx_type = false;
        let mut pattern: Option<ExprId<'ast>> = None;

        if !self.is_ds_scripting() {
            // PHP mode keeps classic syntax: Type $name
            ty = if let Some(t) = self.parse_type() {
                Some(self.arena.alloc(t) as &'ast Type<'ast>)
            } else {
                None
            };
        }

        let by_ref = if matches!(
            self.current_token.kind,
            TokenKind::Ampersand | TokenKind::AmpersandFollowedByVarOrVararg
        ) {
            self.bump();
            true
        } else {
            false
        };

        let variadic = if self.current_token.kind == TokenKind::Ellipsis {
            self.bump();
            true
        } else {
            false
        };

        if self.is_ds() {
            let (param_name, pattern): (&'ast Token, Option<ExprId<'ast>>) = if matches!(
                self.current_token.kind,
                TokenKind::OpenBrace | TokenKind::OpenBracket | TokenKind::List
            ) {
                let pattern = self.parse_phpx_param_pattern().unwrap_or_else(|| {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected destructuring pattern",
                    ));
                    self.arena.alloc(Expr::Error {
                        span: self.current_token.span,
                    })
                });
                let binding = self.pattern_last_binding(pattern).unwrap_or_else(|| {
                    self.errors.push(ParseError::with_help(
                        pattern.span(),
                        "Destructuring parameters require at least one variable binding",
                        "Use a pattern like '{ name }' or '[ first, second ]'.",
                    ));
                    self.arena.alloc(Token {
                        kind: TokenKind::Error,
                        span: pattern.span(),
                    })
                });
                let name_token = self.arena.alloc(Token {
                    kind: TokenKind::Identifier,
                    span: binding.span,
                });
                (name_token, Some(pattern))
            } else if self.current_token.kind == TokenKind::Identifier {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                (token, None)
            } else {
                self.errors.push(ParseError::with_help(self.current_token.span, "DekaScript parameters use bare identifiers", "Write `name: Type`, not `$name: Type`."));
                let token = self.arena.alloc(self.current_token);
                self.bump();
                (token, None)
            };
            if self.current_token.kind == TokenKind::Colon {
                self.bump();
                ty = self.parse_type().map(|t| self.arena.alloc(t) as &'ast Type<'ast>);
                if ty.is_none() { self.errors.push(ParseError::new(self.current_token.span, "Expected parameter type after ':'")); }
            } else if !self.allow_optional_param_types {
                self.errors.push(ParseError::with_help(self.current_token.span, "DekaScript parameters require a type annotation", "Write `name: Type` (for example, `count: number`)."));
            }
            let default = if self.current_token.kind == TokenKind::Eq { self.bump(); Some(self.parse_expr(0)) } else { None };
            let end = default.map_or(param_name.span.end, |expr| expr.span().end);
            if let Some(pattern_expr) = pattern {
                self.push_param_pattern_prologue(pattern_expr, param_name);
            }
            return Param { attributes, modifiers: self.arena.alloc_slice_copy(&modifiers), name: param_name, ty, default, by_ref, variadic, hooks: None, span: Span::new(start, end) };
        }

        // PHPX mode prefers: $name: Type
        // Recovery path: if legacy `Type $name` is used, accept for now but emit a syntax error.
        if self.is_ds_scripting()
            && self.current_token.kind != TokenKind::Variable
            && !matches!(
                self.current_token.kind,
                TokenKind::OpenBrace | TokenKind::OpenBracket | TokenKind::List
            )
        {
            if let Some(t) = self.parse_type() {
                ty = Some(self.arena.alloc(t) as &'ast Type<'ast>);
                old_style_phpx_type = true;
            }
        }

        let param_name = if self.current_token.kind == TokenKind::Variable {
            let param_name = self.arena.alloc(self.current_token);
            self.bump();
            param_name
        } else if self.is_ds_scripting()
            && matches!(
                self.current_token.kind,
                TokenKind::OpenBrace | TokenKind::OpenBracket | TokenKind::List
            )
        {
            pattern = self.parse_phpx_param_pattern();
            if let Some(pattern_expr) = pattern {
                if let Some(binding) = self.pattern_last_binding(pattern_expr) {
                    binding
                } else {
                    self.errors.push(ParseError::with_help(
                        pattern_expr.span(),
                        "Destructuring parameters require at least one variable binding",
                        "Use a pattern like '{ name: $name }' or '[ $first, $second ]'.",
                    ));
                    self.arena.alloc(Token {
                        kind: TokenKind::Error,
                        span: pattern_expr.span(),
                    })
                }
            } else {
                self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: Span::new(start, self.current_token.span.end),
                })
            }
        } else {
            let span = Span::new(start, self.current_token.span.end);
            self.bump();
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span,
            })
        };

        if self.is_ds_scripting() && self.current_token.kind == TokenKind::Colon {
            self.bump();
            if let Some(t) = self.parse_type() {
                ty = Some(self.arena.alloc(t) as &'ast Type<'ast>);
            }
        }

        // Keep legacy `Type $name` syntax available for plain PHP while
        // enforcing `$name: Type` in DekaScript files.
        if old_style_phpx_type && self.is_ds_scripting() {
            self.errors.push(ParseError::with_help(
                param_name.span,
                "DekaScript function parameters must use '$name: Type' syntax",
                "Rewrite parameter as '$name: Type' (for example, '$props: NameProps').",
            ));
        }
        if let Some(pattern_expr) = pattern {
            self.push_param_pattern_prologue(pattern_expr, param_name);
        }

        let default = if self.current_token.kind == TokenKind::Eq {
            self.bump();
            Some(self.parse_expr(0))
        } else {
            None
        };

        let hooks = if !modifiers.is_empty() && self.current_token.kind == TokenKind::OpenBrace {
            Some(self.arena.alloc_slice_copy(&self.parse_property_hooks())
                as &'ast [PropertyHook<'ast>])
        } else {
            None
        };

        let end = if let Some(hooks) = hooks {
            if let Some(last) = hooks.last() {
                last.span.end
            } else {
                self.current_token.span.start
            }
        } else if let Some(expr) = default {
            expr.span().end
        } else {
            param_name.span.end
        };

        Param {
            attributes,
            modifiers: self.arena.alloc_slice_copy(&modifiers),
            name: param_name,
            ty,
            default,
            by_ref,
            variadic,
            hooks,
            span: Span::new(start, end),
        }
    }
}
