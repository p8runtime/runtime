use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn allow_internal_bridge_call(&self) -> bool {
        let Some(path) = self.file_path.as_deref() else {
            // Unit tests and synthetic checks may not carry a path.
            return true;
        };
        path_has_php_modules_bridge(path)
    }

    pub(in crate::phpx::typeck::check) fn check_method_call_signature(
        &mut self,
        target_ty: &Type,
        method: ExprId<'a>,
        args: &'a [crate::parser::ast::Arg<'a>],
        env: &HashMap<String, Type>,
        span: Span,
        receiver_mutable: bool,
    ) -> Type {
        let Some(method_name) = self.extract_static_ident(method) else {
            return Type::Unknown;
        };
        self.check_method_call_signature_by_name(
            target_ty,
            &method_name,
            args,
            env,
            span,
            receiver_mutable,
        )
    }

    pub(in crate::phpx::typeck::check) fn check_method_call_signature_by_name(
        &mut self,
        target_ty: &Type,
        method_name: &str,
        args: &'a [crate::parser::ast::Arg<'a>],
        env: &HashMap<String, Type>,
        span: Span,
        receiver_mutable: bool,
    ) -> Type {
        // Go-style method precedence: an outer struct method shadows promoted
        // embedded methods. If two or more embedded structs promote the same
        // method and the outer struct does not define it, the call is ambiguous.
        if let Type::Struct(name) = target_ty {
            let is_own = self
                .own_struct_methods
                .get(name)
                .map(|own| own.contains(method_name))
                .unwrap_or(false);
            let is_ambiguous = !is_own
                && self
                    .ambiguous_promoted_methods
                    .get(name)
                    .map(|set| set.contains(method_name))
                    .unwrap_or(false);
            if is_ambiguous {
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: format!(
                        "Ambiguous method '{}' on struct '{}'; multiple embedded structs promote it",
                        method_name, name
                    ),
                });
                return Type::Unknown;
            }
        }

        let (owner_label, sig) = match target_ty {
            Type::Struct(name) => (
                Some(format!("struct {}", name)),
                self.struct_methods
                    .get(name)
                    .and_then(|methods| methods.get(method_name))
                    .cloned(),
            ),
            Type::Interface(name) => (
                Some(format!("interface {}", name)),
                self.interfaces
                    .get(name)
                    .and_then(|info| info.methods.get(method_name))
                    .cloned(),
            ),
            Type::Enum(name) => (
                Some(format!("enum {}", name)),
                self.enum_methods
                    .get(name)
                    .and_then(|methods| methods.get(method_name))
                    .cloned(),
            ),
            Type::EnumCase { enum_name, .. } => (
                Some(format!("enum {}", enum_name)),
                self.enum_methods
                    .get(enum_name)
                    .and_then(|methods| methods.get(method_name))
                    .cloned(),
            ),
            _ => (None, None),
        };

        let Some(sig) = sig else {
            if let Some(owner) = owner_label {
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: format!("Unknown method '{}' on {}", method_name, owner),
                });
            }
            return Type::Unknown;
        };

        if sig.mutable && !receiver_mutable {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "Cannot call mutable method '{}' on an immutable receiver",
                    method_name
                ),
            });
        }

        let required = sig.params.iter().filter(|p| p.required).count();
        if args.len() < required {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "Missing arguments for {}(): expected at least {}, got {}",
                    method_name,
                    required,
                    args.len()
                ),
            });
        }

        let mut actuals = Vec::new();
        for arg in args.iter() {
            actuals.push(self.infer_expr_with_env(arg.value, env));
        }

        let mut idx = 0;
        while idx < args.len() {
            let param_ty = if idx >= sig.params.len() {
                if sig.variadic {
                    sig.params.last().and_then(|p| p.ty.as_ref())
                } else {
                    None
                }
            } else {
                sig.params[idx].ty.as_ref()
            };
            if let Some(param_ty) = param_ty {
                if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null))
                    && self.strict_null
                    && !self.type_allows_null(param_ty)
                {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: args[idx].span,
                        message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                    });
                }
                if let Expr::ObjectLiteral { items, span } = *args[idx].value {
                    self.check_object_literal_against_type(items, param_ty, span, env);
                }
                if !self.is_assignable(&actuals[idx], param_ty) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: args[idx].span,
                        message: format!(
                            "Argument {} type mismatch: expected {}, got {}",
                            idx + 1,
                            param_ty,
                            actuals[idx]
                        ),
                    });
                }
            } else if self.strict_null
                && matches!(actuals[idx], Type::Primitive(PrimitiveType::Null))
            {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: args[idx].span,
                    message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                });
            }
            idx += 1;
        }

        sig.return_type.clone().unwrap_or(Type::Unknown)
    }
}
