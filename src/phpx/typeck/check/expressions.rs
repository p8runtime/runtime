use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn check_expr(
        &mut self,
        expr: ExprId<'a>,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
    ) -> Type {
        match *expr {
            Expr::Variable { span, .. } => {
                let raw = token_text(self.source, span);
                if !raw.starts_with('$') {
                    return self.infer_expr_with_env(expr, env);
                }
                let name = raw.trim_start_matches('$');
                if is_builtin_variable(name) {
                    return Type::Unknown;
                }
                if let Some(found) = env.get(name) {
                    return found.clone();
                }
                let suggestion = nearest_name(name, env.keys().map(|k| k.as_str()));
                let mut message = format!("Unknown variable '${}'", name);
                if let Some(suggested) = suggestion {
                    message.push_str(&format!("; did you mean '${}'?", suggested));
                }
                self.errors.push(TypeError { span, message });
                Type::Unknown
            }
            Expr::Null { .. } => Type::Primitive(PrimitiveType::Null),
            Expr::Assign { var, expr, .. } | Expr::AssignRef { var, expr, .. } => {
                let rhs_ty = self.check_expr(expr, env, explicit);
                if let Expr::Variable { span, .. } = *var {
                    let name = token_text(self.source, span)
                        .trim_start_matches('$')
                        .to_string();
                    if self.is_self_destructure_assignment_expr(expr, &name) {
                        explicit.remove(&name);
                    }
                }
                self.assign_to_target(var, &rhs_ty, env, explicit);
                rhs_ty
            }
            Expr::AssignOp { var, expr, .. } => {
                let rhs_ty = self.check_expr(expr, env, explicit);
                self.assign_to_target(var, &rhs_ty, env, explicit);
                rhs_ty
            }
            Expr::DotAccess {
                target,
                property,
                span,
            } => {
                self.check_dot_access(target, property, span, env);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Binary {
                left,
                right,
                op,
                span,
            } => {
                if self.is_null_comparison(op, left, right) && !self.allow_null_comparisons() {
                    self.errors.push(TypeError {
                        span,
                        message: "Null comparisons are not allowed in PHPX; use isset() instead"
                            .to_string(),
                    });
                }
                let _ = self.check_expr(left, env, explicit);
                let _ = self.check_expr(right, env, explicit);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Unary { expr, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Call { func, args, .. } => {
                let _ = self.check_expr(func, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_call_signature(func, args, env)
            }
            Expr::New { class, args, span } => {
                let _ = self.check_expr(class, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.errors.push(TypeError {
                    span,
                    message: "new is not allowed in PHPX; use struct literals".to_string(),
                });
                Type::Unknown
            }
            Expr::MethodCall {
                target,
                method,
                args,
                span,
            } => {
                let target_ty = self.check_expr(target, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_method_call_signature(&target_ty, method, args, env, span)
            }
            Expr::NullsafeMethodCall {
                target,
                method,
                args,
                span,
            } => {
                let target_ty = self.check_expr(target, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_method_call_signature(&target_ty, method, args, env, span)
            }
            Expr::StaticCall {
                class,
                method,
                args,
                span,
            } => {
                let _ = self.check_expr(class, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                if let Some((enum_name, case_name, case_info)) =
                    self.enum_case_lookup(class, method)
                {
                    self.check_enum_case_call(&enum_name, &case_name, &case_info, args, span, env);
                    if enum_name.eq_ignore_ascii_case("Option") {
                        let arg_ty = args
                            .get(0)
                            .map(|arg| self.infer_expr_with_env(arg.value, env))
                            .unwrap_or(Type::Unknown);
                        let case_args = if case_name.eq_ignore_ascii_case("Some") {
                            vec![arg_ty]
                        } else {
                            Vec::new()
                        };
                        return Type::EnumCase {
                            enum_name: "Option".to_string(),
                            case_name,
                            args: case_args,
                        };
                    }
                    if enum_name.eq_ignore_ascii_case("Result") {
                        let arg_ty = args
                            .get(0)
                            .map(|arg| self.infer_expr_with_env(arg.value, env))
                            .unwrap_or(Type::Unknown);
                        let case_args = if case_name.eq_ignore_ascii_case("Ok") {
                            vec![arg_ty, Type::Unknown]
                        } else if case_name.eq_ignore_ascii_case("Err") {
                            vec![Type::Unknown, arg_ty]
                        } else {
                            Vec::new()
                        };
                        return Type::EnumCase {
                            enum_name: "Result".to_string(),
                            case_name,
                            args: case_args,
                        };
                    }
                    return Type::EnumCase {
                        enum_name,
                        case_name,
                        args: Vec::new(),
                    };
                }
                self.check_static_class_ref(class, span);
                Type::Unknown
            }
            Expr::ClassConstFetch {
                class,
                constant,
                span,
            } => {
                let _ = self.check_expr(class, env, explicit);
                if let Some((enum_name, case_name, _)) = self.enum_case_lookup(class, constant) {
                    if enum_name.eq_ignore_ascii_case("Option")
                        || enum_name.eq_ignore_ascii_case("Result")
                    {
                        return Type::EnumCase {
                            enum_name: if enum_name.eq_ignore_ascii_case("Option") {
                                "Option".to_string()
                            } else {
                                "Result".to_string()
                            },
                            case_name,
                            args: Vec::new(),
                        };
                    }
                    return Type::Enum(enum_name);
                }
                self.check_static_class_ref(class, span);
                Type::Unknown
            }
            Expr::PropertyFetch { target, .. } | Expr::NullsafePropertyFetch { target, .. } => {
                let _ = self.check_expr(target, env, explicit);
                Type::Unknown
            }
            Expr::Array { items, .. } => {
                for item in items.iter() {
                    if let Some(key) = item.key {
                        let _ = self.check_expr(key, env, explicit);
                    }
                    let _ = self.check_expr(item.value, env, explicit);
                }
                Type::Array
            }
            Expr::ObjectLiteral { items, .. } => {
                for item in items.iter() {
                    let _ = self.check_expr(item.value, env, explicit);
                }
                self.infer_expr_with_env(expr, env)
            }
            Expr::JsxElement {
                name,
                attributes,
                children,
                ..
            } => {
                self.validate_jsx_element(&name, attributes);
                for attr in attributes.iter() {
                    if let Some(value) = attr.value {
                        self.validate_jsx_expr(value);
                        let _ = self.check_expr(value, env, explicit);
                    }
                }
                for child in children.iter() {
                    if let JsxChild::Expr(expr) = *child {
                        self.validate_jsx_expr(expr);
                        let _ = self.check_expr(expr, env, explicit);
                    }
                }
                Type::VNode
            }
            Expr::JsxFragment { children, .. } => {
                for child in children.iter() {
                    if let JsxChild::Expr(expr) = *child {
                        self.validate_jsx_expr(expr);
                        let _ = self.check_expr(expr, env, explicit);
                    }
                }
                Type::VNode
            }
            Expr::StructLiteral { name, fields, span } => {
                let raw = token_text(self.source, name.span);
                let struct_name = raw.trim_start_matches('\\').to_string();
                let info = if let Some(info) = self.structs.get(&struct_name) {
                    info.clone()
                } else {
                    self.errors.push(TypeError {
                        span: span,
                        message: format!("Unknown struct '{}'", struct_name),
                    });
                    return Type::Unknown;
                };

                let mut seen = HashSet::new();
                for field in fields.iter() {
                    let field_name = token_text(self.source, field.name.span);
                    let field_name = field_name.trim_start_matches('$').to_string();

                    if !seen.insert(field_name.clone()) {
                        self.errors.push(TypeError {
                            span: field.span,
                            message: format!(
                                "Duplicate field '{}' in struct literal '{}'",
                                field_name, struct_name
                            ),
                        });
                        continue;
                    }

                    let expected = info.fields.get(&field_name);
                    if expected.is_none() {
                        self.errors.push(TypeError {
                            span: field.span,
                            message: format!(
                                "Unknown field '{}' in struct literal '{}'",
                                field_name, struct_name
                            ),
                        });
                    }

                    let actual = self.check_expr(field.value, env, explicit);
                    if let Some(expected) = expected {
                        if !self.is_assignable(&actual, expected) {
                            self.errors.push(TypeError {
                                span: field.span,
                                message: format!(
                                    "Field '{}' expects {}, got {}",
                                    field_name, expected, actual
                                ),
                            });
                        }
                    }
                }

                for field in info.fields.keys() {
                    if info.defaults.contains(field) {
                        continue;
                    }
                    if !seen.contains(field) {
                        self.errors.push(TypeError {
                            span: span,
                            message: format!(
                                "Missing field '{}' in struct literal '{}'",
                                field, struct_name
                            ),
                        });
                    }
                }

                Type::Struct(struct_name)
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                let _ = self.check_expr(array, env, explicit);
                if let Some(dim) = dim {
                    let _ = self.check_expr(dim, env, explicit);
                }
                Type::Unknown
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                let _ = self.check_expr(condition, env, explicit);
                if let Some(if_true) = if_true {
                    let _ = self.check_expr(if_true, env, explicit);
                }
                let _ = self.check_expr(if_false, env, explicit);
                Type::Unknown
            }
            Expr::Match {
                condition, arms, ..
            } => {
                let cond_ty = self.check_expr(condition, env, explicit);
                let mut match_ty = Type::Unknown;
                for arm in arms.iter() {
                    let mut arm_env = env.clone();
                    let mut arm_explicit = explicit.clone();
                    self.apply_match_arm_narrowing(condition, arm, &mut arm_env);
                    if let Some(conds) = arm.conditions {
                        for cond in conds.iter() {
                            let _ = self.check_expr(cond, &mut arm_env, &mut arm_explicit);
                        }
                    }
                    let body_ty = self.check_expr(arm.body, &mut arm_env, &mut arm_explicit);
                    match_ty = merge_types(&match_ty, &body_ty);
                }
                self.check_match_exhaustive(&cond_ty, arms, env);
                match_ty
            }
            Expr::AnonymousClass { span, .. } => {
                self.errors.push(TypeError {
                    span,
                    message: "Anonymous classes are not allowed in PHPX".to_string(),
                });
                Type::Unknown
            }
            Expr::Closure { params, body, .. } => {
                let mut inner_env = env.clone();
                let mut inner_explicit = explicit.clone();
                for param in params.iter() {
                    let param_name = token_text(self.source, param.name.span)
                        .trim_start_matches('$')
                        .to_string();
                    let param_ty = if let Some(ty) = param.ty {
                        self.resolve_type(ty)
                    } else {
                        Type::Unknown
                    };
                    inner_env.insert(param_name.clone(), param_ty);
                    inner_explicit.insert(param_name);
                }
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut inner_env, &mut inner_explicit, None);
                }
                Type::Unknown
            }
            Expr::ArrowFunction { params, expr, .. } => {
                let mut inner_env = env.clone();
                let mut inner_explicit = explicit.clone();
                for param in params.iter() {
                    let param_name = token_text(self.source, param.name.span)
                        .trim_start_matches('$')
                        .to_string();
                    let param_ty = if let Some(ty) = param.ty {
                        self.resolve_type(ty)
                    } else {
                        Type::Unknown
                    };
                    inner_env.insert(param_name.clone(), param_ty);
                    inner_explicit.insert(param_name);
                }
                let _ = self.check_expr(expr, &mut inner_env, &mut inner_explicit);
                Type::Unknown
            }
            Expr::Await { expr, span } => {
                if self.fn_depth > 0 && self.async_depth == 0 {
                    self.errors.push(TypeError {
                        span,
                        message: "await is only allowed in async functions (or at top-level in PHPX modules)".to_string(),
                    });
                }
                let awaited_ty = self.check_expr(expr, env, explicit);
                match awaited_ty {
                    Type::Applied { base, args } if base.eq_ignore_ascii_case("Promise") => {
                        args.first().cloned().unwrap_or(Type::Unknown)
                    }
                    Type::Unknown => Type::Unknown,
                    other => {
                        self.errors.push(TypeError {
                            span,
                            message: format!("await expects Promise<T>, got {}", other),
                        });
                        Type::Unknown
                    }
                }
            }
            Expr::Include { expr, .. }
            | Expr::Print { expr, .. }
            | Expr::Clone { expr, .. }
            | Expr::Cast { expr, .. }
            | Expr::Empty { expr, .. }
            | Expr::Eval { expr, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                Type::Unknown
            }
            Expr::Isset { vars, .. } => {
                for var in vars.iter() {
                    let _ = self.check_expr(var, env, explicit);
                }
                Type::Unknown
            }
            Expr::Yield { key, value, .. } => {
                if let Some(key) = key {
                    let _ = self.check_expr(key, env, explicit);
                }
                if let Some(value) = value {
                    let _ = self.check_expr(value, env, explicit);
                }
                Type::Unknown
            }
            Expr::Die { expr, .. } | Expr::Exit { expr, .. } => {
                if let Some(expr) = expr {
                    let _ = self.check_expr(expr, env, explicit);
                }
                Type::Unknown
            }
            Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
                let _ = self.check_expr(var, env, explicit);
                Type::Unknown
            }
            _ => self.infer_expr_with_env(expr, env),
        }
    }

    pub(in crate::phpx::typeck::check) fn check_dot_access(
        &mut self,
        target: ExprId<'a>,
        property: &'a crate::parser::lexer::token::Token,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let target_ty = self.infer_expr_with_env(target, env);
        let prop_name = token_text(self.source, property.span);
        match target_ty {
            Type::ObjectShape(fields) => {
                if !fields.contains_key(&prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!("Unknown object field '{}'", prop_name),
                    });
                }
            }
            Type::Struct(name) => {
                if !self.structs.contains_key(&name) {
                    return;
                }
                match self.resolve_struct_field(&name, &prop_name) {
                    StructFieldResolution::Found(_) => {}
                    StructFieldResolution::Ambiguous => {
                        self.errors.push(TypeError {
                            span,
                            message: format!("Ambiguous promoted field '{}::{}'", name, prop_name),
                        });
                    }
                    StructFieldResolution::Missing => {
                        self.errors.push(TypeError {
                            span,
                            message: format!("Unknown struct field '{}::{}'", name, prop_name),
                        });
                    }
                }
            }
            Type::Interface(name) => {
                let Some(info) = self.interfaces.get(&name) else {
                    return;
                };
                if !info.fields.contains_key(&prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!("Unknown interface field '{}::{}'", name, prop_name),
                    });
                }
            }
            Type::Enum(name) => {
                if !self.enum_allows_field(&name, &prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!("Unknown enum field '{}::{}'", name, prop_name),
                    });
                }
            }
            Type::EnumCase {
                enum_name,
                case_name,
                ..
            } => {
                if !self.enum_case_allows_field(&enum_name, &case_name, &prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown enum field '{}::{}::{}'",
                            enum_name, case_name, prop_name
                        ),
                    });
                }
            }
            Type::Applied { base, .. }
                if base.eq_ignore_ascii_case("Option") || base.eq_ignore_ascii_case("Result") =>
            {
                if prop_name != "name" {
                    self.errors.push(TypeError {
                        span,
                        message: format!("Unknown enum field '{}::{}'", base, prop_name),
                    });
                }
            }
            Type::Union(types) => {
                let mut invalid = false;
                let mut missing = false;
                let mut any_ok = false;
                for ty in types.iter() {
                    match ty {
                        Type::Primitive(PrimitiveType::Null) => {
                            any_ok = true;
                        }
                        Type::ObjectShape(fields) => {
                            any_ok = true;
                            if !fields.contains_key(&prop_name) {
                                missing = true;
                            }
                        }
                        Type::Struct(name) => {
                            any_ok = true;
                            if !self.structs.contains_key(name) {
                                continue;
                            }
                            match self.resolve_struct_field(name, &prop_name) {
                                StructFieldResolution::Found(_) => {}
                                StructFieldResolution::Missing => {
                                    missing = true;
                                }
                                StructFieldResolution::Ambiguous => {
                                    invalid = true;
                                }
                            }
                        }
                        Type::Interface(name) => {
                            any_ok = true;
                            let Some(info) = self.interfaces.get(name) else {
                                continue;
                            };
                            if !info.fields.contains_key(&prop_name) {
                                missing = true;
                            }
                        }
                        Type::Enum(name) => {
                            any_ok = true;
                            if !self.enum_allows_field(name, &prop_name) {
                                missing = true;
                            }
                        }
                        Type::Applied { base, .. }
                            if base.eq_ignore_ascii_case("Option")
                                || base.eq_ignore_ascii_case("Result") =>
                        {
                            any_ok = true;
                            if prop_name != "name" {
                                missing = true;
                            }
                        }
                        Type::EnumCase {
                            enum_name,
                            case_name,
                            ..
                        } => {
                            any_ok = true;
                            if !self.enum_case_allows_field(enum_name, case_name, &prop_name) {
                                missing = true;
                            }
                        }
                        Type::Object | Type::Mixed | Type::Unknown => {
                            any_ok = true;
                        }
                        _ => {
                            invalid = true;
                        }
                    }
                }
                if invalid || (any_ok && missing) {
                    self.errors.push(TypeError {
                        span,
                        message: format!("Unknown object field '{}' for union type", prop_name),
                    });
                }
            }
            _ => {}
        }
    }

    pub(in crate::phpx::typeck::check) fn resolve_struct_field(
        &self,
        struct_name: &str,
        field: &str,
    ) -> StructFieldResolution {
        let mut visited = HashSet::new();
        self.resolve_struct_field_inner(struct_name, field, &mut visited)
    }

    pub(in crate::phpx::typeck::check) fn resolve_struct_field_inner(
        &self,
        struct_name: &str,
        field: &str,
        visited: &mut HashSet<String>,
    ) -> StructFieldResolution {
        if !visited.insert(struct_name.to_string()) {
            return StructFieldResolution::Missing;
        }
        let Some(info) = self.structs.get(struct_name) else {
            return StructFieldResolution::Missing;
        };

        if let Some(ty) = info.fields.get(field) {
            return StructFieldResolution::Found(ty.clone());
        }

        let mut found: Option<Type> = None;
        let mut ambiguous = false;

        for embed in &info.embeds {
            match self.resolve_struct_field_inner(embed, field, visited) {
                StructFieldResolution::Found(ty) => {
                    if found.is_some() {
                        ambiguous = true;
                    } else {
                        found = Some(ty);
                    }
                }
                StructFieldResolution::Ambiguous => {
                    ambiguous = true;
                }
                StructFieldResolution::Missing => {}
            }
        }

        if ambiguous {
            StructFieldResolution::Ambiguous
        } else if let Some(ty) = found {
            StructFieldResolution::Found(ty)
        } else {
            StructFieldResolution::Missing
        }
    }

    pub(in crate::phpx::typeck::check) fn enum_allows_field(
        &self,
        enum_name: &str,
        field: &str,
    ) -> bool {
        if enum_name.eq_ignore_ascii_case("Option") || enum_name.eq_ignore_ascii_case("Result") {
            return field == "name";
        }
        let Some(info) = self.enums.get(enum_name) else {
            return false;
        };
        if field == "name" {
            return true;
        }
        if field == "value" {
            return info.backed.is_some();
        }
        for case in info.cases.values() {
            if !case.params.iter().any(|param| param.name == field) {
                return false;
            }
        }
        !info.cases.is_empty()
    }

    pub(in crate::phpx::typeck::check) fn enum_case_allows_field(
        &self,
        enum_name: &str,
        case_name: &str,
        field: &str,
    ) -> bool {
        if enum_name.eq_ignore_ascii_case("Option") {
            if field == "name" {
                return true;
            }
            return case_name.eq_ignore_ascii_case("Some") && field == "value";
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            if field == "name" {
                return true;
            }
            if case_name.eq_ignore_ascii_case("Ok") {
                return field == "value";
            }
            if case_name.eq_ignore_ascii_case("Err") {
                return field == "error";
            }
            return false;
        }
        let Some(info) = self.enums.get(enum_name) else {
            return false;
        };
        if field == "name" {
            return true;
        }
        if field == "value" {
            return info.backed.is_some();
        }
        let Some(case) = info.cases.get(case_name) else {
            return false;
        };
        case.params.iter().any(|param| param.name == field)
    }

    pub(in crate::phpx::typeck::check) fn type_allows_null(&self, ty: &Type) -> bool {
        if matches!(ty, Type::Primitive(PrimitiveType::Null)) {
            return true;
        }
        match ty {
            Type::Union(types) => types.iter().any(|t| self.type_allows_null(t)),
            _ => false,
        }
    }

    pub(in crate::phpx::typeck::check) fn check_object_literal_against_type(
        &mut self,
        items: &'a [crate::parser::ast::ObjectItem<'a>],
        expected: &Type,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let Type::ObjectShape(expected_fields) = expected else {
            return;
        };
        self.check_object_literal_against_shape(items, expected_fields, span, env);
    }

    pub(in crate::phpx::typeck::check) fn check_object_literal_against_shape(
        &mut self,
        items: &'a [crate::parser::ast::ObjectItem<'a>],
        expected: &BTreeMap<String, ObjectField>,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let mut seen = HashSet::new();
        for item in items.iter() {
            let key = object_key_name(item.key, self.source);
            seen.insert(key.clone());
            let Some(expected_field) = expected.get(&key) else {
                self.errors.push(TypeError {
                    span: item.span,
                    message: format!("Unknown object field '{}' in object literal", key),
                });
                continue;
            };
            let actual = self.infer_expr_with_env(item.value, env);
            if !self.is_assignable(&actual, &expected_field.ty) {
                self.errors.push(TypeError {
                    span: item.span,
                    message: format!(
                        "Object field '{}' has type {}, expected {}",
                        key, actual, expected_field.ty
                    ),
                });
            }
        }

        for (name, field) in expected.iter() {
            if field.optional {
                continue;
            }
            if !seen.contains(name) {
                self.errors.push(TypeError {
                    span,
                    message: format!("Missing required object field '{}'", name),
                });
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn infer_type_params(
        &mut self,
        pattern: &Type,
        actual: &Type,
        inferred: &mut HashMap<String, Type>,
    ) {
        match pattern {
            Type::TypeParam(name) => {
                if let Some(existing) = inferred.get(name) {
                    if matches!(existing, Type::Unknown) {
                        inferred.insert(name.clone(), actual.clone());
                    } else if !self.is_assignable(actual, existing) {
                        let merged = merge_types(existing, actual);
                        inferred.insert(name.clone(), merged);
                    }
                } else {
                    inferred.insert(name.clone(), actual.clone());
                }
            }
            Type::ObjectShape(fields) => {
                if let Type::ObjectShape(actual_fields) = actual {
                    for (name, field) in fields.iter() {
                        if let Some(actual_field) = actual_fields.get(name) {
                            self.infer_type_params(&field.ty, &actual_field.ty, inferred);
                        }
                    }
                }
            }
            Type::Union(options) => {
                for opt in options.iter() {
                    if self.is_assignable(actual, opt) {
                        self.infer_type_params(opt, actual, inferred);
                        break;
                    }
                }
            }
            Type::Applied { base, args } => {
                if let Type::Applied {
                    base: actual_base,
                    args: actual_args,
                } = actual
                {
                    if base == actual_base && args.len() == actual_args.len() {
                        for (idx, arg) in args.iter().enumerate() {
                            self.infer_type_params(arg, &actual_args[idx], inferred);
                        }
                    }
                } else if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: actual_args,
                } = actual
                {
                    if base.eq_ignore_ascii_case(enum_name) {
                        if base.eq_ignore_ascii_case("Option") && args.len() == 1 {
                            if case_name.eq_ignore_ascii_case("Some") {
                                if let Some(actual_inner) = actual_args.get(0) {
                                    self.infer_type_params(&args[0], actual_inner, inferred);
                                }
                            }
                        }
                        if base.eq_ignore_ascii_case("Result") && args.len() == 2 {
                            if case_name.eq_ignore_ascii_case("Ok") {
                                if let Some(actual_ok) = actual_args.get(0) {
                                    self.infer_type_params(&args[0], actual_ok, inferred);
                                }
                            } else if case_name.eq_ignore_ascii_case("Err") {
                                if let Some(actual_err) = actual_args.get(1) {
                                    self.infer_type_params(&args[1], actual_err, inferred);
                                }
                            }
                        }
                    }
                } else if base.eq_ignore_ascii_case("array")
                    && matches!(actual, Type::Array)
                    && args.len() == 1
                {
                    self.infer_type_params(&args[0], &Type::Unknown, inferred);
                }
            }
            _ => {}
        }
    }

    pub(in crate::phpx::typeck::check) fn assign_to_target(
        &mut self,
        target: ExprId<'a>,
        value_ty: &Type,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
    ) {
        match *target {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                let name = name.trim_start_matches('$').to_string();
                let is_null = matches!(value_ty, Type::Primitive(PrimitiveType::Null));
                if let Some(existing) = env.get(&name) {
                    if explicit.contains(&name) {
                        if self.strict_null && is_null && !self.type_allows_null(existing) {
                            self.errors.push(TypeError {
                                span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                        if !self.is_assignable(value_ty, existing) {
                            self.errors.push(TypeError {
                                span,
                                message: format!(
                                    "Type mismatch: expected {}, got {}",
                                    existing, value_ty
                                ),
                            });
                        }
                    } else {
                        if self.strict_null && is_null {
                            self.errors.push(TypeError {
                                span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                        let merged = merge_types(existing, value_ty);
                        env.insert(name.clone(), merged);
                    }
                } else {
                    if self.strict_null && is_null {
                        self.errors.push(TypeError {
                            span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
                                .to_string(),
                        });
                    }
                    env.insert(name.clone(), value_ty.clone());
                }
            }
            Expr::DotAccess {
                target,
                property,
                span,
            } => {
                self.check_dot_access(target, property, span, env);
            }
            Expr::Assign { var, expr, .. } => {
                let default_ty = self.check_expr(expr, env, explicit);
                let merged = merge_types(value_ty, &default_ty);
                self.assign_to_target(var, &merged, env, explicit);
            }
            Expr::Array { items, .. } => {
                for (idx, item) in items.iter().enumerate() {
                    if matches!(item.value, Expr::Error { .. }) {
                        continue;
                    }
                    let key_name = item
                        .key
                        .and_then(|key| self.pattern_key_name_from_expr(key))
                        .unwrap_or_else(|| idx.to_string());
                    let field_ty = self.field_type_for_pattern_key(value_ty, &key_name);
                    self.assign_to_target(item.value, &field_ty, env, explicit);
                }
            }
            Expr::ObjectLiteral { items, .. } => {
                for item in items.iter() {
                    let key_name = self.pattern_key_name(item.key);
                    let field_ty = self.field_type_for_pattern_key(value_ty, &key_name);
                    self.assign_to_target(item.value, &field_ty, env, explicit);
                }
            }
            _ => {}
        }
    }

    pub(in crate::phpx::typeck::check) fn pattern_key_name(&self, key: ObjectKey<'a>) -> String {
        match key {
            ObjectKey::Ident(token) => {
                let raw = token_text(self.source, token.span);
                raw.trim_start_matches('$').to_string()
            }
            ObjectKey::String(token) => {
                let raw = token_text(self.source, token.span);
                raw.trim_matches('"').trim_matches('\'').to_string()
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn pattern_key_name_from_expr(
        &self,
        expr: ExprId<'a>,
    ) -> Option<String> {
        match *expr {
            Expr::String { value, .. } => {
                let raw = std::str::from_utf8(value).ok()?;
                Some(raw.trim_matches('"').trim_matches('\'').to_string())
            }
            Expr::Integer { value, .. } => std::str::from_utf8(value).ok().map(|s| s.to_string()),
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn field_type_for_pattern_key(
        &self,
        source_ty: &Type,
        key: &str,
    ) -> Type {
        match source_ty {
            Type::ObjectShape(fields) => fields
                .get(key)
                .map(|field| field.ty.clone())
                .unwrap_or(Type::Unknown),
            Type::Struct(name) => self
                .structs
                .get(name)
                .and_then(|info| info.fields.get(key).cloned())
                .unwrap_or(Type::Unknown),
            Type::Interface(name) => self
                .interfaces
                .get(name)
                .and_then(|info| info.fields.get(key))
                .map(|field| field.ty.clone())
                .unwrap_or(Type::Unknown),
            Type::Applied { base, args } => {
                if base.eq_ignore_ascii_case("array") {
                    args.first().cloned().unwrap_or(Type::Unknown)
                } else {
                    Type::Unknown
                }
            }
            Type::Union(types) => {
                let mut parts = Vec::new();
                for ty in types {
                    let resolved = self.field_type_for_pattern_key(ty, key);
                    if !matches!(resolved, Type::Unknown) {
                        parts.push(resolved);
                    }
                }
                if parts.is_empty() {
                    Type::Unknown
                } else if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    Type::Union(parts)
                }
            }
            _ => Type::Unknown,
        }
    }
}
