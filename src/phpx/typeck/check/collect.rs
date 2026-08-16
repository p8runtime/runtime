use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn collect_struct_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Class { kind, name, .. } = stmt {
                if *kind != ClassKind::Struct {
                    continue;
                }
                let class_name = token_text(self.source, name.span);
                self.structs
                    .entry(class_name)
                    .or_insert_with(|| StructInfo {
                        fields: BTreeMap::new(),
                        embeds: Vec::new(),
                        defaults: BTreeSet::new(),
                    });
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_interface_names(
        &mut self,
        program: &Program<'a>,
    ) {
        for stmt in program.statements.iter() {
            if let Stmt::Interface { name, .. } = stmt {
                let iface_name = token_text(self.source, name.span);
                self.interfaces
                    .entry(iface_name.clone())
                    .or_insert_with(|| InterfaceInfo {
                        methods: HashMap::new(),
                        fields: BTreeMap::new(),
                    });
                self.interface_shapes.entry(iface_name).or_default();
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_enum_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum {
                name, backed_type, ..
            } = stmt
            else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let backed = backed_type.and_then(|ty| enum_backed_primitive(ty));
            self.enums.entry(enum_name).or_insert_with(|| EnumInfo {
                cases: BTreeMap::new(),
                backed,
            });
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_struct_fields(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Class {
                kind,
                name,
                members,
                ..
            } = stmt
            {
                if *kind != ClassKind::Struct {
                    continue;
                }
                let class_name = token_text(self.source, name.span);
                let mut fields = BTreeMap::new();
                let mut embeds = Vec::new();
                let mut embed_names = HashSet::new();
                let mut defaults = BTreeSet::new();
                let mut declared_fields: BTreeMap<String, Type> = BTreeMap::new();

                // First pass: collect declared fields so annotation validation can
                // validate relation metadata against sibling fields.
                for member in members.iter() {
                    match member {
                        ClassMember::Property { ty, entries, .. } => {
                            let field_type =
                                ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown);
                            for entry in entries.iter() {
                                let field_name = token_text(self.source, entry.name.span);
                                let field_name = field_name.trim_start_matches('$').to_string();
                                declared_fields.insert(field_name, field_type.clone());
                            }
                        }
                        ClassMember::PropertyHook { ty, name, .. } => {
                            let field_name = token_text(self.source, name.span);
                            let field_name = field_name.trim_start_matches('$').to_string();
                            let field_type =
                                ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown);
                            declared_fields.insert(field_name, field_type);
                        }
                        _ => {}
                    }
                }

                for member in members.iter() {
                    match member {
                        ClassMember::Property { ty, entries, .. } => {
                            let field_type = ty.map(|ty| self.resolve_type(ty));
                            for entry in entries.iter() {
                                let field_name = token_text(self.source, entry.name.span);
                                let field_name = field_name.trim_start_matches('$').to_string();
                                self.validate_struct_field_annotations(
                                    &class_name,
                                    &field_name,
                                    entry,
                                    field_type.as_ref(),
                                    &declared_fields,
                                );
                                if embed_names.contains(&field_name) {
                                    self.errors.push(TypeError { severity: Severity::Error,
                                        span: entry.name.span,
                                        message: format!(
                                            "Struct '{}' already embeds '{}'",
                                            class_name, field_name
                                        ),
                                    });
                                }
                                fields.insert(
                                    field_name.clone(),
                                    field_type.clone().unwrap_or(Type::Unknown),
                                );
                                if entry.default.is_some() {
                                    defaults.insert(field_name);
                                }
                            }
                        }
                        ClassMember::PropertyHook {
                            ty, name, default, ..
                        } => {
                            let field_name = token_text(self.source, name.span);
                            let field_name = field_name.trim_start_matches('$').to_string();
                            if embed_names.contains(&field_name) {
                                self.errors.push(TypeError { severity: Severity::Error,
                                    span: name.span,
                                    message: format!(
                                        "Struct '{}' already embeds '{}'",
                                        class_name, field_name
                                    ),
                                });
                            }
                            let field_type =
                                ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown);
                            fields.insert(field_name.clone(), field_type);
                            if default.is_some() {
                                defaults.insert(field_name);
                            }
                        }
                        ClassMember::Embed { types, .. } => {
                            for embed in types.iter() {
                                let embed_name = token_text(self.source, embed.span);
                                if embed_name == class_name {
                                    self.errors.push(TypeError { severity: Severity::Error,
                                        span: embed.span,
                                        message: "Struct cannot embed itself".to_string(),
                                    });
                                    continue;
                                }
                                if !self.structs.contains_key(&embed_name) {
                                    self.errors.push(TypeError { severity: Severity::Error,
                                        span: embed.span,
                                        message: format!(
                                            "Unknown embedded struct '{}'",
                                            embed_name
                                        ),
                                    });
                                    continue;
                                }
                                if fields.contains_key(&embed_name)
                                    || embed_names.contains(&embed_name)
                                {
                                    self.errors.push(TypeError { severity: Severity::Error,
                                        span: embed.span,
                                        message: format!(
                                            "Duplicate embedded struct '{}'",
                                            embed_name
                                        ),
                                    });
                                    continue;
                                }
                                embed_names.insert(embed_name.clone());
                                embeds.push(embed_name.clone());
                                fields.insert(embed_name.clone(), Type::Struct(embed_name));
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(info) = self.structs.get_mut(&class_name) {
                    info.fields = fields;
                    info.embeds = embeds;
                    info.defaults = defaults;
                } else {
                    self.structs.insert(
                        class_name,
                        StructInfo {
                            fields,
                            embeds,
                            defaults,
                        },
                    );
                }
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_interface_methods(
        &mut self,
        program: &Program<'a>,
    ) {
        for stmt in program.statements.iter() {
            let Stmt::Interface { name, members, .. } = stmt else {
                continue;
            };
            let iface_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            let mut fields = BTreeMap::new();
            for member in members.iter() {
                match member {
                    ClassMember::Method {
                        name: method_name,
                        params,
                        return_type,
                        ..
                    } => {
                        let method_name = token_text(self.source, method_name.span);
                        let sig = self.method_signature(params, *return_type);
                        methods.insert(method_name, sig);
                    }
                    ClassMember::Property { ty, entries, .. } => {
                        let field_ty = ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown);
                        for entry in entries.iter() {
                            let field_name = token_text(self.source, entry.name.span);
                            let field_name = field_name.trim_start_matches('$').to_string();
                            fields.insert(
                                field_name,
                                ObjectField {
                                    ty: field_ty.clone(),
                                    optional: false,
                                },
                            );
                        }
                    }
                    ClassMember::PropertyHook { ty, name, .. } => {
                        let field_name = token_text(self.source, name.span);
                        let field_name = field_name.trim_start_matches('$').to_string();
                        fields.insert(
                            field_name,
                            ObjectField {
                                ty: ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown),
                                optional: false,
                            },
                        );
                    }
                    _ => {}
                }
            }
            self.interfaces.insert(
                iface_name.clone(),
                InterfaceInfo {
                    methods,
                    fields: fields.clone(),
                },
            );
            self.interface_shapes.insert(iface_name, fields);
        }
    }

    // DekaScript trait method collection (RFD 19). Mirrors
    // collect_interface_methods above; the difference is tracking whether
    // each method has a default body (`!member_body.is_empty()`), which is
    // what conformance checking at `impl` needs to know is optional.
    pub(in crate::phpx::typeck::check) fn collect_trait_methods(
        &mut self,
        program: &Program<'a>,
    ) {
        for stmt in program.statements.iter() {
            let Stmt::Trait { name, members, .. } = stmt else {
                continue;
            };
            let trait_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    body,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    let has_default = !body.is_empty();
                    methods.insert(method_name, (sig, has_default));
                }
            }
            self.traits.insert(trait_name, TraitInfo { methods });
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_struct_methods(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Class {
                kind,
                name,
                members,
                ..
            } = stmt
            else {
                continue;
            };
            if *kind != ClassKind::Struct {
                continue;
            }
            let struct_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    methods.insert(method_name, sig);
                }
            }
            self.struct_methods.insert(struct_name, methods);
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_enum_methods(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum { name, members, .. } = stmt else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    methods.insert(method_name, sig);
                }
            }
            self.enum_methods.insert(enum_name, methods);
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_enum_cases(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum { name, members, .. } = stmt else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let mut cases = BTreeMap::new();
            for member in members.iter() {
                let ClassMember::Case {
                    name: case_name,
                    payload,
                    span,
                    ..
                } = member
                else {
                    continue;
                };
                let case_name = token_text(self.source, case_name.span);
                if cases.contains_key(&case_name) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: *span,
                        message: format!("Duplicate enum case '{}::{}'", enum_name, case_name),
                    });
                    continue;
                }
                let mut params = Vec::new();
                if let Some(payload) = payload {
                    let mut seen_params = HashSet::new();
                    for param in payload.iter() {
                        if param.by_ref {
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: param.span,
                                message: "Enum case payload parameters cannot be by-reference"
                                    .to_string(),
                            });
                        }
                        if param.variadic {
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: param.span,
                                message: "Enum case payload parameters cannot be variadic"
                                    .to_string(),
                            });
                        }
                        if param.default.is_some() {
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: param.span,
                                message: "Enum case payload parameters cannot have default values"
                                    .to_string(),
                            });
                        }
                        let name = token_text(self.source, param.name.span);
                        let name = name.trim_start_matches('$').to_string();
                        if !seen_params.insert(name.clone()) {
                            self.errors.push(TypeError { severity: Severity::Error,
                                span: param.span,
                                message: format!(
                                    "Duplicate payload field '{}' on enum case {}::{}",
                                    name, enum_name, case_name
                                ),
                            });
                        }
                        let ty = param.ty.map(|ty| self.resolve_type(ty));
                        params.push(EnumParamInfo { name, ty });
                    }
                }
                cases.insert(case_name, EnumCaseInfo { params });
            }
            if let Some(info) = self.enums.get_mut(&enum_name) {
                info.cases = cases;
            } else {
                self.enums.insert(
                    enum_name,
                    EnumInfo {
                        cases,
                        backed: None,
                    },
                );
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn method_signature(
        &mut self,
        params: &'a [crate::parser::ast::Param<'a>],
        return_type: Option<&'a AstType<'a>>,
    ) -> MethodSig {
        let mut sig_params = Vec::new();
        let mut variadic = false;
        for param in params.iter() {
            let ty = param.ty.map(|ty| self.resolve_type(ty));
            let required = param.default.is_none() && !param.variadic;
            if param.variadic {
                variadic = true;
            }
            sig_params.push(ParamSig { ty, required });
        }
        MethodSig {
            params: sig_params,
            return_type: return_type.map(|ty| self.resolve_type(ty)),
            variadic,
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_functions(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                ..
            } = stmt
            {
                let fn_name = token_text(self.source, name.span);
                self.check_poc_severity_warning(&fn_name, name.span);
                let (type_param_sigs, type_param_set) = self.collect_type_param_sigs(type_params);
                let mut param_sigs = Vec::new();
                let mut variadic = false;
                for param in params.iter() {
                    let ty = param
                        .ty
                        .map(|ty| self.resolve_type_with_params(ty, &type_param_set));
                    let required = param.default.is_none() && !param.variadic;
                    if param.variadic {
                        variadic = true;
                    }
                    param_sigs.push(ParamSig { ty, required });
                }
                let sig = FunctionSig {
                    type_params: type_param_sigs,
                    params: param_sigs,
                    return_type: return_type
                        .map(|ty| self.resolve_type_with_params(ty, &type_param_set)),
                    variadic,
                };
                if let Some(ret) = &sig.return_type {
                    self.function_returns.insert(fn_name.clone(), ret.clone());
                }
                self.functions.insert(fn_name, sig);
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_type_aliases(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::TypeAlias {
                name,
                type_params,
                ty,
                span,
            } = stmt
            else {
                continue;
            };
            let alias_name = token_text(self.source, name.span);
            if is_builtin_type_name(&alias_name) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: *span,
                    message: format!("Type alias '{}' shadows a builtin type", alias_name),
                });
                continue;
            }
            if self.structs.contains_key(&alias_name) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: *span,
                    message: format!("Type alias '{}' conflicts with struct name", alias_name),
                });
                continue;
            }
            if self.enums.contains_key(&alias_name) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: *span,
                    message: format!("Type alias '{}' conflicts with enum name", alias_name),
                });
                continue;
            }
            if self.type_aliases.contains_key(&alias_name) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: *span,
                    message: format!("Duplicate type alias '{}'", alias_name),
                });
                continue;
            }
            let (param_sigs, param_set) = self.collect_type_param_sigs(type_params);
            let resolved_body = self.resolve_type_with_params(ty, &param_set);
            self.type_aliases.insert(
                alias_name,
                TypeAliasInfo {
                    params: param_sigs,
                    ty: resolved_body,
                    span: *span,
                },
            );
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_type_param_sigs(
        &mut self,
        params: &'a [TypeParam<'a>],
    ) -> (Vec<TypeParamSig>, HashSet<String>) {
        if params.is_empty() {
            return (Vec::new(), HashSet::new());
        }
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for param in params.iter() {
            let name = token_text(self.source, param.name.span);
            if !seen.insert(name.clone()) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: param.span,
                    message: format!("Duplicate type parameter '{}'", name),
                });
            }
            names.push(name);
        }
        let param_set: HashSet<String> = names.iter().cloned().collect();
        let mut out = Vec::new();
        for (idx, param) in params.iter().enumerate() {
            let constraint = param
                .constraint
                .map(|ty| self.resolve_type_with_params(ty, &param_set));
            out.push(TypeParamSig {
                name: names[idx].clone(),
                constraint,
            });
        }
        (out, param_set)
    }

    pub(in crate::phpx::typeck::check) fn check_call_signature(
        &mut self,
        func: ExprId<'a>,
        args: &'a [crate::parser::ast::Arg<'a>],
        env: &HashMap<String, Type>,
    ) -> Type {
        let Expr::Variable { span, .. } = *func else {
            return Type::Unknown;
        };
        let name = token_text(self.source, span);
        if name.starts_with('$') {
            return Type::Unknown;
        }
        if (name == "__deka_wasm_call"
            || name == "__deka_wasm_call_async"
            || name == "__bridge"
            || name == "__bridge_async"
            || name == "__deka_bridge")
            && !self.allow_internal_bridge_call()
        {
            self.errors.push(TypeError { severity: Severity::Error,
                span: Span::new(span.start, span.end),
                message: format!(
                    "{} is internal-only; import public modules instead (for example: db, postgres, mysql, sqlite, tcp, tls, encoding/json)",
                    name
                ),
            });
            return Type::Unknown;
        }
        let Some(sig) = self.functions.get(&name) else {
            return Type::Unknown;
        };
        let sig = sig.clone();

        let required = sig.params.iter().filter(|p| p.required).count();
        if args.len() < required {
            self.errors.push(TypeError { severity: Severity::Error,
                span: Span::new(span.start, span.end),
                message: format!(
                    "Missing arguments for {}(): expected at least {}, got {}",
                    name,
                    required,
                    args.len()
                ),
            });
        }

        let mut actuals = Vec::new();
        for arg in args.iter() {
            actuals.push(self.infer_expr_with_env(arg.value, env));
        }

        let mut inferred = HashMap::new();
        if !sig.type_params.is_empty() {
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
                    self.infer_type_params(param_ty, &actuals[idx], &mut inferred);
                }
                idx += 1;
            }

            for param in sig.type_params.iter() {
                if !inferred.contains_key(&param.name) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: Span::new(span.start, span.end),
                        message: format!(
                            "Unable to infer type parameter '{}' for {}()",
                            param.name, name
                        ),
                    });
                }
            }

            for param in sig.type_params.iter() {
                let Some(inferred_ty) = inferred.get(&param.name) else {
                    continue;
                };
                if let Some(constraint) = &param.constraint {
                    if !self.is_assignable(inferred_ty, constraint) {
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: Span::new(span.start, span.end),
                            message: format!(
                                "Type argument for '{}' does not satisfy constraint {}",
                                param.name, constraint
                            ),
                        });
                    }
                }
            }
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
                let expected = substitute_type(param_ty, &inferred);
                if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null))
                    && self.strict_null
                    && !self.type_allows_null(&expected)
                {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: args[idx].span,
                        message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                    });
                }
                if let Expr::ObjectLiteral { items, span } = *args[idx].value {
                    self.check_object_literal_against_type(items, &expected, span, env);
                }
                if !self.is_assignable(&actuals[idx], &expected) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: args[idx].span,
                        message: format!(
                            "Argument {} type mismatch: expected {}, got {}",
                            idx + 1,
                            expected,
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

        let ret = sig.return_type.clone().unwrap_or(Type::Unknown);
        substitute_type(&ret, &inferred)
    }
}
