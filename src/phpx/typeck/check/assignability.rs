use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn is_assignable(
        &self,
        source: &Type,
        target: &Type,
    ) -> bool {
        if matches!(target, Type::Unknown | Type::Mixed)
            || matches!(source, Type::Unknown | Type::Mixed)
        {
            return true;
        }
        match target {
            Type::Union(options) => {
                return options.iter().any(|opt| self.is_assignable(source, opt));
            }
            _ => {}
        }
        match source {
            Type::Union(options) => {
                return options.iter().all(|opt| self.is_assignable(opt, target));
            }
            _ => {}
        }
        if let Type::Applied { base, args } = target {
            if base.eq_ignore_ascii_case("Option") && args.len() == 1 {
                if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: source_args,
                } = source
                {
                    if enum_name.eq_ignore_ascii_case("Option") {
                        if case_name.eq_ignore_ascii_case("None") {
                            return true;
                        }
                        if case_name.eq_ignore_ascii_case("Some") {
                            let actual = source_args.get(0).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[0]);
                        }
                    }
                }
            }
            if base.eq_ignore_ascii_case("Result") && args.len() == 2 {
                if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: source_args,
                } = source
                {
                    if enum_name.eq_ignore_ascii_case("Result") {
                        if case_name.eq_ignore_ascii_case("Ok") {
                            let actual = source_args.get(0).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[0]);
                        }
                        if case_name.eq_ignore_ascii_case("Err") {
                            let actual = source_args.get(1).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[1]);
                        }
                    }
                }
            }
            // General rule for user-defined generic enums: an EnumCase value is
            // assignable to an Applied enum type when every payload argument
            // matches the corresponding type parameter substitution.
            if let Type::EnumCase {
                enum_name,
                case_name,
                args: source_args,
            } = source
            {
                if base.eq_ignore_ascii_case(enum_name) {
                    if let Some(info) = self.enums.get(enum_name) {
                        if let Some(case_info) = info.cases.get(case_name) {
                            if case_info.params.is_empty() {
                                return true;
                            }
                            let mapping: HashMap<String, Type> = info
                                .type_params
                                .iter()
                                .cloned()
                                .zip(args.iter().cloned())
                                .collect();
                            for (idx, param) in case_info.params.iter().enumerate() {
                                if let Some(param_ty) = &param.ty {
                                    let expected = substitute_type(param_ty, &mapping);
                                    let actual = source_args.get(idx).unwrap_or(&Type::Unknown);
                                    if !self.is_assignable(actual, &expected) {
                                        return false;
                                    }
                                }
                            }
                            return true;
                        }
                    }
                }
            }
        }
        match target {
            Type::Interface(name) => {
                return self.type_satisfies_interface(source, name);
            }
            _ => {}
        }
        match (source, target) {
            (Type::Interface(a), Type::Interface(b)) => a == b,
            (Type::Interface(_), Type::Object) => true,
            _ => is_assignable_base(source, target),
        }
    }

    pub(in crate::phpx::typeck::check) fn type_satisfies_interface(
        &self,
        source: &Type,
        iface: &str,
    ) -> bool {
        let Some(info) = self.interfaces.get(iface) else {
            return false;
        };
        match source {
            Type::Struct(name) => self.struct_satisfies_interface(name, info),
            Type::Enum(name) => self.enum_satisfies_interface(name, info),
            Type::EnumCase { enum_name, .. } => self.enum_satisfies_interface(enum_name, info),
            Type::Interface(name) => name.eq_ignore_ascii_case(iface),
            Type::ObjectShape(fields) => self.object_shape_satisfies_interface(fields, info),
            _ => false,
        }
    }

    pub(in crate::phpx::typeck::check) fn struct_satisfies_interface(
        &self,
        name: &str,
        iface: &InterfaceInfo,
    ) -> bool {
        let Some(methods) = self.struct_methods.get(name) else {
            return false;
        };
        for (method_name, expected) in iface.methods.iter() {
            let Some(actual) = methods.get(method_name) else {
                return false;
            };
            if !self.method_satisfies_interface(actual, expected) {
                return false;
            }
        }
        true
    }

    pub(in crate::phpx::typeck::check) fn enum_satisfies_interface(
        &self,
        name: &str,
        iface: &InterfaceInfo,
    ) -> bool {
        let Some(methods) = self.enum_methods.get(name) else {
            return false;
        };
        for (method_name, expected) in iface.methods.iter() {
            let Some(actual) = methods.get(method_name) else {
                return false;
            };
            if !self.method_satisfies_interface(actual, expected) {
                return false;
            }
        }
        true
    }

    pub(in crate::phpx::typeck::check) fn method_satisfies_interface(
        &self,
        actual: &MethodSig,
        expected: &MethodSig,
    ) -> bool {
        if expected.variadic && !actual.variadic {
            return false;
        }

        let expected_required = expected.params.iter().filter(|p| p.required).count();
        let actual_required = actual.params.iter().filter(|p| p.required).count();
        if actual_required > expected_required {
            return false;
        }

        if expected.params.len() > actual.params.len() && !actual.variadic {
            return false;
        }

        let actual_last = actual.params.last();
        for (idx, expected_param) in expected.params.iter().enumerate() {
            let actual_param = if idx < actual.params.len() {
                &actual.params[idx]
            } else {
                match actual_last {
                    Some(param) => param,
                    None => return false,
                }
            };

            let expected_ty = expected_param.ty.as_ref().cloned().unwrap_or(Type::Mixed);
            let actual_ty = actual_param.ty.as_ref().cloned().unwrap_or(Type::Mixed);
            if !self.is_assignable(&expected_ty, &actual_ty) {
                return false;
            }
        }

        if let Some(expected_ret) = expected.return_type.as_ref() {
            let actual_ret = actual.return_type.as_ref().cloned().unwrap_or(Type::Mixed);
            if !self.is_assignable(&actual_ret, expected_ret) {
                return false;
            }
        }
        true
    }

    pub(in crate::phpx::typeck::check) fn object_shape_satisfies_interface(
        &self,
        fields: &BTreeMap<String, ObjectField>,
        iface: &InterfaceInfo,
    ) -> bool {
        for (field_name, expected_field) in iface.fields.iter() {
            let Some(actual_field) = fields.get(field_name) else {
                if expected_field.optional {
                    continue;
                }
                return false;
            };
            if actual_field.optional && !expected_field.optional {
                return false;
            }
            if !self.is_assignable(&actual_field.ty, &expected_field.ty) {
                return false;
            }
        }
        true
    }
}

