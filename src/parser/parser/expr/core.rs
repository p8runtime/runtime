use super::super::Parser;
use crate::parser::ast::{
    Arg, ArrayItem, AssignOp, AttributeGroup, BinaryOp, CastKind, Expr, ExprId, IncludeKind,
    MagicConstKind, MatchArm, ObjectItem, ObjectKey, ParseError, UnaryOp, UnsafeCatch,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;

impl<'src, 'ast> Parser<'src, 'ast> {
    pub(in crate::parser::parser) fn parse_expr(&mut self, min_bp: u8) -> ExprId<'ast> {
        let mut left = self.parse_nud();
        let mut just_parsed_ternary = false;
        let mut just_parsed_elvis = false;

        loop {
            // PHPX ASI guard: a line break before `(` should terminate the expression
            // instead of continuing as a call chain on the next line.
            if self.is_phpx()
                && self.current_token.kind == TokenKind::OpenParen
                && self.has_line_terminator_between(self.prev_token.span, self.current_token.span)
            {
                break;
            }

            if self.current_token.kind == TokenKind::Dot && self.is_phpx() {
                let dot_span = self.current_token.span;
                let next = self.next_token;
                let left_span = left.span();
                let tight = left_span.end == dot_span.start && dot_span.end == next.span.start;

                if tight
                    && (next.kind == TokenKind::Identifier
                        || next.kind == TokenKind::StringVarname
                        || next.kind.is_semi_reserved())
                {
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump(); // consume .
                    let property = self.arena.alloc(self.current_token);
                    self.bump();
                    let span = Span::new(left_span.start, property.span.end);
                    left = self.arena.alloc(Expr::DotAccess {
                        target: left,
                        property,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
            }

            if self.is_ds() && self.current_token.kind == TokenKind::Dot {
                self.errors.push(ParseError::with_help(
                    self.current_token.span,
                    "PHP `.` concatenation is not part of DekaScript",
                    "Use `+` to concatenate strings; property access must be written without spaces.",
                ));
            }

            let op = match self.current_token.kind {
                TokenKind::Plus => BinaryOp::Plus,
                TokenKind::Minus => BinaryOp::Minus,
                TokenKind::Asterisk => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                TokenKind::Dot => BinaryOp::Concat,
                TokenKind::EqEq => BinaryOp::EqEq,
                TokenKind::EqEqEq => BinaryOp::EqEqEq,
                TokenKind::BangEq => BinaryOp::NotEq,
                TokenKind::BangEqEq => BinaryOp::NotEqEq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                TokenKind::AmpersandAmpersand => BinaryOp::And,
                TokenKind::PipePipe => BinaryOp::Or,
                TokenKind::Ampersand
                | TokenKind::AmpersandFollowedByVarOrVararg
                | TokenKind::AmpersandNotFollowedByVarOrVararg => BinaryOp::BitAnd,
                TokenKind::Pipe => BinaryOp::BitOr,
                TokenKind::PipeGt => BinaryOp::Pipe,
                TokenKind::Caret => BinaryOp::BitXor,
                TokenKind::LogicalAnd => BinaryOp::LogicalAnd,
                TokenKind::LogicalOr | TokenKind::Insteadof => BinaryOp::LogicalOr,
                TokenKind::LogicalXor => BinaryOp::LogicalXor,
                TokenKind::Coalesce => BinaryOp::Coalesce,
                TokenKind::Spaceship => BinaryOp::Spaceship,
                TokenKind::Pow => BinaryOp::Pow,
                TokenKind::Sl => BinaryOp::ShiftLeft,
                TokenKind::Sr => BinaryOp::ShiftRight,
                TokenKind::InstanceOf => BinaryOp::Instanceof,
                TokenKind::Question => {
                    // Ternary: a ? b : c
                    // PHP allows any expression in both branches, including low-precedence ones
                    let l_bp = 40;
                    if l_bp < min_bp {
                        break;
                    }

                    let current_is_elvis = self.next_token.kind == TokenKind::Colon;

                    if just_parsed_ternary && (!just_parsed_elvis || !current_is_elvis) {
                        self.errors.push(ParseError::new(self.current_token.span, "Unparenthesized `a ? b : c ? d : e` is not supported. Use either `(a ? b : c) ? d : e` or `a ? b : (c ? d : e)`"));
                    }

                    self.bump();

                    let if_true = if self.current_token.kind != TokenKind::Colon {
                        Some(self.parse_expr(0))
                    } else {
                        None
                    };

                    if self.current_token.kind == TokenKind::Colon {
                        self.bump();
                    }

                    // Use l_bp + 1 to enforce left-associativity for the else branch,
                    // which allows us to detect the unparenthesized nesting in the next iteration.
                    let if_false = self.parse_expr(l_bp + 1);

                    let span = Span::new(left.span().start, if_false.span().end);
                    left = self.arena.alloc(Expr::Ternary {
                        condition: left,
                        if_true,
                        if_false,
                        span,
                    });
                    just_parsed_ternary = true;
                    just_parsed_elvis = current_is_elvis;
                    continue;
                }
                TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::MulEq
                | TokenKind::DivEq
                | TokenKind::ModEq
                | TokenKind::ConcatEq
                | TokenKind::AndEq
                | TokenKind::OrEq
                | TokenKind::XorEq
                | TokenKind::SlEq
                | TokenKind::SrEq
                | TokenKind::PowEq
                | TokenKind::CoalesceEq => {
                    let op = match self.current_token.kind {
                        TokenKind::PlusEq => AssignOp::Plus,
                        TokenKind::MinusEq => AssignOp::Minus,
                        TokenKind::MulEq => AssignOp::Mul,
                        TokenKind::DivEq => AssignOp::Div,
                        TokenKind::ModEq => AssignOp::Mod,
                        TokenKind::ConcatEq => AssignOp::Concat,
                        TokenKind::AndEq => AssignOp::BitAnd,
                        TokenKind::OrEq => AssignOp::BitOr,
                        TokenKind::XorEq => AssignOp::BitXor,
                        TokenKind::SlEq => AssignOp::ShiftLeft,
                        TokenKind::SrEq => AssignOp::ShiftRight,
                        TokenKind::PowEq => AssignOp::Pow,
                        TokenKind::CoalesceEq => AssignOp::Coalesce,
                        _ => unreachable!(),
                    };

                    let l_bp = 35; // Same as Assignment
                    if l_bp < min_bp && (min_bp >= 80 || !self.is_assignable(left)) {
                        break;
                    }

                    if !self.is_assignable(left) {
                        if self.is_reassociable_assignment_target(left) {
                            self.bump();
                            let right = self.parse_expr(l_bp - 1);
                            left = self.reassociate_assignment(left, right, Some(op));
                            continue;
                        }

                        self.errors.push(ParseError::new(
                            left.span(),
                            "Assignments can only happen to writable values",
                        ));
                    }

                    self.bump();
                    let right = self.parse_expr(l_bp - 1);
                    let span = Span::new(left.span().start, right.span().end);
                    left = self.arena.alloc(Expr::AssignOp {
                        var: left,
                        op,
                        expr: right,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Eq => {
                    // Assignment: $a = 1
                    let l_bp = 35; // Higher than 'and' (30), lower than 'ternary' (40)
                    if l_bp < min_bp {
                        // Special check for PHP grammar quirk:
                        // If LHS is assignable, assignment binds tighter than anything (effectively),
                        // because "expr = ..." is invalid, only "var = ..." is valid.
                        // However, this only applies to lower precedence operators (<= &&).
                        // Higher precedence operators (like &, |, +, ++, $) do not allow assignment on RHS.
                        if min_bp >= 80 || !self.is_assignable(left) {
                            break;
                        }
                    }

                    if !self.is_assignable(left) {
                        if self.is_reassociable_assignment_target(left) {
                            self.bump();
                            let right = self.parse_expr(l_bp - 1);
                            left = self.reassociate_assignment(left, right, None);
                            continue;
                        }

                        self.errors.push(ParseError::new(
                            left.span(),
                            "Assignments can only happen to writable values",
                        ));
                    }

                    self.bump();

                    // Assignment by reference: $a =& $b
                    if matches!(
                        self.current_token.kind,
                        TokenKind::Ampersand
                            | TokenKind::AmpersandFollowedByVarOrVararg
                            | TokenKind::AmpersandNotFollowedByVarOrVararg
                    ) {
                        self.bump();
                        let right = self.parse_expr(l_bp - 1);
                        let span = Span::new(left.span().start, right.span().end);
                        left = self.arena.alloc(Expr::AssignRef {
                            var: left,
                            expr: right,
                            span,
                        });
                        continue;
                    }

                    // Right associative
                    let right = self.parse_expr(l_bp - 1);

                    let span = Span::new(left.span().start, right.span().end);
                    left = self.arena.alloc(Expr::Assign {
                        var: left,
                        expr: right,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::OpenBracket => {
                    // Array Dimension Fetch: $a[1]
                    let l_bp = 210; // Very high
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let dim = if self.current_token.kind == TokenKind::CloseBracket {
                        None
                    } else {
                        Some(self.parse_expr(0))
                    };

                    let end = if self.current_token.kind == TokenKind::CloseBracket {
                        let end = self.current_token.span.end;
                        self.bump();
                        end
                    } else {
                        self.current_token.span.start
                    };

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::ArrayDimFetch {
                        array: left,
                        dim,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }

                TokenKind::NullSafeArrow => {
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let prop_or_method = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    if self.current_token.kind == TokenKind::OpenParen {
                        let (args, args_span) = self.parse_call_arguments();
                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::NullsafeMethodCall {
                            target: left,
                            method: prop_or_method,
                            args,
                            span,
                        });
                    } else {
                        let span = Span::new(left.span().start, prop_or_method.span().end);
                        left = self.arena.alloc(Expr::NullsafePropertyFetch {
                            target: left,
                            property: prop_or_method,
                            span,
                        });
                    }
                    continue;
                }
                TokenKind::Arrow => {
                    // Property Fetch or Method Call: $a->b or $a->b()
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    // Expect identifier or variable (for dynamic property)
                    // For now assume identifier
                    let prop_or_method = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        // We need to wrap this token in an Expr
                        // Reusing Variable/Identifier logic from parse_nud would be good but we need to call it explicitly or just handle it here
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            // Using Variable for now, should be Identifier if it's a name
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        // Error
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    // Check for method call
                    if self.current_token.kind == TokenKind::OpenParen {
                        let (args, args_span) = self.parse_call_arguments();

                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::MethodCall {
                            target: left,
                            method: prop_or_method,
                            args,
                            span,
                        });
                    } else {
                        // Property Fetch
                        let span = Span::new(left.span().start, prop_or_method.span().end);
                        left = self.arena.alloc(Expr::PropertyFetch {
                            target: left,
                            property: prop_or_method,
                            span,
                        });
                    }
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::DoubleColon => {
                    // Static Property/Method/Const: A::b, A::b(), A::CONST
                    let l_bp = 210;
                    if l_bp < min_bp {
                        break;
                    }
                    self.bump();

                    let member = if matches!(
                        self.current_token.kind,
                        TokenKind::OpenBrace | TokenKind::DollarOpenCurlyBraces
                    ) {
                        self.bump();
                        let expr = self.parse_expr(0);
                        if self.current_token.kind == TokenKind::CloseBrace {
                            self.bump();
                        }
                        expr
                    } else if self.current_token.kind == TokenKind::Dollar {
                        let start = self.current_token.span.start;
                        self.bump();
                        if self.current_token.kind == TokenKind::OpenBrace {
                            self.bump();
                            let expr = self.parse_expr(0);
                            if self.current_token.kind == TokenKind::CloseBrace {
                                self.bump();
                            }
                            expr
                        } else if self.current_token.kind == TokenKind::Variable {
                            let token = self.current_token;
                            self.bump();
                            let span = Span::new(start, token.span.end);
                            self.arena.alloc(Expr::Variable { name: span, span })
                        } else {
                            self.arena.alloc(Expr::Error {
                                span: Span::new(start, self.current_token.span.end),
                            })
                        }
                    } else if self.current_token.kind == TokenKind::Identifier
                        || self.current_token.kind == TokenKind::Variable
                        || self.current_token.kind.is_semi_reserved()
                    {
                        let token = self.current_token;
                        self.bump();
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    } else {
                        self.arena.alloc(Expr::Error {
                            span: self.current_token.span,
                        })
                    };

                    if self.current_token.kind == TokenKind::OpenParen {
                        // Static Method Call
                        let (args, args_span) = self.parse_call_arguments();
                        let span = Span::new(left.span().start, args_span.end);
                        left = self.arena.alloc(Expr::StaticCall {
                            class: left,
                            method: member,
                            args,
                            span,
                        });
                    } else {
                        // Class Const Fetch (or static property if it starts with $)
                        // For now assume const fetch if identifier
                        let span = Span::new(left.span().start, member.span().end);
                        left = self.arena.alloc(Expr::ClassConstFetch {
                            class: left,
                            constant: member,
                            span,
                        });
                    }
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::OpenParen => {
                    // Function Call
                    let l_bp = 190;
                    if l_bp < min_bp {
                        break;
                    }

                    let (args, args_span) = self.parse_call_arguments();

                    let span = Span::new(left.span().start, args_span.end);
                    left = self.arena.alloc(Expr::Call {
                        func: left,
                        args,
                        span,
                    });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Inc => {
                    let l_bp = 180;
                    if l_bp < min_bp {
                        break;
                    }
                    let end = self.current_token.span.end;
                    self.bump();

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::PostInc { var: left, span });
                    just_parsed_ternary = false;
                    continue;
                }
                TokenKind::Dec => {
                    let l_bp = 180;
                    if l_bp < min_bp {
                        break;
                    }
                    let end = self.current_token.span.end;
                    self.bump();

                    let span = Span::new(left.span().start, end);
                    left = self.arena.alloc(Expr::PostDec { var: left, span });
                    just_parsed_ternary = false;
                    continue;
                }
                _ => break,
            };

            let (l_bp, r_bp) = self.infix_binding_power(op);
            if l_bp < min_bp {
                break;
            }

            self.bump();
            let right = self.parse_expr(r_bp);

            let span = Span::new(left.span().start, right.span().end);
            left = self.arena.alloc(Expr::Binary {
                left,
                op,
                right,
                span,
            });
            just_parsed_ternary = false;
        }

        left
    }

    fn parse_nud(&mut self) -> ExprId<'ast> {
        let mut attributes = &[] as &'ast [AttributeGroup<'ast>];
        if self.current_token.kind == TokenKind::Attribute {
            attributes = self.parse_attributes();
        }

        let token = self.current_token;
        match token.kind {
            TokenKind::Lt => {
                if self.is_phpx() {
                    return self.parse_jsx_element();
                }
                self.errors
                    .push(ParseError::new(token.span, "Unexpected '<' in expression"));
                self.bump();
                self.arena.alloc(Expr::Error { span: token.span })
            }
            TokenKind::Empty => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Empty {
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Isset
            | TokenKind::LogicalOr
            | TokenKind::Insteadof
            | TokenKind::LogicalAnd
            | TokenKind::LogicalXor => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut vars = bumpalo::collections::Vec::new_in(self.arena);
                vars.push(self.parse_expr(0));
                while self.current_token.kind == TokenKind::Comma {
                    self.bump();
                    if self.current_token.kind == TokenKind::CloseParen {
                        break;
                    }
                    vars.push(self.parse_expr(0));
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Isset {
                    vars: vars.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::Eval => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Eval {
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Die | TokenKind::Exit => {
                let start = token.span.start;
                let is_die = token.kind == TokenKind::Die;
                self.bump();
                let expr = if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                    let e = if self.current_token.kind == TokenKind::CloseParen {
                        None
                    } else {
                        Some(self.parse_expr(0))
                    };
                    if self.current_token.kind == TokenKind::CloseParen {
                        self.bump();
                    }
                    e
                } else {
                    None
                };
                let end = self.current_token.span.end;
                let span = Span::new(start, end);
                if is_die {
                    self.arena.alloc(Expr::Die { expr, span })
                } else {
                    self.arena.alloc(Expr::Exit { expr, span })
                }
            }
            TokenKind::Dir
            | TokenKind::File
            | TokenKind::Line
            | TokenKind::FuncC
            | TokenKind::ClassC
            | TokenKind::TraitC
            | TokenKind::MethodC
            | TokenKind::NsC
            | TokenKind::PropertyC => {
                let span = token.span;
                self.bump();
                self.arena.alloc(Expr::MagicConst {
                    kind: match token.kind {
                        TokenKind::Dir => MagicConstKind::Dir,
                        TokenKind::File => MagicConstKind::File,
                        TokenKind::Line => MagicConstKind::Line,
                        TokenKind::FuncC => MagicConstKind::Function,
                        TokenKind::ClassC => MagicConstKind::Class,
                        TokenKind::TraitC => MagicConstKind::Trait,
                        TokenKind::MethodC => MagicConstKind::Method,
                        TokenKind::NsC => MagicConstKind::Namespace,
                        TokenKind::PropertyC => MagicConstKind::Property,
                        _ => unreachable!(),
                    },
                    span,
                })
            }
            TokenKind::Include
            | TokenKind::IncludeOnce
            | TokenKind::Require
            | TokenKind::RequireOnce => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "include and require are not part of DekaScript",
                        "Use an explicit DekaScript import instead.",
                    ));
                }
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(0);
                let end = expr.span().end;
                self.arena.alloc(Expr::Include {
                    kind: match token.kind {
                        TokenKind::Include => IncludeKind::Include,
                        TokenKind::IncludeOnce => IncludeKind::IncludeOnce,
                        TokenKind::Require => IncludeKind::Require,
                        TokenKind::RequireOnce => IncludeKind::RequireOnce,
                        _ => unreachable!(),
                    },
                    expr,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Print => {
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(31);
                let span = Span::new(start, expr.span().end);
                self.arena.alloc(Expr::Print { expr, span })
            }
            TokenKind::Yield | TokenKind::YieldFrom => {
                let start = token.span.start;
                self.bump();

                let mut is_from = token.kind == TokenKind::YieldFrom;
                if !is_from && self.current_token.kind == TokenKind::Identifier {
                    let text = self.lexer.slice(self.current_token.span);
                    let mut lowered = text.to_vec();
                    lowered.make_ascii_lowercase();
                    if lowered == b"from" {
                        is_from = true;
                        self.bump(); // consume 'from'
                    }
                }

                if is_from {
                    let value = self.parse_expr(31);
                    let span = Span::new(start, value.span().end);
                    return self.arena.alloc(Expr::Yield {
                        key: None,
                        value: Some(value),
                        from: true,
                        span,
                    });
                }

                if matches!(
                    self.current_token.kind,
                    TokenKind::SemiColon
                        | TokenKind::CloseTag
                        | TokenKind::Eof
                        | TokenKind::CloseBrace
                        | TokenKind::Comma
                ) {
                    let span = Span::new(start, self.current_token.span.start);
                    return self.arena.alloc(Expr::Yield {
                        key: None,
                        value: None,
                        from: false,
                        span,
                    });
                }

                let first = self.parse_expr(31);
                let (key, value) = if self.current_token.kind == TokenKind::DoubleArrow {
                    self.bump();
                    let val = self.parse_expr(31);
                    (Some(first), val)
                } else {
                    (None, first)
                };
                let span = Span::new(start, value.span().end);
                self.arena.alloc(Expr::Yield {
                    key,
                    value: Some(value),
                    from: false,
                    span,
                })
            }

            TokenKind::Throw => {
                // Throw expression (PHP 8+): reuse error node to avoid a new variant
                let start = token.span.start;
                self.bump();
                let expr = self.parse_expr(0);
                let span = Span::new(start, expr.span().end);
                self.arena.alloc(Expr::Error { span })
            }

            TokenKind::Function => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                self.parse_closure_expr(attributes, false, false, start)
            }
            TokenKind::Fn => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                self.parse_arrow_function(attributes, false, false, start)
            }
            TokenKind::Static => {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump();
                match self.current_token.kind {
                    TokenKind::Function => {
                        self.bump();
                        self.parse_closure_expr(attributes, false, true, start)
                    }
                    TokenKind::Fn => {
                        self.bump();
                        self.parse_arrow_function(attributes, false, true, start)
                    }
                    TokenKind::DoubleColon => {
                        // static scope resolution (e.g., static::CONST)
                        self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        })
                    }
                    _ => self.arena.alloc(Expr::Variable {
                        name: token.span,
                        span: token.span,
                    }),
                }
            }
            TokenKind::Identifier if self.token_eq_ident(&token, b"await") => {
                if !self.is_phpx() {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "await is only available in PHPX mode",
                        "Use a .phpx file for async/await support.",
                    ));
                } else if self.fn_depth > 0 && self.async_fn_depth == 0 {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "await is only allowed in async functions",
                        "Mark the function as async or move await to module top level.",
                    ));
                }
                self.bump();
                let awaited = self.parse_expr(180);
                let span = Span::new(token.span.start, awaited.span().end);
                self.arena.alloc(Expr::Await {
                    expr: awaited,
                    span,
                })
            }
            TokenKind::Identifier
                if self.is_phpx()
                    && self.token_eq_ident(&token, b"async")
                    && self.next_token.kind == TokenKind::Function =>
            {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump(); // async
                self.bump(); // function
                self.parse_closure_expr(attributes, true, false, start)
            }
            TokenKind::Identifier
                if self.is_phpx()
                    && self.token_eq_ident(&token, b"async")
                    && self.next_token.kind == TokenKind::Fn =>
            {
                let start = if let Some(first) = attributes.first() {
                    first.span.start
                } else {
                    token.span.start
                };
                self.bump(); // async
                self.bump(); // fn
                self.parse_arrow_function(attributes, true, false, start)
            }
            TokenKind::New => {
                if self.is_phpx() {
                    self.errors.push(ParseError::new(
                        token.span,
                        "new is not allowed in PHPX; use struct literals instead",
                    ));
                }
                self.bump();

                let attributes = if self.current_token.kind == TokenKind::Attribute {
                    self.parse_attributes()
                } else {
                    &[]
                };

                // Parse optional modifiers for anonymous class
                let mut modifiers = std::vec::Vec::new();
                while matches!(
                    self.current_token.kind,
                    TokenKind::Abstract | TokenKind::Final | TokenKind::Readonly
                ) {
                    modifiers.push(self.current_token);
                    self.bump();
                }

                if self.current_token.kind == TokenKind::Class {
                    let (class, args) = self
                        .parse_anonymous_class(attributes, self.arena.alloc_slice_copy(&modifiers));
                    let span = Span::new(token.span.start, class.span().end);
                    self.arena.alloc(Expr::New { class, args, span })
                } else {
                    if !attributes.is_empty() || !modifiers.is_empty() {
                        let start = if let Some(attr) = attributes.first() {
                            attr.span.start
                        } else {
                            modifiers.first().unwrap().span.start
                        };
                        let end = if let Some(attr) = attributes.last() {
                            attr.span.end
                        } else {
                            modifiers.last().unwrap().span.end
                        };
                        self.errors.push(ParseError::new(Span::new(start, end), "Attributes and modifiers are only allowed on anonymous classes in new expression"));
                    }

                    let class = self.parse_expr(200); // High binding power to grab the class name

                    let (args, end_pos) = if self.current_token.kind == TokenKind::OpenParen {
                        let (a, s) = self.parse_call_arguments();
                        (a, s.end)
                    } else {
                        (&[] as &[Arg], class.span().end)
                    };

                    let span = Span::new(token.span.start, end_pos);
                    self.arena.alloc(Expr::New { class, args, span })
                }
            }
            TokenKind::Clone => {
                self.bump();
                let expr = self.parse_expr(200);
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Clone { expr, span })
            }
            TokenKind::Match => {
                let start = token.span.start;
                self.bump(); // Eat match

                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let condition = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }

                if self.current_token.kind == TokenKind::OpenBrace {
                    self.bump();
                }

                let mut arms = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::SemiColon {
                        self.errors
                            .push(ParseError::new(self.current_token.span, "Unexpected ';'"));
                        self.bump();
                        continue;
                    }

                    let arm_start = self.current_token.span.start;

                    let conditions = if self.current_token.kind == TokenKind::Default {
                        self.bump();
                        None
                    } else {
                        let mut conds = bumpalo::collections::Vec::new_in(self.arena);
                        conds.push(self.parse_expr(0));
                        while self.current_token.kind == TokenKind::Comma {
                            self.bump();
                            if self.current_token.kind == TokenKind::DoubleArrow {
                                break;
                            }
                            conds.push(self.parse_expr(0));
                        }
                        Some(conds.into_bump_slice() as &'ast [ExprId<'ast>])
                    };

                    if self.current_token.kind == TokenKind::DoubleArrow {
                        self.bump();
                    }

                    let body = self.parse_expr(0);

                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }

                    let arm_end = body.span().end;

                    arms.push(MatchArm {
                        conditions,
                        body,
                        span: Span::new(arm_start, arm_end),
                    });
                }

                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                }

                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Match {
                    condition,
                    arms: arms.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::Unsafe => {
                let start = token.span.start;
                self.bump(); // Eat unsafe

                if self.current_token.kind == TokenKind::OpenBrace {
                    self.bump();
                }
                let body = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseBrace {
                    self.bump();
                }

                let catch = if self.current_token.kind == TokenKind::Catch {
                    let catch_start = self.current_token.span.start;
                    self.bump();

                    if self.current_token.kind == TokenKind::OpenParen {
                        self.bump();
                    }

                    let var = if matches!(
                        self.current_token.kind,
                        TokenKind::Identifier | TokenKind::Variable
                    ) {
                        let t = self.arena.alloc(self.current_token);
                        self.bump();
                        &*t
                    } else {
                        self.errors.push(ParseError::new(
                            self.current_token.span,
                            "Expected catch variable",
                        ));
                        self.arena.alloc(Token {
                            kind: TokenKind::Error,
                            span: self.current_token.span,
                        })
                    };

                    if self.current_token.kind == TokenKind::CloseParen {
                        self.bump();
                    }

                    if self.current_token.kind == TokenKind::OpenBrace {
                        self.bump();
                    }
                    let catch_body = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    }
                    let catch_end = catch_body.span().end;

                    let catch_node = self.arena.alloc(UnsafeCatch {
                        var,
                        body: catch_body,
                        span: Span::new(catch_start, catch_end),
                    });
                    Some(&*catch_node)
                } else {
                    None
                };

                let finally = if self.current_token.kind == TokenKind::Finally {
                    self.bump();
                    if self.current_token.kind == TokenKind::OpenBrace {
                        self.bump();
                    }
                    let finally_body = self.parse_expr(0);
                    if self.current_token.kind == TokenKind::CloseBrace {
                        self.bump();
                    }
                    Some(finally_body)
                } else {
                    None
                };

                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Unsafe {
                    body,
                    catch,
                    finally,
                    span: Span::new(start, end),
                })
            }
            TokenKind::Dollar => {
                let start = self.current_token.span.start;
                self.bump();

                if self.current_token.kind == TokenKind::OpenBrace {
                    self.bump();
                    let expr = self.parse_expr(0);
                    let end = if self.current_token.kind == TokenKind::CloseBrace {
                        let end = self.current_token.span.end;
                        self.bump();
                        end
                    } else {
                        self.current_token.span.start
                    };

                    let span = Span::new(start, end);
                    self.arena
                        .alloc(Expr::IndirectVariable { name: expr, span })
                } else {
                    let expr = self.parse_expr(200);
                    let span = Span::new(start, expr.span().end);
                    self.arena
                        .alloc(Expr::IndirectVariable { name: expr, span })
                }
            }
            TokenKind::StringVarname => {
                self.bump();
                self.arena.alloc(Expr::Variable {
                    name: token.span,
                    span: token.span,
                })
            }
            TokenKind::Variable => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "DekaScript references use bare identifiers",
                        "Write `name`, not `$name`.",
                    ));
                }
                self.bump();
                self.arena.alloc(Expr::Variable {
                    name: token.span,
                    span: token.span,
                })
            }
            TokenKind::LNumber => {
                self.bump();
                self.arena.alloc(Expr::Integer {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::DNumber => {
                self.bump();
                self.arena.alloc(Expr::Float {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::StringLiteral => {
                self.bump();
                self.arena.alloc(Expr::String {
                    value: self.arena.alloc_slice_copy(self.lexer.slice(token.span)),
                    span: token.span,
                })
            }
            TokenKind::DoubleQuote => self.parse_interpolated_string(TokenKind::DoubleQuote),
            TokenKind::StartHeredoc => self.parse_interpolated_string(TokenKind::EndHeredoc),
            TokenKind::Backtick => self.parse_interpolated_string(TokenKind::Backtick),
            TokenKind::TypeTrue => {
                self.bump();
                self.arena.alloc(Expr::Boolean {
                    value: true,
                    span: token.span,
                })
            }
            TokenKind::TypeFalse => {
                self.bump();
                self.arena.alloc(Expr::Boolean {
                    value: false,
                    span: token.span,
                })
            }
            TokenKind::TypeNull => {
                self.bump();
                self.arena.alloc(Expr::Null { span: token.span })
            }
            TokenKind::Identifier
            | TokenKind::Namespace
            | TokenKind::NsSeparator
            | TokenKind::Enum
            | TokenKind::TypeInt
            | TokenKind::TypeFloat
            | TokenKind::TypeBool
            | TokenKind::TypeString
            | TokenKind::TypeVoid
            | TokenKind::TypeNever
            | TokenKind::TypeMixed
            | TokenKind::TypeIterable
            | TokenKind::TypeObject
            | TokenKind::TypeCallable
            | TokenKind::Readonly => {
                let name = self.parse_name();
                if self.is_ds()
                    && (self.lexer.slice(name.span).eq_ignore_ascii_case(b"count")
                        || matches!(
                            self.lexer.slice(name.span),
                            b"strlen"
                                | b"substr"
                                | b"array_keys"
                                | b"array_values"
                                | b"is_array"
                                | b"in_array"
                                | b"array_map"
                                | b"array_filter"
                        ))
                {
                    self.errors.push(ParseError::with_help(
                        name.span,
                        "PHP built-ins are not part of DekaScript",
                        "Use DekaScript string and collection methods instead.",
                    ));
                }
                if self.is_phpx() && self.current_token.kind == TokenKind::OpenBrace {
                    return self.parse_struct_literal(name, name.span.start);
                }
                self.arena.alloc(Expr::Variable {
                    name: name.span,
                    span: name.span,
                })
            }
            TokenKind::Bang => {
                self.bump();
                let expr = self.parse_expr(160); // BP for !
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Unary {
                    op: UnaryOp::Not,
                    expr,
                    span,
                })
            }
            TokenKind::Minus
            | TokenKind::Plus
            | TokenKind::BitNot
            | TokenKind::At
            | TokenKind::Inc
            | TokenKind::Dec
            | TokenKind::Ampersand
            | TokenKind::AmpersandFollowedByVarOrVararg
            | TokenKind::AmpersandNotFollowedByVarOrVararg => {
                let op = match token.kind {
                    TokenKind::Minus => UnaryOp::Minus,
                    TokenKind::Plus => UnaryOp::Plus,
                    TokenKind::BitNot => UnaryOp::BitNot,
                    TokenKind::At => UnaryOp::ErrorSuppress,
                    TokenKind::Inc => UnaryOp::PreInc,
                    TokenKind::Dec => UnaryOp::PreDec,
                    TokenKind::Ampersand
                    | TokenKind::AmpersandFollowedByVarOrVararg
                    | TokenKind::AmpersandNotFollowedByVarOrVararg => UnaryOp::Reference,
                    _ => unreachable!(),
                };
                self.bump();
                let expr = self.parse_expr(180); // BP for unary +, -, ~, ++, --
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Unary { op, expr, span })
            }
            TokenKind::IntCast
            | TokenKind::BoolCast
            | TokenKind::FloatCast
            | TokenKind::StringCast
            | TokenKind::ArrayCast
            | TokenKind::ObjectCast
            | TokenKind::UnsetCast
            | TokenKind::VoidCast => {
                if self.is_ds() {
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "PHP casts are not part of DekaScript",
                        "Use explicit DekaScript conversion APIs instead.",
                    ));
                }
                let kind = match token.kind {
                    TokenKind::IntCast => CastKind::Int,
                    TokenKind::BoolCast => CastKind::Bool,
                    TokenKind::FloatCast => CastKind::Float,
                    TokenKind::StringCast => CastKind::String,
                    TokenKind::ArrayCast => CastKind::Array,
                    TokenKind::ObjectCast => CastKind::Object,
                    TokenKind::UnsetCast => CastKind::Unset,
                    TokenKind::VoidCast => CastKind::Void,
                    _ => unreachable!(),
                };
                self.bump();
                let expr = self.parse_expr(180); // BP for casts (same as unary)
                let span = Span::new(token.span.start, expr.span().end);
                self.arena.alloc(Expr::Cast { kind, expr, span })
            }
            TokenKind::Array => {
                if self.is_ds() {
                    if self.ds_array_callable_declared {
                        self.bump();
                        return self.arena.alloc(Expr::Variable {
                            name: token.span,
                            span: token.span,
                        });
                    }
                    self.errors.push(ParseError::with_help(
                        token.span,
                        "PHP array() is not part of DekaScript",
                        "Use `[]` for a list or `{}` for an object.",
                    ));
                }
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseParen
                    && self.current_token.kind != TokenKind::Eof
                {
                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::List => {
                let start = token.span.start;
                self.bump();
                if self.current_token.kind == TokenKind::OpenParen {
                    self.bump();
                }
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseParen
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::Comma {
                        // Empty slot in list()
                        items.push(ArrayItem {
                            key: None,
                            value: self.arena.alloc(Expr::Error {
                                span: self.current_token.span,
                            }),
                            by_ref: false,
                            unpack: false,
                            span: self.current_token.span,
                        });
                        self.bump();
                        continue;
                    }
                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                        // allow trailing comma
                        if self.current_token.kind == TokenKind::CloseParen {
                            break;
                        }
                    }
                }
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenBracket => {
                // Short array syntax [1, 2, 3]
                let start = token.span.start;
                self.bump();
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBracket
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::Comma {
                        // Empty slot in short array destructuring [a, , b]
                        items.push(ArrayItem {
                            key: None,
                            value: self.arena.alloc(Expr::Error {
                                span: self.current_token.span,
                            }),
                            by_ref: false,
                            unpack: false,
                            span: self.current_token.span,
                        });
                        self.bump();
                        continue;
                    }

                    items.push(self.parse_array_item());
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                    }
                }
                if self.current_token.kind == TokenKind::CloseBracket {
                    self.bump();
                }
                let end = self.current_token.span.end;
                self.arena.alloc(Expr::Array {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenBrace => {
                if !self.is_phpx() && !self.is_ds() {
                    self.bump();
                    return self.arena.alloc(Expr::Error { span: token.span });
                }

                let start = token.span.start;
                self.bump(); // consume {
                let mut items = bumpalo::collections::Vec::new_in(self.arena);
                while self.current_token.kind != TokenKind::CloseBrace
                    && self.current_token.kind != TokenKind::Eof
                {
                    if self.current_token.kind == TokenKind::Comma {
                        self.bump();
                        continue;
                    }

                    // Spread element: `...expr`
                    if self.current_token.kind == TokenKind::Ellipsis {
                        let ellipsis_tok = self.arena.alloc(self.current_token);
                        let ellipsis_start = self.current_token.span.start;
                        self.bump(); // ...
                        let expr = self.parse_expr(0);
                        let spread = self.arena.alloc(Expr::Spread {
                            expr,
                            span: Span::new(ellipsis_start, expr.span().end),
                        });
                        let span = Span::new(ellipsis_start, spread.span().end);
                        items.push(ObjectItem {
                            key: ObjectKey::Ident(ellipsis_tok),
                            value: spread,
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
                        continue;
                    }

                    let (key, key_start) = match self.current_token.kind {
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
                                "Expected identifier, string literal, or '...' in object literal",
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
                            "Expected ':' after object key",
                        ));
                    }

                    let value = self.parse_expr(0);
                    let span = Span::new(key_start, value.span().end);
                    items.push(ObjectItem { key, value, span });

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

                self.arena.alloc(Expr::ObjectLiteral {
                    items: items.into_bump_slice(),
                    span: Span::new(start, end),
                })
            }
            TokenKind::OpenParen => {
                if self.looks_like_parenthesized_arrow_function() {
                    let start = token.span.start;
                    return self.parse_parenthesized_arrow_function(attributes, start);
                }
                self.bump();
                let expr = self.parse_expr(0);
                if self.current_token.kind == TokenKind::CloseParen {
                    self.bump();
                }
                expr
            }
            TokenKind::Error => {
                self.errors
                    .push(ParseError::new(token.span, "Unexpected token"));
                self.bump();
                self.arena.alloc(Expr::Error { span: token.span })
            }
            _ => {
                // Error recovery
                let is_terminator = matches!(
                    token.kind,
                    TokenKind::SemiColon
                        | TokenKind::CloseBrace
                        | TokenKind::CloseTag
                        | TokenKind::Eof
                );

                self.errors
                    .push(ParseError::new(token.span, "Syntax error"));

                if is_terminator {
                    // Do not consume terminator, let the statement parser handle it
                    self.arena.alloc(Expr::Error {
                        span: Span::new(token.span.start, token.span.start),
                    })
                } else {
                    self.bump();
                    self.arena.alloc(Expr::Error { span: token.span })
                }
            }
        }
    }
}
