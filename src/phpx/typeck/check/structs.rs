use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn validate_struct_field_annotations(
        &mut self,
        struct_name: &str,
        field_name: &str,
        entry: &PropertyEntry<'a>,
        field_type: Option<&Type>,
        declared_fields: &BTreeMap<String, Type>,
    ) {
        let mut seen = HashSet::new();
        for ann in entry.annotations.iter() {
            let ann_name = token_text(self.source, ann.name.span).to_string();
            if !seen.insert(ann_name.clone()) {
                self.errors.push(TypeError {
                    span: ann.span,
                    message: format!(
                        "Duplicate annotation '@{}' on struct field '{}::{}'",
                        ann_name, struct_name, field_name
                    ),
                });
                continue;
            }

            match ann_name.as_str() {
                "id" | "unique" | "autoIncrement" => {
                    if !ann.args.is_empty() {
                        self.errors.push(TypeError {
                            span: ann.span,
                            message: format!(
                                "Annotation '@{}' does not accept arguments",
                                ann_name
                            ),
                        });
                    }
                }
                "index" => {
                    if ann.args.len() > 1 {
                        self.errors.push(TypeError {
                            span: ann.span,
                            message: "Annotation '@index' accepts at most one argument".to_string(),
                        });
                    }
                    if ann.args.len() == 1 && !matches!(ann.args[0], Expr::String { .. }) {
                        self.errors.push(TypeError {
                            span: ann.args[0].span(),
                            message: "Annotation '@index' argument must be a string literal"
                                .to_string(),
                        });
                    }
                }
                "map" => {
                    if ann.args.len() != 1 {
                        self.errors.push(TypeError {
                            span: ann.span,
                            message: "Annotation '@map' requires exactly one string argument"
                                .to_string(),
                        });
                    } else if !matches!(ann.args[0], Expr::String { .. }) {
                        self.errors.push(TypeError {
                            span: ann.args[0].span(),
                            message: "Annotation '@map' argument must be a string literal"
                                .to_string(),
                        });
                    }
                }
                "default" => {
                    if ann.args.len() != 1 {
                        self.errors.push(TypeError {
                            span: ann.span,
                            message: "Annotation '@default' requires exactly one argument"
                                .to_string(),
                        });
                    }
                }
                "relation" => {
                    if ann.args.len() != 3 {
                        self.errors.push(TypeError {
                            span: ann.span,
                            message: "Annotation '@relation' requires exactly three string arguments: kind, model, foreignKey".to_string(),
                        });
                    } else {
                        let normalize = |raw: &[u8]| {
                            let s = String::from_utf8_lossy(raw).to_string();
                            s.trim_matches('"').trim_matches('\'').to_string()
                        };
                        let kind = if let Expr::String { value, .. } = ann.args[0] {
                            Some(normalize(value))
                        } else {
                            None
                        };
                        let model = if let Expr::String { value, .. } = ann.args[1] {
                            Some(normalize(value))
                        } else {
                            None
                        };
                        let foreign_key = if let Expr::String { value, .. } = ann.args[2] {
                            Some(normalize(value))
                        } else {
                            None
                        };

                        if kind.is_none() {
                            self.errors.push(TypeError {
                                span: ann.args[0].span(),
                                message: "Annotation '@relation' first argument (kind) must be a string literal".to_string(),
                            });
                        }
                        if model.is_none() {
                            self.errors.push(TypeError {
                                span: ann.args[1].span(),
                                message: "Annotation '@relation' second argument (model) must be a string literal".to_string(),
                            });
                        }
                        if foreign_key.is_none() {
                            self.errors.push(TypeError {
                                span: ann.args[2].span(),
                                message: "Annotation '@relation' third argument (foreignKey) must be a string literal".to_string(),
                            });
                        }

                        if let Some(kind) = kind {
                            if kind != "hasMany" && kind != "belongsTo" && kind != "hasOne" {
                                self.errors.push(TypeError {
                                    span: ann.args[0].span(),
                                    message: "Annotation '@relation' kind must be one of: hasMany, belongsTo, hasOne".to_string(),
                                });
                            }
                            let inferred_model = relation_model_from_field_type(field_type, &kind);
                            if let Some(expected_model) = inferred_model {
                                if let Some(ref model) = model {
                                    if *model != expected_model {
                                        self.errors.push(TypeError {
                                            span: ann.args[1].span(),
                                            message: format!(
                                                "Annotation '@relation' model '{}' does not match field type model '{}'",
                                                model, expected_model
                                            ),
                                        });
                                    }
                                }
                            }
                            if kind == "hasMany" {
                                let is_array = match field_type {
                                    Some(Type::Array) => true,
                                    Some(Type::Applied { base, .. }) => {
                                        base.eq_ignore_ascii_case("array")
                                    }
                                    _ => false,
                                };
                                if !is_array {
                                    self.errors.push(TypeError {
                                        span: ann.span,
                                        message: format!(
                                            "Annotation '@relation(\"hasMany\", ...)' requires array field type on '{}::{}'",
                                            struct_name, field_name
                                        ),
                                    });
                                }
                            }
                            if kind == "belongsTo" || kind == "hasOne" {
                                if let Some(ref fk) = foreign_key {
                                    if fk == field_name {
                                        self.errors.push(TypeError {
                                            span: ann.args[2].span(),
                                            message: format!(
                                                "Annotation '@relation' foreignKey '{}' cannot reference relation field '{}::{}'",
                                                fk, struct_name, field_name
                                            ),
                                        });
                                    } else if !declared_fields.contains_key(fk) {
                                        self.errors.push(TypeError {
                                            span: ann.args[2].span(),
                                            message: format!(
                                                "Annotation '@relation' foreignKey '{}' was not found on struct '{}'",
                                                fk, struct_name
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    self.errors.push(TypeError {
                        span: ann.span,
                        message: format!(
                            "Unknown struct field annotation '@{}' on '{}::{}'",
                            ann_name, struct_name, field_name
                        ),
                    });
                }
            }

            if ann_name == "autoIncrement" {
                let is_int = matches!(field_type, Some(Type::Primitive(PrimitiveType::Int)));
                if !is_int {
                    self.errors.push(TypeError {
                        span: ann.span,
                        message: format!(
                            "Annotation '@autoIncrement' requires int field type on '{}::{}'",
                            struct_name, field_name
                        ),
                    });
                }
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn check_struct_defaults(
        &mut self,
        members: &'a [ClassMember<'a>],
    ) {
        for member in members.iter() {
            match member {
                ClassMember::Property { ty, entries, .. } => {
                    let expected = ty.map(|ty| self.resolve_type(ty));
                    for entry in entries.iter() {
                        self.check_property_default(entry, expected.as_ref());
                    }
                }
                ClassMember::PropertyHook {
                    ty, name, default, ..
                } => {
                    if let Some(expected) = ty.map(|ty| self.resolve_type(ty)) {
                        if let Some(default) = default {
                            if !self.is_constant_expr(default) {
                                self.errors.push(TypeError {
                                    span: member_span(member),
                                    message: "Struct field defaults must be constant expressions"
                                        .to_string(),
                                });
                            }
                            let actual = self.infer_expr_with_env(*default, &HashMap::new());
                            if !self.is_assignable(&actual, &expected) {
                                let prop_name = token_text(self.source, name.span);
                                self.errors.push(TypeError {
                                    span: member_span(member),
                                    message: format!(
                                        "Default value for {} has type {}, expected {}",
                                        prop_name, actual, expected
                                    ),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn check_property_default(
        &mut self,
        entry: &PropertyEntry<'a>,
        expected: Option<&Type>,
    ) {
        let Some(expected) = expected else {
            return;
        };
        let Some(default) = entry.default else {
            return;
        };
        if !self.is_constant_expr(default) {
            self.errors.push(TypeError {
                span: entry.span,
                message: "Struct field defaults must be constant expressions".to_string(),
            });
        }
        let actual = self.infer_expr_with_env(default, &HashMap::new());
        if !self.is_assignable(&actual, expected) {
            let name = token_text(self.source, entry.name.span);
            self.errors.push(TypeError {
                span: entry.span,
                message: format!(
                    "Default value for {} has type {}, expected {}",
                    name, actual, expected
                ),
            });
        }
    }

    pub(in crate::phpx::typeck::check) fn is_constant_expr(&self, expr: &Expr<'a>) -> bool {
        match expr {
            Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Boolean { .. }
            | Expr::Null { .. } => true,
            Expr::Array { items, .. } => items.iter().all(|item| {
                if item.unpack {
                    return false;
                }
                let key_ok = item
                    .key
                    .map(|key| self.is_constant_expr(key))
                    .unwrap_or(true);
                key_ok && self.is_constant_expr(item.value)
            }),
            Expr::ObjectLiteral { items, .. } => {
                items.iter().all(|item| self.is_constant_expr(item.value))
            }
            Expr::StructLiteral { fields, .. } => fields
                .iter()
                .all(|field| self.is_constant_expr(field.value)),
            Expr::ClassConstFetch { .. } => true,
            Expr::Binary {
                op, left, right, ..
            } => {
                matches!(op, BinaryOp::BitOr)
                    && self.is_constant_expr(left)
                    && self.is_constant_expr(right)
            }
            Expr::Unary { op, expr, .. } => {
                matches!(op, UnaryOp::Plus | UnaryOp::Minus) && self.is_constant_expr(expr)
            }
            _ => false,
        }
    }
}
