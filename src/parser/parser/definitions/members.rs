use super::super::{ParseError, Parser};
use crate::parser::ast::{
    AttributeGroup, ClassConst, ClassMember, FieldAnnotation, Name,
    Param, PropertyEntry, PropertyHook, PropertyHookBody, Stmt, StmtId, TraitAdaptation,
    TraitMethodRef, Type,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

use super::{ClassMemberCtx, ModifierContext};

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_class_member(
        &mut self,
        ctx: ClassMemberCtx,
    ) -> ClassMember<'ast> {
        let doc_comment = self.current_doc_comment;
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
            TokenKind::Public
                | TokenKind::Protected
                | TokenKind::Private
                | TokenKind::PublicSet
                | TokenKind::ProtectedSet
                | TokenKind::PrivateSet
                | TokenKind::Static
                | TokenKind::Abstract
                | TokenKind::Final
                | TokenKind::Readonly
        ) {
            let token = self.current_token;
            self.bump();
            modifiers.push(token);
        }
        self.validate_modifiers(&modifiers, ModifierContext::Other);

        // DekaScript enum bodies use JS-style variant lists:
        // `enum Status { Loading, Ready, Failed }` or `enum Option<T> { Some(T), None }`.
        // This must be resolved before the PHP-style `case Name;` path.
        if self.is_ds()
            && matches!(ctx, ClassMemberCtx::Enum { backed: false })
            && (self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved())
        {
            let name = self.arena.alloc(self.current_token);
            self.bump();

            let payload = self.parse_ds_enum_payload();

            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            }

            let end = self.current_token.span.end;
            return ClassMember::Case {
                attributes,
                name,
                value: None,
                payload,
                doc_comment,
                span: Span::new(start, end),
            };
        }

        if self.current_token.kind == TokenKind::Case {
            self.bump();
            let name = if self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved()
            {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                token
            } else {
                self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: Span::default(),
                })
            };

            let payload = if self.is_phpx() && self.current_token.kind == TokenKind::OpenParen {
                Some(self.parse_parameter_list())
            } else {
                None
            };

            let value = if self.current_token.kind == TokenKind::Eq {
                self.bump();
                Some(self.parse_expr(0))
            } else {
                None
            };

            if !matches!(ctx, ClassMemberCtx::Enum { .. }) {
                self.errors
                    .push(ParseError::new(name.span, "case not allowed here"));
            } else if matches!(ctx, ClassMemberCtx::Enum { backed: true }) && value.is_none() {
                self.errors.push(ParseError::new(
                    name.span,
                    "backed enum cases require a value",
                ));
            } else if matches!(ctx, ClassMemberCtx::Enum { backed: false }) && value.is_some() {
                self.errors.push(ParseError::new(
                    name.span,
                    "pure enum cases cannot have values",
                ));
            } else if matches!(ctx, ClassMemberCtx::Enum { backed: true }) && payload.is_some() {
                self.errors.push(ParseError::new(
                    name.span,
                    "backed enum cases cannot have payloads",
                ));
            }

            self.expect_semicolon();

            let end = self.current_token.span.end;
            return ClassMember::Case {
                attributes,
                name,
                value,
                payload,
                doc_comment,
                span: Span::new(start, end),
            };
        }

        if self.current_token.kind == TokenKind::Use {
            if self.is_phpx() {
                let is_struct = matches!(
                    ctx,
                    ClassMemberCtx::Class {
                        is_struct: true,
                        ..
                    }
                );
                if !is_struct {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "use is only allowed for struct composition in PHPX",
                    ));
                }
                self.bump();
                let mut types = std::vec::Vec::new();
                loop {
                    types.push(self.parse_name());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
                if self.current_token.kind == TokenKind::OpenBrace {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Trait adaptations are not allowed in PHPX",
                    ));
                    self.bump();
                    while self.current_token.kind != TokenKind::CloseBrace
                        && self.current_token.kind != TokenKind::Eof
                    {
                        self.bump();
                    }
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    }
                } else {
                    self.expect_semicolon();
                }
                let end = self.current_token.span.end;
                return ClassMember::Embed {
                    attributes,
                    types: self.arena.alloc_slice_copy(&types),
                    doc_comment,
                    span: Span::new(start, end),
                };
            }
            self.bump();
            let mut traits = std::vec::Vec::new();
            loop {
                traits.push(self.parse_name());
                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            let mut adaptations = std::vec::Vec::new();

            if self.current_token.kind == TokenKind::OpenBrace {
                self.bump();
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    let method = self.parse_trait_method_ref();
                    let adapt_span_start = method.span.start;

                    if self.current_token.kind == TokenKind::Insteadof {
                        self.bump();
                        let mut insteads = std::vec::Vec::new();
                        loop {
                            insteads.push(self.parse_name());
                            if self.current_token.kind == TokenKind::Comma {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                        adaptations.push(TraitAdaptation::Precedence {
                            method,
                            insteadof: self.arena.alloc_slice_copy(&insteads),
                            span: Span::new(adapt_span_start, self.current_token.span.end),
                        });
                    } else if self.current_token.kind == TokenKind::As {
                        self.bump();
                        let visibility = if matches!(
                            self.current_token.kind,
                            TokenKind::Public | TokenKind::Protected | TokenKind::Private
                        ) {
                            let v = self.arena.alloc(self.current_token);
                            self.bump();
                            Some(v)
                        } else {
                            None
                        };

                        let alias = if self.current_token.kind == TokenKind::Identifier
                            || self.current_token.kind.is_semi_reserved()
                        {
                            let a = self.arena.alloc(self.current_token);
                            self.bump();
                            Some(a)
                        } else {
                            None
                        };

                        adaptations.push(TraitAdaptation::Alias {
                            method,
                            alias: alias.map(|t| &*t),
                            visibility: visibility.map(|t| &*t),
                            span: Span::new(adapt_span_start, self.current_token.span.end),
                        });
                    } else {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected insteadof or as in trait adaptation",
                        ));
                        // try to recover to next semicolon
                    }

                    if self.current_token.kind == TokenKind::SemiColon {
                        self.bump();
                    } else {
                        self.expect_semicolon();
                    }
                }
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                }
            } else {
                self.expect_semicolon();
            }

            let end = self.current_token.span.end;
            return ClassMember::TraitUse {
                attributes,
                traits: self.arena.alloc_slice_copy(&traits),
                adaptations: self.arena.alloc_slice_copy(&adaptations),
                doc_comment,
                span: Span::new(start, end),
            };
        }

        if self.current_token.kind == TokenKind::Function
            || (self.current_token.kind == TokenKind::Fn
                && self.is_ds()
                && matches!(ctx, ClassMemberCtx::Interface))
        {
            let function_token = self.current_token;
            self.bump();
            if self.is_ds() && function_token.kind == TokenKind::Function {
                self.errors.push(ParseError::with_help(
                    function_token.span,
                    "DekaScript uses `fn` for function declarations, not `function`",
                    "Change `function` to `fn`.",
                ));
            }
            let name = if self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved()
            {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                token
            } else {
                self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: Span::default(),
                })
            };

            let params = self.parse_parameter_list();
            let return_type = self.parse_return_type();

            let mut has_body_flag = false;
            let raw_body = if self.current_token.kind == TokenKind::OpenBrace {
                has_body_flag = true;
                let body_stmt = self.parse_block();
                match body_stmt {
                    Stmt::Block { statements, .. } => *statements,
                    _ => self.arena.alloc_slice_copy(&[body_stmt]) as &'ast [StmtId<'ast>],
                }
            } else {
                self.expect_semicolon();
                &[] as &'ast [StmtId<'ast>]
            };
            let prologue = self.take_param_destructure_prologue();
            if !prologue.is_empty() && raw_body.is_empty() {
                self.errors.push(ParseError::with_help(
                    name.span,
                    "Parameter destructuring requires a method body",
                    "Provide a method body or remove destructuring from parameters.",
                ));
            }
            let body = if prologue.is_empty() || raw_body.is_empty() {
                raw_body
            } else {
                let mut merged = std::vec::Vec::with_capacity(prologue.len() + raw_body.len());
                merged.extend_from_slice(prologue);
                merged.extend_from_slice(raw_body);
                self.arena.alloc_slice_copy(&merged)
            };

            let end = if body.is_empty() {
                self.current_token.span.end
            } else {
                body.last().unwrap().span().end
            };

            self.validate_modifiers(&modifiers, ModifierContext::Method);

            let mut method_is_abstract = modifiers.iter().any(|m| m.kind == TokenKind::Abstract);
            if matches!(ctx, ClassMemberCtx::Interface) {
                method_is_abstract = true; // interfaces imply abstract
            }
            let has_body = has_body_flag || !body.is_empty();
            if method_is_abstract && has_body {
                self.errors.push(ParseError::new(
                    Span::new(start, start),
                    "abstract method cannot have a body",
                ));
            }
            if matches!(ctx, ClassMemberCtx::Interface) {
                if has_body {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "interface methods cannot have a body",
                    ));
                }
                if modifiers.iter().any(|m| {
                    matches!(
                        m.kind,
                        TokenKind::Protected | TokenKind::Private | TokenKind::Final
                    )
                }) {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "invalid modifier in interface method",
                    ));
                }
            }
            if let ClassMemberCtx::Class { is_abstract, .. } = ctx {
                if method_is_abstract && !is_abstract {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "abstract method in non-abstract class",
                    ));
                }
                if !method_is_abstract && !has_body {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "non-abstract method must have a body",
                    ));
                }
            }
            if matches!(ctx, ClassMemberCtx::Enum { .. }) && method_is_abstract {
                self.errors.push(ParseError::new(
                    Span::new(start, start),
                    "abstract methods not allowed in enums",
                ));
            }

            let is_ctor = self.token_eq_ident(name, b"__construct");
            let is_struct = matches!(
                ctx,
                ClassMemberCtx::Class {
                    is_struct: true,
                    ..
                }
            );
            if self.is_phpx() && is_struct && is_ctor {
                self.errors.push(ParseError::new(
                    name.span,
                    "constructors are not allowed in PHPX structs; use struct literals",
                ));
            }
            if !is_ctor {
                for param in params.iter() {
                    if param.modifiers.is_empty() {
                        continue;
                    }
                    self.errors.push(ParseError::new(
                        param.span,
                        "property promotion only allowed in constructors",
                    ));
                }
            } else {
                for param in params.iter() {
                    if param.modifiers.is_empty() {
                        continue;
                    }
                    let has_visibility = param.modifiers.iter().any(|m| {
                        matches!(
                            m.kind,
                            TokenKind::Public | TokenKind::Protected | TokenKind::Private
                        )
                    });
                    let vis_count = param
                        .modifiers
                        .iter()
                        .filter(|m| {
                            matches!(
                                m.kind,
                                TokenKind::Public | TokenKind::Protected | TokenKind::Private
                            )
                        })
                        .count();
                    let has_readonly = param
                        .modifiers
                        .iter()
                        .any(|m| m.kind == TokenKind::Readonly);
                    let readonly_count = param
                        .modifiers
                        .iter()
                        .filter(|m| m.kind == TokenKind::Readonly)
                        .count();

                    if matches!(ctx, ClassMemberCtx::Interface | ClassMemberCtx::Trait) {
                        self.errors.push(ParseError::new(
                            param.span,
                            "property promotion not allowed in interfaces/traits",
                        ));
                        continue;
                    }

                    if vis_count > 1 {
                        self.errors.push(ParseError::new(
                            param.span,
                            "multiple visibilities in promoted parameter",
                        ));
                    }
                    if !has_visibility {
                        let message = if has_readonly {
                            "readonly promotion requires visibility"
                        } else {
                            "promoted parameter requires visibility"
                        };
                        self.errors.push(ParseError::new(param.span, message));
                    }
                    if param.by_ref {
                        self.errors.push(ParseError::new(
                            param.span,
                            "promoted parameter cannot be by-reference",
                        ));
                    }
                    if param.variadic {
                        self.errors.push(ParseError::new(
                            param.span,
                            "promoted parameter cannot be variadic",
                        ));
                    }
                    if has_readonly && param.ty.is_none() {
                        self.errors.push(ParseError::new(
                            param.span,
                            "readonly promoted property requires a type",
                        ));
                    }
                    if param.ty.is_none()
                        && matches!(
                            ctx,
                            ClassMemberCtx::Class {
                                is_readonly: true,
                                ..
                            }
                        )
                    {
                        self.errors.push(ParseError::new(
                            param.span,
                            "readonly property requires a type",
                        ));
                    }
                    if readonly_count > 1 {
                        self.errors
                            .push(ParseError::new(param.span, "Duplicate readonly modifier"));
                    }
                }
            }

            ClassMember::Method {
                attributes,
                modifiers: self.arena.alloc_slice_copy(&modifiers),
                name,
                params,
                return_type,
                body,
                doc_comment,
                span: Span::new(start, end),
            }
        } else if self.current_token.kind == TokenKind::Const {
            self.bump();

            let ty = self.parse_type();
            let mut const_type = None;
            let mut first_name = None;

            if let Some(t) = ty {
                if self.current_token.kind == TokenKind::Identifier
                    || self.current_token.kind.is_semi_reserved()
                {
                    const_type = Some(self.arena.alloc(t) as &'ast Type<'ast>);
                } else {
                    match t {
                        Type::Simple(token) => {
                            first_name = Some(token);
                        }
                        Type::Name(name) => {
                            if name.parts.len() == 1 {
                                first_name = Some(&name.parts[0]);
                            } else {
                                self.errors.push(ParseError::new(
                                    name.span,
                                    "Class constant must be an identifier",
                                ));
                                first_name = Some(&name.parts[0]);
                            }
                        }
                        _ => {
                            self.errors.push(ParseError::new(
                                self.current_token.span,
                                "Expected identifier",
                            ));
                            first_name = Some(self.arena.alloc(Token {
                                kind: TokenKind::Error,
                                span: Span::default(),
                            }));
                        }
                    }
                }
            }

            let mut consts = std::vec::Vec::new();
            let mut first = true;

            loop {
                let name = if let (true, Some(name)) = (first, first_name) {
                    name
                } else if self.current_token.kind == TokenKind::Identifier
                    || self.current_token.kind.is_semi_reserved()
                {
                    let token = self.arena.alloc(self.current_token);
                    self.bump();
                    token
                } else {
                    self.arena.alloc(Token {
                        kind: TokenKind::Error,
                        span: Span::default(),
                    })
                };
                first = false;

                if self.current_token.kind == TokenKind::Eq {
                    self.bump();
                }

                let value = self.parse_expr(0);
                consts.push(ClassConst {
                    name,
                    value,
                    span: Span::new(name.span.start, value.span().end),
                });

                if self.current_token.kind == TokenKind::Comma {
                    self.bump();
                    continue;
                } else {
                    break;
                }
            }

            self.expect_semicolon();

            self.validate_const_modifiers(&modifiers, ctx);
            let end = self.current_token.span.end;

            ClassMember::Const {
                attributes,
                modifiers: self.arena.alloc_slice_copy(&modifiers),
                ty: const_type,
                consts: self.arena.alloc_slice_copy(&consts),
                doc_comment,
                span: Span::new(start, end),
            }
        } else {
            // Property
            let is_struct = matches!(
                ctx,
                ClassMemberCtx::Class {
                    is_struct: true,
                    ..
                }
            );
            let is_phpx_interface = self.is_phpx() && matches!(ctx, ClassMemberCtx::Interface);
            // DekaScript struct embedding: a bare type name inside a struct body.
            if self.is_ds()
                && is_struct
                && self.current_token.kind == TokenKind::Identifier
                && self.next_token.kind != TokenKind::Colon
                && self.next_token.kind != TokenKind::Question
            {
                let ty_start = self.current_token.span.start;
                let ty = match self.parse_type() {
                    Some(ty) => ty,
                    None => {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected embedded type name",
                        ));
                        self.sync_to_statement_end();
                        return ClassMember::Embed {
                            attributes,
                            types: &[],
                            doc_comment,
                            span: Span::new(start, self.current_token.span.end),
                        };
                    }
                };
                let ty_end = self.current_token.span.end;

                let embed_name = match ty {
                    Type::Name(name) => name,
                    _ => {
                        self.errors.push(ParseError::new(
                            Span::new(ty_start, ty_end),
                            "Embedded type must be a simple type name",
                        ));
                        Name {
                            parts: &[],
                            span: Span::new(ty_start, ty_end),
                        }
                    }
                };

                if self.current_token.kind == TokenKind::SemiColon {
                    self.bump();
                } else if self.current_token.kind == TokenKind::CloseBrace
                    || (self.is_ds()
                        && matches!(
                            self.current_token.kind,
                            TokenKind::Identifier | TokenKind::Variable
                        ))
                {
                    // DekaScript allows embedded types to omit the trailing
                    // semicolon before another member or the closing brace.
                } else {
                    self.expect_semicolon();
                }

                let end = self.current_token.span.end;
                return ClassMember::Embed {
                    attributes,
                    types: self.arena.alloc_slice_copy(&[embed_name]),
                    doc_comment,
                    span: Span::new(start, end),
                };
            }

            // DekaScript interface fields may be marked mutable: `mut name: Type`.
            let is_mut_field = self.is_ds()
                && is_phpx_interface
                && self.current_token.kind == TokenKind::Identifier
                && self.token_eq_ident(&self.current_token, b"mut")
                && matches!(
                    self.next_token.kind,
                    TokenKind::Identifier | TokenKind::Variable
                )
                && (self.lookahead_kind(2) == Some(TokenKind::Colon)
                    || (self.lookahead_kind(2) == Some(TokenKind::Question)
                        && self.lookahead_kind(3) == Some(TokenKind::Colon)));
            let field_name_offset = if is_mut_field { 1 } else { 0 };
            let field_name_token = if is_mut_field {
                self.next_token
            } else {
                self.current_token
            };
            let is_field_name_token = if self.is_ds() {
                matches!(
                    field_name_token.kind,
                    TokenKind::Identifier | TokenKind::Variable
                )
            } else {
                field_name_token.kind == TokenKind::Variable
            };
            let next_is_field_colon =
                self.lookahead_kind(field_name_offset + 1) == Some(TokenKind::Colon);
            let next_is_optional_field = self.lookahead_kind(field_name_offset + 1)
                == Some(TokenKind::Question)
                && self.lookahead_kind(field_name_offset + 2) == Some(TokenKind::Colon);
            if self.is_phpx()
                && (is_struct || is_phpx_interface)
                && is_field_name_token
                && (next_is_field_colon || next_is_optional_field)
            {
                if !modifiers.is_empty() {
                    self.errors.push(ParseError::new(
                        modifiers.first().map(|t| t.span).unwrap_or_default(),
                        if is_struct {
                            "struct fields do not use visibility modifiers in PHPX"
                        } else {
                            "interface fields do not use visibility modifiers in PHPX"
                        },
                    ));
                }

                let is_mut = is_mut_field;
                if is_mut_field {
                    self.bump(); // mut
                }

                let name = self.arena.alloc(self.current_token);
                self.bump(); // field name

                let mut optional = false;
                if self.current_token.kind == TokenKind::Question {
                    optional = true;
                    self.bump(); // ?
                }

                if self.current_token.kind != TokenKind::Colon {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected ':' after field name",
                    ));
                } else {
                    self.bump(); // :
                }

                let ty = if let Some(t) = self.parse_type() {
                    Some(self.arena.alloc(t) as &'ast Type<'ast>)
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "struct fields require an explicit type",
                    ));
                    None
                };

                let default = if self.current_token.kind == TokenKind::Eq {
                    self.bump();
                    Some(self.parse_expr(0))
                } else {
                    None
                };

                let annotations = self.parse_struct_field_annotations();
                if self.current_token.kind == TokenKind::SemiColon {
                    self.bump();
                } else if self.current_token.kind == TokenKind::CloseBrace && self.is_ds() {
                    // DekaScript allows the last struct field to omit the trailing semicolon.
                } else {
                    self.expect_semicolon();
                }

                // `name: T?` is sugar for an optional `Option<T>` field.
                let is_option_ty = ty
                    .map(|t| matches!(t, Type::Option(_)))
                    .unwrap_or(false);
                let entry = PropertyEntry {
                    name,
                    default,
                    annotations: self.arena.alloc_slice_copy(&annotations),
                    optional: optional || is_option_ty,
                    is_mut,
                    span: Span::new(
                        name.span.start,
                        default.map(|e| e.span().end).unwrap_or(name.span.end),
                    ),
                };
                let end = self.current_token.span.end;
                return ClassMember::Property {
                    attributes,
                    modifiers: self.arena.alloc_slice_copy(&modifiers),
                    ty,
                    entries: self.arena.alloc_slice_copy(&[entry]),
                    doc_comment,
                    span: Span::new(start, end),
                };
            }

            self.validate_modifiers(&modifiers, ModifierContext::Property);
            if matches!(ctx, ClassMemberCtx::Enum { .. }) {
                self.errors.push(ParseError::new(
                    Span::new(start, start),
                    "enums cannot declare properties",
                ));
            }
            let class_is_readonly = matches!(
                ctx,
                ClassMemberCtx::Class {
                    is_readonly: true,
                    ..
                }
            );
            let mut ty = None;
            if self.current_token.kind != TokenKind::Variable
                && let Some(t) = self.parse_type()
            {
                ty = Some(self.arena.alloc(t) as &'ast Type<'ast>);
            }

            let name = if self.current_token.kind == TokenKind::Variable {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                token
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected variable",
                ));

                let is_terminator = matches!(
                    self.current_token.kind,
                    TokenKind::SemiColon
                        | TokenKind::CloseBrace
                        | TokenKind::CloseTag
                        | TokenKind::Eof
                );

                if !is_terminator {
                    self.bump();
                }

                self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: Span::default(),
                })
            };

            if self.is_phpx() && is_struct {
                self.errors.push(ParseError::new(
                    name.span,
                    "struct fields must use `$name: Type` syntax in PHPX",
                ));
            }
            if is_phpx_interface {
                self.errors.push(ParseError::new(
                    name.span,
                    "interface fields must use `$name: Type` syntax in PHPX",
                ));
            }

            let default = if self.current_token.kind == TokenKind::Eq {
                self.bump();
                Some(self.parse_expr(0))
            } else {
                None
            };

            if modifiers.iter().any(|m| m.kind == TokenKind::Readonly) && ty.is_none() {
                self.errors.push(ParseError::new(
                    Span::new(start, start),
                    "readonly property requires a type",
                ));
            }
            if class_is_readonly && ty.is_none() {
                self.errors.push(ParseError::new(
                    Span::new(start, start),
                    "readonly property requires a type",
                ));
            }

            // Property hooks
            if self.current_token.kind == TokenKind::OpenBrace {
                if is_phpx_interface {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "interface fields cannot declare property hooks in PHPX",
                    ));
                }
                let hooks = self.parse_property_hooks();
                // self.expect_semicolon(); // Hooks do not require semicolon
                let end = self.current_token.span.end;
                ClassMember::PropertyHook {
                    attributes,
                    modifiers: self.arena.alloc_slice_copy(&modifiers),
                    ty,
                    name,
                    default,
                    hooks: self.arena.alloc_slice_copy(&hooks),
                    doc_comment,
                    span: Span::new(start, end),
                }
            } else {
                if matches!(ctx, ClassMemberCtx::Interface) && !self.is_phpx() {
                    self.errors.push(ParseError::new(
                        Span::new(start, start),
                        "interfaces cannot declare properties",
                    ));
                }
                if modifiers.iter().any(|m| m.kind == TokenKind::Abstract) {
                    self.errors.push(ParseError::new(
                        modifiers.first().map(|t| t.span).unwrap_or_default(),
                        "Properties cannot be declared abstract",
                    ));
                }

                let mut entries = std::vec::Vec::new();
                entries.push(crate::parser::ast::PropertyEntry {
                    name,
                    default,
                    annotations: &[],
                    optional: false,
                    is_mut: false,
                    span: Span::new(
                        name.span.start,
                        default.map(|e| e.span().end).unwrap_or(name.span.end),
                    ),
                });

                while self.current_token.kind == TokenKind::Comma {
                    self.bump();
                    let name = if self.current_token.kind == TokenKind::Variable {
                        let token = self.arena.alloc(self.current_token);
                        self.bump();
                        token
                    } else {
                        self.bump();
                        self.arena.alloc(Token {
                            kind: TokenKind::Error,
                            span: Span::default(),
                        })
                    };

                    let default = if self.current_token.kind == TokenKind::Eq {
                        self.bump();
                        Some(self.parse_expr(0))
                    } else {
                        None
                    };

                    entries.push(crate::parser::ast::PropertyEntry {
                        name,
                        default,
                        annotations: &[],
                        optional: false,
                        is_mut: false,
                        span: Span::new(
                            name.span.start,
                            default.map(|e| e.span().end).unwrap_or(name.span.end),
                        ),
                    });
                }

                if is_phpx_interface {
                    for entry in entries.iter() {
                        if entry.default.is_some() {
                            self.errors.push(ParseError::new(
                                entry.span,
                                "interface fields cannot have default values in PHPX",
                            ));
                        }
                    }
                }

                self.expect_semicolon();

                let end = self.current_token.span.end;

                ClassMember::Property {
                    attributes,
                    modifiers: self.arena.alloc_slice_copy(&modifiers),
                    ty,
                    entries: self.arena.alloc_slice_copy(&entries),
                    doc_comment,
                    span: Span::new(start, end),
                }
            }
        }
    }

    pub(in crate::parser::parser) fn parse_trait_method_ref(&mut self) -> TraitMethodRef<'ast> {
        let start = self.current_token.span.start;

        let name = self.parse_name();

        if self.current_token.kind == TokenKind::DoubleColon {
            self.bump(); // Eat ::

            let method = if self.current_token.kind == TokenKind::Identifier {
                let t = self.arena.alloc(self.current_token);
                self.bump();
                &*t
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected method name",
                ));
                let t = self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: self.current_token.span,
                });
                self.bump();
                &*t
            };

            return TraitMethodRef {
                trait_name: Some(name),
                method,
                span: Span::new(start, method.span.end),
            };
        }

        if name.parts.len() > 1 {
            self.errors.push(ParseError::new(
                name.span,
                "Method name cannot be qualified",
            ));
        }

        let method = if let Some(first) = name.parts.first() {
            first
        } else {
            self.arena.alloc(Token {
                kind: TokenKind::Error,
                span: name.span,
            })
        };

        TraitMethodRef {
            trait_name: None,
            method,
            span: Span::new(start, method.span.end),
        }
    }

    pub(in crate::parser::parser) fn parse_property_hooks(&mut self) -> Vec<PropertyHook<'ast>> {
        let mut hooks = std::vec::Vec::new();
        self.bump(); // eat {
        while self.current_token.kind != TokenKind::CloseBrace
            && self.current_token.kind != TokenKind::Eof
        {
            let mut attributes = &[] as &'ast [AttributeGroup<'ast>];
            if self.current_token.kind == TokenKind::Attribute {
                attributes = self.parse_attributes();
            }

            let mut modifiers = std::vec::Vec::new();
            while matches!(
                self.current_token.kind,
                TokenKind::Public
                    | TokenKind::Protected
                    | TokenKind::Private
                    | TokenKind::Static
                    | TokenKind::Abstract
                    | TokenKind::Final
                    | TokenKind::Readonly
            ) {
                modifiers.push(self.current_token);
                self.bump();
            }
            self.validate_modifiers(&modifiers, ModifierContext::Method);

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

            let start = self.current_token.span.start;
            let name = if self.current_token.kind == TokenKind::Identifier {
                let t = self.arena.alloc(self.current_token);
                self.bump();
                t
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected hook name",
                ));
                let t = self.arena.alloc(Token {
                    kind: TokenKind::Error,
                    span: self.current_token.span,
                });
                self.bump();
                t
            };

            let params = if matches!(self.current_token.kind, TokenKind::OpenParen) {
                self.parse_parameter_list()
            } else {
                &[] as &'ast [Param<'ast>]
            };

            let body = match self.current_token.kind {
                TokenKind::SemiColon => {
                    self.bump();
                    PropertyHookBody::None
                }
                TokenKind::OpenBrace => {
                    let stmt = self.parse_block();
                    match stmt {
                        Stmt::Block { statements, .. } => PropertyHookBody::Statements(statements),
                        _ => PropertyHookBody::Statements(self.arena.alloc_slice_copy(&[stmt])),
                    }
                }
                TokenKind::DoubleArrow => {
                    self.bump();
                    let expr = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::SemiColon {
                        self.bump();
                    }
                    PropertyHookBody::Expr(expr)
                }
                _ => {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Invalid property hook body",
                    ));
                    PropertyHookBody::None
                }
            };

            let end = match body {
                PropertyHookBody::None => name.span.end,
                PropertyHookBody::Expr(e) => e.span().end,
                PropertyHookBody::Statements(stmts) => {
                    if let Some(last) = stmts.last() {
                        last.span().end
                    } else {
                        self.current_token.span.end
                    }
                }
            };

            hooks.push(PropertyHook {
                attributes,
                modifiers: self.arena.alloc_slice_copy(&modifiers),
                name,
                params,
                by_ref,
                body,
                span: Span::new(start, end),
            });
        }
        if self.current_token.kind == TokenKind::CloseBrace {
            self.bump();
        }
        hooks
    }

    /// Parse a DekaScript enum variant payload of the form `(T, U)` where each
    /// item is a type. Each item becomes a synthetic `Param` whose name points
    /// back at the type text so the emitter and typechecker have a stable
    /// runtime identifier.
    pub(in crate::parser::parser) fn parse_ds_enum_payload(&mut self) -> Option<&'ast [Param<'ast>]> {
        // The PHP lexer merges `(string)`, `(int)`, etc. into a single cast token.
        // In DekaScript enum payloads these are type annotations, not casts, so
        // treat the cast token as a one-item payload and synthesize the
        // corresponding primitive type.
        if let Some(ty_kind) = self.current_token.kind.cast_to_type_kind() {
            let cast_span = self.current_token.span;
            // The cast token spans `(string)`; the type text is the inner part.
            let type_span = Span::new(cast_span.start + 1, cast_span.end.saturating_sub(1));
            self.bump();
            let ty = self.arena.alloc(Type::Simple(self.arena.alloc(Token {
                kind: ty_kind,
                span: type_span,
            })));
            let name = self.arena.alloc(Token {
                kind: TokenKind::Variable,
                span: type_span,
            });
            return Some(self.arena.alloc_slice_copy(&[Param {
                attributes: &[],
                modifiers: &[],
                name,
                ty: Some(ty),
                default: None,
                by_ref: false,
                variadic: false,
                hooks: None,
                span: type_span,
            }]));
        }

        if self.current_token.kind != TokenKind::OpenParen {
            return None;
        }
        self.bump(); // eat (
        let mut params = std::vec::Vec::new();
        while self.current_token.kind != TokenKind::CloseParen
            && self.current_token.kind != TokenKind::Eof
        {
            let type_start = self.current_token.span.start;
            let ty = match self.parse_type() {
                Some(ty) => ty,
                None => {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected type in enum payload",
                    ));
                    break;
                }
            };
            let type_end = self.current_token.span.start;
            let ty = self.arena.alloc(ty) as &'ast Type<'ast>;
            let name = self.arena.alloc(Token {
                kind: TokenKind::Variable,
                span: Span::new(type_start, type_end),
            });
            params.push(Param {
                attributes: &[],
                modifiers: &[],
                name,
                ty: Some(ty),
                default: None,
                by_ref: false,
                variadic: false,
                hooks: None,
                span: Span::new(type_start, type_end),
            });
            if self.current_token.kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        if self.current_token.kind == TokenKind::CloseParen {
            self.bump();
        } else {
            self.errors.push(ParseError::new(
                self.current_token.span,
                "Expected ')' after enum payload",
            ));
        }
        Some(self.arena.alloc_slice_copy(&params))
    }

    pub(in crate::parser::parser) fn parse_struct_field_annotations(
        &mut self,
    ) -> Vec<FieldAnnotation<'ast>> {
        let mut annotations = std::vec::Vec::new();
        while self.current_token.kind == TokenKind::At {
            let start = self.current_token.span.start;
            self.bump(); // eat @

            let name = if self.current_token.kind == TokenKind::Identifier
                || self.current_token.kind.is_semi_reserved()
            {
                let token = self.arena.alloc(self.current_token);
                self.bump();
                token
            } else {
                self.errors.push(ParseError::new(
                    self.current_token.span,
                    "Expected annotation name after '@'",
                ));
                break;
            };

            let mut args = std::vec::Vec::new();
            let mut end = name.span.end;
            if self.current_token.kind == TokenKind::OpenParen {
                self.bump(); // eat (
                if self.current_token.kind != TokenKind::CloseParen {
                    loop {
                        let arg = self.parse_expr(0);
                        end = arg.span().end;
                        args.push(arg);
                        if self.current_token.kind == TokenKind::Comma {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    end = self.current_token.span.end;
                    self.bump(); // eat )
                } else {
                    self.errors.push(ParseError::new(
                        self.current_token.span,
                        "Expected ')' after annotation arguments",
                    ));
                }
            }

            annotations.push(FieldAnnotation {
                name,
                args: self.arena.alloc_slice_copy(&args),
                span: Span::new(start, end),
            });
        }
        annotations
    }
}
