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
                name,
                type_params,
                backed_type,
                ..
            } = stmt
            else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let backed = backed_type.and_then(|ty| enum_backed_primitive(ty));
            let (type_param_sigs, _) = self.collect_type_param_sigs(type_params);
            let type_param_names = type_param_sigs.into_iter().map(|s| s.name).collect();
            self.enums.entry(enum_name).or_insert_with(|| EnumInfo {
                cases: BTreeMap::new(),
                backed,
                type_params: type_param_names,
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
                        let sig = self.method_signature(params, *return_type, false);
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
                                    optional: entry.optional,
                                    is_mut: entry.is_mut,
                                },
                            );
                        }
                    }
                    ClassMember::PropertyHook { ty, name, modifiers, .. } => {
                        let field_name = token_text(self.source, name.span);
                        let field_name = field_name.trim_start_matches('$').to_string();
                        let is_mut = modifiers.iter().any(|token| {
                            token_text(self.source, token.span)
                                .eq_ignore_ascii_case("mut")
                        });
                        fields.insert(
                            field_name,
                            ObjectField {
                                ty: ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown),
                                optional: false,
                                is_mut,
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
            let mut own = HashSet::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type, false);
                    methods.insert(method_name.clone(), sig);
                    own.insert(method_name);
                }
            }
            self.struct_methods.insert(struct_name.clone(), methods);
            self.own_struct_methods.insert(struct_name, own);
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_receiver_methods(
        &mut self,
        program: &Program<'a>,
    ) {
        for stmt in program.statements.iter() {
            let Stmt::ReceiverMethod {
                receiver,
                name,
                params,
                return_type,
                ..
            } = stmt
            else {
                continue;
            };
            let receiver_ty = self.resolve_type(receiver.ty);
            let target_name = match &receiver_ty {
                Type::Struct(name) => name.clone(),
                Type::Enum(name) => name.clone(),
                _ => continue,
            };
            let method_name = token_text(self.source, name.span);
            let sig = self.method_signature(params, *return_type, receiver.is_mut);
            let methods = if self.structs.contains_key(&target_name) {
                self.struct_methods.entry(target_name.clone()).or_default()
            } else {
                self.enum_methods.entry(target_name.clone()).or_default()
            };
            methods.insert(method_name.clone(), sig);
            if self.structs.contains_key(&target_name) {
                self.own_struct_methods
                    .entry(target_name)
                    .or_default()
                    .insert(method_name);
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn promote_embedded_struct_methods(&mut self) {
        let struct_names: Vec<String> = self.structs.keys().cloned().collect();
        for struct_name in struct_names {
            let mut methods = self
                .struct_methods
                .get(&struct_name)
                .cloned()
                .unwrap_or_default();
            let own = self
                .own_struct_methods
                .get(&struct_name)
                .cloned()
                .unwrap_or_default();
            let mut origins: HashMap<String, Vec<String>> = HashMap::new();
            if let Some(embeds) = self.structs.get(&struct_name).map(|info| info.embeds.clone()) {
                for embed in embeds {
                    let mut visited = HashSet::new();
                    self.collect_promoted_method_origins(
                        &embed,
                        &embed,
                        &mut origins,
                        &mut visited,
                    );
                }
            }
            let mut ambiguous = HashSet::new();
            for (method_name, sources) in &origins {
                if sources.len() > 1 && !own.contains(method_name) {
                    ambiguous.insert(method_name.clone());
                }
            }
            if !ambiguous.is_empty() {
                self.ambiguous_promoted_methods
                    .insert(struct_name.clone(), ambiguous.clone());
            }
            for (method_name, sources) in origins {
                if own.contains(&method_name) || methods.contains_key(&method_name) {
                    continue;
                }
                if ambiguous.contains(&method_name) {
                    continue;
                }
                let source = &sources[0];
                if let Some(sig) = self
                    .struct_methods
                    .get(source)
                    .and_then(|m| m.get(&method_name))
                    .cloned()
                {
                    methods.insert(method_name, sig);
                }
            }
            if !methods.is_empty() {
                self.struct_methods.insert(struct_name, methods);
            }
        }
    }

    fn collect_promoted_method_origins(
        &self,
        top_embed: &str,
        embed: &str,
        origins: &mut HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
    ) {
        if !visited.insert(embed.to_string()) {
            return;
        }
        if let Some(embed_methods) = self.struct_methods.get(embed) {
            for name in embed_methods.keys() {
                origins
                    .entry(name.clone())
                    .or_default()
                    .push(top_embed.to_string());
            }
        }
        if let Some(info) = self.structs.get(embed) {
            for next in info.embeds.clone() {
                self.collect_promoted_method_origins(top_embed, &next, origins, visited);
            }
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
                    let sig = self.method_signature(params, *return_type, false);
                    methods.insert(method_name, sig);
                }
            }
            self.enum_methods.insert(enum_name, methods);
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_enum_cases(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum {
                name,
                type_params,
                members,
                ..
            } = stmt
            else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let (_, type_param_set) = self.collect_type_param_sigs(type_params);
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
                        let ty = param
                            .ty
                            .map(|ty| self.resolve_type_with_params(ty, &type_param_set));
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
                        type_params: type_param_set.into_iter().collect(),
                    },
                );
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn method_signature(
        &mut self,
        params: &'a [crate::parser::ast::Param<'a>],
        return_type: Option<&'a AstType<'a>>,
        mutable: bool,
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
            mutable,
        }
    }

    pub(in crate::phpx::typeck::check) fn collect_functions(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let stmt = export_decl_stmt(stmt);
            if let Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                ..
            } = stmt
            {
                let fn_name = token_text(self.source, name.span);
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
                    let param_types: Vec<Type> = sig
                        .params
                        .iter()
                        .map(|p| p.ty.clone().unwrap_or(Type::Unknown))
                        .collect();
                    self.function_value_types.insert(
                        fn_name.clone(),
                        Type::Function {
                            params: param_types,
                            return_type: Box::new(ret.clone()),
                        },
                    );
                }
                self.functions.insert(fn_name, sig);
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn infer_function_return_types(
        &mut self,
        program: &Program<'a>,
    ) {
        for stmt in program.statements.iter() {
            let stmt = export_decl_stmt(stmt);
            let Stmt::Function {
                name,
                is_async,
                type_params,
                params,
                return_type,
                body,
                ..
            } = stmt
            else {
                continue;
            };
            if return_type.is_some() {
                continue;
            }
            let fn_name = token_text(self.source, name.span);
            let (_, type_param_set) = self.collect_type_param_sigs(type_params);

            let mut fn_env: HashMap<String, Type> = HashMap::new();
            for param in params.iter() {
                let param_name = token_text(self.source, param.name.span)
                    .trim_start_matches('$')
                    .to_string();
                let param_ty = param
                    .ty
                    .map(|ty| self.resolve_type_with_params(ty, &type_param_set))
                    .unwrap_or(Type::Unknown);
                fn_env.insert(param_name, param_ty);
            }

            let returns = self.collect_return_types(body, &fn_env);
            let inferred = self.merge_return_types(&fn_name, name.span, returns);
            let final_ty = if *is_async {
                // Avoid Promise<Promise<T>> if the body already returns a Promise.
                match inferred {
                    Type::Applied { base, args } if base.eq_ignore_ascii_case("Promise") => {
                        Type::Applied {
                            base: "Promise".to_string(),
                            args: args.clone(),
                        }
                    }
                    other => Type::Applied {
                        base: "Promise".to_string(),
                        args: vec![other],
                    },
                }
            } else {
                inferred
            };
            self.function_returns.insert(fn_name, final_ty);
        }
    }

    fn collect_return_types(
        &self,
        body: &'a [StmtId<'a>],
        env: &HashMap<String, Type>,
    ) -> Vec<(Type, Option<Span>)> {
        let mut out = Vec::new();
        for stmt in body.iter() {
            self.collect_return_types_from_stmt(stmt, env, &mut out);
        }
        out
    }

    fn collect_return_types_from_stmt(
        &self,
        stmt: &'a Stmt<'a>,
        env: &HashMap<String, Type>,
        out: &mut Vec<(Type, Option<Span>)>,
    ) {
        match stmt {
            Stmt::Return { expr, span } => {
                let ty = expr
                    .map(|e| self.infer_expr_with_env(e, env))
                    .unwrap_or(Type::Primitive(PrimitiveType::Null));
                out.push((ty, Some(*span)));
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                for s in then_block.iter() {
                    self.collect_return_types_from_stmt(s, env, out);
                }
                if let Some(else_block) = else_block {
                    for s in else_block.iter() {
                        self.collect_return_types_from_stmt(s, env, out);
                    }
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Foreach { body, .. } => {
                for s in body.iter() {
                    self.collect_return_types_from_stmt(s, env, out);
                }
            }
            Stmt::Switch { cases, .. } => {
                for case in cases.iter() {
                    for s in case.body.iter() {
                        self.collect_return_types_from_stmt(s, env, out);
                    }
                }
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                for s in body.iter() {
                    self.collect_return_types_from_stmt(s, env, out);
                }
                for catch in catches.iter() {
                    for s in catch.body.iter() {
                        self.collect_return_types_from_stmt(s, env, out);
                    }
                }
                if let Some(finally) = finally {
                    for s in finally.iter() {
                        self.collect_return_types_from_stmt(s, env, out);
                    }
                }
            }
            Stmt::Block { statements, .. } => {
                for s in statements.iter() {
                    self.collect_return_types_from_stmt(s, env, out);
                }
            }
            _ => {}
        }
    }

    fn merge_return_types(
        &mut self,
        fn_name: &str,
        fn_span: Span,
        returns: Vec<(Type, Option<Span>)>,
    ) -> Type {
        let mut candidate: Option<Type> = None;
        for (ty, span) in returns {
            if matches!(ty, Type::Unknown) {
                continue;
            }
            match candidate {
                None => candidate = Some(ty),
                Some(ref existing) => {
                    if !self.is_assignable(&ty, existing) && !self.is_assignable(existing, &ty) {
                        let err_span = span.unwrap_or(fn_span);
                        self.errors.push(TypeError {
                            severity: Severity::Error,
                            span: err_span,
                            message: format!(
                                "Inferred return type for '{}' is incompatible across return branches",
                                fn_name
                            ),
                        });
                        return Type::Unknown;
                    }
                    candidate = Some(merge_types(existing, &ty));
                }
            }
        }
        candidate.unwrap_or(Type::Unknown)
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
            || name == "__deka_bridge"
            || name == "__deka_host")
            && !self.allow_internal_bridge_call()
        {
            self.errors.push(TypeError { severity: Severity::Error,
                span: Span::new(span.start, span.end),
                message: format!(
                    "{} is internal-only; use `bridge kind.action(args)` from a host-granted stdlib package",
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
                        message: "Null is not allowed in DekaScript; use Option<T> instead".to_string(),
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
                    message: "Null is not allowed in DekaScript; use Option<T> instead".to_string(),
                });
            }
            idx += 1;
        }

        let ret = sig.return_type.clone().unwrap_or(Type::Unknown);
        substitute_type(&ret, &inferred)
    }
}
