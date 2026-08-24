use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn infer_expr_with_env(
        &self,
        expr: ExprId<'a>,
        env: &HashMap<String, Type>,
    ) -> Type {
        let ctx = InferContext {
            source: self.source,
            vars: env,
            structs: &self.structs,
            interfaces: &self.interface_shapes,
            functions: &self.function_returns,
            function_value_types: &self.function_value_types,
            enums: &self.enums,
        };
        infer_expr(expr, &ctx)
    }

    pub(in crate::phpx::typeck::check) fn extract_static_ident(
        &self,
        expr: ExprId<'a>,
    ) -> Option<String> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                if name.starts_with('$') {
                    None
                } else {
                    Some(name)
                }
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn enum_case_lookup(
        &self,
        class: ExprId<'a>,
        member: ExprId<'a>,
    ) -> Option<(String, String, EnumCaseInfo, Vec<String>)> {
        let class_name = self.extract_static_ident(class)?;
        let case_name = self.extract_static_ident(member)?;
        if let Some(case_info) = self.builtin_enum_case_info(&class_name, &case_name) {
            let enum_name = if class_name.eq_ignore_ascii_case("Option") {
                "Option".to_string()
            } else {
                "Result".to_string()
            };
            let canonical_case = if enum_name == "Option" {
                if case_name.eq_ignore_ascii_case("Some") {
                    "Some"
                } else {
                    "None"
                }
            } else if case_name.eq_ignore_ascii_case("Ok") {
                "Ok"
            } else {
                "Err"
            };
            return Some((enum_name, canonical_case.to_string(), case_info, Vec::new()));
        }
        let info = self.enums.get(&class_name)?;
        let case_info = info.cases.get(&case_name)?;
        let type_params = info.type_params.clone();
        Some((class_name, case_name, case_info.clone(), type_params))
    }

    pub(in crate::phpx::typeck::check) fn enum_case_from_expr(
        &self,
        expr: ExprId<'a>,
    ) -> Option<(String, String)> {
        match *expr {
            Expr::Variable { span, .. } => {
                // DekaScript shorthand: bare `Some`/`None`/`Ok`/`Err` in a match arm.
                let name = token_text(self.source, span);
                self.prelude_enum_case(&name)
            }
            Expr::Call { func, .. } => {
                // Payload patterns: `Some(v)`, `Msg.Text(b)`, `Msg::Text(b)`.
                self.enum_case_from_expr(func)
            }
            Expr::ClassConstFetch {
                class, constant, ..
            } => self
                .enum_case_lookup(class, constant)
                .map(|(enum_name, case_name, _, _)| (enum_name, case_name)),
            Expr::StaticCall { class, method, .. } => self
                .enum_case_lookup(class, method)
                .map(|(enum_name, case_name, _, _)| (enum_name, case_name)),
            Expr::DotAccess { target, property, .. } => {
                // DekaScript enum variant access: `Status.Ready`.
                let class_name = self.extract_static_ident(target)?;
                let case_name = token_text(self.source, property.span);
                let info = self.enums.get(&class_name)?;
                if info.cases.contains_key(&case_name) {
                    Some((class_name, case_name))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn prelude_enum_case(&self, name: &str) -> Option<(String, String)> {
        if name.eq_ignore_ascii_case("Some") || name.eq_ignore_ascii_case("None") {
            Some(("Option".to_string(), name.to_string()))
        } else if name.eq_ignore_ascii_case("Ok") || name.eq_ignore_ascii_case("Err") {
            Some(("Result".to_string(), name.to_string()))
        } else {
            None
        }
    }

    pub(in crate::phpx::typeck::check) fn builtin_enum_case_info(
        &self,
        enum_name: &str,
        case_name: &str,
    ) -> Option<EnumCaseInfo> {
        if enum_name.eq_ignore_ascii_case("Option") {
            if case_name.eq_ignore_ascii_case("Some") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "value".to_string(),
                        ty: None,
                        unnamed: false,
                    }],
                });
            }
            if case_name.eq_ignore_ascii_case("None") {
                return Some(EnumCaseInfo { params: Vec::new() });
            }
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            if case_name.eq_ignore_ascii_case("Ok") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "value".to_string(),
                        ty: None,
                        unnamed: false,
                    }],
                });
            }
            if case_name.eq_ignore_ascii_case("Err") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "error".to_string(),
                        ty: None,
                        unnamed: false,
                    }],
                });
            }
        }
        None
    }

    pub(in crate::phpx::typeck::check) fn builtin_enum_cases(
        &self,
        enum_name: &str,
    ) -> Option<Vec<String>> {
        if enum_name.eq_ignore_ascii_case("Option") {
            return Some(vec!["Some".to_string(), "None".to_string()]);
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            return Some(vec!["Ok".to_string(), "Err".to_string()]);
        }
        None
    }

    pub(in crate::phpx::typeck::check) fn builtin_enum_args_from_type(
        &self,
        enum_name: &str,
        ty: &Type,
    ) -> Option<Vec<Type>> {
        match ty {
            Type::Applied { base, args } if base.eq_ignore_ascii_case(enum_name) => {
                Some(args.clone())
            }
            Type::EnumCase {
                enum_name: name,
                args,
                ..
            } if name.eq_ignore_ascii_case(enum_name) && !args.is_empty() => Some(args.clone()),
            Type::Union(types) => {
                for ty in types {
                    if let Some(args) = self.builtin_enum_args_from_type(enum_name, ty) {
                        return Some(args);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn check_enum_case_call(
        &mut self,
        enum_name: &str,
        case_name: &str,
        case_info: &EnumCaseInfo,
        type_params: &[String],
        args: &'a [crate::parser::ast::Arg<'a>],
        span: Span,
        env: &HashMap<String, Type>,
    ) -> Vec<Type> {
        if case_info.params.is_empty() {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "Enum case {}::{} has no payload; use {}::{} without calling it",
                    enum_name, case_name, enum_name, case_name
                ),
            });
            return type_params.iter().map(|_| Type::Unknown).collect();
        }

        if args.len() != case_info.params.len() {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "Enum case {}::{} expects {} arguments, got {}",
                    enum_name,
                    case_name,
                    case_info.params.len(),
                    args.len()
                ),
            });
            return type_params.iter().map(|_| Type::Unknown).collect();
        }

        let mut inferred: HashMap<String, Type> = HashMap::new();
        for (idx, param) in case_info.params.iter().enumerate() {
            let arg = &args[idx];
            let actual = self.infer_expr_with_env(arg.value, env);
            if let Some(expected) = &param.ty {
                self.infer_type_params(expected, &actual, &mut inferred);
            }
        }

        for (idx, param) in case_info.params.iter().enumerate() {
            let arg = &args[idx];
            let actual = self.infer_expr_with_env(arg.value, env);
            if let Some(expected) = &param.ty {
                let expected = substitute_type(expected, &inferred);
                if let Expr::ObjectLiteral {
                    items,
                    span: obj_span,
                } = *arg.value
                {
                    self.check_object_literal_against_type(items, &expected, obj_span, env);
                }
                if !self.is_assignable(&actual, &expected) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: arg.span,
                        message: format!(
                            "Enum case {}::{} argument {} has type {}, expected {}",
                            enum_name,
                            case_name,
                            idx + 1,
                            actual,
                            expected
                        ),
                    });
                }
            }
        }

        type_params
            .iter()
            .map(|name| inferred.get(name).cloned().unwrap_or(Type::Unknown))
            .collect()
    }

    pub(in crate::phpx::typeck::check) fn enum_names_from_type(
        &self,
        ty: &Type,
    ) -> Option<(Vec<String>, bool)> {
        match ty {
            Type::Enum(name) => Some((vec![name.clone()], false)),
            Type::EnumCase { enum_name, .. } => Some((vec![enum_name.clone()], false)),
            Type::Applied { base, .. }
                if base.eq_ignore_ascii_case("Option") || base.eq_ignore_ascii_case("Result") =>
            {
                let name = if base.eq_ignore_ascii_case("Option") {
                    "Option".to_string()
                } else {
                    "Result".to_string()
                };
                Some((vec![name], false))
            }
            Type::Union(types) => {
                let mut names = Vec::new();
                let mut has_null = false;
                for ty in types.iter() {
                    match ty {
                        Type::Enum(name) => names.push(name.clone()),
                        Type::EnumCase { enum_name, .. } => names.push(enum_name.clone()),
                        Type::Applied { base, .. }
                            if base.eq_ignore_ascii_case("Option")
                                || base.eq_ignore_ascii_case("Result") =>
                        {
                            let name = if base.eq_ignore_ascii_case("Option") {
                                "Option".to_string()
                            } else {
                                "Result".to_string()
                            };
                            names.push(name);
                        }
                        Type::Primitive(PrimitiveType::Null) => has_null = true,
                        _ => return None,
                    }
                }
                if names.is_empty() {
                    None
                } else {
                    Some((names, has_null))
                }
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn check_match_exhaustive(
        &mut self,
        cond_ty: &Type,
        arms: &'a [crate::parser::ast::MatchArm<'a>],
        _env: &HashMap<String, Type>,
    ) {
        let Some((enum_names, allows_null)) = self.enum_names_from_type(cond_ty) else {
            return;
        };

        let mut covered: HashMap<String, HashSet<String>> = HashMap::new();
        for name in enum_names.iter() {
            covered.insert(name.clone(), HashSet::new());
        }
        let mut null_covered = false;

        for arm in arms.iter() {
            let Some(conds) = arm.conditions else {
                return;
            };
            for cond in conds.iter() {
                if matches!(*cond, Expr::Null { .. }) {
                    null_covered = true;
                    continue;
                }
                if let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) {
                    if let Some(entry) = covered.get_mut(&enum_name) {
                        entry.insert(case_name);
                    } else {
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: arm.span,
                            message: format!(
                                "Match arm uses enum case '{}::{}' that is not part of this match",
                                enum_name, case_name
                            ),
                        });
                        return;
                    }
                } else {
                    // Mixed conditions: skip exhaustiveness checking.
                    return;
                }
            }
        }

        for enum_name in enum_names.iter() {
            let case_names = if let Some(info) = self.enums.get(enum_name) {
                info.cases.keys().cloned().collect::<Vec<_>>()
            } else if let Some(builtin) = self.builtin_enum_cases(enum_name) {
                builtin
            } else {
                continue;
            };
            let Some(seen) = covered.get(enum_name) else {
                continue;
            };
            for case_name in case_names.iter() {
                if !seen.contains(case_name) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: arms.last().map(|arm| arm.span).unwrap_or_default(),
                        message: format!(
                            "Match on {} is not exhaustive; missing case {}::{}",
                            enum_name, enum_name, case_name
                        ),
                    });
                    return;
                }
            }
        }

        if allows_null && !null_covered {
            self.errors.push(TypeError { severity: Severity::Error,
                span: arms.last().map(|arm| arm.span).unwrap_or_default(),
                message: "Match on nullable enum is not exhaustive; missing null arm".to_string(),
            });
        }
    }

    pub(in crate::phpx::typeck::check) fn apply_match_arm_narrowing(
        &self,
        condition: ExprId<'a>,
        arm: &crate::parser::ast::MatchArm<'a>,
        env: &mut HashMap<String, Type>,
    ) {
        let Some(var_name) = self.extract_var_name(condition) else {
            return;
        };
        let Some(conds) = arm.conditions else {
            return;
        };
        let current_ty = env.get(&var_name);
        let mut cases = Vec::new();
        for cond in conds.iter() {
            let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) else {
                return;
            };
            cases.push((enum_name, case_name));
        }
        if cases.is_empty() {
            return;
        }
        let narrowed = if cases.len() == 1 {
            let (enum_name, case_name) = &cases[0];
            let args = current_ty
                .and_then(|ty| self.builtin_enum_args_from_type(enum_name, ty))
                .unwrap_or_default();
            Type::EnumCase {
                enum_name: enum_name.clone(),
                case_name: case_name.clone(),
                args,
            }
        } else {
            let mut variants = Vec::new();
            for (enum_name, case_name) in cases.into_iter() {
                let args = current_ty
                    .and_then(|ty| self.builtin_enum_args_from_type(&enum_name, ty))
                    .unwrap_or_default();
                variants.push(Type::EnumCase {
                    enum_name,
                    case_name,
                    args,
                });
            }
            Type::Union(variants)
        };
        env.insert(var_name, narrowed);
        self.bind_match_arm_payloads(condition, arm, env);
    }

    /// Bind `Msg::Text(b)` / `Ok(v)` pattern variables into the arm env.
    /// Payload types come from the case definition, with type parameters
    /// filled from the match subject.
    fn bind_match_arm_payloads(
        &self,
        subject: ExprId<'a>,
        arm: &crate::parser::ast::MatchArm<'a>,
        env: &mut HashMap<String, Type>,
    ) {
        let Some(conds) = arm.conditions else {
            return;
        };
        if conds.len() != 1 {
            return;
        }
        let cond = conds[0];
        let args = match *cond {
            Expr::Call { args, .. } | Expr::StaticCall { args, .. } => args,
            _ => return,
        };
        let Some((enum_name, case_name)) = self.enum_case_from_expr(cond) else {
            return;
        };
        let case_info = self
            .enums
            .get(&enum_name)
            .and_then(|info| info.cases.get(&case_name))
            .cloned()
            .or_else(|| self.builtin_enum_case_info(&enum_name, &case_name));
        let Some(case_info) = case_info else {
            return;
        };
        if args.len() != case_info.params.len() {
            return;
        }
        let subject_ty = self.extract_var_name(subject).and_then(|name| env.get(&name).cloned());
        let type_args = subject_ty
            .as_ref()
            .and_then(|ty| self.builtin_enum_args_from_type(&enum_name, ty))
            .unwrap_or_default();
        let type_params = self
            .enums
            .get(&enum_name)
            .map(|info| info.type_params.clone())
            .unwrap_or_else(|| {
                if enum_name.eq_ignore_ascii_case("Option") {
                    vec!["T".to_string()]
                } else if enum_name.eq_ignore_ascii_case("Result") {
                    vec!["T".to_string(), "E".to_string()]
                } else {
                    Vec::new()
                }
            });
        let subst: HashMap<String, Type> = type_params
            .iter()
            .cloned()
            .zip(type_args.into_iter())
            .collect();
        for (arg, param) in args.iter().zip(case_info.params.iter()) {
            let Expr::Variable { span, .. } = *arg.value else {
                continue;
            };
            let binding = token_text(self.source, span)
                .trim_start_matches('$')
                .to_string();
            if binding == "_" {
                continue;
            }
            let ty = param
                .ty
                .as_ref()
                .map(|ty| substitute_type(ty, &subst))
                .or_else(|| {
                    if enum_name.eq_ignore_ascii_case("Option") && param.name == "value" {
                        subst.get("T").cloned()
                    } else if enum_name.eq_ignore_ascii_case("Result") && param.name == "value" {
                        subst.get("T").cloned()
                    } else if enum_name.eq_ignore_ascii_case("Result") && param.name == "error" {
                        subst.get("E").cloned()
                    } else {
                        None
                    }
                })
                .unwrap_or(Type::Unknown);
            env.insert(binding, ty);
        }
    }

    pub(in crate::phpx::typeck::check) fn narrow_env_for_condition(
        &self,
        condition: ExprId<'a>,
        env: &HashMap<String, Type>,
        truthy: bool,
    ) -> HashMap<String, Type> {
        let mut out = env.clone();
        self.apply_condition_narrowing(condition, &mut out, truthy);
        out
    }

    pub(in crate::phpx::typeck::check) fn apply_condition_narrowing(
        &self,
        condition: ExprId<'a>,
        env: &mut HashMap<String, Type>,
        truthy: bool,
    ) {
        match *condition {
            Expr::Binary {
                op, left, right, ..
            } => {
                if let Some(var_name) = self.null_compare_var(left, right) {
                    match op {
                        BinaryOp::EqEqEq | BinaryOp::EqEq => {
                            if truthy {
                                self.narrow_var_to_null(&var_name, env);
                            } else {
                                self.remove_null_from_var(&var_name, env);
                            }
                        }
                        BinaryOp::NotEqEq | BinaryOp::NotEq => {
                            if truthy {
                                self.remove_null_from_var(&var_name, env);
                            } else {
                                self.narrow_var_to_null(&var_name, env);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Expr::Isset { vars, .. } => {
                if truthy {
                    for var in vars.iter() {
                        if let Some(name) = self.extract_var_name(*var) {
                            self.remove_null_from_var(&name, env);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::phpx::typeck::check) fn null_compare_var(
        &self,
        left: ExprId<'a>,
        right: ExprId<'a>,
    ) -> Option<String> {
        if matches!(*left, Expr::Null { .. }) {
            return self.extract_var_name(right);
        }
        if matches!(*right, Expr::Null { .. }) {
            return self.extract_var_name(left);
        }
        None
    }

    pub(in crate::phpx::typeck::check) fn extract_var_name(
        &self,
        expr: ExprId<'a>,
    ) -> Option<String> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                Some(name.trim_start_matches('$').to_string())
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn check_static_class_ref(
        &mut self,
        class: ExprId<'a>,
        span: Span,
    ) {
        let Some(name) = self.extract_static_ident(class) else {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: "Dynamic class references are not allowed in DekaScript".to_string(),
            });
            return;
        };
        if self.structs.contains_key(&name) || self.enums.contains_key(&name) {
            return;
        }
        self.errors.push(TypeError { severity: Severity::Error,
            span,
            message: format!("Unknown type '{}' in DekaScript; classes are not allowed", name),
        });
    }

    pub(in crate::phpx::typeck::check) fn remove_null_from_var(
        &self,
        name: &str,
        env: &mut HashMap<String, Type>,
    ) {
        let Some(existing) = env.get(name) else {
            return;
        };
        let updated = remove_null(existing);
        env.insert(name.to_string(), updated);
    }

    pub(in crate::phpx::typeck::check) fn narrow_var_to_null(
        &self,
        name: &str,
        env: &mut HashMap<String, Type>,
    ) {
        let Some(existing) = env.get(name) else {
            return;
        };
        let updated = keep_only_null(existing);
        env.insert(name.to_string(), updated);
    }

    pub(in crate::phpx::typeck::check) fn is_null_comparison(
        &self,
        op: BinaryOp,
        left: ExprId<'a>,
        right: ExprId<'a>,
    ) -> bool {
        matches!(
            op,
            BinaryOp::EqEqEq
                | BinaryOp::EqEq
                | BinaryOp::NotEqEq
                | BinaryOp::NotEq
                | BinaryOp::Spaceship
        ) && (matches!(*left, Expr::Null { .. }) || matches!(*right, Expr::Null { .. }))
    }

    pub(in crate::phpx::typeck::check) fn allow_null_comparisons(&self) -> bool {
        let Some(path) = self.file_path.as_ref() else {
            return false;
        };
        let Some(root) = find_modules_root(path) else {
            return false;
        };
        root.join("stdlib.json").is_file()
    }
}
