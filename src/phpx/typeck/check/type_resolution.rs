use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn resolve_type(&mut self, ty: &AstType<'a>) -> Type {
        let mut visiting = HashSet::new();
        let params = HashSet::new();
        self.resolve_type_internal(ty, &mut visiting, &params)
    }

    pub(in crate::phpx::typeck::check) fn resolve_type_with_params(
        &mut self,
        ty: &AstType<'a>,
        params: &HashSet<String>,
    ) -> Type {
        let mut visiting = HashSet::new();
        self.resolve_type_internal(ty, &mut visiting, params)
    }

    pub(in crate::phpx::typeck::check) fn resolve_type_internal(
        &mut self,
        ty: &AstType<'a>,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        match ty {
            AstType::Simple(token) => {
                if token.kind == TokenKind::TypeNull {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: token.span,
                        message: "Null types are not allowed in PHPX; use Option<T> instead"
                            .to_string(),
                    });
                }
                self.resolve_named_type(token_text(self.source, token.span), visiting, params)
            }
            AstType::Name(name) => self.resolve_name_type(name, visiting, params),
            AstType::Union(types) => {
                if let Some(span) = self.find_null_type_span(types) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span,
                        message: "Nullable unions are not allowed in PHPX; use Option<T> instead"
                            .to_string(),
                    });
                }
                let mut out = Vec::new();
                for ty in types.iter() {
                    out.push(self.resolve_type_internal(ty, visiting, params));
                }
                Type::Union(out)
            }
            AstType::Intersection(types) => {
                let mut out = Vec::new();
                for ty in types.iter() {
                    out.push(self.resolve_type_internal(ty, visiting, params));
                }
                if out.is_empty() {
                    Type::Unknown
                } else if out.len() == 1 {
                    out[0].clone()
                } else {
                    Type::Union(out)
                }
            }
            AstType::Nullable(inner) => {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: self.type_span(inner),
                    message: "Nullable types are not allowed in PHPX; use Option<T> instead"
                        .to_string(),
                });
                let inner = self.resolve_type_internal(inner, visiting, params);
                Type::Union(vec![inner, Type::Primitive(PrimitiveType::Null)])
            }
            AstType::ObjectShape(fields) => {
                let mut map = BTreeMap::new();
                for field in fields.iter() {
                    let name = parse_type_field_name(self.source, field.name.span);
                    let ty = self.resolve_type_internal(field.ty, visiting, params);
                    map.insert(
                        name,
                        ObjectField {
                            ty,
                            optional: field.optional,
                        },
                    );
                }
                Type::ObjectShape(map)
            }
            AstType::Applied { base, args } => {
                let base_name = match self.base_type_name(base) {
                    Some(name) => name,
                    None => return Type::Unknown,
                };
                if base_name.eq_ignore_ascii_case("Option") && args.len() != 1 {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: self.type_span(base),
                        message: "Option<T> expects exactly one type argument".to_string(),
                    });
                }
                if base_name.eq_ignore_ascii_case("Result") && args.len() != 2 {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: self.type_span(base),
                        message: "Result<T, E> expects exactly two type arguments".to_string(),
                    });
                }
                if base_name.eq_ignore_ascii_case("array") && args.len() != 1 {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: self.type_span(base),
                        message: "array<T> expects exactly one type argument".to_string(),
                    });
                }
                if base_name.eq_ignore_ascii_case("Promise") && args.len() != 1 {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: self.type_span(base),
                        message: "Promise<T> expects exactly one type argument".to_string(),
                    });
                }
                let mut resolved_args = Vec::new();
                for arg in args.iter() {
                    resolved_args.push(self.resolve_type_internal(arg, visiting, params));
                }
                if let Some(instantiated) =
                    self.resolve_alias_applied(&base_name, &resolved_args, visiting)
                {
                    instantiated
                } else {
                    if !base_name.eq_ignore_ascii_case("Option")
                        && !base_name.eq_ignore_ascii_case("Result")
                        && !base_name.eq_ignore_ascii_case("array")
                        && !base_name.eq_ignore_ascii_case("Promise")
                    {
                        self.errors.push(TypeError { severity: Severity::Error,
                            span: self.type_span(base),
                            message: format!(
                                "Unknown generic type '{}' in PHPX; classes are not allowed",
                                base_name
                            ),
                        });
                        Type::Unknown
                    } else {
                        Type::Applied {
                            base: base_name,
                            args: resolved_args,
                        }
                    }
                }
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn type_span(&self, ty: &AstType<'a>) -> Span {
        match ty {
            AstType::Simple(token) => token.span,
            AstType::Name(name) => name.parts.first().map(|p| p.span).unwrap_or_default(),
            AstType::Union(types) | AstType::Intersection(types) => {
                types.first().map(|t| self.type_span(t)).unwrap_or_default()
            }
            AstType::Nullable(inner) => self.type_span(inner),
            AstType::ObjectShape(fields) => {
                fields.first().map(|field| field.span).unwrap_or_default()
            }
            AstType::Applied { base, .. } => self.type_span(base),
        }
    }

    pub(in crate::phpx::typeck::check) fn find_null_type_span(
        &self,
        types: &'a [AstType<'a>],
    ) -> Option<Span> {
        for ty in types.iter() {
            match ty {
                AstType::Simple(token) if token.kind == TokenKind::TypeNull => {
                    return Some(token.span);
                }
                AstType::Nullable(inner) => return Some(self.type_span(inner)),
                AstType::Union(inner) | AstType::Intersection(inner) => {
                    if let Some(span) = self.find_null_type_span(inner) {
                        return Some(span);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub(in crate::phpx::typeck::check) fn resolve_named_type(
        &mut self,
        name: String,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "int" | "integer" | "number" => Type::Primitive(PrimitiveType::Int),
            "float" | "double" => Type::Primitive(PrimitiveType::Float),
            "bool" | "boolean" => Type::Primitive(PrimitiveType::Bool),
            "string" => Type::Primitive(PrimitiveType::String),
            "bytes" => Type::Primitive(PrimitiveType::Bytes),
            "null" => Type::Primitive(PrimitiveType::Null),
            "array" => Type::Array,
            "object" => Type::Object,
            "mixed" => Type::Mixed,
            _ => {
                if params.contains(&name) {
                    Type::TypeParam(name)
                } else if let Some(alias) = self.resolve_alias(&name, visiting) {
                    alias
                } else if name.eq_ignore_ascii_case("Option") {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: Span::new(0, 0),
                        message: "Option<T> requires a type argument".to_string(),
                    });
                    Type::Unknown
                } else if name.eq_ignore_ascii_case("Result") {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: Span::new(0, 0),
                        message: "Result<T, E> requires type arguments".to_string(),
                    });
                    Type::Unknown
                } else if self.enums.contains_key(&name) {
                    Type::Enum(name)
                } else if self.interfaces.contains_key(&name) {
                    Type::Interface(name)
                } else if self.structs.contains_key(&name) {
                    Type::Struct(name)
                } else {
                    Type::Object
                }
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn resolve_name_type(
        &mut self,
        name: &Name<'a>,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        let mut out = String::new();
        for (idx, part) in name.parts.iter().enumerate() {
            let text = token_text(self.source, part.span);
            if idx > 0 {
                out.push('\\');
            }
            out.push_str(text.trim_matches('\\'));
        }
        if out.eq_ignore_ascii_case("Option") {
            self.errors.push(TypeError { severity: Severity::Error,
                span: name.span,
                message: "Option<T> requires a type argument".to_string(),
            });
            return Type::Unknown;
        }
        if out.eq_ignore_ascii_case("Result") {
            self.errors.push(TypeError { severity: Severity::Error,
                span: name.span,
                message: "Result<T, E> requires type arguments".to_string(),
            });
            return Type::Unknown;
        }
        if !self.is_known_named_type(&out, params) {
            self.errors.push(TypeError { severity: Severity::Error,
                span: name.span,
                message: format!("Unknown type '{}' in PHPX; classes are not allowed", out),
            });
            return Type::Unknown;
        }
        self.resolve_named_type(out, visiting, params)
    }

    pub(in crate::phpx::typeck::check) fn resolve_alias(
        &mut self,
        name: &str,
        visiting: &mut HashSet<String>,
    ) -> Option<Type> {
        if let Some(resolved) = self.resolved_aliases.get(name) {
            return Some(resolved.clone());
        }
        let info = match self.type_aliases.get(name) {
            Some(info) => info,
            None => return None,
        };
        if !info.params.is_empty() {
            self.errors.push(TypeError { severity: Severity::Error,
                span: info.span,
                message: format!("Type alias '{}' requires type arguments", name),
            });
            return Some(Type::Unknown);
        }
        if !visiting.insert(name.to_string()) {
            self.errors.push(TypeError { severity: Severity::Error,
                span: info.span,
                message: format!("Recursive type alias '{}'", name),
            });
            return Some(Type::Unknown);
        }
        let resolved = info.ty.clone();
        visiting.remove(name);
        self.resolved_aliases
            .insert(name.to_string(), resolved.clone());
        Some(resolved)
    }

    pub(in crate::phpx::typeck::check) fn is_known_named_type(
        &self,
        name: &str,
        params: &HashSet<String>,
    ) -> bool {
        if params.contains(name) {
            return true;
        }
        if name.eq_ignore_ascii_case("int")
            || name.eq_ignore_ascii_case("integer")
            || name.eq_ignore_ascii_case("number")
            || name.eq_ignore_ascii_case("float")
            || name.eq_ignore_ascii_case("double")
            || name.eq_ignore_ascii_case("bool")
            || name.eq_ignore_ascii_case("boolean")
            || name.eq_ignore_ascii_case("string")
            || name.eq_ignore_ascii_case("bytes")
            || name.eq_ignore_ascii_case("null")
            || name.eq_ignore_ascii_case("array")
            || name.eq_ignore_ascii_case("object")
            || name.eq_ignore_ascii_case("mixed")
            || name.eq_ignore_ascii_case("option")
            || name.eq_ignore_ascii_case("result")
        {
            return true;
        }
        self.type_aliases.contains_key(name)
            || self.structs.contains_key(name)
            || self.enums.contains_key(name)
            || self.interfaces.contains_key(name)
    }

    pub(in crate::phpx::typeck::check) fn resolve_alias_applied(
        &mut self,
        name: &str,
        args: &[Type],
        visiting: &mut HashSet<String>,
    ) -> Option<Type> {
        let info = self.type_aliases.get(name)?;
        if info.params.len() != args.len() {
            self.errors.push(TypeError { severity: Severity::Error,
                span: info.span,
                message: format!(
                    "Type alias '{}' expects {} type arguments, got {}",
                    name,
                    info.params.len(),
                    args.len()
                ),
            });
            return Some(Type::Unknown);
        }
        if !visiting.insert(name.to_string()) {
            self.errors.push(TypeError { severity: Severity::Error,
                span: info.span,
                message: format!("Recursive type alias '{}'", name),
            });
            return Some(Type::Unknown);
        }
        let mut mapping = HashMap::new();
        for (idx, param) in info.params.iter().enumerate() {
            let arg = args[idx].clone();
            if let Some(constraint) = &param.constraint {
                if !self.is_assignable(&arg, constraint) {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span: info.span,
                        message: format!(
                            "Type argument {} for '{}' does not satisfy constraint {}",
                            idx + 1,
                            name,
                            constraint
                        ),
                    });
                }
            }
            mapping.insert(param.name.clone(), arg);
        }
        let resolved = substitute_type(&info.ty, &mapping);
        visiting.remove(name);
        Some(resolved)
    }

    pub(in crate::phpx::typeck::check) fn base_type_name(
        &self,
        base: &AstType<'a>,
    ) -> Option<String> {
        match base {
            AstType::Simple(token) => Some(token_text(self.source, token.span)),
            AstType::Name(name) => {
                let mut out = String::new();
                for (idx, part) in name.parts.iter().enumerate() {
                    let text = token_text(self.source, part.span);
                    if idx > 0 {
                        out.push('\\');
                    }
                    out.push_str(text.trim_matches('\\'));
                }
                Some(out)
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn check_wasm_stubs(&mut self) {
        let Some(file_path) = self.file_path.as_ref() else {
            return;
        };
        let Some(modules_root) = find_modules_root(file_path) else {
            return;
        };

        let source = String::from_utf8_lossy(self.source);
        let regex = match Regex::new(
            r#"(?m)^[\t \r]*import\s+\{[^}]+\}\s+from\s+['"]([^'"]+)['"]\s*(?:as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;?\s*$"#,
        ) {
            Ok(regex) => regex,
            Err(_) => return,
        };

        for caps in regex.captures_iter(&source) {
            let kind = caps.get(2).map(|m| m.as_str());
            if kind != Some("wasm") {
                continue;
            }
            let Some(matched) = caps.get(0) else {
                continue;
            };
            let Some(spec) = caps.get(1).map(|m| m.as_str()) else {
                continue;
            };

            if let Err(message) = resolve_wasm_stub(spec, file_path, &modules_root) {
                self.errors.push(TypeError { severity: Severity::Error,
                    span: Span::new(matched.start(), matched.end()),
                    message,
                });
            }
        }
    }
}
