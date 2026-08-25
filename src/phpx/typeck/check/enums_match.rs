use super::*;

#[derive(Clone, Debug)]
enum MatchPat {
    Irrefutable,
    Ctor {
        enum_name: String,
        case_name: String,
        args: Vec<MatchPat>,
    },
    Other,
}

impl MatchPat {
    fn matches_ctor(&self, enum_name: &str, case_name: &str) -> bool {
        match self {
            MatchPat::Ctor {
                enum_name: en,
                case_name: cn,
                ..
            } => en.eq_ignore_ascii_case(enum_name) && cn.eq_ignore_ascii_case(case_name),
            _ => false,
        }
    }

    fn covers_ctor_fully(&self) -> bool {
        match self {
            MatchPat::Irrefutable => true,
            MatchPat::Ctor { args, .. } => {
                args.iter().all(|arg| matches!(arg, MatchPat::Irrefutable))
            }
            MatchPat::Other => false,
        }
    }

    fn first_arg(&self) -> Option<&MatchPat> {
        match self {
            MatchPat::Ctor { args, .. } => args.first(),
            _ => None,
        }
    }
}

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
            Expr::DotAccess {
                target, property, ..
            } => {
                // DekaScript enum variant access: `Status.Ready`.
                let class_name = self.extract_static_ident(target)?;
                let case_name = token_text(self.source, property.span);
                if self
                    .builtin_enum_case_info(&class_name, &case_name)
                    .is_some()
                {
                    return self
                        .prelude_enum_case(&case_name)
                        .or_else(|| Some((class_name, case_name)));
                }
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
            self.errors.push(TypeError {
                severity: Severity::Error,
                span,
                message: format!(
                    "Enum case {}::{} has no payload; use {}::{} without calling it",
                    enum_name, case_name, enum_name, case_name
                ),
            });
            return type_params.iter().map(|_| Type::Unknown).collect();
        }

        if args.len() != case_info.params.len() {
            self.errors.push(TypeError {
                severity: Severity::Error,
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
                    self.errors.push(TypeError {
                        severity: Severity::Error,
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

    fn is_match_wildcard(&self, expr: ExprId<'a>) -> bool {
        self.expr_is_hole(expr)
    }

    fn match_has_catch_all(&self, arms: &[crate::parser::ast::MatchArm<'a>]) -> bool {
        arms.iter().any(|arm| match arm.conditions {
            None => true,
            Some(conds) => conds.iter().any(|cond| self.is_match_wildcard(*cond)),
        })
    }

    /// deka#281: every `match` must cover the scrutinee type. `_` and
    /// `default` are catch-alls. Literals do not cover `number`/`string`.
    /// Nested constructors (`Ok(Some(v))`) cover only that inner case.
    pub(in crate::phpx::typeck::check) fn check_match_exhaustive(
        &mut self,
        cond_ty: &Type,
        arms: &'a [crate::parser::ast::MatchArm<'a>],
        _env: &HashMap<String, Type>,
    ) {
        if self.match_has_catch_all(arms) {
            return;
        }

        let mut pats: Vec<MatchPat> = Vec::new();
        let mut bools: HashSet<bool> = HashSet::new();
        let mut null_covered = false;
        let expected_enums = self
            .enum_names_from_type(cond_ty)
            .map(|(names, _)| names.into_iter().collect::<HashSet<_>>());

        for arm in arms.iter() {
            let Some(conds) = arm.conditions else {
                return;
            };
            for cond in conds.iter() {
                if self.is_match_wildcard(*cond) {
                    return;
                }
                if matches!(*cond, Expr::Null { .. }) {
                    null_covered = true;
                    continue;
                }
                if let Expr::Boolean { value, .. } = *cond {
                    bools.insert(*value);
                    continue;
                }
                if let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) {
                    if let Some(expected) = expected_enums.as_ref() {
                        if !expected.contains(&enum_name) {
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: arm.span,
                                message: format!(
                                    "Match arm uses enum case '{}::{}' that is not part of this match",
                                    enum_name, case_name
                                ),
                            });
                            return;
                        }
                    }
                }
                pats.push(self.expr_to_match_pat(*cond, cond_ty));
            }
        }

        if let Some(message) = self.match_uncovered(cond_ty, &pats, &bools, null_covered) {
            self.errors.push(TypeError {
                severity: Severity::Error,
                span: arms.last().map(|arm| arm.span).unwrap_or_default(),
                message,
            });
        }
    }

    fn match_uncovered(
        &self,
        ty: &Type,
        pats: &[MatchPat],
        bools: &HashSet<bool>,
        null_covered: bool,
    ) -> Option<String> {
        if pats.iter().any(|pat| matches!(pat, MatchPat::Irrefutable)) {
            return None;
        }
        if let Type::EnumCase {
            enum_name,
            case_name,
            ..
        } = ty
        {
            return self.missing_nested_case(ty, enum_name, pats, Some(case_name.as_str()));
        }
        if let Some(enum_name) = self.enum_name_of_type(ty) {
            return self.missing_nested_case(ty, &enum_name, pats, None);
        }
        match ty {
            Type::Primitive(PrimitiveType::Bool) => {
                let mut missing = Vec::new();
                if !bools.contains(&true) {
                    missing.push("true");
                }
                if !bools.contains(&false) {
                    missing.push("false");
                }
                if missing.is_empty() {
                    None
                } else {
                    Some(format!(
                        "Match on bool is not exhaustive; missing {}",
                        missing.join(" and ")
                    ))
                }
            }
            Type::Primitive(PrimitiveType::Null) => {
                if null_covered {
                    None
                } else {
                    Some("Match on null is not exhaustive; missing null arm".to_string())
                }
            }
            Type::Union(types) => {
                for inner in types {
                    if let Some(message) = self.match_uncovered(inner, pats, bools, null_covered) {
                        return Some(message);
                    }
                }
                None
            }
            Type::Unknown | Type::Mixed => None,
            _ => Some(format!("Match on {} is not exhaustive; add a `_` arm", ty)),
        }
    }

    fn enum_name_of_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Enum(name)
            | Type::EnumCase {
                enum_name: name, ..
            } => Some(name.clone()),
            Type::Applied { base, .. }
                if base.eq_ignore_ascii_case("Option") || base.eq_ignore_ascii_case("Result") =>
            {
                if base.eq_ignore_ascii_case("Option") {
                    Some("Option".to_string())
                } else {
                    Some("Result".to_string())
                }
            }
            Type::Applied { base, .. } if self.enums.contains_key(base) => Some(base.clone()),
            _ => None,
        }
    }

    fn missing_nested_case(
        &self,
        ty: &Type,
        enum_name: &str,
        pats: &[MatchPat],
        only_case: Option<&str>,
    ) -> Option<String> {
        let case_names = if let Some(case_name) = only_case {
            vec![case_name.to_string()]
        } else if let Some(info) = self.enums.get(enum_name) {
            info.cases.keys().cloned().collect::<Vec<_>>()
        } else {
            self.builtin_enum_cases(enum_name)?
        };
        for case_name in case_names {
            let matching: Vec<&MatchPat> = pats
                .iter()
                .filter(|pat| pat.matches_ctor(enum_name, &case_name))
                .collect();
            if matching.is_empty() {
                return Some(format!(
                    "Match on {} is not exhaustive; missing {}",
                    self.coverage_root_name(ty, enum_name),
                    self.format_missing_ctor(enum_name, &case_name, None)
                ));
            }
            if matching.iter().any(|pat| pat.covers_ctor_fully()) {
                continue;
            }
            let payload_tys = self.case_payload_types(enum_name, &case_name, ty);
            if payload_tys.len() != 1 {
                continue;
            }
            let subpats: Vec<MatchPat> = matching
                .iter()
                .map(|pat| pat.first_arg().cloned().unwrap_or(MatchPat::Irrefutable))
                .collect();
            let payload_ty = self.coverage_type_from_pats(&payload_tys[0], &subpats);
            if let Some(inner) = self.match_uncovered(&payload_ty, &subpats, &HashSet::new(), false)
            {
                let inner_pat = inner
                    .rsplit("missing ")
                    .next()
                    .unwrap_or(inner.as_str())
                    .to_string();
                return Some(format!(
                    "Match on {} is not exhaustive; missing {}",
                    self.coverage_root_name(ty, enum_name),
                    self.format_missing_ctor(enum_name, &case_name, Some(&inner_pat))
                ));
            }
        }
        None
    }

    fn coverage_root_name(&self, ty: &Type, enum_name: &str) -> String {
        match ty {
            Type::Applied { .. } => ty.name(),
            _ => enum_name.to_string(),
        }
    }

    fn format_missing_ctor(&self, enum_name: &str, case_name: &str, inner: Option<&str>) -> String {
        let head = if enum_name.eq_ignore_ascii_case("Option")
            || enum_name.eq_ignore_ascii_case("Result")
        {
            case_name.to_string()
        } else {
            format!("{enum_name}::{case_name}")
        };
        match inner {
            Some(inner) => format!("{head}({inner})"),
            None => head,
        }
    }

    fn coverage_type_from_pats(&self, declared: &Type, pats: &[MatchPat]) -> Type {
        if self.enum_name_of_type(declared).is_some() && !self.type_is_open(declared) {
            return declared.clone();
        }
        let mut names = HashSet::new();
        for pat in pats {
            if let MatchPat::Ctor { enum_name, .. } = pat {
                names.insert(enum_name.clone());
            }
        }
        if names.len() == 1 {
            let name = names.into_iter().next().unwrap();
            if name.eq_ignore_ascii_case("Option") {
                return Type::Applied {
                    base: "Option".to_string(),
                    args: vec![Type::Unknown],
                };
            }
            if name.eq_ignore_ascii_case("Result") {
                return Type::Applied {
                    base: "Result".to_string(),
                    args: vec![Type::Unknown, Type::Unknown],
                };
            }
            return Type::Enum(name);
        }
        declared.clone()
    }

    fn type_is_open(&self, ty: &Type) -> bool {
        matches!(ty, Type::Unknown | Type::Mixed)
    }

    pub(in crate::phpx::typeck::check) fn apply_match_arm_narrowing(
        &mut self,
        condition: ExprId<'a>,
        arm: &crate::parser::ast::MatchArm<'a>,
        env: &mut HashMap<String, Type>,
        cond_ty: &Type,
    ) {
        if let Some(var_name) = self.extract_var_name(condition) {
            if let Some(conds) = arm.conditions {
                let mut cases = Vec::new();
                let mut all_ctors = true;
                for cond in conds.iter() {
                    if let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) {
                        cases.push((enum_name, case_name));
                    } else {
                        all_ctors = false;
                        break;
                    }
                }
                if all_ctors && !cases.is_empty() {
                    let narrowed = if cases.len() == 1 {
                        let (enum_name, case_name) = &cases[0];
                        let args = self
                            .builtin_enum_args_from_type(enum_name, cond_ty)
                            .unwrap_or_default();
                        Type::EnumCase {
                            enum_name: enum_name.clone(),
                            case_name: case_name.clone(),
                            args,
                        }
                    } else {
                        let mut variants = Vec::new();
                        for (enum_name, case_name) in cases.into_iter() {
                            let args = self
                                .builtin_enum_args_from_type(&enum_name, cond_ty)
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
                }
            }
        }
        self.bind_match_arm_payloads(arm, env, cond_ty);
    }

    /// Bind `Msg::Text(b)` / `Ok(v)` / `Ok(Some(v))` pattern variables.
    fn bind_match_arm_payloads(
        &mut self,
        arm: &crate::parser::ast::MatchArm<'a>,
        env: &mut HashMap<String, Type>,
        cond_ty: &Type,
    ) {
        let Some(conds) = arm.conditions else {
            return;
        };
        if conds.len() != 1 {
            return;
        }
        self.bind_pattern(conds[0], cond_ty, env);
    }

    fn bind_pattern(&mut self, expr: ExprId<'a>, expected: &Type, env: &mut HashMap<String, Type>) {
        if self.is_match_wildcard(expr) {
            return;
        }
        if let Some((enum_name, case_name, args)) = self.ctor_parts(expr) {
            if !self.ctor_fits_type(&enum_name, expected) {
                self.errors.push(TypeError {
                    severity: Severity::Error,
                    span: expr.span(),
                    message: format!(
                        "Pattern '{}::{}' does not match {}",
                        enum_name,
                        case_name,
                        expected.name()
                    ),
                });
                return;
            }
            if args.is_empty() {
                return;
            }
            let payloads = self.case_payload_types(&enum_name, &case_name, expected);
            if args.len() != payloads.len() {
                self.errors.push(TypeError {
                    severity: Severity::Error,
                    span: expr.span(),
                    message: format!(
                        "Enum case {}::{} expects {} arguments, got {}",
                        enum_name,
                        case_name,
                        payloads.len(),
                        args.len()
                    ),
                });
                return;
            }
            for (arg, payload_ty) in args.iter().zip(payloads.iter()) {
                self.bind_pattern(arg.value, payload_ty, env);
            }
            return;
        }
        if let Expr::Variable { span, .. } = *expr {
            let binding = token_text(self.source, span)
                .trim_start_matches('$')
                .to_string();
            if binding != "_" {
                env.insert(binding, expected.clone());
            }
        }
    }

    fn ctor_parts(&self, expr: ExprId<'a>) -> Option<(String, String, &'a [Arg<'a>])> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                let (enum_name, case_name) = self.prelude_enum_case(&name)?;
                Some((enum_name, case_name, &[][..]))
            }
            Expr::Call { func, args, .. } => {
                let (enum_name, case_name) = self.enum_case_from_expr(func)?;
                Some((enum_name, case_name, args))
            }
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let (enum_name, case_name, _, _) = self.enum_case_lookup(class, method)?;
                Some((enum_name, case_name, args))
            }
            Expr::DotAccess { .. } | Expr::ClassConstFetch { .. } => {
                let (enum_name, case_name) = self.enum_case_from_expr(expr)?;
                Some((enum_name, case_name, &[][..]))
            }
            _ => None,
        }
    }

    fn ctor_fits_type(&self, enum_name: &str, expected: &Type) -> bool {
        if self.type_is_open(expected) {
            return true;
        }
        match self.enum_name_of_type(expected) {
            Some(name) => name.eq_ignore_ascii_case(enum_name),
            None => false,
        }
    }

    fn expr_to_match_pat(&self, expr: ExprId<'a>, expected: &Type) -> MatchPat {
        if self.is_match_wildcard(expr) {
            return MatchPat::Irrefutable;
        }
        if let Some((enum_name, case_name, args)) = self.ctor_parts(expr) {
            if self.ctor_fits_type(&enum_name, expected) {
                if args.is_empty() {
                    return MatchPat::Ctor {
                        enum_name,
                        case_name,
                        args: Vec::new(),
                    };
                }
                let payloads = self.case_payload_types(&enum_name, &case_name, expected);
                let nested = args
                    .iter()
                    .enumerate()
                    .map(|(idx, arg)| {
                        let payload_ty = payloads.get(idx).unwrap_or(&Type::Unknown);
                        self.expr_to_match_pat(arg.value, payload_ty)
                    })
                    .collect();
                return MatchPat::Ctor {
                    enum_name,
                    case_name,
                    args: nested,
                };
            }
        }
        if matches!(*expr, Expr::Variable { .. }) {
            return MatchPat::Irrefutable;
        }
        MatchPat::Other
    }

    fn case_payload_types(&self, enum_name: &str, case_name: &str, subject: &Type) -> Vec<Type> {
        let subst = self.enum_type_subst(enum_name, subject);
        let case_info = self
            .enums
            .get(enum_name)
            .and_then(|info| info.cases.get(case_name))
            .cloned()
            .or_else(|| self.builtin_enum_case_info(enum_name, case_name));
        let Some(case_info) = case_info else {
            return Vec::new();
        };
        case_info
            .params
            .iter()
            .map(|param| {
                param
                    .ty
                    .as_ref()
                    .map(|ty| substitute_type(ty, &subst))
                    .or_else(|| {
                        if enum_name.eq_ignore_ascii_case("Option") && param.name == "value" {
                            subst.get("T").cloned()
                        } else if enum_name.eq_ignore_ascii_case("Result") && param.name == "value"
                        {
                            subst.get("T").cloned()
                        } else if enum_name.eq_ignore_ascii_case("Result") && param.name == "error"
                        {
                            subst.get("E").cloned()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(Type::Unknown)
            })
            .collect()
    }

    fn enum_type_subst(&self, enum_name: &str, subject: &Type) -> HashMap<String, Type> {
        let type_params = self
            .enums
            .get(enum_name)
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
        let type_args = self
            .builtin_enum_args_from_type(enum_name, subject)
            .unwrap_or_else(|| {
                if let Type::Applied { base, args } = subject {
                    if base.eq_ignore_ascii_case(enum_name) {
                        return args.clone();
                    }
                }
                Vec::new()
            });
        type_params.into_iter().zip(type_args).collect()
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
            self.errors.push(TypeError {
                severity: Severity::Error,
                span,
                message: "Dynamic class references are not allowed in DekaScript".to_string(),
            });
            return;
        };
        if self.structs.contains_key(&name) || self.enums.contains_key(&name) {
            return;
        }
        self.errors.push(TypeError {
            severity: Severity::Error,
            span,
            message: format!(
                "Unknown type '{}' in DekaScript; classes are not allowed",
                name
            ),
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
