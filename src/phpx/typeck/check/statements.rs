use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn check_stmt(
        &mut self,
        stmt: &Stmt<'a>,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        return_type: Option<&Type>,
        mut_env: &mut HashSet<String>,
    ) {
        match stmt {
            Stmt::Return { expr, span } => {
                if let Some(expected) = return_type {
                    let actual = expr
                        .map(|expr| self.check_expr(expr, env, explicit, mut_env))
                        .unwrap_or_else(|| Type::Primitive(PrimitiveType::Null));
                    if let Some(expr) = expr {
                        if let Expr::Null { span: null_span } = *expr {
                            if self.strict_null && !self.type_allows_null(expected) {
                                self.errors.push(TypeError { severity: Severity::Error,
                                    span: *null_span,
                                    message: "Null is not allowed in DekaScript; use Option<T> instead"
                                        .to_string(),
                                });
                            }
                        }
                    } else if self.strict_null && !self.type_allows_null(expected) {
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: *span,
                            message: "Null is not allowed in DekaScript; use Option<T> instead"
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
                        self.errors.push(TypeError { severity: Severity::Error,
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
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: *null_span,
                                message: "Null is not allowed in DekaScript; use Option<T> instead"
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
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: *span,
                            message: "Null is not allowed in DekaScript; use Option<T> instead"
                                .to_string(),
                        });
                    }
                }
                let _ = self.check_expr(expr, env, explicit, mut_env);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let _ = self.check_expr(condition, env, explicit, mut_env);
                let mut then_env = self.narrow_env_for_condition(condition, env, true);
                let mut then_explicit = explicit.clone();
                let mut then_mut_env = mut_env.clone();
                for stmt in then_block.iter() {
                    self.check_stmt(
                        stmt,
                        &mut then_env,
                        &mut then_explicit,
                        return_type,
                        &mut then_mut_env,
                    );
                }
                if let Some(else_block) = else_block {
                    let mut else_env = self.narrow_env_for_condition(condition, env, false);
                    let mut else_explicit = explicit.clone();
                    let mut else_mut_env = mut_env.clone();
                    for stmt in else_block.iter() {
                        self.check_stmt(
                            stmt,
                            &mut else_env,
                            &mut else_explicit,
                            return_type,
                            &mut else_mut_env,
                        );
                    }
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let _ = self.check_expr(condition, env, explicit, mut_env);
                let mut loop_env = self.narrow_env_for_condition(condition, env, true);
                let mut loop_explicit = explicit.clone();
                let mut loop_mut_env = mut_env.clone();
                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut loop_env,
                        &mut loop_explicit,
                        return_type,
                        &mut loop_mut_env,
                    );
                }
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();
                let mut loop_mut_env = mut_env.clone();
                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut loop_env,
                        &mut loop_explicit,
                        return_type,
                        &mut loop_mut_env,
                    );
                }
                let _ = self.check_expr(condition, env, explicit, mut_env);
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                for expr in init.iter() {
                    let _ = self.check_expr(expr, env, explicit, mut_env);
                }
                for expr in condition.iter() {
                    let _ = self.check_expr(expr, env, explicit, mut_env);
                }
                for expr in loop_expr.iter() {
                    let _ = self.check_expr(expr, env, explicit, mut_env);
                }
                let mut loop_env = if condition.len() == 1 {
                    self.narrow_env_for_condition(condition[0], env, true)
                } else {
                    env.clone()
                };
                let mut loop_explicit = explicit.clone();
                let mut loop_mut_env = mut_env.clone();
                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut loop_env,
                        &mut loop_explicit,
                        return_type,
                        &mut loop_mut_env,
                    );
                }
            }
            Stmt::Foreach {
                expr,
                key_var,
                value_var,
                body,
                ..
            } => {
                let _ = self.check_expr(expr, env, explicit, mut_env);
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();
                let mut loop_mut_env = mut_env.clone();

                if let Expr::Variable { name, .. } = *value_var {
                    let value_name = token_text(self.source, *name)
                        .trim_start_matches('$')
                        .to_string();
                    loop_env.insert(value_name.clone(), Type::Unknown);
                    loop_explicit.insert(value_name.clone());
                    loop_mut_env.insert(value_name);
                }

                if let Some(key_expr) = key_var {
                    if let Expr::Variable { name, .. } = *key_expr {
                        let key_name = token_text(self.source, *name)
                            .trim_start_matches('$')
                            .to_string();
                        loop_env.insert(key_name.clone(), Type::Unknown);
                        loop_explicit.insert(key_name.clone());
                        loop_mut_env.insert(key_name);
                    }
                }

                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut loop_env,
                        &mut loop_explicit,
                        return_type,
                        &mut loop_mut_env,
                    );
                }
            }
            Stmt::Block { statements, .. } => {
                let mut block_env = env.clone();
                let mut block_explicit = explicit.clone();
                let mut block_mut_env = mut_env.clone();
                for stmt in statements.iter() {
                    self.check_stmt(
                        stmt,
                        &mut block_env,
                        &mut block_explicit,
                        return_type,
                        &mut block_mut_env,
                    );
                }
            }
            Stmt::Function {
                name,
                is_async,
                type_params,
                params,
                return_type: fn_return,
                body,
                ..
            } => {
                let fn_name = token_text(self.source, name.span);
                let (type_param_sigs, type_param_set) = self.collect_type_param_sigs(type_params);
                let mut fn_env: HashMap<String, Type> = HashMap::new();
                let mut fn_explicit: HashSet<String> = HashSet::new();
                let mut fn_mut_env: HashSet<String> = HashSet::new();
                let destructured_params = self.detect_destructured_param_carriers(params, body);
                for param in params.iter() {
                    let param_name = token_text(self.source, param.name.span);
                    let param_name = param_name.trim_start_matches('$').to_string();
                    if self.param_is_mut(param) {
                        fn_mut_env.insert(param_name.clone());
                    }
                    if destructured_params.contains(&param_name) {
                        if let Some(ty) = param.ty {
                            let resolved = self.resolve_type_with_params(ty, &type_param_set);
                            if let Type::Struct(ref name) = resolved {
                                self.errors.push(TypeError { severity: Severity::Error,
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
                        fn_explicit.insert(param_name.clone());
                    } else {
                        // Untyped params are still valid variables in scope
                        fn_env.insert(param_name.clone(), Type::Unknown);
                    }
                    if let Some(default) = param.default {
                        if let Some(ty) = param.ty {
                            let expected = self.resolve_type_with_params(ty, &type_param_set);
                            let actual = self.check_expr(default, env, explicit, mut_env);
                            if !self.is_assignable(&actual, &expected) {
                                self.errors.push(TypeError { severity: Severity::Error,
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
                let inferred_return = if expected_return.is_none() {
                    self.function_returns.get(&fn_name).cloned()
                } else {
                    None
                };
                let effective_return = expected_return.or(inferred_return);
                let body_return = if *is_async {
                    match effective_return.as_ref() {
                        Some(Type::Applied { base, args })
                            if base.eq_ignore_ascii_case("Promise") =>
                        {
                            Some(args.first().cloned().unwrap_or(Type::Unknown))
                        }
                        Some(other) => {
                            self.errors.push(TypeError { severity: Severity::Error,
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
                    effective_return.clone()
                };
                self.fn_depth += 1;
                if *is_async {
                    self.async_depth += 1;
                }
                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut fn_env,
                        &mut fn_explicit,
                        body_return.as_ref(),
                        &mut fn_mut_env,
                    );
                }
                if *is_async {
                    self.async_depth = self.async_depth.saturating_sub(1);
                }
                self.fn_depth = self.fn_depth.saturating_sub(1);

                let _ = type_param_sigs;
            }
            Stmt::ReceiverMethod {
                name,
                is_async,
                receiver,
                params,
                return_type: fn_return,
                body,
                ..
            } => {
                let _method_name = token_text(self.source, name.span);
                let type_param_set = HashSet::new();
                let mut fn_env: HashMap<String, Type> = HashMap::new();
                let mut fn_explicit: HashSet<String> = HashSet::new();
                let mut fn_mut_env: HashSet<String> = HashSet::new();

                // Bind the receiver variable in the method body's environment.
                let receiver_name = token_text(self.source, receiver.var.span)
                    .trim_start_matches('$')
                    .to_string();
                let receiver_ty = self.resolve_type(receiver.ty);
                fn_env.insert(receiver_name.clone(), receiver_ty);
                fn_explicit.insert(receiver_name.clone());
                if receiver.is_mut {
                    fn_mut_env.insert(receiver_name);
                }

                let destructured_params = self.detect_destructured_param_carriers(params, body);
                for param in params.iter() {
                    let param_name = token_text(self.source, param.name.span);
                    let param_name = param_name.trim_start_matches('$').to_string();
                    if self.param_is_mut(param) {
                        fn_mut_env.insert(param_name.clone());
                    }
                    if destructured_params.contains(&param_name) {
                        if let Some(ty) = param.ty {
                            let resolved = self.resolve_type_with_params(ty, &type_param_set);
                            if let Type::Struct(ref name) = resolved {
                                self.errors.push(TypeError { severity: Severity::Error,
                                    span: param.span,
                                    message: format!(
                                        "Destructured parameter '${}' cannot use struct type '{}'; use interface '{}' or Object<{{...}}>",
                                        param_name, name, name
                                    ),
                                });
                            }
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
                        fn_explicit.insert(param_name.clone());
                    } else {
                        fn_env.insert(param_name.clone(), Type::Unknown);
                    }
                    if let Some(default) = param.default {
                        if let Some(ty) = param.ty {
                            let expected = self.resolve_type_with_params(ty, &type_param_set);
                            let actual = self.check_expr(default, env, explicit, mut_env);
                            if !self.is_assignable(&actual, &expected) {
                                self.errors.push(TypeError { severity: Severity::Error,
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
                            self.errors.push(TypeError { severity: Severity::Error,
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
                    self.check_stmt(
                        stmt,
                        &mut fn_env,
                        &mut fn_explicit,
                        body_return.as_ref(),
                        &mut fn_mut_env,
                    );
                }
                if *is_async {
                    self.async_depth = self.async_depth.saturating_sub(1);
                }
                self.fn_depth = self.fn_depth.saturating_sub(1);
            }
            Stmt::Class { kind, members, .. } => {
                if *kind == ClassKind::Struct {
                    self.check_struct_defaults(members);
                }
            }
            Stmt::TypeAlias { .. } => {}
            Stmt::Static { vars, .. } => {
                // DekaScript `let` is represented as Stmt::Static. Bind the
                // initializer type into the current environment so subsequent
                // references (including struct method calls) can resolve.
                for var in vars.iter() {
                    if let Expr::Variable { span, .. } = *var.var {
                        let name = token_text(self.source, span)
                            .trim_start_matches('$')
                            .to_string();
                        if let Some(default) = var.default {
                            let ty = self.infer_expr_with_env(default, env);
                            env.insert(name.clone(), ty);
                        } else {
                            env.insert(name.clone(), Type::Unknown);
                        }
                        mut_env.insert(name);
                    }
                }
            }
            Stmt::Const { consts, .. } => {
                // DekaScript `const` is also used at module scope; treat it
                // like an immutable binding for type-resolution purposes.
                for c in consts.iter() {
                    let name = token_text(self.source, c.name.span)
                        .trim_start_matches('$')
                        .to_string();
                    let ty = self.infer_expr_with_env(c.value, env);
                    env.insert(name.clone(), ty);
                    explicit.insert(name);
                }
            }
            Stmt::Export { item, .. } => {
                if let ExportItem::Decl(decl) = item {
                    self.check_stmt(decl, env, explicit, return_type, mut_env);
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

    pub(in crate::phpx::typeck::check) fn param_is_mut(
        &self,
        param: &crate::parser::ast::Param<'a>,
    ) -> bool {
        param.modifiers.iter().any(|token| {
            token_text(self.source, token.span)
                .eq_ignore_ascii_case("mut")
        })
    }
}


