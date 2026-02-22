use crate::parser::ast::{Name, ParseError, Program, StmtId};
use crate::parser::lexer::{
    Lexer, LexerMode,
    token::{Token, TokenKind},
};
use bumpalo::Bump;
use std::path::Path;

use crate::parser::span::Span;

mod attributes;
mod control_flow;
mod definitions;
mod expr;
mod stmt;
#[cfg(test)]
mod tests;
mod types;

#[allow(dead_code)]
pub trait TokenSource<'src> {
    fn current(&self) -> &Token;
    fn lookahead(&self, n: usize) -> &Token;
    fn bump(&mut self);
    fn set_mode(&mut self, mode: LexerMode);
}

pub struct Parser<'src, 'ast> {
    pub(super) lexer: Lexer<'src>, // In real impl, this would be wrapped in a TokenSource
    pub(super) arena: &'ast Bump,
    pub(super) current_token: Token,
    pub(super) next_token: Token,
    pub(super) prev_token: Token,
    pub(super) errors: std::vec::Vec<ParseError>,
    pub(super) current_doc_comment: Option<Span>,
    pub(super) next_doc_comment: Option<Span>,
    pub(super) seen_non_declare_stmt: bool,
    pub(super) mode: ParserMode,
    pub(super) param_destructure_prologue: std::vec::Vec<StmtId<'ast>>,
    pub(super) fn_depth: usize,
    pub(super) async_fn_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserMode {
    Php,
    Phpx,
    PhpxInternal,
}

pub fn detect_parser_mode(source: &[u8], file_path: Option<&Path>) -> ParserMode {
    let mut start_idx = 0usize;
    while start_idx < source.len() && source[start_idx].is_ascii_whitespace() {
        start_idx += 1;
    }
    let trimmed = &source[start_idx..];
    if trimmed.starts_with(b"/*__DEKA_PHPX_INTERNAL__*/") {
        return ParserMode::PhpxInternal;
    }
    if trimmed.starts_with(b"/*__DEKA_PHPX__*/") {
        return ParserMode::Phpx;
    }

    if let Some(path) = file_path {
        if path.extension().and_then(|ext| ext.to_str()) == Some("phpx") {
            return ParserMode::Phpx;
        }
        // Cached PHPX modules are emitted as .php files under php_modules/.cache/phpx.
        // They contain generated namespace/wrapper code and must run in internal PHPX mode.
        if path.extension().and_then(|ext| ext.to_str()) == Some("php")
            && path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("/.cache/phpx/")
        {
            return ParserMode::PhpxInternal;
        }
    }

    ParserMode::Php
}

impl<'src, 'ast> Parser<'src, 'ast> {
    pub fn new(lexer: Lexer<'src>, arena: &'ast Bump) -> Self {
        Self::new_with_mode(lexer, arena, ParserMode::Php)
    }

    pub fn new_with_mode(mut lexer: Lexer<'src>, arena: &'ast Bump, mode: ParserMode) -> Self {
        if matches!(mode, ParserMode::Phpx | ParserMode::PhpxInternal) {
            lexer.start_in_scripting();
        }
        let mut parser = Self {
            lexer,
            arena,
            current_token: Token {
                kind: TokenKind::Eof,
                span: Span::default(),
            },
            next_token: Token {
                kind: TokenKind::Eof,
                span: Span::default(),
            },
            prev_token: Token {
                kind: TokenKind::Eof,
                span: Span::default(),
            },
            errors: std::vec::Vec::new(),
            current_doc_comment: None,
            next_doc_comment: None,
            seen_non_declare_stmt: false,
            mode,
            param_destructure_prologue: std::vec::Vec::new(),
            fn_depth: 0,
            async_fn_depth: 0,
        };
        parser.bump();
        parser.bump();
        parser
    }

    pub(super) fn is_phpx(&self) -> bool {
        matches!(self.mode, ParserMode::Phpx | ParserMode::PhpxInternal)
    }

    pub(super) fn allow_phpx_namespace(&self) -> bool {
        self.mode == ParserMode::PhpxInternal
    }

