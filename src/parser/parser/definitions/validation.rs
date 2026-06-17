use super::super::{ParseError, Parser};
use crate::parser::lexer::token::{Token, TokenKind};

use super::{ClassMemberCtx, ModifierContext};

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn validate_modifiers(
        &mut self,
        modifiers: &[Token],
        ctx: ModifierContext,
    ) {
        let mut has_public = false;
        let mut has_protected = false;
        let mut has_private = false;
        let mut has_abstract = false;
        let mut has_final = false;
        let mut has_static = false;
        let mut has_readonly = false;
        let mut has_set_visibility = false;

        for m in modifiers {
            match m.kind {
                TokenKind::Public => {
                    if has_public || has_protected || has_private {
                        self.errors
                            .push(ParseError::new(m.span, "Multiple visibility modifiers"));
                    }
                    has_public = true;
                }
                TokenKind::Protected => {
                    if has_public || has_protected || has_private {
                        self.errors
                            .push(ParseError::new(m.span, "Multiple visibility modifiers"));
                    }
                    has_protected = true;
                }
                TokenKind::Private => {
                    if has_public || has_protected || has_private {
                        self.errors
                            .push(ParseError::new(m.span, "Multiple visibility modifiers"));
                    }
                    has_private = true;
                }
                TokenKind::PublicSet | TokenKind::ProtectedSet | TokenKind::PrivateSet => {
                    if has_set_visibility {
                        self.errors
                            .push(ParseError::new(m.span, "Multiple set visibility modifiers"));
                    }
                    has_set_visibility = true;
                }
                TokenKind::Abstract => {
                    if has_abstract {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate abstract modifier"));
                    }
                    has_abstract = true;
                }
                TokenKind::Final => {
                    if has_final {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate final modifier"));
                    }
                    has_final = true;
                }
                TokenKind::Static => {
                    if has_static {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate static modifier"));
                    }
                    has_static = true;
                }
                TokenKind::Readonly => {
                    if has_readonly {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate readonly modifier"));
                    }
                    has_readonly = true;
                }
                _ => {}
            }
        }

        if has_abstract && has_final {
            self.errors.push(ParseError::new(
                modifiers.first().map(|t| t.span).unwrap_or_default(),
                "abstract and final cannot be combined",
            ));
        }

        // readonly is only valid on properties; flag when used on methods
        if matches!(ctx, ModifierContext::Method)
            && modifiers.iter().any(|m| m.kind == TokenKind::Readonly)
        {
            self.errors.push(ParseError::new(
                modifiers.first().map(|t| t.span).unwrap_or_default(),
                "readonly not allowed on methods",
            ));
        }

        if matches!(ctx, ModifierContext::Method)
            && modifiers.iter().any(|m| {
                matches!(
                    m.kind,
                    TokenKind::PublicSet | TokenKind::ProtectedSet | TokenKind::PrivateSet
                )
            })
        {
            self.errors.push(ParseError::new(
                modifiers.first().map(|t| t.span).unwrap_or_default(),
                "asymmetric visibility not allowed on methods",
            ));
        }

        if matches!(ctx, ModifierContext::Property) {
            /*
            if modifiers
                .iter()
                .any(|m| matches!(m.kind, TokenKind::Abstract | TokenKind::Final))
            {
                self.errors.push(ParseError::new(modifiers.first().map(|t| t.span).unwrap_or_default(), "abstract/final not allowed on properties"));
            }
            */
            let has_static = modifiers.iter().any(|m| m.kind == TokenKind::Static);
            if has_static && modifiers.iter().any(|m| m.kind == TokenKind::Readonly) {
                self.errors.push(ParseError::new(
                    modifiers.first().map(|t| t.span).unwrap_or_default(),
                    "readonly properties cannot be static",
                ));
            }
            // promotion and visibility rules will be enforced at constructor parsing time; placeholder here.
        }
    }

    pub(in crate::parser::parser) fn validate_class_modifiers(&mut self, modifiers: &[Token]) {
        let mut seen_abstract = false;
        let mut seen_final = false;
        let mut seen_readonly = false;

        for m in modifiers {
            match m.kind {
                TokenKind::Abstract => {
                    if seen_abstract {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate abstract modifier"));
                    }
                    seen_abstract = true;
                }
                TokenKind::Final => {
                    if seen_final {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate final modifier"));
                    }
                    seen_final = true;
                }
                TokenKind::Readonly => {
                    if seen_readonly {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate readonly modifier"));
                    }
                    seen_readonly = true;
                }
                _ => {}
            }
        }

        if seen_abstract && seen_final {
            self.errors.push(ParseError::new(
                modifiers.first().map(|t| t.span).unwrap_or_default(),
                "abstract and final cannot be combined",
            ));
        }
    }

    pub(in crate::parser::parser) fn validate_const_modifiers(
        &mut self,
        modifiers: &[Token],
        ctx: ClassMemberCtx,
    ) {
        let mut seen_visibility: Option<TokenKind> = None;
        let mut seen_final = false;

        for m in modifiers {
            match m.kind {
                TokenKind::Public | TokenKind::Protected | TokenKind::Private => {
                    if seen_visibility.is_some() {
                        self.errors
                            .push(ParseError::new(m.span, "Multiple visibility modifiers"));
                    }
                    if matches!(ctx, ClassMemberCtx::Interface) && m.kind != TokenKind::Public {
                        self.errors.push(ParseError::new(
                            m.span,
                            "Interface constants must be public",
                        ));
                    }
                    seen_visibility = Some(m.kind);
                }
                TokenKind::Final => {
                    if seen_final {
                        self.errors
                            .push(ParseError::new(m.span, "Duplicate final modifier"));
                    }
                    seen_final = true;
                }
                TokenKind::Abstract => {
                    self.errors.push(ParseError::new(
                        m.span,
                        "abstract not allowed on class constants",
                    ));
                }
                TokenKind::Static => {
                    self.errors.push(ParseError::new(
                        m.span,
                        "static not allowed on class constants",
                    ));
                }
                TokenKind::Readonly => {
                    self.errors.push(ParseError::new(
                        m.span,
                        "readonly not allowed on class constants",
                    ));
                }
                _ => {}
            }
        }
    }
}
