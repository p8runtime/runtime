use super::*;

pub(in crate::phpx::typeck::check) struct CheckContext<'a> {
    #[allow(dead_code)]
    pub(in crate::phpx::typeck::check) source: &'a [u8],
    pub(in crate::phpx::typeck::check) file_path: Option<PathBuf>,
    pub(in crate::phpx::typeck::check) errors: Vec<TypeError>,
    pub(in crate::phpx::typeck::check) structs: HashMap<String, StructInfo>,
    pub(in crate::phpx::typeck::check) struct_methods: HashMap<String, HashMap<String, MethodSig>>,
    /// Methods declared directly on each struct (body methods + receiver methods),
    /// used to distinguish own methods from promoted embedded methods.
    pub(in crate::phpx::typeck::check) own_struct_methods: HashMap<String, HashSet<String>>,
    /// Promoted method names that are ambiguous because two or more embedded
    /// structs provide the same name. A direct struct override is required.
    pub(in crate::phpx::typeck::check) ambiguous_promoted_methods: HashMap<String, HashSet<String>>,
    pub(in crate::phpx::typeck::check) enums: HashMap<String, EnumInfo>,
    pub(in crate::phpx::typeck::check) enum_methods: HashMap<String, HashMap<String, MethodSig>>,
    pub(in crate::phpx::typeck::check) interfaces: HashMap<String, InterfaceInfo>,
    pub(in crate::phpx::typeck::check) interface_shapes:
        HashMap<String, BTreeMap<String, ObjectField>>,
    pub(in crate::phpx::typeck::check) functions: HashMap<String, FunctionSig>,
    pub(in crate::phpx::typeck::check) function_returns: HashMap<String, Type>,
    pub(in crate::phpx::typeck::check) imported: HashMap<String, String>,
    pub(in crate::phpx::typeck::check) type_aliases: HashMap<String, TypeAliasInfo>,
    pub(in crate::phpx::typeck::check) resolved_aliases: HashMap<String, Type>,
    pub(in crate::phpx::typeck::check) fn_depth: usize,
    pub(in crate::phpx::typeck::check) async_depth: usize,
    pub(in crate::phpx::typeck::check) strict_null: bool,
}

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn new(source: &'a [u8], file_path: Option<&Path>) -> Self {
        Self {
            source,
            file_path: file_path.map(|path| path.to_path_buf()),
            errors: Vec::new(),
            structs: HashMap::new(),
            struct_methods: HashMap::new(),
            own_struct_methods: HashMap::new(),
            ambiguous_promoted_methods: HashMap::new(),
            enums: HashMap::new(),
            enum_methods: HashMap::new(),
            interfaces: HashMap::new(),
            interface_shapes: HashMap::new(),
            functions: HashMap::new(),
            function_returns: HashMap::new(),
            imported: HashMap::new(),
            type_aliases: HashMap::new(),
            resolved_aliases: HashMap::new(),
            fn_depth: 0,
            async_depth: 0,
            strict_null: std::env::var("PHPX_STRICT_NULL")
                .map(|value| {
                    let value = value.trim().to_ascii_lowercase();
                    value == "1" || value == "true" || value == "yes" || value == "on"
                })
                .unwrap_or(false),
        }
    }

    pub(in crate::phpx::typeck::check) fn new_with_externals(
        source: &'a [u8],
        file_path: Option<&Path>,
        externals: &HashMap<String, ExternalFunctionSig>,
    ) -> Self {
        let mut ctx = Self::new(source, file_path);
        for (name, sig) in externals {
            ctx.functions.insert(name.clone(), sig.to_internal());
        }
        ctx
    }

    pub(in crate::phpx::typeck::check) fn seed_imports(
        &mut self,
        program: &Program<'a>,
        imports: &HashMap<String, TypeckProgramSummary>,
    ) {
        for stmt in program.statements.iter() {
            let Stmt::Import { specs, from, .. } = stmt else {
                continue;
            };
            let module_path =
                String::from_utf8_lossy(&self.source[from.span.start..from.span.end]).to_string();
            let module_path = unquote_module_path(&module_path);
            let Some(summary) = imports.get(&module_path) else {
                self.errors.push(TypeError {
                    span: from.span,
                    message: format!("Module '{}' not found", module_path),
                    severity: Severity::Error,
                });
                continue;
            };
            for spec in specs.iter() {
                let remote = String::from_utf8_lossy(
                    &self.source[spec.remote.span.start..spec.remote.span.end],
                )
                .to_string();
                let local = String::from_utf8_lossy(
                    &self.source[spec.local.span.start..spec.local.span.end],
                )
                .to_string();
                if let Some(info) = summary.functions.get(&remote) {
                    self.functions.insert(
                        local.clone(),
                        FunctionSig {
                            type_params: Vec::new(),
                            params: info
                                .params
                                .iter()
                                .map(|p| ParamSig {
                                    ty: p.ty.clone(),
                                    required: p.required,
                                })
                                .collect(),
                            return_type: info.return_type.clone(),
                            variadic: info.variadic,
                        },
                    );
                    continue;
                }
                if summary.structs.contains_key(&remote) {
                    // Structs are not currently movable across module summaries
                    // because StructInfo carries private inference state. For
                    // Phase 1 we only validate function imports; type imports
                    // are accepted silently to avoid false positives.
                    continue;
                }
                if summary.enums.contains_key(&remote) {
                    continue;
                }
                if summary.type_aliases.contains_key(&remote) {
                    self.type_aliases.insert(
                        local.clone(),
                        TypeAliasInfo {
                            params: Vec::new(),
                            ty: summary.type_aliases[&remote].clone(),
                            span: spec.remote.span,
                        },
                    );
                    continue;
                }
                self.errors.push(TypeError {
                    span: spec.remote.span,
                    message: format!("'{}' is not exported by '{}'", remote, module_path),
                    severity: Severity::Error,
                });
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn check_program(&mut self, program: &Program<'a>) {
        self.check_wasm_stubs();
        self.collect_imported_names(program);
        self.collect_struct_names(program);
        self.collect_interface_names(program);
        self.collect_enum_names(program);
        self.collect_type_aliases(program);
        self.collect_struct_fields(program);
        self.collect_interface_methods(program);
        self.collect_struct_methods(program);
        self.collect_receiver_methods(program);
        self.promote_embedded_struct_methods();
        self.collect_enum_methods(program);
        self.collect_enum_cases(program);
        self.collect_functions(program);
        self.infer_function_return_types(program);
        let mut env: HashMap<String, Type> = HashMap::new();
        let mut explicit: HashSet<String> = HashSet::new();
        let mut mut_env: HashSet<String> = HashSet::new();
        for stmt in program.statements.iter() {
            self.check_stmt(stmt, &mut env, &mut explicit, None, &mut mut_env);
        }
    }
}

fn unquote_module_path(path: &str) -> String {
    let path = path.trim();
    if path.len() >= 2 {
        let bytes = path.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[bytes.len() - 1] as char;
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return path[1..path.len() - 1].to_string();
        }
    }
    path.to_string()
}
