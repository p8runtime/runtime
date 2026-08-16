use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn check_stmt(
        &mut self,
        stmt: &Stmt<'a>,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        return_type: Option<&Type>,
    ) {
        match stmt {
            Stmt::Return { expr, span } => {
                if let Some(expected) = return_type {
                    let actual = expr
                        .map(|expr| self.check_expr(expr, env, explicit))
                        .unwrap_or_else(|| Type::Primitive(PrimitiveType::Null));
                    if let Some(expr) = expr {
                        if let Expr::Null { span: null_span } = *expr {
                            if self.strict_null && !self.type_allows_null(expected) {
                                self.errors.push(TypeError {
                                    span: *null_span,
                                    message: "Null is not allowed in PHPX; use Option<T> instead"
                                        .to_string(),
                                });
                            }
                        }
                    } else if self.strict_null && !self.type_allows_null(expected) {
                        self.errors.push(TypeError {
                            span: *span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
                                .to_string(),
                        });
                    }
                    if let Some(expr) = expr {
                        if let Expr::ObjectLiteral {
                            items,
                            span: obj_span,
                        } = *expr
                        {
                            self.check_object_literal_against_type(items, expected, *obj_span, env);
                        }
                    }
                    if !self.is_assignable(&actual, expected) {
                        self.errors.push(TypeError {
                            span: *span,
                            message: format!(
                                "Return type mismatch: expected {}, got {}",
                                expected, actual
                            ),
                        });
                    }
                }
                if self.strict_null && return_type.is_none() {
                    if let Some(expr) = expr {
                        if let Expr::Null { span: null_span } = *expr {
                            self.errors.push(TypeError {
                                span: *null_span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                    }
                }
            }
            Stmt::Expression { expr, .. } => {
                // Register cql bindings as variables in scope
                if let Expr::Cql { name, .. } = *expr {
                    let binding = token_text(self.source, name.span);
                    env.insert(binding, Type::Unknown);
                }
                if self.strict_null {
                    if let Expr::Null { span } = *expr {
                        self.errors.push(TypeError {
                            span: *span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
                                .to_string(),
                        });
                    }
                }
                let _ = self.check_expr(expr, env, explicit);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let _ = self.check_expr(condition, env, explicit);
                let mut then_env = self.narrow_env_for_condition(condition, env, true);
                let mut then_explicit = explicit.clone();
                for stmt in then_block.iter() {
                    self.check_stmt(stmt, &mut then_env, &mut then_explicit, return_type);
                }
                if let Some(else_block) = else_block {
                    let mut else_env = self.narrow_env_for_condition(condition, env, false);
                    let mut else_explicit = explicit.clone();
                    for stmt in else_block.iter() {
                        self.check_stmt(stmt, &mut else_env, &mut else_explicit, return_type);
                    }
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let _ = self.check_expr(condition, env, explicit);
                let mut loop_env = self.narrow_env_for_condition(condition, env, true);
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
                let _ = self.check_expr(condition, env, explicit);
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                for expr in init.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                for expr in condition.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                for expr in loop_expr.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                let mut loop_env = if condition.len() == 1 {
                    self.narrow_env_for_condition(condition[0], env, true)
                } else {
                    env.clone()
                };
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::Foreach {
                expr,
                key_var,
                value_var,
                body,
                ..
            } => {
                let _ = self.check_expr(expr, env, explicit);
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();

                if let Expr::Variable { name, .. } = *value_var {
                    let value_name = token_text(self.source, *name)
                        .trim_start_matches('$')
                        .to_string();
                    loop_env.insert(value_name.clone(), Type::Unknown);
                    loop_explicit.insert(value_name);
                }

                if let Some(key_expr) = key_var {
                    if let Expr::Variable { name, .. } = *key_expr {
                        let key_name = token_text(self.source, *name)
                            .trim_start_matches('$')
                            .to_string();
                        loop_env.insert(key_name.clone(), Type::Unknown);
                        loop_explicit.insert(key_name);
                    }
                }

                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::Block { statements, .. } => {
                let mut block_env = env.clone();
                let mut block_explicit = explicit.clone();
                for stmt in statements.iter() {
                    self.check_stmt(stmt, &mut block_env, &mut block_explicit, return_type);
                }
            }
            Stmt::Function {
                is_async,
                type_params,
                params,
                return_type: fn_return,
                body,
                ..
            } => {
                let (type_param_sigs, type_param_set) = self.collect_type_param_sigs(type_params);
                let mut fn_env: HashMap<String, Type> = HashMap::new();
                let mut fn_explicit: HashSet<String> = HashSet::new();
                let destructured_params = self.detect_destructured_param_carriers(params, body);
                for param in params.iter() {
                    let param_name = token_text(self.source, param.name.span);
                    let param_name = param_name.trim_start_matches('$').to_string();
                    if destructured_params.contains(&param_name) {
                        if let Some(ty) = param.ty {
                            let resolved = self.resolve_type_with_params(ty, &type_param_set);
                            if let Type::Struct(ref name) = resolved {
                                self.errors.push(TypeError {
                                    span: param.span,
                                    message: format!(
                                        "Destructured parameter '${}' cannot use struct type '{}'; use interface '{}' or Object<{{...}}>",
                                        param_name, name, name
                                    ),
                                });
                            }
                            // Keep the original carrier variable in scope so lowered
                            // destructuring assignments (e.g. $name = $name.name) type-check.
                            let binding_ty =
                                self.field_type_for_pattern_key(&resolved, &param_name);
                            let binding_ty = if matches!(binding_ty, Type::Unknown) {
                                resolved
                            } else {
                                binding_ty
                            };
                            fn_env.insert(param_name.clone(), binding_ty);
                            fn_explicit.insert(param_name.clone());
                        }
                        continue;
                    }
                    if let Some(ty) = param.ty {
                        let resolved = self.resolve_type_with_params(ty, &type_param_set);
                        fn_env.insert(param_name.clone(), resolved);
                        fn_explicit.insert(param_name);
                    } else {
                        // Untyped params are still valid variables in scope
                        fn_env.insert(param_name.clone(), Type::Unknown);
                    }
                    if let Some(default) = param.default {
                        if let Some(ty) = param.ty {
                            let expected = self.resolve_type_with_params(ty, &type_param_set);
                            let actual = self.check_expr(default, env, explicit);
                            if !self.is_assignable(&actual, &expected) {
                                self.errors.push(TypeError {
                                    span: param.span,
                                    message: format!(
                                        "Default parameter type mismatch: expected {}, got {}",
                                        expected, actual
                                    ),
                                });
                            }
                        }
                    }
                }
                let expected_return =
                    fn_return.map(|ty| self.resolve_type_with_params(ty, &type_param_set));
                let body_return = if *is_async {
                    match expected_return.as_ref() {
                        Some(Type::Applied { base, args })
                            if base.eq_ignore_ascii_case("Promise") =>
                        {
                            Some(args.first().cloned().unwrap_or(Type::Unknown))
                        }
                        Some(other) => {
                            self.errors.push(TypeError {
                                span: stmt.span(),
                                message: format!(
                                    "Async function must declare Promise<T> return type, got {}",
                                    other
                                ),
                            });
                            Some(Type::Unknown)
                        }
                        None => None,
                    }
                } else {
                    expected_return.clone()
                };
                self.fn_depth += 1;
                if *is_async {
                    self.async_depth += 1;
                }
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut fn_env, &mut fn_explicit, body_return.as_ref());
                }
                if *is_async {
                    self.async_depth = self.async_depth.saturating_sub(1);
                }
                self.fn_depth = self.fn_depth.saturating_sub(1);

                let _ = type_param_sigs;
            }
            Stmt::Class { kind, members, .. } => {
                if *kind == ClassKind::Struct {
                    self.check_struct_defaults(members);
                }
            }
            Stmt::TypeAlias { .. } => {}
            Stmt::Impl {
                trait_name,
                target,
                members,
                span,
                ..
            } => {
                // RFD 19 conformance check: every non-default method the
                // trait declares must be provided somewhere in this impl
                // block. Signature compatibility is intentionally not yet
                // checked here (method presence is the load-bearing gap that
                // was silently accepted before this) -- narrower than the
                // full rule, real progress over the status quo.
                if let Some(trait_ref) = trait_name {
                    let trait_key = token_text(self.source, trait_ref.parts[0].span);
                    let target_key = token_text(self.source, target.parts[0].span);
                    if let Some(info) = self.traits.get(&trait_key).cloned() {
                        let provided: HashSet<String> = members
                            .iter()
                            .filter_map(|m| match m {
                                ClassMember::Method { name, .. } => {
                                    Some(token_text(self.source, name.span))
                                }
                                _ => None,
                            })
                            .collect();
                        let mut missing: Vec<&str> = info
                            .methods
                            .iter()
                            .filter(|(name, (_, has_default))| {
                                !has_default && !provided.contains(name.as_str())
                            })
                            .map(|(name, _)| name.as_str())
                            .collect();
                        missing.sort();
                        if !missing.is_empty() {
                            self.errors.push(TypeError {
                                span: *span,
                                message: format!(
                                    "{target_key} does not implement all methods required by {trait_key}: missing {}",
                                    missing.join(", ")
                                ),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::phpx::typeck::check) fn detect_destructured_param_carriers(
        &self,
        params: &[crate::parser::ast::Param<'a>],
        body: &[StmtId<'a>],
    ) -> HashSet<String> {
        let param_names: HashSet<String> = params
            .iter()
            .map(|param| {
                token_text(self.source, param.name.span)
                    .trim_start_matches('$')
                    .to_string()
            })
            .collect();
        let mut out = HashSet::new();
        for stmt in body.iter().take(24) {
            let Stmt::Expression { expr, .. } = &**stmt else {
                continue;
            };
            let Expr::Assign { var, expr, .. } = *expr else {
                continue;
            };
            if let Expr::Variable { span, .. } = *var {
                let name = token_text(self.source, *span)
                    .trim_start_matches('$')
                    .to_string();
                if !param_names.contains(&name) {
                    continue;
                }
                if self.expr_contains_named_var(expr, &name) {
                    out.insert(name);
                }
                continue;
            }
            if let Expr::Variable { span, .. } = *expr {
                let name = token_text(self.source, *span)
                    .trim_start_matches('$')
                    .to_string();
                if !param_names.contains(&name) {
                    continue;
                }
                if matches!(*var, Expr::ObjectLiteral { .. } | Expr::Array { .. }) {
                    out.insert(name);
                }
            }
        }
        out
    }

    pub(in crate::phpx::typeck::check) fn expr_contains_named_var(
        &self,
        expr: ExprId<'a>,
        name: &str,
    ) -> bool {
        match *expr {
            Expr::Variable { span, .. } => {
                token_text(self.source, span).trim_start_matches('$') == name
            }
            Expr::Binary { left, right, .. } => {
                self.expr_contains_named_var(left, name)
                    || self.expr_contains_named_var(right, name)
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                self.expr_contains_named_var(array, name)
                    || dim
                        .map(|dim| self.expr_contains_named_var(dim, name))
                        .unwrap_or(false)
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                self.expr_contains_named_var(target, name)
                    || self.expr_contains_named_var(property, name)
            }
            Expr::DotAccess { target, .. } => self.expr_contains_named_var(target, name),
            Expr::Assign { var, expr, .. } => {
                self.expr_contains_named_var(var, name) || self.expr_contains_named_var(expr, name)
            }
            Expr::ObjectLiteral { items, .. } => items
                .iter()
                .any(|item| self.expr_contains_named_var(item.value, name)),
            Expr::Array { items, .. } => items
                .iter()
                .any(|item| self.expr_contains_named_var(item.value, name)),
            _ => false,
        }
    }

    pub(in crate::phpx::typeck::check) fn is_self_destructure_assignment_expr(
        &self,
        expr: ExprId<'a>,
        name: &str,
    ) -> bool {
        match *expr {
            Expr::PropertyFetch { target, .. } | Expr::ArrayDimFetch { array: target, .. } => {
                self.expr_is_named_var(target, name)
            }
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                ..
            } => self.is_self_destructure_assignment_expr(left, name),
            _ => false,
        }
    }

    pub(in crate::phpx::typeck::check) fn expr_is_named_var(
        &self,
        expr: ExprId<'a>,
        name: &str,
    ) -> bool {
        match *expr {
            Expr::Variable { span, .. } => {
                token_text(self.source, span).trim_start_matches('$') == name
            }
            _ => false,
        }
    }
}
