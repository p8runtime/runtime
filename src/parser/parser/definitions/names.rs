use super::super::{ParseError, Parser};
use crate::parser::ast::{
    Name, Type, TypeParam,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;


impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn token_eq_ident(&self, token: &Token, ident: &[u8]) -> bool {
        let slice = self.lexer.slice(token.span);
        slice.eq_ignore_ascii_case(ident)
    }

    pub(in crate::parser::parser) fn parse_type_params(&mut self) -> &'ast [TypeParam<'ast>] {
        if self.current_token.kind != TokenKind::Lt {
            return &[];
        }
        let start = self.current_token.span.start;
        self.bump(); // consume '<'

        let mut params = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::Gt && self.current_token.kind != TokenKind::Eof
        {
            let name_token = if self.current_token.kind == TokenKind::Identifier {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                token
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected type parameter name",
                ));
                break;
            };

            let mut constraint: Option<&'ast Type<'ast>> = None;
            if self.current_token.kind == TokenKind::Colon {
                self.bump();
                if let Some(ty) = self.parse_type() {
                    constraint = Some(self.arena.alloc(ty));
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected type parameter constraint",
                    ));
                }
            }

            params.push(TypeParam {
                name: name_token,
                constraint,
                span: name_token.span,
            });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }

        if self.current_token.kind == TokenKind::Gt {
            self.bump();
        } else {
            self.errors.push(ParseError::new(
                Span::new(start, self.current_token.span.end),
                "Expected '>' after type parameters",
            ));
        }

        params.into_bump_slice()
    }

    pub(in crate::parser::parser) fn name_eq(&self, a: &Name<'ast>, b: &Name<'ast>) -> bool {
        if a.parts.len() != b.parts.len() {
            return false;
        }
        a.parts.iter().zip(b.parts.iter()).all(|(x, y)| {
            self.lexer
                .slice(x.span)
                .eq_ignore_ascii_case(self.lexer.slice(y.span))
        })
    }

    pub(in crate::parser::parser) fn name_eq_token(&self, name: &Name<'ast>, tok: &Token) -> bool {
        if name.parts.len() != 1 {
            return false;
        }
        self.lexer
            .slice(name.parts[0].span)
            .eq_ignore_ascii_case(self.lexer.slice(tok.span))
    }
}