fn is_assignable_base(source: &Type, target: &Type) -> bool {
    if matches!(target, Type::Unknown | Type::Mixed)
        || matches!(source, Type::Unknown | Type::Mixed)
    {
        return true;
    }
    match target {
        Type::Union(options) => {
            return options.iter().any(|opt| is_assignable_base(source, opt));
        }
        _ => {}
    }
    match source {
        Type::Union(options) => {
            return options.iter().all(|opt| is_assignable_base(opt, target));
        }
        _ => {}
    }
    match (source, target) {
        (Type::TypeParam(a), Type::TypeParam(b)) => a == b,
        (Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
            (PrimitiveType::Int, PrimitiveType::Float) => true,
            _ => a == b,
        },
        (Type::Array, Type::Array) => true,
        (Type::Component, Type::Component) => true,
        (Type::Struct(a), Type::Struct(b)) => a == b,
        (Type::Enum(a), Type::Enum(b)) => a == b,
        (Type::EnumCase { enum_name, .. }, Type::Enum(target_name)) => enum_name == target_name,
        (
            Type::EnumCase {
                enum_name: a_enum,
                case_name: a_case,
                ..
            },
            Type::EnumCase {
                enum_name: b_enum,
                case_name: b_case,
                ..
            },
        ) => a_enum == b_enum && a_case == b_case,
        (Type::ObjectShape(fields), Type::ObjectShape(expected)) => {
            expected
                .iter()
                .all(|(name, expected_field)| match fields.get(name) {
                    Some(actual_field) => {
                        if actual_field.optional && !expected_field.optional {
                            return false;
                        }
                        is_assignable_base(&actual_field.ty, &expected_field.ty)
                    }
                    None => expected_field.optional,
                })
        }
        (
            Type::Applied {
                base: base_a,
                args: args_a,
            },
            Type::Applied {
                base: base_b,
                args: args_b,
            },
        ) => {
            base_a.eq_ignore_ascii_case(base_b)
                && args_a.len() == args_b.len()
                && args_a
                    .iter()
                    .zip(args_b.iter())
                    .all(|(a, b)| is_assignable_base(a, b))
        }
        (Type::Array, Type::Applied { base, .. }) if base.eq_ignore_ascii_case("array") => true,
        (Type::Applied { base, .. }, Type::Array) if base.eq_ignore_ascii_case("array") => true,
        (Type::ObjectShape(_), Type::Object)
        | (Type::Struct(_), Type::Object)
        | (Type::Enum(_), Type::Object)
        | (Type::EnumCase { .. }, Type::Object)
        | (Type::Component, Type::Object)
        | (Type::Object, Type::Object)
        | (Type::Interface(_), Type::Object) => true,
        _ => false,
    }
}
