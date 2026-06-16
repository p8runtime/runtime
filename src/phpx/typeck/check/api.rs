use super::*;

impl TypeError {
    pub fn to_human_readable(&self, source: &[u8]) -> String {
        let Some(info) = self.span.line_info(source) else {
            return format!("type error: {}", self.message);
        };
        let line_str = String::from_utf8_lossy(info.line_text);
        let gutter_width = info.line.to_string().len();
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let highlight_len = std::cmp::max(
            1,
            std::cmp::min(
                self.span.len(),
                info.line_text.len().saturating_sub(padding),
            ),
        );

        let mut marker = String::new();
        marker.push_str(&" ".repeat(padding));
        marker.push_str(&"^".repeat(highlight_len));

        format!(
            "type error: {}\n --> line {}, column {}\n{gutter}|\n{line_no:>width$} | {line_src}\n{gutter}| {marker}",
            self.message,
            info.line,
            info.column,
            gutter = " ".repeat(gutter_width + 1),
            line_no = info.line,
            width = gutter_width,
            line_src = line_str,
            marker = marker,
        )
    }
}

pub fn check_program(program: &Program, source: &[u8]) -> Result<(), Vec<TypeError>> {
    check_program_with_path(program, source, None)
}

pub fn check_program_with_path(
    program: &Program,
    source: &[u8],
    file_path: Option<&Path>,
) -> Result<(), Vec<TypeError>> {
    let mut ctx = CheckContext::new(source, file_path);
    ctx.check_program(program);

    if ctx.errors.is_empty() {
        Ok(())
    } else {
        Err(ctx.errors)
    }
}

pub fn check_program_with_path_and_externals(
    program: &Program,
    source: &[u8],
    file_path: Option<&Path>,
    externals: &HashMap<String, ExternalFunctionSig>,
) -> Result<(), Vec<TypeError>> {
    let mut ctx = CheckContext::new_with_externals(source, file_path, externals);
    ctx.check_program(program);

    if ctx.errors.is_empty() {
        Ok(())
    } else {
        Err(ctx.errors)
    }
}

pub fn summarize_program_with_path(
    program: &Program,
    source: &[u8],
    file_path: Option<&Path>,
) -> Result<TypeckProgramSummary, Vec<TypeError>> {
    let mut ctx = CheckContext::new(source, file_path);
    ctx.check_program(program);

    if !ctx.errors.is_empty() {
        return Err(ctx.errors);
    }

    let functions = ctx
        .functions
        .into_iter()
        .map(|(name, sig)| {
            (
                name,
                TypeckFunctionInfo {
                    params: sig
                        .params
                        .into_iter()
                        .map(|param| TypeckParamInfo {
                            ty: param.ty,
                            required: param.required,
                        })
                        .collect(),
                    return_type: sig.return_type,
                    variadic: sig.variadic,
                },
            )
        })
        .collect();

    Ok(TypeckProgramSummary {
        structs: ctx.structs,
        enums: ctx.enums,
        functions,
    })
}

pub fn format_type_errors(errors: &[TypeError], source: &[u8]) -> String {
    let mut out = String::new();
    for (idx, err) in errors.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&err.to_human_readable(source));
    }
    out
}

pub fn external_functions_from_stub(
    program: &Program,
    source: &[u8],
) -> HashMap<String, ExternalFunctionSig> {
    let mut ctx = CheckContext::new(source, None);
    ctx.collect_struct_names(program);
    ctx.collect_interface_names(program);
    ctx.collect_enum_names(program);
    ctx.collect_type_aliases(program);
    ctx.collect_struct_fields(program);
    ctx.collect_interface_methods(program);
    ctx.collect_struct_methods(program);
    ctx.collect_enum_methods(program);
    ctx.collect_enum_cases(program);

    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        let Stmt::Function {
            name,
            type_params,
            params,
            return_type,
            ..
        } = stmt
        else {
            continue;
        };
        let (type_param_sigs, type_param_set) = ctx.collect_type_param_sigs(type_params);
        let mut external_params = Vec::new();
        let mut variadic = false;
        for param in params.iter() {
            if param.variadic {
                variadic = true;
            }
            let required = param.default.is_none() && !param.variadic;
            let ty = param
                .ty
                .map(|ty| ctx.resolve_type_with_params(ty, &type_param_set));
            external_params.push(ExternalParamSig { ty, required });
        }
        let return_ty = return_type.map(|ty| ctx.resolve_type_with_params(ty, &type_param_set));
        let type_params = type_param_sigs
            .iter()
            .map(|sig| ExternalTypeParamSig {
                name: sig.name.clone(),
                constraint: sig.constraint.clone(),
            })
            .collect();
        let fn_name = token_text(source, name.span);
        out.insert(
            fn_name,
            ExternalFunctionSig {
                type_params,
                params: external_params,
                return_type: return_ty,
                variadic,
            },
        );
    }

    out
}

impl ExternalFunctionSig {
    pub(in crate::phpx::typeck::check) fn to_internal(&self) -> FunctionSig {
        FunctionSig {
            type_params: self
                .type_params
                .iter()
                .map(|param| TypeParamSig {
                    name: param.name.clone(),
                    constraint: param.constraint.clone(),
                })
                .collect(),
            params: self
                .params
                .iter()
                .map(|param| ParamSig {
                    ty: param.ty.clone(),
                    required: param.required,
                })
                .collect(),
            return_type: self.return_type.clone(),
            variadic: self.variadic,
        }
    }
}
