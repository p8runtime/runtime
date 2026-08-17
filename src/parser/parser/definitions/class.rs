use super::super::{ParseError, Parser};
use crate::parser::ast::{Arg, AttributeGroup, ClassKind, Expr, ExprId, Stmt, StmtId, Type};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

use super::ClassMemberCtx;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_class(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        modifiers: &'ast [Token],
        doc_comment: Option<Span>,
    ) -> StmtId<'ast> {
        self.parse_class_with_kind(attributes, modifiers, doc_comment, ClassKind::Class)
    }

    pub(in crate::parser::parser) fn parse_class_with_kind(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        modifiers: &'ast [Token],
        doc_comment: Option<Span>,
        kind: ClassKind,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else if let Some(first) = modifiers.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        if self.is_phpx() && kind == ClassKind::Class {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "classes are not allowed in PHPX; use struct instead",
            ));
        }
        self.bump(); // Eat class/struct

        let name = if matches!(
            self.current_token.kind,
            TokenKind::Identifier | TokenKind::Enum | TokenKind::Match
        ) {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            // Error recovery
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: Span::default(),
            })
        };

        let mut extends = None;
        if self.current_token.kind == TokenKind::Extends {
            self.bump();
            let parent = self.parse_name();
            /*
            if self.name_eq_token(&parent, name) {
                self.errors.push(ParseError::new(parent.span, "class cannot extend itself"));
            }
            */
            extends = Some(parent);
        }

        let mut implements = std::vec::Vec::new();
        if self.current_token.kind == TokenKind::Implements {
            self.bump();
            loop {
                implements.push(self.parse_name());
                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            for (i, n) in implements.iter().enumerate() {
                if self.name_eq_token(n, name) {
                    self.errors
                        .push(ParseError::new(n.span, "class cannot implement itself"));
                }
                for prev in implements.iter().take(i) {
                    if self.name_eq(prev, n) {
                        self.errors.push(ParseError::new(
                            n.span,
                            "duplicate interface in implements list",
                        ));
                        break;
                    }
                }
            }
        }
        if self.is_phpx() && kind == ClassKind::Struct {
            if let Some(parent) = extends.as_ref() {
                self.errors.push(ParseError::new(
                    parent.span,
                    "structs cannot extend other types in PHPX",
                ));
            }
            if let Some(first) = implements.first() {
                self.errors.push(ParseError::new(
                    first.span,
                    "structs cannot implement interfaces in PHPX; interfaces are structural",
                ));
            }
        }

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            return self.arena.alloc(Stmt::Class {
                kind,
                attributes,
                modifiers,
                name,
                extends,
                implements: self.arena.alloc_slice_copy(&implements),
                members: &[],
                doc_comment,
                span: Span::new(start, self.current_token.span.end),
            });
        }

        let class_is_abstract = modifiers.iter().any(|m| m.kind == TokenKind::Abstract);
        let class_is_readonly = modifiers.iter().any(|m| m.kind == TokenKind::Readonly);
        self.validate_class_modifiers(modifiers);

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Class {
                is_abstract: class_is_abstract,
                is_readonly: class_is_readonly,
                is_struct: kind == ClassKind::Struct,
            }));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Class {
            kind,
            attributes,
            modifiers,
            name,
            extends,
            implements: self.arena.alloc_slice_copy(&implements),
            members: self.arena.alloc_slice_copy(&members),
            doc_comment,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn parse_anonymous_class(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        modifiers: &'ast [Token],
    ) -> (ExprId<'ast>, &'ast [Arg<'ast>]) {
        let start = if let Some(attr) = attributes.first() {
            attr.span.start
        } else if let Some(m) = modifiers.first() {
            m.span.start
        } else {
            self.current_token.span.start
        };
        if self.is_phpx() {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "anonymous classes are not allowed in PHPX",
            ));
        }
        self.bump(); // eat class

        let (ctor_args, ctor_end) = if self.current_token.kind == TokenKind::OpenParen {
            let (args, span) = self.parse_call_arguments();
            (args, span.end)
        } else {
            (&[] as &[Arg], self.current_token.span.start)
        };

        let mut extends = None;
        if self.current_token.kind == TokenKind::Extends {
            self.bump();
            extends = Some(self.parse_name());
        }

        let mut implements = std::vec::Vec::new();
        if self.current_token.kind == TokenKind::Implements {
            self.bump();
            loop {
                implements.push(self.parse_name());
                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            for i in 0..implements.len() {
                for prev in implements.iter().take(i) {
                    if self.name_eq(prev, &implements[i]) {
                        self.errors.push(ParseError::new(
                            implements[i].span,
                            "duplicate interface in implements list",
                        ));
                        break;
                    }
                }
            }
        }

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            let span = Span::new(start, self.current_token.span.end);
            return (
                self.arena.alloc(Expr::AnonymousClass {
                    attributes,
                    modifiers,
                    args: ctor_args,
                    extends,
                    implements: self.arena.alloc_slice_copy(&implements),
                    members: &[],
                    span,
                }),
                ctor_args,
            );
        }

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Class {
                is_abstract: false,
                is_readonly: false,
                is_struct: false,
            }));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end.max(ctor_end);

        (
            self.arena.alloc(Expr::AnonymousClass {
                attributes,
                modifiers,
                args: ctor_args,
                extends,
                implements: self.arena.alloc_slice_copy(&implements),
                members: self.arena.alloc_slice_copy(&members),
                span: Span::new(start, end),
            }),
            ctor_args,
        )
    }

    pub(in crate::parser::parser) fn parse_interface(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        self.bump(); // Eat interface

        let name = if matches!(
            self.current_token.kind,
            TokenKind::Identifier | TokenKind::Match
        ) {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: Span::default(),
            })
        };

        let mut extends = std::vec::Vec::new();
        if self.current_token.kind == TokenKind::Extends {
            self.bump();
            loop {
                extends.push(self.parse_name());
                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            for (i, n) in extends.iter().enumerate() {
                if self.name_eq_token(n, name) {
                    self.errors
                        .push(ParseError::new(n.span, "interface cannot extend itself"));
                }
                for prev in extends.iter().take(i) {
                    if self.name_eq(prev, n) {
                        self.errors.push(ParseError::new(
                            n.span,
                            "duplicate interface in extends list",
                        ));
                        break;
                    }
                }
            }
        }
        if self.is_phpx() {
            if let Some(first) = extends.first() {
                self.errors.push(ParseError::new(
                    first.span,
                    "interface inheritance is not allowed in PHPX",
                ));
            }
        }

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            return self.arena.alloc(Stmt::Interface {
                attributes,
                name,
                extends: self.arena.alloc_slice_copy(&extends),
                members: &[],
                doc_comment,
                span: Span::new(start, self.current_token.span.end),
            });
        }

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Interface));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Interface {
            attributes,
            name,
            extends: self.arena.alloc_slice_copy(&extends),
            members: self.arena.alloc_slice_copy(&members),
            doc_comment,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn parse_trait(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        // DekaScript traits are a distinct feature from PHP horizontal-reuse
        // traits (RFD 19) -- reject only in legacy PHPX, not in .ds.
        if self.is_phpx() && !self.is_ds() {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "traits are not allowed in PHPX",
            ));
        }
        self.bump(); // Eat trait

        let name = if matches!(
            self.current_token.kind,
            TokenKind::Identifier | TokenKind::Match
        ) {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: Span::default(),
            })
        };

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            return self.arena.alloc(Stmt::Trait {
                attributes,
                name,
                members: &[],
                doc_comment,
                span: Span::new(start, self.current_token.span.end),
            });
        }

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Trait));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Trait {
            attributes,
            name,
            members: self.arena.alloc_slice_copy(&members),
            doc_comment,
            span: Span::new(start, end),
        })
    }

    // DekaScript `impl Type { }` / `impl Trait for Type { }` (RFD 19).
    // Reached only from .ds mode via a contextual `impl` identifier check in
    // stmt.rs (mirroring the existing `struct`/`type` contextual-keyword
    // pattern) -- `impl` is not a reserved token, so it can never collide
    // with an identifier of that name anywhere else in the language.
    pub(in crate::parser::parser) fn parse_impl(
        &mut self,
        doc_comment: Option<Span>,
    ) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // eat 'impl'

        // DekaScript `impl mut Type { ... }` / `impl mut Trait for Type { ... }`.
        // `mut` is a contextual keyword here only; it is not a reserved token.
        let is_mut = self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"mut");
        if is_mut {
            self.bump(); // eat 'mut'
        }

        let first = self.parse_name();

        let (trait_name, target) = if self.current_token.kind == TokenKind::For {
            self.bump(); // eat 'for'
            let target = self.parse_name();
            (Some(first), target)
        } else {
            (None, first)
        };

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            return self.arena.alloc(Stmt::Impl {
                trait_name,
                target,
                members: &[],
                is_mut,
                doc_comment,
                span: Span::new(start, self.current_token.span.end),
            });
        }

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Impl));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Impl {
            trait_name,
            target,
            members: self.arena.alloc_slice_copy(&members),
            is_mut,
            doc_comment,
            span: Span::new(start, end),
        })
    }

    pub(in crate::parser::parser) fn parse_enum(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        self.bump(); // Eat enum

        let name = if self.current_token.kind == TokenKind::Identifier {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: Span::default(),
            })
        };

        let type_params = if self.is_phpx() && self.current_token.kind == TokenKind::Lt {
            self.parse_type_params()
        } else {
            &[]
        };

        let backed_type = if self.current_token.kind == TokenKind::Colon {
            self.bump();
            self.parse_type()
                .map(|t| self.arena.alloc(t) as &'ast Type<'ast>)
        } else {
            None
        };

        let mut implements = std::vec::Vec::new();
        if self.current_token.kind == TokenKind::Implements {
            self.bump();
            loop {
                implements.push(self.parse_name());
                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            for (i, n) in implements.iter().enumerate() {
                if self.name_eq_token(n, name) {
                    self.errors
                        .push(ParseError::new(n.span, "enum cannot implement itself"));
                }
                for prev in implements.iter().take(i) {
                    if self.name_eq(prev, n) {
                        self.errors.push(ParseError::new(
                            n.span,
                            "duplicate interface in implements list",
                        ));
                        break;
                    }
                }
            }
        }

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Expected '{'"));
            return self.arena.alloc(Stmt::Enum {
                attributes,
                name,
                type_params,
                backed_type,
                implements: self.arena.alloc_slice_copy(&implements),
                members: &[],
                doc_comment,
                span: Span::new(start, self.current_token.span.end),
            });
        }

        let mut members = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
            && self.current_token.kind != TokenKind::CloseTag
        {
            members.push(self.parse_class_member(ClassMemberCtx::Enum {
                backed: backed_type.is_some(),
            }));
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors
                .push(ParseError::new(self.current_token.span, "Missing '}'"));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Enum {
            attributes,
            name,
            type_params,
            backed_type,
            implements: self.arena.alloc_slice_copy(&implements),
            members: self.arena.alloc_slice_copy(&members),
            doc_comment,
            span: Span::new(start, end),
        })
    }
}
