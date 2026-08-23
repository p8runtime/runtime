use super::Parser;
use crate::parser::ast::{ObjectShapeField, Type};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    fn parse_type_atomic(&mut self) -> Option<Type<'ast>> {
        if self.current_token.kind == TokenKind::Question {
            // PHP's prefix nullable `?T`. DekaScript spells this `T?`, which
            // parse_type desugars to Option<T>. Rejected here rather than in
            // the typechecker so the error points at the `?` itself.
            let span = self.current_token.span;
            self.bump();
            let ty = self.parse_type_atomic()?;
            self.errors.push(crate::parser::ast::ParseError::with_help(
                span,
                "Prefix `?T` is not DekaScript syntax",
                "Write the type as `T?`, or use `Option<T>` explicitly.",
            ));
            Some(ty)
        } else if self.current_token.kind == TokenKind::TypeObject
            && self.next_token.kind == TokenKind::Lt
        {
            self.bump(); // consume 'object'
            self.parse_object_shape_type()
        } else if self.current_token.kind == TokenKind::Identifier
            && self.next_token.kind == TokenKind::Lt
            && self
                .lexer
                .slice(self.current_token.span)
                .eq_ignore_ascii_case(b"object")
        {
            self.bump(); // consume 'Object'
            self.parse_object_shape_type()
        } else if self.is_ds() && self.current_token.kind == TokenKind::Fn {
            self.bump(); // consume 'fn'
            let params = self.parse_function_type_params()?;
            let return_type = self.parse_function_type_return()?;
            Some(Type::Function {
                params,
                return_type,
            })
        } else if self.current_token.kind == TokenKind::OpenParen {
            self.bump();
            let ty = self.parse_type()?;
            if self.current_token.kind == TokenKind::CloseParen {
                self.bump();
            }
            Some(ty)
        } else if matches!(
            self.current_token.kind,
            TokenKind::Array
                | TokenKind::Static
                | TokenKind::TypeInt
                | TokenKind::TypeString
                | TokenKind::TypeBytes
                | TokenKind::TypeBool
                | TokenKind::TypeFloat
                | TokenKind::TypeVoid
                | TokenKind::TypeObject
                | TokenKind::TypeMixed
                | TokenKind::TypeNever
                | TokenKind::TypeNull
                | TokenKind::TypeFalse
                | TokenKind::TypeTrue
                | TokenKind::TypeIterable
                | TokenKind::TypeCallable
                | TokenKind::LogicalOr
                | TokenKind::Insteadof
                | TokenKind::LogicalAnd
                | TokenKind::LogicalXor
        ) {
            let t = self.arena.alloc(self.current_token);
            self.bump();
            Some(Type::Simple(t))
        } else if matches!(
            self.current_token.kind,
            TokenKind::Namespace | TokenKind::NsSeparator | TokenKind::Identifier
        ) || self.current_token.kind.is_semi_reserved()
        {
            let name = self.parse_name();
            Some(Type::Name(name))
        } else {
            None
        }
        .map(|ty| {
            if self.current_token.kind == TokenKind::Lt {
                if let Some(args) = self.parse_type_args() {
                    return Type::Applied {
                        base: self.arena.alloc(ty),
                        args,
                    };
                }
            }
            ty
        })
    }

    fn parse_function_type_params(&mut self) -> Option<&'ast [Type<'ast>]> {
        // PHP's lexer tokenises `(int)` as a single cast token. In a
        // DekaScript function type such as `fn(int) int`, treat that token as
        // the parameter type `int` without requiring an opening '('.
        if self.is_ds() {
            if let Some(type_kind) = self.current_token.kind.cast_to_type_kind() {
                // PHP's lexer tokenises `(int)` as a single cast token. In a
                // DekaScript function type such as `fn(int) int`, treat that
                // token as the parameter type `int`.
                let cast_span = self.current_token.span;
                self.bump();
                let inner_span = Span::new(cast_span.start + 1, cast_span.end - 1);
                let ty = Type::Simple(
                    self.arena.alloc(Token {
                        kind: type_kind,
                        span: inner_span,
                    }),
                );
                return Some(self.arena.alloc_slice_copy(&[ty]));
            }
        }

        if self.current_token.kind != TokenKind::OpenParen {
            return None;
        }
        self.bump(); // consume '('
        let mut params = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseParen
            && self.current_token.kind != TokenKind::Eof
        {
            if let Some(param) = self.parse_type() {
                params.push(param);
            } else {
                break;
            }
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        if self.current_token.kind == TokenKind::CloseParen {
            self.bump();
        }
        Some(params.into_bump_slice())
    }

    fn parse_function_type_return(&mut self) -> Option<&'ast Type<'ast>> {
        // DekaScript function types use a return type without a colon:
        // `fn(int, int) int`.
        if let Some(ty) = self.parse_type() {
            Some(self.arena.alloc(ty))
        } else {
            None
        }
    }

    fn parse_object_shape_type(&mut self) -> Option<Type<'ast>> {
        if self.current_token.kind != TokenKind::Lt {
            return None;
        }
        self.bump(); // consume '<'
        let shape = self.parse_object_shape_fields()?;
        if self.current_token.kind == TokenKind::Gt {
            self.bump();
        }

        Some(shape)
    }

    fn parse_object_shape_fields(&mut self) -> Option<Type<'ast>> {
        if self.current_token.kind != TokenKind::OpenBrace {
            return None;
        }
        self.bump(); // consume '{'

        let mut fields = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            let name_token = match self.current_token.kind {
                TokenKind::Identifier | TokenKind::StringLiteral => {
                    let t = self.arena.alloc(self.current_token);
                    self.bump();
                    t
                }
                _ => break,
            };

            let mut optional = false;
            if self.current_token.kind == TokenKind::Question {
                optional = true;
                self.bump();
            }
            if self.current_token.kind != TokenKind::Colon {
                break;
            }
            self.bump();
            let ty = match self.parse_type() {
                Some(ty) => ty,
                None => break,
            };
            let field = ObjectShapeField {
                name: name_token,
                optional,
                ty: self.arena.alloc(ty),
                span: name_token.span,
            };
            fields.push(field);
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            if self.current_token.kind == TokenKind::CloseBrace {
                break;
            }
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        }

        Some(Type::ObjectShape(fields.into_bump_slice()))
    }

    fn parse_type_args(&mut self) -> Option<&'ast [Type<'ast>]> {
        if self.current_token.kind != TokenKind::Lt {
            return None;
        }
        self.bump(); // consume '<'
        let mut args = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::Gt && self.current_token.kind != TokenKind::Eof
        {
            if let Some(arg) = self.parse_type() {
                args.push(arg);
            } else {
                break;
            }
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }
        if self.current_token.kind == TokenKind::Gt {
            self.bump();
        }
        Some(args.into_bump_slice())
    }

    fn parse_type_intersection(&mut self) -> Option<Type<'ast>> {
        let mut left = self.parse_type_atomic()?;

        if matches!(
            self.current_token.kind,
            TokenKind::Ampersand | TokenKind::AmpersandNotFollowedByVarOrVararg
        ) {
            // Check lookahead to distinguish from by-ref param
            if !(self.next_token.kind == TokenKind::Identifier
                || self.next_token.kind == TokenKind::Question
                || self.next_token.kind == TokenKind::OpenParen
                || (self.is_ds_scripting() && self.next_token.kind == TokenKind::OpenBrace)
                || self.next_token.kind == TokenKind::NsSeparator
                || self.next_token.kind.is_semi_reserved())
            {
                return Some(left);
            }

            let mut types = bumpalo::collections::Vec::new_in(self.arena);
            types.push(left);
            while matches!(
                self.current_token.kind,
                TokenKind::Ampersand | TokenKind::AmpersandNotFollowedByVarOrVararg
            ) {
                if !(self.next_token.kind == TokenKind::Identifier
                    || self.next_token.kind == TokenKind::Question
                    || self.next_token.kind == TokenKind::OpenParen
                    || (self.is_ds_scripting() && self.next_token.kind == TokenKind::OpenBrace)
                    || self.next_token.kind == TokenKind::NsSeparator
                    || self.next_token.kind.is_semi_reserved())
                {
                    break;
                }

                self.bump();
                if let Some(right) = self.parse_type_atomic() {
                    types.push(right);
                } else {
                    break;
                }
            }
            left = Type::Intersection(types.into_bump_slice());
        }
        Some(left)
    }

    pub(super) fn parse_type(&mut self) -> Option<Type<'ast>> {
        let mut left = self.parse_type_intersection()?;

        // DekaScript postfix optional shorthand: `T?` is sugar for `Option<T>`.
        if self.current_token.kind == TokenKind::Question {
            self.bump();
            left = Type::Option(self.arena.alloc(left));
        }

        if self.current_token.kind == TokenKind::Pipe {
            let mut types = bumpalo::collections::Vec::new_in(self.arena);
            types.push(left);
            while self.current_token.kind == TokenKind::Pipe {
                self.bump();
                if let Some(right) = self.parse_type_intersection() {
                    types.push(right);
                } else {
                    break;
                }
            }
            left = Type::Union(types.into_bump_slice());
        }
        Some(left)
    }
}
