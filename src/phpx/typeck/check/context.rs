use super::*;

pub(in crate::phpx::typeck::check) struct CheckContext<'a> {
    #[allow(dead_code)]
    pub(in crate::phpx::typeck::check) source: &'a [u8],
    pub(in crate::phpx::typeck::check) file_path: Option<PathBuf>,
    pub(in crate::phpx::typeck::check) errors: Vec<TypeError>,
    pub(in crate::phpx::typeck::check) structs: HashMap<String, StructInfo>,
    pub(in crate::phpx::typeck::check) struct_methods: HashMap<String, HashMap<String, MethodSig>>,
    pub(in crate::phpx::typeck::check) enums: HashMap<String, EnumInfo>,
    pub(in crate::phpx::typeck::check) enum_methods: HashMap<String, HashMap<String, MethodSig>>,
    pub(in crate::phpx::typeck::check) interfaces: HashMap<String, InterfaceInfo>,
    pub(in crate::phpx::typeck::check) traits: HashMap<String, TraitInfo>,
    pub(in crate::phpx::typeck::check) impls: HashMap<String, Vec<ImplRecord>>,
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
            enums: HashMap::new(),
            enum_methods: HashMap::new(),
            interfaces: HashMap::new(),
            traits: HashMap::new(),
            impls: HashMap::new(),
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

    pub(in crate::phpx::typeck::check) fn check_program(&mut self, program: &Program<'a>) {
        self.check_wasm_stubs();
        self.collect_imported_names();
        self.collect_struct_names(program);
        self.collect_interface_names(program);
        self.collect_enum_names(program);
        self.collect_type_aliases(program);
        self.collect_struct_fields(program);
        self.collect_interface_methods(program);
        self.collect_trait_methods(program);
        self.collect_struct_methods(program);
        self.collect_enum_methods(program);
        self.collect_enum_cases(program);
        self.collect_functions(program);
        let mut env: HashMap<String, Type> = HashMap::new();
        let mut explicit: HashSet<String> = HashSet::new();
        for stmt in program.statements.iter() {
            self.check_stmt(stmt, &mut env, &mut explicit, None);
        }
        self.check_trait_conflicts();
    }
}
