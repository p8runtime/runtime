use super::{LexerMode, Parser, ParserMode, Token};
use crate::parser::ast::{
    AttributeGroup, Catch, ClassConst, ClassKind, CqlParam, ExportItem, ImportExportSpec, ParseError,
    Receiver, StaticVar, Stmt, StmtId, UseItem, UseKind,
};
use crate::parser::lexer::token::TokenKind;
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(super) fn parse_stmt(&mut self) -> StmtId<'ast> {
        self.parse_stmt_impl(false)
    }

    pub(super) fn parse_top_stmt(&mut self) -> StmtId<'ast> {
        let stmt = self.parse_stmt_impl(true);

        // Track non-declare statements for strict_types position enforcement
        // Ignore Nop (opening tags) and Declare statements
        match stmt {
            crate::parser::ast::Stmt::Nop { .. } | crate::parser::ast::Stmt::Declare { .. } => {
                // Don't set the flag for Nop or Declare
            }
            _ => {
                // Any other statement means strict_types can no longer be first
                self.seen_non_declare_stmt = true;
            }
        }

        stmt
    }

    fn parse_stmt_impl(&mut self, top_level: bool) -> StmtId<'ast> {
        self.lexer.set_mode(LexerMode::Standard);

        let doc_comment = self.current_doc_comment;

        if self.is_ds()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"let")
        {
            return self.parse_ds_let();
        }

        if self.is_ds()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"impl")
        {
            let impl_span = self.current_token.span;
            self.errors.push(ParseError::with_help(
                impl_span,
                "impl is not part of DekaScript",
                "Use a receiver method instead: `fn (self Type) method() { ... }`.",
            ));
            // Recover by skipping the impl target and any trailing block so that
            // only the primary error is reported.
            self.bump(); // impl
            while self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind == TokenKind::For
            {
                self.bump();
            }
            if self.current_token.kind == TokenKind::OpenBrace {
                self.bump(); // {
                let mut depth = 1;
                while depth > 0 && self.current_token.kind != TokenKind::Eof {
                    if self.current_token.kind == TokenKind::OpenBrace {
                        depth += 1;
                    } else if self.current_token.kind == TokenKind::CloseBrace {
                        depth -= 1;
                    }
                    self.bump();
                }
            }
            if self.current_token.kind == TokenKind::SemiColon {
                self.bump();
            }
            return self.arena.alloc(crate::parser::ast::Stmt::Error {
                span: impl_span,
            });
        }

        if self.current_token.kind == TokenKind::Identifier
            && self.next_token.kind == TokenKind::Colon
        {
            let label_token = self.arena.alloc(self.current_token);
            let start = label_token.span.start;
            let colon_span = self.next_token.span;
            self.bump(); // identifier
            self.bump(); // colon
            let span = Span::new(start, colon_span.end);
            return self.arena.alloc(crate::parser::ast::Stmt::Label {
                name: label_token,
                span,
            });
        }

        if self.is_ds_scripting()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"struct")
        {
            if self.is_ds() && !top_level {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "struct declarations are only allowed at the top level in DekaScript",
                ));
            }
            return self.parse_class_with_kind(&[], &[], doc_comment, ClassKind::Struct);
        }

        if self.is_ds_scripting()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"type")
        {
            return self.parse_type_alias(top_level);
        }

        // ECMAScript-style imports/exports are supported in PHPX and DekaScript.
        if self.is_ds_scripting()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"import")
        {
            return self.parse_import_stmt();
        }
        if self.is_ds_scripting()
            && self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"export")
        {
            return self.parse_export_stmt(&[], doc_comment, top_level);
        }

        // `cql` is a true keyword; `query` is context-sensitive (identifier unless followed by name + =)
        if self.is_ds_scripting()
            && (self.current_token.kind == TokenKind::Cql
                || (self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"query")))
            && self.next_token.kind == TokenKind::Identifier
        {
            return self.parse_cql_stmt();
        }

        match self.current_token.kind {
            TokenKind::Attribute => {
                let attributes = self.parse_attributes();
                if self.is_ds_scripting()
                    && self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"struct")
                {
                    if self.is_ds() && !top_level {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "struct declarations are only allowed at the top level in DekaScript",
                        ));
                    }
                    return self.parse_class_with_kind(
                        attributes,
                        &[],
                        doc_comment,
                        ClassKind::Struct,
                    );
                }
                if self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"async")
                    && ((self.is_ds() && self.next_token.kind == TokenKind::Fn)
                        || (self.is_ds_scripting() && self.next_token.kind == TokenKind::Function))
                {
                    self.bump(); // async
                    if self.is_ds() {
                        return self.parse_ds_fn_or_receiver(attributes, doc_comment, true);
                    } else {
                        return self.parse_function(attributes, doc_comment, true);
                    }
                }
                if self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"async")
                    && self.mode == ParserMode::Php
                    && self.next_token.kind == TokenKind::Function
                {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "async functions are not allowed in PHP mode; use PHPX or DekaScript",
                    ));
                    self.bump(); // async
                    return self.parse_function(attributes, doc_comment, false);
                }
                if self.is_ds_scripting()
                    && self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"export")
                {
                    return self.parse_export_stmt(attributes, doc_comment, top_level);
                }
                match self.current_token.kind {
                    TokenKind::Function => {
                        if self.is_ds() {
                            self.errors.push(ParseError::with_help(
                                self.current_token.span,
                                "DekaScript uses `fn` for function declarations, not `function`",
                                "Change `function` to `fn`.",
                            ));
                        }
                        self.parse_function(attributes, doc_comment, false)
                    }
                    TokenKind::Fn if self.is_ds() => {
                        if !top_level {
                            self.errors.push(ParseError::new(
                                self.current_token.span,
                                "function declarations are only allowed at the top level in DekaScript",
                            ));
                        }
                        self.parse_ds_fn_or_receiver(attributes, doc_comment, false)
                    }
                    TokenKind::Class => self.parse_class(attributes, &[], doc_comment),
                    TokenKind::Interface => self.parse_interface(attributes, doc_comment),
                    TokenKind::Trait => {
                        if self.is_ds() {
                            self.errors.push(ParseError::with_help(
                                self.current_token.span,
                                "trait is not part of DekaScript",
                                "Use a receiver method instead: `fn (self Type) method() { ... }`.",
                            ));
                        }
                        self.parse_trait(attributes, doc_comment)
                    }
                    TokenKind::Enum => {
                        if self.is_ds() && !top_level {
                            self.errors.push(ParseError::new(
                                self.current_token.span,
                                "enum declarations are only allowed at the top level in DekaScript",
                            ));
                        }
                        self.parse_enum(attributes, doc_comment)
                    }
                    TokenKind::Const => self.parse_const_stmt(attributes, doc_comment),
                    TokenKind::Final | TokenKind::Abstract | TokenKind::Readonly => {
                        let mut modifiers = std::vec::Vec::new();
                        while matches!(
                            self.current_token.kind,
                            TokenKind::Final | TokenKind::Abstract | TokenKind::Readonly
                        ) {
                            modifiers.push(self.current_token);
                            self.bump();
                        }

                        if self.current_token.kind == TokenKind::Class {
                            self.parse_class(
                                attributes,
                                self.arena.alloc_slice_copy(&modifiers),
                                doc_comment,
                            )
                        } else if self.is_ds_scripting()
                            && self.current_token.kind == TokenKind::Identifier
                            && self.token_eq_ident(&self.current_token, b"struct")
                        {
                            self.parse_class_with_kind(
                                attributes,
                                self.arena.alloc_slice_copy(&modifiers),
                                doc_comment,
                                ClassKind::Struct,
                            )
                        } else {
                            self.arena.alloc(Stmt::Error {
                                span: self.current_token.span,
                            })
                        }
                    }
                    _ => self.arena.alloc(Stmt::Error {
                        span: self.current_token.span,
                    }),
                }
            }
            TokenKind::Identifier
                if self.token_eq_ident(&self.current_token, b"async")
                    && self.mode == ParserMode::Php
                    && self.next_token.kind == TokenKind::Function =>
            {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "async functions are not allowed in PHP mode; use PHPX or DekaScript",
                ));
                self.bump(); // async
                self.parse_function(&[], doc_comment, false)
            }
            TokenKind::Identifier
                if self.token_eq_ident(&self.current_token, b"async")
                    && ((self.is_ds() && self.next_token.kind == TokenKind::Fn)
                        || (self.is_ds_scripting() && self.next_token.kind == TokenKind::Function)) =>
            {
                self.bump(); // async
                if self.is_ds() {
                    self.parse_ds_fn_or_receiver(&[], doc_comment, true)
                } else {
                    self.parse_function(&[], doc_comment, true)
                }
            }
            TokenKind::Final | TokenKind::Abstract | TokenKind::Readonly => {
                let mut modifiers = std::vec::Vec::new();
                while matches!(
                    self.current_token.kind,
                    TokenKind::Final | TokenKind::Abstract | TokenKind::Readonly
                ) {
                    modifiers.push(self.current_token);
                    self.bump();
                }

                if self.current_token.kind == TokenKind::Class {
                    self.parse_class(&[], self.arena.alloc_slice_copy(&modifiers), doc_comment)
                } else if self.is_ds_scripting()
                    && self.current_token.kind == TokenKind::Identifier
                    && self.token_eq_ident(&self.current_token, b"struct")
                {
                    self.parse_class_with_kind(
                        &[],
                        self.arena.alloc_slice_copy(&modifiers),
                        doc_comment,
                        ClassKind::Struct,
                    )
                } else {
                    self.arena.alloc(Stmt::Error {
                        span: self.current_token.span,
                    })
                }
            }
            TokenKind::HaltCompiler => {
                if !top_level {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "__halt_compiler() can only be used from the outermost scope",
                    ));
                }
                let start = self.current_token.span.start;
                self.bump();
                // Parentheses are required by grammar: T_HALT_COMPILER '(' ')' ';'
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected '(' after __halt_compiler",
                    ));
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected ')' after __halt_compiler(",
                    ));
                }
                self.expect_semicolon();

                let end = self.current_token.span.end;
                self.arena.alloc(Stmt::HaltCompiler {
                    span: Span::new(start, end),
                })
            }
            TokenKind::Echo | TokenKind::OpenTagEcho => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        self.current_token.span,
                        "echo is not part of DekaScript",
                        "Return a value from a function or use an explicit output module.",
                    ));
                }
                self.parse_echo()
            }
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::Do => self.parse_do_while(),
            TokenKind::For => {
                if self.is_ds() && self.next_token.kind == TokenKind::OpenParen {
                    self.parse_ds_for_of()
                } else {
                    self.parse_for()
                }
            }
            TokenKind::Foreach => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        self.current_token.span,
                        "foreach is not part of DekaScript",
                        "Use `for (const item of items) { ... }`.",
                    ));
                }
                self.parse_foreach()
            }
            TokenKind::Function => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        self.current_token.span,
                        "DekaScript uses `fn` for function declarations, not `function`",
                        "Change `function` to `fn`.",
                    ));
                }
                self.parse_function(&[], doc_comment, false)
            }
            TokenKind::Fn if self.is_ds() => {
                if !top_level {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "function declarations are only allowed at the top level in DekaScript",
                    ));
                }
                self.parse_ds_fn_or_receiver(&[], doc_comment, false)
            }
            TokenKind::Class => {
                self.reject_ds_php_statement("Use DekaScript structs or plain object values.");
                self.parse_class(&[], &[], doc_comment)
            }
            TokenKind::Interface => self.parse_interface(&[], doc_comment),
            TokenKind::Trait => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        self.current_token.span,
                        "trait is not part of DekaScript",
                        "Use a receiver method instead: `fn (self Type) method() { ... }`.",
                    ));
                }
                self.parse_trait(&[], doc_comment)
            }
            TokenKind::Enum => {
                if self.is_ds() && !top_level {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "enum declarations are only allowed at the top level in DekaScript",
                    ));
                }
                self.parse_enum(&[], doc_comment)
            }
            TokenKind::Namespace => {
                self.reject_ds_php_statement("Use explicit module imports and exports.");
                if self.is_ds_scripting() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "namespace is not allowed in DekaScript; use import instead",
                    ));
                }
                if !top_level {
                    self.errors.push(ParseError::new(self.current_token.span, "Namespace declaration statement has to be the very first statement or after any declare call in the script"));
                }
                self.parse_namespace()
            }
            TokenKind::Use => {
                if self.is_ds_scripting() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "use is not allowed in DekaScript; use import instead",
                    ));
                }
                if !top_level {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Use declarations are only allowed at the top level",
                    ));
                }
                self.parse_use()
            }
            TokenKind::Switch => self.parse_switch(),
            TokenKind::Try => {
                self.reject_ds_php_statement(
                    "Use Result or Option values for fallible operations.",
                );
                if self.is_ds_scripting() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "try/catch is not allowed in PHPX; use Result/Option instead",
                    ));
                }
                self.parse_try()
            }
            TokenKind::Throw => {
                self.reject_ds_php_statement("Return an explicit Result error value instead.");
                if self.is_ds_scripting() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "throw is not allowed in PHPX; use Result/Option instead",
                    ));
                }
                self.parse_throw()
            }
            TokenKind::Const => {
                if !top_level && !self.is_ds() {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Const declarations are only allowed at the top level",
                    ));
                }
                self.parse_const_stmt(&[], doc_comment)
            }
            TokenKind::Goto => self.parse_goto(),
            TokenKind::Break => self.parse_break(),
            TokenKind::Continue => self.parse_continue(),
            TokenKind::Declare => self.parse_declare(),
            TokenKind::Global => {
                self.reject_ds_php_statement(
                    "Pass values explicitly through parameters and returns.",
                );
                self.parse_global()
            }
            TokenKind::Static => {
                self.reject_ds_php_statement("Use a module const or let binding instead.");
                if matches!(
                    self.next_token.kind,
                    TokenKind::Variable
                        | TokenKind::AmpersandFollowedByVarOrVararg
                        | TokenKind::AmpersandNotFollowedByVarOrVararg
                ) {
                    self.parse_static()
                } else {
                    let start = self.current_token.span.start;
                    let expr = self.parse_expr(0);
                    self.expect_semicolon();
                    let end = self.current_token.span.end;
                    self.arena.alloc(Stmt::Expression {
                        expr,
                        span: Span::new(start, end),
                    })
                }
            }
            TokenKind::Unset => self.parse_unset(),
            TokenKind::OpenBrace => self.parse_block(),
            TokenKind::SemiColon => {
                let span = self.current_token.span;
                self.bump();
                self.arena.alloc(Stmt::Nop { span })
            }
            TokenKind::CloseBrace => {
                self.errors
                    .push(ParseError::new(self.current_token.span, "Unexpected '}'"));
                let span = self.current_token.span;
                self.bump();
                self.arena.alloc(Stmt::Error { span })
            }
            TokenKind::CloseTag => {
                let span = self.current_token.span;
                self.bump();
                self.arena.alloc(Stmt::Nop { span })
            }
            TokenKind::OpenTag => {
                let span = self.current_token.span;
                self.bump();
                self.arena.alloc(Stmt::Nop { span })
            }
            TokenKind::InlineHtml => {
                let start = self.current_token.span.start;
                let value = self
                    .arena
                    .alloc_slice_copy(self.lexer.slice(self.current_token.span));
                self.bump();
                let end = self.current_token.span.end;
                self.arena.alloc(Stmt::InlineHtml {
                    value,
                    span: Span::new(start, end),
                })
            }
            _ => {
                // Assume expression statement
                let start = self.current_token.span.start;
                let expr = self.parse_expr(0);
                self.expect_semicolon();
                let end = self.current_token.span.end; // Approximate

                self.arena.alloc(Stmt::Expression {
                    expr,
                    span: Span::new(start, end),
                })
            }
        }
    }

    fn parse_echo(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump();

        let mut exprs = std::vec::Vec::new();
        exprs.push(self.parse_expr(0));

        while self.current_token.kind == TokenKind::Comma {
            self.bump();
            exprs.push(self.parse_expr(0));
        }

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Echo {
            exprs: self.arena.alloc_slice_copy(&exprs),
            span: Span::new(start, end),
        })
    }

    fn parse_return(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        let keyword_span = self.current_token.span;
        self.bump();

        let expr = if self.is_ds_scripting()
            && self.has_line_terminator_between(keyword_span, self.current_token.span)
        {
            None
        } else if matches!(
            self.current_token.kind,
            TokenKind::SemiColon | TokenKind::CloseTag | TokenKind::Eof | TokenKind::CloseBrace
        ) {
            None
        } else {
            Some(self.parse_expr(0))
        };

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Return {
            expr,
            span: Span::new(start, end),
        })
    }

    fn parse_type_alias(&mut self, top_level: bool) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat 'type'

        if !top_level {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "type aliases are only allowed at the top level",
            ));
        }

        let name_token = if self.current_token.kind == TokenKind::Identifier {
            let t = self.arena.alloc(self.current_token);
            self.bump();
            t
        } else {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected type alias name",
            ));
            self.sync_to_statement_end();
            return self.arena.alloc(Stmt::Error {
                span: Span::new(start, self.current_token.span.end),
            });
        };

        let type_params = if self.is_ds_scripting() && self.current_token.kind == TokenKind::Lt {
            self.parse_type_params()
        } else {
            &[]
        };

        if self.current_token.kind != TokenKind::Eq {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected '=' in type alias",
            ));
            self.sync_to_statement_end();
            return self.arena.alloc(Stmt::Error {
                span: Span::new(start, self.current_token.span.end),
            });
        }
        self.bump();

        let ty = match self.parse_type() {
            Some(ty) => ty,
            None => {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected type expression in alias",
                ));
                self.sync_to_statement_end();
                return self.arena.alloc(Stmt::Error {
                    span: Span::new(start, self.current_token.span.end),
                });
            }
        };

        self.expect_semicolon();
        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::TypeAlias {
            name: name_token,
            type_params,
            ty: self.arena.alloc(ty),
            span: Span::new(start, end),
        })
    }

    pub(super) fn parse_block(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;

        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump(); // Eat {
        } else {
            self.errors.push(crate::parser::ast::ParseError::new(
                self.current_token.span,
                "Expected '{'",
            ));
            return self.arena.alloc(Stmt::Error {
                span: self.current_token.span,
            });
        }

        let mut statements = bumpalo::collections::Vec::new_in(self.arena);
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            statements.push(self.parse_stmt());
        }

        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        } else {
            self.errors.push(crate::parser::ast::ParseError::new(
                self.current_token.span,
                "Missing '}'",
            ));
        }

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Block {
            statements: statements.into_bump_slice(),
            span: Span::new(start, end),
        })
    }

    fn parse_namespace(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat namespace

        let name = if self.current_token.kind == TokenKind::Identifier
            || self.current_token.kind == TokenKind::NsSeparator
            || self.current_token.kind == TokenKind::Namespace
        {
            Some(self.parse_name())
        } else {
            None
        };

        let body = if self.current_token.kind == TokenKind::OpenBrace {
            self.bump();
            let mut statements = bumpalo::collections::Vec::new_in(self.arena);
            while self.current_token.kind != TokenKind::CloseBrace
                && self.current_token.kind != TokenKind::Eof
            {
                statements.push(self.parse_top_stmt());
            }
            if self.current_token.kind == TokenKind::CloseBrace {
                self.bump();
            } else {
                self.errors.push(crate::parser::ast::ParseError::new(
                    self.current_token.span,
                    "Missing '}'",
                ));
            }
            Some(statements.into_bump_slice() as &'ast [StmtId<'ast>])
        } else {
            self.expect_semicolon();
            None
        };

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Namespace {
            name,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_use(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat use

        let kind = if self.current_token.kind == TokenKind::Function {
            self.bump();
            UseKind::Function
        } else if self.current_token.kind == TokenKind::Const {
            self.bump();
            UseKind::Const
        } else {
            UseKind::Normal
        };

        let mut uses = std::vec::Vec::new();
        loop {
            let mut item_kind = kind;
            if matches!(
                self.current_token.kind,
                TokenKind::Function | TokenKind::Const
            ) {
                item_kind = if self.current_token.kind == TokenKind::Function {
                    self.bump();
                    UseKind::Function
                } else {
                    self.bump();
                    UseKind::Const
                };
            }

            let prefix = self.parse_name();

            if self.current_token.kind == TokenKind::OpenBrace {
                self.bump(); // Eat {
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    let mut element_kind = item_kind;
                    if matches!(
                        self.current_token.kind,
                        TokenKind::Function | TokenKind::Const
                    ) {
                        element_kind = if self.current_token.kind == TokenKind::Function {
                            self.bump();
                            UseKind::Function
                        } else {
                            self.bump();
                            UseKind::Const
                        };
                    }
                    let suffix = self.parse_name();

                    let alias = if self.current_token.kind == TokenKind::As {
                        self.bump();
                        if self.current_token.kind == TokenKind::Identifier {
                            let token = self.arena.alloc(self.current_token);
                            self.bump();
                            Some(token as &Token)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let mut full_parts = std::vec::Vec::new();
                    full_parts.extend_from_slice(prefix.parts);
                    full_parts.extend_from_slice(suffix.parts);

                    let full_name = crate::parser::ast::Name {
                        parts: self.arena.alloc_slice_copy(&full_parts),
                        span: Span::new(prefix.span.start, suffix.span.end),
                    };

                    uses.push(UseItem {
                        name: full_name,
                        alias,
                        kind: element_kind,
                        span: Span::new(
                            prefix.span.start,
                            alias.map(|a| a.span.end).unwrap_or(suffix.span.end),
                        ),
                    });

                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                } else {
                    self.errors.push(crate::parser::ast::ParseError::new(
                        self.current_token.span,
                        "Missing '}'",
                    ));
                }
            } else {
                let alias = if self.current_token.kind == TokenKind::As {
                    self.bump();
                    if self.current_token.kind == TokenKind::Identifier {
                        let token = self.arena.alloc(self.current_token);
                        self.bump();
                        Some(token as &Token)
                    } else {
                        None
                    }
                } else {
                    None
                };

                uses.push(UseItem {
                    name: prefix,
                    alias,
                    kind: item_kind,
                    span: Span::new(
                        prefix.span.start,
                        alias.map(|a| a.span.end).unwrap_or(prefix.span.end),
                    ),
                });
            }

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Use {
            uses: self.arena.alloc_slice_copy(&uses),
            kind,
            span: Span::new(start, end),
        })
    }

    fn parse_try(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat try

        let body_stmt = self.parse_block();
        let body: &'ast [StmtId<'ast>] = match body_stmt {
            Stmt::Block { statements, .. } => statements,
            _ => self.arena.alloc_slice_copy(&[body_stmt]) as &'ast [StmtId<'ast>],
        };

        let mut catches = std::vec::Vec::new();
        while self.current_token.kind == TokenKind::Catch {
            let catch_start = self.current_token.span.start;
            self.bump();

            if self.current_token.kind == TokenKind::OpenParen {
                self.bump();
            }

            // Types
            let mut types = std::vec::Vec::new();
            loop {
                types.push(self.parse_name());
                if self.current_token.kind == TokenKind::Pipe {
                    self.bump();
                    continue;
                }
                break;
            }

            let var = if self.current_token.kind == TokenKind::Variable {
                let t = self.arena.alloc(self.current_token);
                self.bump();
                Some(&*t)
            } else {
                None
            };

            if self.current_token.kind == TokenKind::CloseParen {
                self.bump();
            }

            let catch_body_stmt = self.parse_block();
            let catch_body: &'ast [StmtId<'ast>] = match catch_body_stmt {
                Stmt::Block { statements, .. } => statements,
                _ => self.arena.alloc_slice_copy(&[catch_body_stmt]) as &'ast [StmtId<'ast>],
            };

            let catch_end = self.current_token.span.end; // Approximate

            catches.push(Catch {
                types: self.arena.alloc_slice_copy(&types),
                var,
                body: catch_body,
                span: Span::new(catch_start, catch_end),
            });
        }

        let finally = if self.current_token.kind == TokenKind::Finally {
            self.bump();
            let finally_stmt = self.parse_block();
            match finally_stmt {
                Stmt::Block { statements, .. } => Some(*statements),
                _ => Some(self.arena.alloc_slice_copy(&[finally_stmt]) as &'ast [StmtId<'ast>]),
            }
        } else {
            None
        };

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Try {
            body,
            catches: self.arena.alloc_slice_copy(&catches),
            finally,
            span: Span::new(start, end),
        })
    }

    fn parse_throw(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat throw

        let expr = self.parse_expr(0);

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Throw {
            expr,
            span: Span::new(start, end),
        })
    }

    fn parse_const_stmt(
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
        self.bump(); // const

        let mut consts = std::vec::Vec::new();
        loop {
            let name = if self.current_token.kind == TokenKind::Identifier {
                let tok = self.arena.alloc(self.current_token);
                self.bump();
                tok
            } else {
                self.errors.push(if self.is_ds() {
                    ParseError::with_help(
                        self.current_token.span,
                        "DekaScript const declarations require bare identifiers",
                        "Write `const name = value;`, not `const $name = value;`.",
                    )
                } else {
                    ParseError::new(self.current_token.span, "Expected identifier")
                });
                self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: self.current_token.span,
                })
            };

            if self.current_token.kind == TokenKind::Eq {
                self.bump();
            } else {
                self.errors.push(crate::parser::ast::ParseError::new(
                    self.current_token.span,
                    "Expected '='",
                ));
            }
            let value = self.parse_expr(0);
            let span = Span::new(name.span.start, value.span().end);
            consts.push(ClassConst { name, value, span });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }

        self.expect_semicolon();
        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Const {
            attributes,
            consts: self.arena.alloc_slice_copy(&consts),
            doc_comment,
            span: Span::new(start, end),
        })
    }

    /// Parse the DekaScript mutable binding form. We reuse the existing
    /// `Static` AST node as a compact internal representation; it is lowered
    /// to a lexical `let` by the JS emitter and never exposes PHP `static`
    /// semantics to `.ds` authors.
    fn parse_ds_let(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // let
        let mut vars = std::vec::Vec::new();
        loop {
            if self.current_token.kind != TokenKind::Identifier {
                self.errors.push(ParseError::with_help(
                    self.current_token.span,
                    "DekaScript let declarations require a bare identifier",
                    "Write `let name = value;`.",
                ));
                break;
            }
            let name = self.current_token;
            self.bump();
            let var = self.arena.alloc(crate::parser::ast::Expr::Variable {
                name: name.span,
                span: name.span,
            });
            let default = if self.current_token.kind == TokenKind::Eq {
                self.bump();
                Some(self.parse_expr(0))
            } else {
                self.errors.push(ParseError::with_help(
                    self.current_token.span,
                    "DekaScript let declarations require an initializer",
                    "Write `let name = value;`.",
                ));
                None
            };
            let end = default.map_or(name.span.end, |expr| expr.span().end);
            vars.push(StaticVar {
                var,
                default,
                span: Span::new(name.span.start, end),
            });
            if self.current_token.kind != TokenKind::Comma {
                break;
            }
            self.bump();
        }
        self.expect_semicolon();
        let end = self.current_token.span.end;
        self.arena.alloc(Stmt::Static {
            vars: self.arena.alloc_slice_copy(&vars),
            span: Span::new(start, end),
        })
    }

    fn reject_ds_php_statement(&mut self, help: &'static str) {
        if self.is_ds() {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "PHP/PHPX construct is not part of DekaScript",
                help,
            ));
        }
    }

    fn parse_global(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat global

        let mut vars = std::vec::Vec::new();
        loop {
            vars.push(self.parse_expr(0));
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Global {
            vars: self.arena.alloc_slice_copy(&vars),
            span: Span::new(start, end),
        })
    }

    fn parse_static(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat static

        let mut vars = std::vec::Vec::new();
        loop {
            let var = self.parse_expr(0);
            let default = if self.current_token.kind == TokenKind::Eq {
                self.bump();
                Some(self.parse_expr(0))
            } else {
                None
            };

            let span = if let Some(def) = default {
                Span::new(var.span().start, def.span().end)
            } else {
                var.span()
            };

            vars.push(StaticVar { var, default, span });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Static {
            vars: self.arena.alloc_slice_copy(&vars),
            span: Span::new(start, end),
        })
    }

    fn parse_unset(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // Eat unset

        if self.current_token.kind == TokenKind::OpenParen {
            self.bump();
        }

        let mut vars = std::vec::Vec::new();
        loop {
            vars.push(self.parse_expr(0));
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                if self.current_token.kind == TokenKind::CloseParen {
                    break;
                }
            } else {
                break;
            }
        }

        if self.current_token.kind == TokenKind::CloseParen {
            self.bump();
        }

        self.expect_semicolon();

        let end = self.current_token.span.end;

        self.arena.alloc(Stmt::Unset {
            vars: self.arena.alloc_slice_copy(&vars),
            span: Span::new(start, end),
        })
    }

    /// Parse `cql <name> = <cypher body> ;` or `query <name> = <cypher body> ;`
    ///
    /// The Cypher body is captured as raw text (a Span) — everything between `=` and `;`.
    /// $variable references within the body are extracted as CqlParam entries.
    fn parse_cql_stmt(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // consume `cql` or `query`

        // Expect an identifier for the binding name
        if self.current_token.kind != TokenKind::Identifier {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Expected a name after cql/query",
                "cql requires a binding name, e.g. `cql results = MATCH (n) RETURN n;`",
            ));
            let span = Span::new(start, self.current_token.span.end);
            return self.arena.alloc(Stmt::Error { span });
        }
        let name = self.arena.alloc(self.current_token);
        self.bump(); // consume name

        // Expect `=`
        if self.current_token.kind != TokenKind::Eq {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Expected '=' after cql binding name",
                "cql requires an assignment, e.g. `cql results = MATCH (n) RETURN n;`",
            ));
            let span = Span::new(start, self.current_token.span.end);
            return self.arena.alloc(Stmt::Error { span });
        }
        self.bump(); // consume `=`

        // Capture everything until `;` as the raw Cypher body
        let cypher_start = self.current_token.span.start;
        let mut params = Vec::new();

        // Scan forward, collecting tokens until semicolon or EOF
        while self.current_token.kind != TokenKind::SemiColon
            && self.current_token.kind != TokenKind::Eof
        {
            // Collect $param references
            if self.current_token.kind == TokenKind::Variable {
                let var_span = self.current_token.span;
                // Variable span includes the $, param name is everything after it
                let param_name_span = Span::new(var_span.start + 1, var_span.end);
                let param_name = self.lexer.slice(param_name_span);
                let param_name = self.arena.alloc_slice_copy(param_name);
                params.push(CqlParam {
                    name: param_name,
                    span: var_span,
                });
            }
            self.bump();
        }

        let cypher_end = self.current_token.span.start; // up to the semicolon
        let cypher = Span::new(cypher_start, cypher_end);

        self.expect_semicolon();

        let end = self.current_token.span.end;
        let params = self.arena.alloc_slice_copy(&params);

        let expr = self.arena.alloc(crate::parser::ast::Expr::Cql {
            name,
            cypher,
            params,
            span: Span::new(start, end),
        });

        self.arena.alloc(Stmt::Expression {
            expr,
            span: Span::new(start, end),
        })
    }

    /// Parse an ECMAScript-style named import:
    /// `import { a, b as c } from "./mod";`
    fn parse_import_stmt(&mut self) -> StmtId<'ast> {
        let start = self.current_token.span.start;
        self.bump(); // import

        if self.current_token.kind != TokenKind::OpenBrace {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Expected '{' after import",
                "Use named imports: `import { name } from './module';`.",
            ));
            self.sync_to_statement_end();
            let end = self.current_token.span.end;
            return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
        }
        self.bump(); // {

        let mut specs = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            let remote = if self.current_token.kind == TokenKind::Identifier {
                let tok = self.arena.alloc(self.current_token);
                self.bump();
                tok
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected import specifier name",
                ));
                let span = self.current_token.span;
                self.sync_to_statement_end();
                return self.arena.alloc(Stmt::Error {
                    span: Span::new(start, span.end),
                });
            };

            let local: &'ast Token;
            if self.current_token.kind == TokenKind::As {
                self.bump(); // as
                if self.current_token.kind == TokenKind::Identifier {
                    local = self.arena.alloc(self.current_token);
                    self.bump();
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected local name after 'as'",
                    ));
                    let span = self.current_token.span;
                    self.sync_to_statement_end();
                    return self.arena.alloc(Stmt::Error {
                        span: Span::new(start, span.end),
                    });
                }
            } else {
                local = remote;
            }

            let spec_span = Span::new(remote.span.start, local.span.end);
            specs.push(ImportExportSpec {
                remote,
                local,
                span: spec_span,
            });

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
                continue;
            }
            break;
        }

        if self.current_token.kind != TokenKind::CloseBrace {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected '}' after import specifiers",
            ));
            self.sync_to_statement_end();
            let end = self.current_token.span.end;
            return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
        }
        self.bump(); // }

        if !(self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"from"))
        {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Expected 'from' after import specifiers",
                "Use: `import { name } from './module';`.",
            ));
            self.sync_to_statement_end();
            let end = self.current_token.span.end;
            return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
        }
        self.bump(); // from

        if self.current_token.kind != TokenKind::StringLiteral {
            self.errors.push(ParseError::with_help(
                self.current_token.span,
                "Expected module path string after 'from'",
                "Use: `import { name } from './module';`.",
            ));
            self.sync_to_statement_end();
            let end = self.current_token.span.end;
            return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
        }
        let from = self.arena.alloc(self.current_token);
        let end = self.current_token.span.end;
        self.bump();

        self.expect_semicolon();

        self.arena.alloc(Stmt::Import {
            specs: self.arena.alloc_slice_copy(&specs),
            from,
            span: Span::new(start, end),
        })
    }

    /// Parse an ECMAScript-style export:
    /// `export fn name() {}`, `export function name() {}`,
    /// `export const name = ...;`, `export { a, b as c };`,
    /// `export { a, b as c } from "./mod";`.
    fn parse_export_stmt(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
        _top_level: bool,
    ) -> StmtId<'ast> {
        let start = if let Some(doc) = doc_comment {
            doc.start
        } else if let Some(first) = attributes.first() {
            first.span.start
        } else {
            self.current_token.span.start
        };
        self.bump(); // export

        // Named export list: `export { a, b as c } [from "..."];`
        if self.current_token.kind == TokenKind::OpenBrace {
            self.bump(); // {
            let mut specs = std::vec::Vec::new();
            while self.current_token.kind != TokenKind::CloseBrace
                && self.current_token.kind != TokenKind::Eof
            {
                let local = if self.current_token.kind == TokenKind::Identifier {
                    let tok = self.arena.alloc(self.current_token);
                    self.bump();
                    tok
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected export specifier name",
                    ));
                    self.sync_to_statement_end();
                    let end = self.current_token.span.end;
                    return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
                };

                let remote: &'ast Token;
                if self.current_token.kind == TokenKind::As {
                    self.bump(); // as
                    if self.current_token.kind == TokenKind::Identifier {
                        remote = self.arena.alloc(self.current_token);
                        self.bump();
                    } else {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected exported name after 'as'",
                        ));
                        self.sync_to_statement_end();
                        let end = self.current_token.span.end;
                        return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
                    }
                } else {
                    remote = local;
                }

                let spec_span = Span::new(local.span.start, remote.span.end);
                specs.push(ImportExportSpec {
                    remote,
                    local,
                    span: spec_span,
                });

                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                    continue;
                }
                break;
            }

            if self.current_token.kind != TokenKind::CloseBrace {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected '}' after export specifiers",
                ));
                self.sync_to_statement_end();
                let end = self.current_token.span.end;
                return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
            }
            let close_brace_span = self.current_token.span;
            self.bump(); // }

            let from = if self.current_token.kind == TokenKind::Identifier
                && self.token_eq_ident(&self.current_token, b"from")
            {
                self.bump(); // from
                if self.current_token.kind != TokenKind::StringLiteral {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected module path string after 'from'",
                    ));
                    self.sync_to_statement_end();
                    let end = self.current_token.span.end;
                    return self.arena.alloc(Stmt::Error { span: Span::new(start, end) });
                }
                let from_tok: &'ast Token = self.arena.alloc(self.current_token);
                let end = self.current_token.span.end;
                self.bump();
                Some((from_tok, end))
            } else {
                None
            };

            self.expect_semicolon();
            let end = from.map(|(_, end)| end).unwrap_or(close_brace_span.end);
            return self.arena.alloc(Stmt::Export {
                item: ExportItem::Named {
                    specs: self.arena.alloc_slice_copy(&specs),
                    from: from.map(|(tok, _)| tok),
                },
                span: Span::new(start, end),
            });
        }

        // Declaration exports.
        let is_async = self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"async");
        if is_async {
            self.bump();
        }

        if self.is_ds()
            && self.current_token.kind == TokenKind::Fn
            && self.next_token.kind == TokenKind::Identifier
        {
            let decl = self.parse_function(attributes, doc_comment, is_async);
            return self.arena.alloc(Stmt::Export {
                item: ExportItem::Decl(decl),
                span: Span::new(start, decl.span().end),
            });
        }

        if !self.is_ds()
            && self.current_token.kind == TokenKind::Function
            && self.next_token.kind == TokenKind::Identifier
        {
            let decl = self.parse_function(attributes, doc_comment, is_async);
            return self.arena.alloc(Stmt::Export {
                item: ExportItem::Decl(decl),
                span: Span::new(start, decl.span().end),
            });
        }

        if self.is_ds() && self.current_token.kind == TokenKind::Const {
            let decl = self.parse_const_stmt(attributes, doc_comment);
            return self.arena.alloc(Stmt::Export {
                item: ExportItem::Decl(decl),
                span: Span::new(start, decl.span().end),
            });
        }

        self.errors.push(ParseError::with_help(
            self.current_token.span,
            "Unsupported export syntax",
            "Use `export fn name() {}`, `export const name = ...;`, or `export { name };`.",
        ));
        self.sync_to_statement_end();
        let end = self.current_token.span.end;
        self.arena.alloc(Stmt::Error { span: Span::new(start, end) })
    }

    /// Parse a DekaScript `fn` declaration: either a top-level function or a
    /// receiver method. Falls back to an expression statement for arrow
    /// functions at statement position.
    fn parse_ds_fn_or_receiver(
        &mut self,
        attributes: &'ast [AttributeGroup<'ast>],
        doc_comment: Option<Span>,
        is_async: bool,
    ) -> StmtId<'ast> {
        if self.next_token.kind == TokenKind::Identifier && self.ds_fn_looks_like_function() {
            self.parse_function(attributes, doc_comment, is_async)
        } else if self.is_receiver_method_start() {
            self.parse_receiver_method(attributes, doc_comment, is_async)
        } else {
            // Treat as a DekaScript function literal expression statement.
            let start = self.current_token.span.start;
            self.bump(); // consume 'fn'
            let expr = self.parse_ds_function_literal(&[], false, false, start);
            self.expect_semicolon();
            let end = self.current_token.span.end;
            self.arena.alloc(Stmt::Expression {
                expr,
                span: Span::new(start, end),
            })
        }
    }

    /// Returns true when the current token is `fn` followed by a function
    /// declaration: an identifier, optional generic parameters, and then '('.
    fn ds_fn_looks_like_function(&self) -> bool {
        let mut i = 2;
        if self.lookahead_kind(i) == Some(TokenKind::Lt) {
            i += 1;
            let mut depth: i32 = 1;
            while depth > 0 {
                match self.lookahead_kind(i) {
                    Some(TokenKind::Lt) => depth += 1,
                    Some(TokenKind::Gt) => depth -= 1,
                    Some(TokenKind::Eof) | None => return false,
                    _ => {}
                }
                i += 1;
            }
        }
        self.lookahead_kind(i) == Some(TokenKind::OpenParen)
    }

    /// Returns true when the current token is `fn` followed by a Go-style
    /// receiver clause: `fn (var [mut] Type) methodName(`.
    fn is_receiver_method_start(&self) -> bool {
        if self.current_token.kind != TokenKind::Fn
            || self.next_token.kind != TokenKind::OpenParen
        {
            return false;
        }
        let mut depth: i32 = 1;
        let mut i: usize = 2;
        while depth > 0 {
            match self.lookahead_kind(i) {
                Some(TokenKind::OpenParen) => depth += 1,
                Some(TokenKind::CloseParen) => depth -= 1,
                Some(TokenKind::Eof) | None => return false,
                _ => {}
            }
            i += 1;
        }
        self.lookahead_kind(i) == Some(TokenKind::Identifier)
            && self.lookahead_kind(i + 1) == Some(TokenKind::OpenParen)
    }

    /// Parse a DekaScript receiver method:
    /// `fn (p Person) greet() { ... }` or `fn (p mut Person) setName(name: string) { ... }`.
    fn parse_receiver_method(
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
        self.bump(); // fn

        if self.current_token.kind != TokenKind::OpenParen {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected '(' after 'fn' for receiver method",
            ));
            return self.arena.alloc(Stmt::Error {
                span: Span::new(start, self.current_token.span.end),
            });
        }
        let receiver_open = self.current_token.span.start;
        self.bump(); // (

        let var = if self.current_token.kind == TokenKind::Identifier {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected receiver variable name",
            ));
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: self.current_token.span,
            })
        };

        let is_mut = if self.current_token.kind == TokenKind::Identifier
            && self.token_eq_ident(&self.current_token, b"mut")
        {
            self.bump();
            true
        } else {
            false
        };

        let ty = match self.parse_type() {
            Some(ty) => self.arena.alloc(ty),
            None => {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected receiver type",
                ));
                self.arena.alloc(crate::parser::ast::Type::Simple(
                    self.arena.alloc(Token {
                        kind: TokenKind::Error,
                        span: self.current_token.span,
                    }),
                ))
            }
        };

        let receiver_close = if self.current_token.kind == TokenKind::CloseParen {
            let span = self.current_token.span;
            self.bump(); // )
            span.end
        } else {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected ')' after receiver type",
            ));
            self.current_token.span.start
        };
        let receiver = self.arena.alloc(Receiver {
            var,
            is_mut,
            ty,
            span: Span::new(receiver_open, receiver_close),
        });

        let name = if self.current_token.kind == TokenKind::Identifier {
            let token = self.arena.alloc(self.current_token);
            self.bump();
            token
        } else {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected method name",
            ));
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: self.current_token.span,
            })
        };

        let params = self.parse_parameter_list();
        let return_type = self.parse_return_type();

        let body_stmt = self.with_function_context(is_async, |parser| parser.parse_block());
        let body: &'ast [StmtId<'ast>] = match body_stmt {
            Stmt::Block { statements, .. } => statements,
            _ => self.arena.alloc_slice_copy(&[body_stmt]) as &'ast [StmtId<'ast>],
        };
        let prologue = self.take_param_destructure_prologue();
        let body = if prologue.is_empty() {
            body
        } else {
            let mut merged = std::vec::Vec::with_capacity(prologue.len() + body.len());
            merged.extend_from_slice(prologue);
            merged.extend_from_slice(body);
            self.arena.alloc_slice_copy(&merged)
        };

        let end = if body.is_empty() {
            self.current_token.span.end
        } else {
            body.last().unwrap().span().end
        };

        self.arena.alloc(Stmt::ReceiverMethod {
            attributes,
            name,
            is_async,
            receiver,
            params,
            return_type,
            body,
            doc_comment,
            span: Span::new(start, end),
        })
    }
}