    pub(super) fn take_param_destructure_prologue(&mut self) -> &'ast [StmtId<'ast>] {
        let prologue = std::mem::take(&mut self.param_destructure_prologue);
        self.arena.alloc_slice_copy(&prologue)
    }

    pub(super) fn with_function_context<T>(
        &mut self,
        is_async: bool,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.fn_depth += 1;
        if is_async {
            self.async_fn_depth += 1;
        }
        let out = f(self);
        if is_async {
            self.async_fn_depth = self.async_fn_depth.saturating_sub(1);
        }
        self.fn_depth = self.fn_depth.saturating_sub(1);
        out
    }

    fn bump(&mut self) {
        self.prev_token = self.current_token;
        self.current_token = self.next_token;
        self.current_doc_comment = self.next_doc_comment;
        self.next_doc_comment = None;
        loop {
            let token = self.lexer.next().unwrap_or(Token {
                kind: TokenKind::Eof,
                span: Span::default(),
            });
            if token.kind == TokenKind::DocComment {
                self.next_doc_comment = Some(token.span);
            } else if token.kind != TokenKind::Comment {
                self.next_token = token;
                break;
            }
        }
    }

    fn has_line_terminator_between(&self, left: Span, right: Span) -> bool {
        if right.start <= left.end {
            return false;
        }
        let slice = self.lexer.slice(Span::new(left.end, right.start));
        slice.iter().any(|&b| b == b'\n')
    }

    fn can_insert_implicit_semicolon(&self) -> bool {
        if !self.is_phpx() {
            return false;
        }
        self.has_line_terminator_between(self.prev_token.span, self.current_token.span)
    }

    fn expect_semicolon(&mut self) {
        if self.current_token.kind == TokenKind::SemiColon {
            self.bump();
        } else if self.current_token.kind == TokenKind::CloseTag {
            // Implicit semicolon at close tag
        } else if self.current_token.kind == TokenKind::Eof {
            // Implicit semicolon at EOF
        } else if self.can_insert_implicit_semicolon() {
            // Implicit semicolon at line terminator (PHPX only)
        } else {
            // Error: Missing semicolon
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Missing semicolon",
            ));
            // Recovery: Assume it was there and continue.
            // We do NOT bump the current token because it belongs to the next statement.
            self.sync_to_statement_end();
        }
    }

    pub(super) fn parse_name(&mut self) -> Name<'ast> {
        let start = self.current_token.span.start;
        let mut parts = std::vec::Vec::new();

        if self.current_token.kind == TokenKind::NsSeparator {
            parts.push(self.current_token);
            self.bump();
        } else if self.current_token.kind == TokenKind::Namespace {
            parts.push(self.current_token);
            self.bump();
            if self.current_token.kind == TokenKind::NsSeparator {
                parts.push(self.current_token);
                self.bump();
            }
        }

        loop {
            if self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved()
            {
                parts.push(self.current_token);
                self.bump();
            } else {
                break;
            }

            if self.current_token.kind == TokenKind::NsSeparator {
                parts.push(self.current_token);
                self.bump();
            } else {
                break;
            }
        }

        let end = if parts.is_empty() {
            start
        } else {
            parts.last().unwrap().span.end
        };

        Name {
            parts: self.arena.alloc_slice_copy(&parts),
            span: Span::new(start, end),
        }
    }

    pub fn parse_program(&mut self) -> Program<'ast> {
        let mut statements = std::vec::Vec::new(); // Temporary vec, will be moved to arena

        while self.current_token.kind != TokenKind::Eof {
            statements.push(self.parse_top_stmt());
        }

        let span = if let (Some(first), Some(last)) = (statements.first(), statements.last()) {
            Span::new(first.span().start, last.span().end)
        } else {
            Span::default()
        };

        Program {
            statements: self.arena.alloc_slice_copy(&statements),
            errors: self.arena.alloc_slice_copy(&self.errors),
            span,
        }
    }

    fn sync_to_statement_end(&mut self) {
        while !matches!(
            self.current_token.kind,
            TokenKind::SemiColon | TokenKind::CloseBrace | TokenKind::CloseTag | TokenKind::Eof
        ) {
            self.bump();
        }
        if self.current_token.kind == TokenKind::SemiColon {
            self.bump();
        }
    }
}
