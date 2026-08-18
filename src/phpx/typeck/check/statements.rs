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
                                    message: "Null is not allowed in PHPX; use Option<T> instead"
                                        .to_string(),
                                });
                            }
                        }
                    } else if self.strict_null && !self.type_allows_null(expected) {
                        self.errors.push(TypeError { severity: Severity::Error,
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
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: *span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
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
                    fn_mut_env.insert(param_name.clone());
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
                // Resolve every provided method's signature exactly once,
                // regardless of whether this is an inherent or trait impl.
                // This is what makes the declared param/return types get
                // validated at all -- resolve_type/resolve_name_type push
                // errors as a side effect of being called. Before this,
                // inherent impls (trait_name: None) skipped this block
                // entirely and a completely invented type name in an
                // inherent impl's signature typechecked clean.
                let target_key = token_text(self.source, target.parts[0].span);

                // RFD 19: validate `self.field` / `this.field` accesses
                // inside an impl-block method body against the target struct's
                // actual fields. The receiver can be spelled either `self`
                // (explicit `self: Self` parameter) or `this` (implicit
                // receiver, the DekaScript convention shown in the tour); both
                // refer to the same target instance.
                if let Some(struct_info) = self.structs.get(&target_key).cloned() {
                    let known_fields: std::collections::BTreeSet<String> =
                        struct_info.fields.keys().cloned().collect();
                    for member in members.iter() {
                        if let ClassMember::Method { body, .. } = member {
                            let mut validator = SelfFieldValidator {
                                source: self.source,
                                known_fields: known_fields.clone(),
                                errors: Vec::new(),
                            };
                            for stmt in body.iter() {
                                validator.visit_stmt(*stmt);
                            }
                            self.errors.extend(validator.errors);
                        }
                    }
                }

                let provided: HashMap<String, MethodSig> = members
                    .iter()
                    .filter_map(|m| match m {
                        ClassMember::Method {
                            name,
                            params,
                            return_type,
                            ..
                        } => Some((
                            token_text(self.source, name.span),
                            self.method_signature(params, *return_type, false),
                        )),
                        _ => None,
                    })
                    .collect();

                // RFD 19 conformance check, two parts: (1) every
                // non-default trait method must be present, (2) every
                // provided method's actual signature must match what the
                // trait declared. Exact equality, not variance -- traits
                // have no generics yet, so this is the correct scope for
                // now (see RFD 19 design notes on "prefer simplicity").
                if let Some(trait_ref) = trait_name {
                    let trait_key = token_text(self.source, trait_ref.parts[0].span);
                    if let Some(info) = self.traits.get(&trait_key).cloned() {

                        let mut missing: Vec<&str> = info
                            .methods
                            .iter()
                            .filter(|(name, (_, has_default))| {
                                !has_default && !provided.contains_key(name.as_str())
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
                                severity: Severity::Error,
                            });
                        }

                        let mut mismatched: Vec<String> = info
                            .methods
                            .iter()
                            .filter_map(|(name, (required_sig, _))| {
                                let provided_sig = provided.get(name)?;
                                (provided_sig != required_sig).then(|| name.clone())
                            })
                            .collect();
                        mismatched.sort();
                        for name in mismatched {
                            self.errors.push(TypeError {
                                span: *span,
                                message: format!(
                                    "{target_key}.{name} does not match the signature {trait_key} requires"
                                ),
                                severity: Severity::Error,
                            });
                        }
                    }

                    // Record for the cross-trait conflict pass (RFD 19),
                    // which runs after every statement has been seen -- an
                    // impl appearing earlier in the file may need to know
                    // about a sibling impl for the same target that only
                    // appears later.
                    let provided_names: HashSet<String> = members
                        .iter()
                        .filter_map(|m| match m {
                            ClassMember::Method { name, .. } => {
                                Some(token_text(self.source, name.span))
                            }
                            _ => None,
                        })
                        .collect();
                    self.impls.entry(target_key).or_default().push(ImplRecord {
                        trait_name: trait_key,
                        provided: provided_names,
                        span: *span,
                    });
                }
            }
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
            _ => {}
        }
    }

    // RFD 19 multi-trait default-method conflict rule. Java's rule, not
    // Rust's: forced to resolve at the impl site rather than deferred to an
    // ambiguous call site, because Rust's alternative needs qualified
    // trait-method call syntax as a second feature just to make the escape
    // hatch reachable. Two cases, both errors:
    //   1. Two implemented traits declare the same method name with
    //      DIFFERENT signatures -- always an error. No single method body
    //      can have two different signatures at once, regardless of who
    //      writes it or whether either side is a default.
    //   2. Same signature, BOTH traits provide a default body, and NEITHER
    //      impl block for this target explicitly provides an override --
    //      ambiguous, must be resolved by an explicit override.
    // Not an error: only one side is default (satisfying the abstract one
    // satisfies both); both sides abstract (this repo's existing
    // missing-method check already requires each to be provided per block,
    // which is Rust's real behaviour too -- a trait requirement cannot be
    // satisfied by an inherent or sibling-trait method, only by that impl
    // block itself); or at least one impl block for this target already
    // provides the method explicitly.
    pub(in crate::phpx::typeck::check) fn check_trait_conflicts(&mut self) {
        for (target_key, impls) in self.impls.clone() {
            if impls.len() < 2 {
                continue;
            }
            let mut seen: HashMap<String, (String, MethodSig)> = HashMap::new();
            let any_provides = |method: &str| impls.iter().any(|r| r.provided.contains(method));
            for record in &impls {
                let Some(trait_info) = self.traits.get(&record.trait_name).cloned() else {
                    continue;
                };
                for (method_name, (sig, has_default)) in &trait_info.methods {
                    match seen.get(method_name) {
                        None => {
                            seen.insert(method_name.clone(), (record.trait_name.clone(), sig.clone()));
                        }
                        Some((other_trait, other_sig)) => {
                            if other_trait == &record.trait_name {
                                continue; // same trait seen twice isn't a cross-trait conflict
                            }
                            if other_sig != sig {
                                self.errors.push(TypeError {
                                    span: record.span,
                                    message: format!(
                                        "{target_key}.{method_name} is required by both {other_trait} and {} with incompatible signatures",
                                        record.trait_name
                                    ),
                                    severity: Severity::Error,
                                });
                            } else if *has_default && !any_provides(method_name) {
                                // both sides equal signature; only ambiguous
                                // if this trait's copy is also a default AND
                                // nothing on the type provides an override.
                                // (the has_default on the FIRST-seen trait's
                                // copy was already implied by reaching here
                                // without an earlier missing-method error.)
                                self.errors.push(TypeError {
                                    span: record.span,
                                    message: format!(
                                        "{target_key}.{method_name} has a default implementation in both {other_trait} and {}; provide an explicit override to resolve the ambiguity",
                                        record.trait_name
                                    ),
                                    severity: Severity::Error,
                                });
                            }
                        }
                    }
                }
            }
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

impl<'ast> Visitor<'ast> for SelfFieldValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        // A `self.method(...)` / `this.method(...)` call parses as
        // `Expr::Call { func: DotAccess { target: self/this, property: method },
        // args }` in DekaScript -- the `.` operator has no dedicated
        // method-call parse branch the way PHP's `->` does. Without this arm,
        // every legitimate receiver method call gets misdiagnosed as an
        // unknown field access.
        if let Expr::Call { func, args, .. } = *expr {
            let is_receiver_method_call = matches!(
                *func,
                Expr::DotAccess { target, .. }
                    if matches!(
                        token_text(self.source, target.span()).as_str(),
                        "self" | "this"
                    )
            );
            if !is_receiver_method_call {
                self.visit_expr(func);
            }
            for arg in args.iter() {
                self.visit_arg(arg);
            }
            return;
        }
        if let Expr::DotAccess { target, property, span } = *expr {
            let target_text = token_text(self.source, target.span());
            if matches!(target_text.as_str(), "self" | "this") {
                let field_name = token_text(self.source, property.span);
                if !self.known_fields.contains(&field_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "{target_text}.{field_name} does not refer to a declared field"
                        ),
                        severity: Severity::Error,
                    });
                }
            }
        }
        walk_expr(self, expr);
    }
}
