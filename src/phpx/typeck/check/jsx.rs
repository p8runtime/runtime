use super::*;
use crate::parser::ast::{Program, Stmt};

impl<'ast> Visitor<'ast> for JsxExprValidator {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match *expr {
            Expr::Assign { span, .. }
            | Expr::AssignRef { span, .. }
            | Expr::AssignOp { span, .. } => {
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: "Statements not allowed in JSX expressions".to_string(),
                });
            }
            Expr::Yield { span, .. } => {
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: "Statements not allowed in JSX expressions".to_string(),
                });
            }
            Expr::Error { span } => {
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: "Invalid JSX expression".to_string(),
                });
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn validate_jsx_expr(&mut self, expr: ExprId<'a>) {
        let mut validator = JsxExprValidator { errors: Vec::new() };
        validator.visit_expr(expr);
        self.errors.extend(validator.errors);
    }

    pub(in crate::phpx::typeck::check) fn validate_jsx_element(
        &mut self,
        name: &Name<'a>,
        attributes: &'a [crate::parser::ast::JsxAttribute<'a>],
        env: &mut HashMap<String, Type>,
    ) {
        let raw = token_text(self.source, name.span);
        let raw = raw.trim();
        if raw.is_empty() {
            return;
        }
        let trimmed = raw.trim_start_matches('\\');
        let last = trimmed.rsplit('\\').next().unwrap_or(trimmed);
        if last.is_empty() {
            return;
        }
        let mut chars = last.chars();
        let is_component = chars
            .next()
            .map(|ch| ch.is_ascii_uppercase())
            .unwrap_or(false);
        let has_uppercase = last.chars().any(|ch| ch.is_ascii_uppercase());

        if !is_component && has_uppercase {
            self.errors.push(TypeError { severity: Severity::Error,
                span: name.span,
                message: format!(
                    "JSX component '{}' must be capitalized (use <{} />)",
                    last,
                    capitalize_jsx_name(last)
                ),
            });
            return;
        }

        if is_component && !self.is_known_component_name(last) {
            self.errors.push(TypeError { severity: Severity::Error,
                span: name.span,
                message: format!(
                    "Unknown component '{}'; import it or define function {}()",
                    last, last
                ),
            });
            return;
        }

        if is_component {
            self.validate_component_props(last, attributes, name.span, env);
        }
    }

    pub(in crate::phpx::typeck::check) fn validate_component_props(
        &mut self,
        component: &str,
        attributes: &'a [crate::parser::ast::JsxAttribute<'a>],
        span: Span,
        env: &mut HashMap<String, Type>,
    ) {
        self.validate_component_signature(component, span);

        let mut attrs = HashSet::new();
        let mut attr_spans: HashMap<String, Span> = HashMap::new();
        let mut spread_unknown = false;
        for attr in attributes.iter() {
            if let Some(value) = attr.value {
                if let Expr::Spread { expr, .. } = value {
                    let spread_ty = self.infer_expr_with_env(expr, env);
                    if let Some(fields) = self.object_type_fields(&spread_ty) {
                        for (name, _field) in fields {
                            attrs.insert(name);
                        }
                    } else if matches!(spread_ty, Type::Object | Type::Mixed | Type::Unknown) {
                        spread_unknown = true;
                    }
                    continue;
                }
            }
            let name = token_text(self.source, attr.name.span);
            attrs.insert(name.clone());
            attr_spans.insert(name, attr.name.span);
        }

        match component {
            "Link" => {
                if !attrs.contains("to") {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span,
                        message: "Link requires prop 'to'".to_string(),
                    });
                }
            }
            "ContextProvider" => {
                if !attrs.contains("ctx") {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span,
                        message: "ContextProvider requires prop 'ctx'".to_string(),
                    });
                }
                if !attrs.contains("value") {
                    self.errors.push(TypeError { severity: Severity::Error,
                        span,
                        message: "ContextProvider requires prop 'value'".to_string(),
                    });
                }
            }
            _ => {}
        }

        let Some(sig) = self.functions.get(component).cloned() else {
            return;
        };
        let Some(props_ty) = sig.params.first().and_then(|param| param.ty.clone()) else {
            return;
        };
        if matches!(props_ty, Type::Struct(_)) || !self.is_component_props_type(&props_ty) {
            return;
        }
        let Some(expected_fields) = self.component_props_fields(&props_ty) else {
            return;
        };

        for (attr_name, attr_span) in attr_spans.iter() {
            if expected_fields.contains_key(attr_name) {
                continue;
            }
            let suggestion = nearest_name(attr_name, expected_fields.keys().map(|k| k.as_str()));
            let mut message = format!("Unknown prop '{}' for component '{}'", attr_name, component);
            if let Some(suggested) = suggestion {
                message.push_str(&format!("; did you mean '{}'?", suggested));
            }
            self.errors.push(TypeError { severity: Severity::Error,
                span: *attr_span,
                message,
            });
        }

        if !spread_unknown {
            for (field_name, field) in expected_fields.iter() {
                if field.optional || attrs.contains(field_name) {
                    continue;
                }
                self.errors.push(TypeError { severity: Severity::Error,
                    span,
                    message: format!(
                        "Missing required prop '{}' for component '{}'",
                        field_name, component
                    ),
                });
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn validate_component_signature(
        &mut self,
        component: &str,
        span: Span,
    ) {
        let strict = std::env::var("PHPX_STRICT_JSX_TYPES")
            .map(|value| {
                let value = value.trim().to_ascii_lowercase();
                value == "1" || value == "true" || value == "yes" || value == "on"
            })
            .unwrap_or(false);
        if !strict {
            return;
        }

        let Some(sig) = self.functions.get(component).cloned() else {
            return;
        };

        if sig.variadic || sig.params.len() != 1 {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "JSX component '{}' must accept exactly one typed props parameter",
                    component
                ),
            });
            return;
        }

        let Some(props_ty) = sig.params.first().and_then(|param| param.ty.clone()) else {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "JSX component '{}' props parameter must be typed (use interface or Object<{{...}}>)",
                    component
                ),
            });
            return;
        };

        if let Type::Struct(name) = &props_ty {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "JSX component '{}' props type '{}' cannot be a struct; use interface '{}' or Object<{{...}}>",
                    component, name, name
                ),
            });
            return;
        }

        if !self.is_component_props_type(&props_ty) {
            self.errors.push(TypeError { severity: Severity::Error,
                span,
                message: format!(
                    "JSX component '{}' props type must be interface or object shape, got {}",
                    component, props_ty
                ),
            });
        }
    }

    pub(in crate::phpx::typeck::check) fn is_component_props_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Interface(_) | Type::Object | Type::ObjectShape(_) => true,
            Type::Applied { base, args } => {
                if base == "Object" {
                    return true;
                }
                if (base == "Option" || base == "Result") && !args.is_empty() {
                    return self.is_component_props_type(&args[0]);
                }
                false
            }
            Type::Union(types) => {
                !types.is_empty()
                    && types
                        .iter()
                        .all(|inner| self.is_component_props_type(inner))
            }
            _ => false,
        }
    }

    pub(in crate::phpx::typeck::check) fn component_props_fields(
        &self,
        ty: &Type,
    ) -> Option<BTreeMap<String, ObjectField>> {
        match ty {
            Type::Interface(name) => self.interfaces.get(name).map(|info| info.fields.clone()),
            Type::ObjectShape(fields) => Some(fields.clone()),
            Type::Applied { base, args } if base == "Object" => args.first().and_then(|arg| {
                if let Type::ObjectShape(fields) = arg {
                    Some(fields.clone())
                } else {
                    None
                }
            }),
            Type::Union(types) => {
                let mut merged: BTreeMap<String, ObjectField> = BTreeMap::new();
                for inner in types {
                    let Some(fields) = self.component_props_fields(inner) else {
                        continue;
                    };
                    for (name, field) in fields {
                        match merged.get_mut(&name) {
                            Some(existing) => {
                                existing.ty = merge_types(&existing.ty, &field.ty);
                                existing.optional = existing.optional || field.optional;
                            }
                            None => {
                                merged.insert(name, field);
                            }
                        }
                    }
                }
                if merged.is_empty() {
                    None
                } else {
                    Some(merged)
                }
            }
            _ => None,
        }
    }

    pub(in crate::phpx::typeck::check) fn is_known_component_name(&self, name: &str) -> bool {
        self.functions.contains_key(name) || self.imported.contains_key(name)
    }

    pub(in crate::phpx::typeck::check) fn collect_imported_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Import { specs, from, .. } = stmt else {
                continue;
            };
            let module = String::from_utf8_lossy(from.text(self.source)).to_string();
            for spec in specs.iter() {
                let local = String::from_utf8_lossy(spec.local.text(self.source)).to_string();
                self.imported.insert(local, module.clone());
            }
        }
    }
}
