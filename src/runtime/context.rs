use crate::compiler::chunk::UserFunc;
use crate::core::interner::Interner;
use crate::core::value::{Handle, Symbol, Val, Visibility};
use crate::runtime::extension::Extension;
use crate::runtime::registry::ExtensionRegistry;
use crate::runtime::resource_manager::ResourceManager;
use crate::vm::engine::VM;
use indexmap::IndexMap;
#[cfg(unix)]
use libc;
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub type NativeHandler = fn(&mut VM, args: &[Handle]) -> Result<Handle, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProfile {
    Server,
    Adwa,
}

impl HostProfile {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "adwa" => Self::Adwa,
            _ => Self::Server,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Adwa => "adwa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCapability {
    Fs,
    Net,
    ProcessEnv,
    ClockRandom,
    Db,
    WasmImports,
}

impl HostCapability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fs => "fs",
            Self::Net => "net",
            Self::ProcessEnv => "process_env",
            Self::ClockRandom => "clock_random",
            Self::Db => "db",
            Self::WasmImports => "wasm_imports",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HostCapabilities {
    pub fs: bool,
    pub net: bool,
    pub process_env: bool,
    pub clock_random: bool,
    pub db: bool,
    pub wasm_imports: bool,
}

impl HostCapabilities {
    pub fn for_profile(profile: HostProfile) -> Self {
        match profile {
            HostProfile::Server => Self {
                fs: true,
                net: true,
                process_env: true,
                clock_random: true,
                db: true,
                wasm_imports: true,
            },
            HostProfile::Adwa => Self {
                fs: true,
                net: true,
                process_env: false,
                clock_random: true,
                db: false,
                wasm_imports: true,
            },
        }
    }

    pub fn allows(&self, cap: HostCapability) -> bool {
        match cap {
            HostCapability::Fs => self.fs,
            HostCapability::Net => self.net,
            HostCapability::ProcessEnv => self.process_env,
            HostCapability::ClockRandom => self.clock_random,
            HostCapability::Db => self.db,
            HostCapability::WasmImports => self.wasm_imports,
        }
    }

    fn apply_disable_list(&mut self, disable_csv: &str) {
        for raw in disable_csv.split(',') {
            match raw.trim().to_ascii_lowercase().as_str() {
                "fs" => self.fs = false,
                "net" => self.net = false,
                "process" | "process_env" | "env" => self.process_env = false,
                "clock" | "random" | "clock_random" => self.clock_random = false,
                "db" => self.db = false,
                "wasm" | "wasm_imports" => self.wasm_imports = false,
                _ => {}
            }
        }
    }
}

/// PHP configuration settings
#[derive(Debug, Clone)]
pub struct PhpConfig {
    /// Error reporting level (E_ALL = 32767)
    pub error_reporting: u32,
    /// Maximum script execution time in seconds
    pub max_execution_time: i64,
    /// Default timezone for date/time functions
    pub timezone: String,
    /// Working directory for script execution
    pub working_dir: Option<PathBuf>,
    /// Runtime host profile (`server` or `adwa`)
    pub host_profile: HostProfile,
    /// Capability matrix derived from the host profile
    pub host_capabilities: HostCapabilities,
}

impl Default for PhpConfig {
    fn default() -> Self {
        let host_profile = std::env::var("DEKA_HOST_PROFILE")
            .ok()
            .map(|value| HostProfile::parse(&value))
            .unwrap_or(HostProfile::Server);
        let mut host_capabilities = HostCapabilities::for_profile(host_profile);
        if let Ok(disable_csv) = std::env::var("DEKA_HOST_DISABLE_CAPS") {
            host_capabilities.apply_disable_list(&disable_csv);
        }
        Self {
            error_reporting: 32767, // E_ALL
            max_execution_time: 30,
            timezone: "UTC".to_string(),
            working_dir: None,
            host_profile,
            host_capabilities,
        }
    }
}

impl PhpConfig {
    pub fn capability_for_bridge_kind(kind: &str) -> Option<HostCapability> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "fs" => Some(HostCapability::Fs),
            "net" | "tcp" | "tls" => Some(HostCapability::Net),
            "process" | "env" => Some(HostCapability::ProcessEnv),
            "clock" | "random" => Some(HostCapability::ClockRandom),
            "db" | "postgres" | "mysql" | "sqlite" => Some(HostCapability::Db),
            "wasm" | "component" => Some(HostCapability::WasmImports),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeHint {
    Int,
    Float,
    String,
    Bool,
    Array,
    Object,
    Callable,
    Iterable,
    Mixed,
    Void,
    Never,
    Null,
    Class(Symbol),
    Union(Vec<TypeHint>),
    Intersection(Vec<TypeHint>),
}

#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: Symbol,
    pub type_hint: Option<TypeHint>,
    pub is_reference: bool,
    pub is_variadic: bool,
    pub default_value: Option<Val>,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct MethodSignature {
    pub parameters: Vec<ParameterInfo>,
    pub return_type: Option<TypeHint>,
}

#[derive(Debug, Clone)]
pub struct MethodEntry {
    pub name: Symbol,
    pub func: Rc<UserFunc>,
    pub visibility: Visibility,
    pub is_static: bool,
    pub declaring_class: Symbol,
    pub is_abstract: bool,
    pub signature: MethodSignature,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct AttributeArg {
    pub name: Option<Symbol>,
    pub value: Val,
}

#[derive(Debug, Clone)]
pub struct AttributeInstance {
    pub name: Symbol,
    pub args: Vec<AttributeArg>,
}

#[derive(Debug, Clone)]
pub struct NativeMethodEntry {
    pub name: Symbol,
    pub handler: NativeHandler,
    pub visibility: Visibility,
    pub is_static: bool,
    pub declaring_class: Symbol,
}

#[derive(Debug, Clone)]
pub struct PropertyEntry {
    pub default_value: Val,
    pub visibility: Visibility,
    pub type_hint: Option<TypeHint>,
    pub is_readonly: bool,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct StaticPropertyEntry {
    pub value: Val,
    pub visibility: Visibility,
    pub type_hint: Option<TypeHint>,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct ClassConstEntry {
    pub value: Val,
    pub visibility: Visibility,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct ClassDef {
    pub name: Symbol,
    pub parent: Option<Symbol>,
    pub is_interface: bool,
    pub is_trait: bool,
    pub is_abstract: bool,
    pub is_enum: bool,
    pub is_struct: bool,
    pub enum_backed_type: Option<EnumBackedType>,
    pub interfaces: Vec<Symbol>,
    pub traits: Vec<Symbol>,
    pub embeds: Vec<Symbol>,
    pub methods: HashMap<Symbol, MethodEntry>,
    pub properties: IndexMap<Symbol, PropertyEntry>, // Instance properties with type hints
    pub constants: HashMap<Symbol, ClassConstEntry>,
    pub static_properties: HashMap<Symbol, StaticPropertyEntry>, // Static properties with type hints
    pub abstract_methods: HashSet<Symbol>,
    pub allows_dynamic_properties: bool, // Set by #[AllowDynamicProperties] attribute
    pub attributes: Vec<AttributeInstance>,
    pub enum_cases: Vec<EnumCaseDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumBackedType {
    Int,
    String,
}

#[derive(Debug, Clone)]
pub struct EnumCaseDef {
    pub name: Symbol,
    pub value: Option<Val>,
    pub handle: Handle,
    pub payload_params: Vec<Symbol>,
}

#[derive(Debug, Clone)]
pub struct HeaderEntry {
    pub key: Option<Vec<u8>>, // Normalized lowercase header name
    pub line: Vec<u8>,        // Original header line bytes
}

#[derive(Debug, Clone)]
pub struct ErrorInfo {
    pub error_type: i64,
    pub message: String,
    pub file: String,
    pub line: i64,
}

pub struct EngineContext {
    pub registry: ExtensionRegistry,
}

impl EngineContext {
    pub fn new() -> Self {
        let mut registry = ExtensionRegistry::new();

        // Register Core extension (MUST BE FIRST - contains all built-in functions)
        use crate::runtime::core_extension::CoreExtension;
        registry
            .register_extension(Box::new(CoreExtension))
            .expect("Failed to register Core extension");

        use crate::runtime::date_extension::DateExtension;
        registry
            .register_extension(Box::new(DateExtension))
            .expect("Failed to register Date extension");

        // Register Hash extension
        use crate::runtime::hash_extension::HashExtension;
        registry
            .register_extension(Box::new(HashExtension))
            .expect("Failed to register Hash extension");

        // Register JSON extension
        use crate::runtime::json_extension::JsonExtension;
        registry
            .register_extension(Box::new(JsonExtension))
            .expect("Failed to register JSON extension");

        // Register MySQLi extension
        #[cfg(feature = "mysqli")]
        {
            use crate::runtime::mysqli_extension::MysqliExtension;
            registry
                .register_extension(Box::new(MysqliExtension))
                .expect("Failed to register MySQLi extension");
        }

        // Register PDO extension
        #[cfg(feature = "pdo")]
        {
            use crate::runtime::pdo_extension::PdoExtension;
            registry
                .register_extension(Box::new(PdoExtension))
                .expect("Failed to register PDO extension");
        }

        // Register Zlib extension
        use crate::runtime::zlib_extension::ZlibExtension;
        registry
            .register_extension(Box::new(ZlibExtension))
            .expect("Failed to register Zlib extension");

        // Register MBString extension
        use crate::runtime::mb_extension::MbStringExtension;
        registry
            .register_extension(Box::new(MbStringExtension))
            .expect("Failed to register mbstring extension");

        // Register Zip extension
        #[cfg(feature = "zip")]
        {
            use crate::runtime::zip_extension::ZipExtension;
            registry
                .register_extension(Box::new(ZipExtension))
                .expect("Failed to register Zip extension");
        }

        // Register OpenSSL extension
        #[cfg(feature = "openssl")]
        {
            use crate::runtime::openssl_extension::OpenSSLExtension;
            registry
                .register_extension(Box::new(OpenSSLExtension))
                .expect("Failed to register OpenSSL extension");
        }

        Self { registry }
    }
}

pub struct RequestContext {
    pub engine: Arc<EngineContext>,
    pub config: PhpConfig,
    pub globals: HashMap<Symbol, Handle>,
    pub constants: HashMap<Symbol, Val>,
    pub user_functions: HashMap<Symbol, Rc<UserFunc>>,
    pub classes: HashMap<Symbol, ClassDef>,
    pub class_aliases: HashMap<Symbol, Symbol>,
    pub included_files: HashSet<String>,
    pub autoloaders: Vec<Handle>,
    pub interner: Interner,
    pub last_error: Option<ErrorInfo>,
    pub headers: Vec<HeaderEntry>,
    pub http_status: Option<i64>,
    pub native_methods: HashMap<(Symbol, Symbol), NativeMethodEntry>,
    pub next_resource_id: u64,
    /// Generic extension data storage keyed by TypeId
    pub extension_data: HashMap<TypeId, Box<dyn Any>>,
    /// Unified resource manager for type-safe resource handling
    pub resource_manager: ResourceManager,
}

impl RequestContext {
    pub fn new(engine: Arc<EngineContext>) -> Self {
        Self::with_config(engine, PhpConfig::default())
    }

    pub fn with_config(engine: Arc<EngineContext>, config: PhpConfig) -> Self {
        let mut ctx = Self {
            engine: Arc::clone(&engine),
            config,
            globals: HashMap::new(),
            constants: HashMap::new(),
            user_functions: HashMap::new(),
            classes: HashMap::new(),
            class_aliases: HashMap::new(),
            included_files: HashSet::new(),
            autoloaders: Vec::new(),
            interner: Interner::new(),
            last_error: None,
            headers: Vec::new(),
            http_status: None,
            native_methods: HashMap::new(),
            next_resource_id: 1,
            extension_data: HashMap::new(),
            resource_manager: ResourceManager::new(),
        };

        #[cfg(unix)]
        {
            // Match PHP defaults: only LC_CTYPE follows environment.
            if let Ok(c_locale) = CString::new("") {
                unsafe {
                    libc::setlocale(libc::LC_CTYPE, c_locale.as_ptr());
                }
            }
        }

        // Copy constants from extension registry in bulk
        ctx.copy_engine_constants();

        // Materialize classes from extensions
        ctx.materialize_extension_classes();

        // Call RINIT for all extensions
        engine.registry.invoke_request_init(&mut ctx).ok();

        ctx
    }

    /// Copy constants from engine registry in bulk
    ///
    /// Two-phase constant initialization:
    /// 1. Copy extension-provided constants from engine registry (bulk operation)
    /// 2. Register core PHP constants (version info, error levels, system constants)
    ///
    /// This split is necessary because:
    /// - Extension constants are registered during MINIT at engine startup
    /// - Core PHP constants (PHP_VERSION, E_ERROR, etc.) must exist in every request
    /// - Bulk copy from registry is O(n), avoiding individual re-insertion overhead
    ///
    /// Performance: O(n) where n = number of engine constants
    fn copy_engine_constants(&mut self) {
        // Phase 1: Copy all extension constants (O(n) bulk operation)
        for (name, val) in self.engine.registry.constants() {
            let sym = self.interner.intern(name);
            self.constants.insert(sym, val.clone());
        }

        // Phase 2: Register fundamental PHP constants
        self.register_builtin_constants();
    }

    fn materialize_extension_classes(&mut self) {
        let native_classes: Vec<_> = self.engine.registry.classes().values().cloned().collect();
        for native_class in native_classes {
            let class_sym = self.interner.intern(&native_class.name);
            let parent_sym = native_class
                .parent
                .as_ref()
                .map(|p| self.interner.intern(p));
            let mut interfaces = Vec::new();
            for iface in &native_class.interfaces {
                interfaces.push(self.interner.intern(iface));
            }

            let mut constants = HashMap::new();
            for (name, (val, visibility)) in &native_class.constants {
                constants.insert(
                    self.interner.intern(name),
                    ClassConstEntry {
                        value: val.clone(),
                        visibility: *visibility,
                        attributes: Vec::new(),
                    },
                );
            }

            self.classes.insert(
                class_sym,
                ClassDef {
                    name: class_sym,
                    parent: parent_sym,
                    is_interface: native_class.is_interface,
                    is_trait: native_class.is_trait,
                    is_abstract: false,
                    is_enum: false,
                    is_struct: false,
                    enum_backed_type: None,
                    interfaces,
                    traits: Vec::new(),
                    embeds: Vec::new(),
                    methods: HashMap::new(),
                    properties: IndexMap::new(),
                    constants,
                    static_properties: HashMap::new(),
                    abstract_methods: HashSet::new(),
                    allows_dynamic_properties: true,
                    attributes: Vec::new(),
                    enum_cases: Vec::new(),
                },
            );

            for (name, native_method) in &native_class.methods {
                let method_sym = self.interner.intern(name);
                self.native_methods.insert(
                    (class_sym, method_sym),
                    NativeMethodEntry {
                        name: method_sym,
                        handler: native_method.handler,
                        visibility: native_method.visibility,
                        is_static: native_method.is_static,
                        declaring_class: class_sym,
                    },
                );
            }
        }
    }

    /// Get immutable reference to extension-specific data
    ///
    /// # Type Safety
    /// Each extension should use its own unique type (e.g., JsonExtensionData)
    /// to avoid collisions in the TypeId-based storage.
    ///
    /// # Returns
    /// - `Some(&T)` if data of type T exists
    /// - `None` if no data of type T has been stored
    pub fn get_extension_data<T: 'static>(&self) -> Option<&T> {
        self.extension_data
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Get mutable reference to extension-specific data
    ///
    /// # Type Safety
    /// Each extension should use its own unique type (e.g., JsonExtensionData)
    /// to avoid collisions in the TypeId-based storage.
    ///
    /// # Returns
    /// - `Some(&mut T)` if data of type T exists
    /// - `None` if no data of type T has been stored
    pub fn get_extension_data_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.extension_data
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut::<T>())
    }

    /// Store extension-specific data
    ///
    /// # Type Safety
    /// Each extension should use its own unique type (e.g., JsonExtensionData)
    /// to avoid collisions in the TypeId-based storage.
    ///
    /// # Example
    /// ```rust,ignore
    /// struct MyExtensionData {
    ///     counter: u32,
    /// }
    /// ctx.set_extension_data(MyExtensionData { counter: 0 });
    /// ```
    pub fn set_extension_data<T: 'static>(&mut self, data: T) {
        self.extension_data
            .insert(TypeId::of::<T>(), Box::new(data));
    }

    /// Get or initialize extension-specific data
    ///
    /// If data of type T does not exist, initialize it using the provided closure.
    /// Returns a mutable reference to the data (existing or newly initialized).
    ///
    /// # Type Safety
    /// Each extension should use its own unique type (e.g., JsonExtensionData)
    /// to avoid collisions in the TypeId-based storage.
    ///
    /// # Example
    /// ```rust,ignore
    /// let data = ctx.get_or_init_extension_data(|| MyExtensionData { counter: 0 });
    /// data.counter += 1;
    /// ```
    pub fn get_or_init_extension_data<T: 'static, F>(&mut self, init: F) -> &mut T
    where
        F: FnOnce() -> T,
    {
        self.extension_data
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(init()))
            .downcast_mut::<T>()
            .expect("TypeId mismatch in extension_data")
    }
}

#[cfg(test)]
mod tests {
    use super::{HostCapability, HostCapabilities, HostProfile, PhpConfig};

    #[test]
    fn host_profile_parse_defaults_to_server() {
        assert_eq!(HostProfile::parse("server"), HostProfile::Server);
        assert_eq!(HostProfile::parse("adwa"), HostProfile::Adwa);
        assert_eq!(HostProfile::parse("unknown"), HostProfile::Server);
    }

    #[test]
    fn host_capabilities_for_adwa_limits_db_and_env() {
        let caps = HostCapabilities::for_profile(HostProfile::Adwa);
        assert!(caps.allows(HostCapability::Fs));
        assert!(caps.allows(HostCapability::Net));
        assert!(!caps.allows(HostCapability::Db));
        assert!(!caps.allows(HostCapability::ProcessEnv));
    }

    #[test]
    fn bridge_kind_maps_to_capability() {
        assert_eq!(
            PhpConfig::capability_for_bridge_kind("db"),
            Some(HostCapability::Db)
        );
        assert_eq!(
            PhpConfig::capability_for_bridge_kind("tls"),
            Some(HostCapability::Net)
        );
        assert_eq!(
            PhpConfig::capability_for_bridge_kind("env"),
            Some(HostCapability::ProcessEnv)
        );
        assert_eq!(PhpConfig::capability_for_bridge_kind("unknown"), None);
    }
}

impl RequestContext {
    /// Register core PHP constants that are not provided by extensions
    ///
    /// This method only registers fundamental PHP constants that must exist
    /// in every request context. Extension-specific constants (output control,
    /// URL parsing, date formats, string functions, etc.) are registered by
    /// their respective extensions via ExtensionRegistry.
    ///
    /// Core constants registered here:
    /// - PHP version info (PHP_VERSION, PHP_VERSION_ID, etc.)
    /// - System constants (PHP_OS, PHP_SAPI, PHP_EOL)
    /// - Path separators (DIRECTORY_SEPARATOR, PATH_SEPARATOR)
    /// - Error reporting levels (E_ERROR, E_WARNING, etc.)
    fn register_builtin_constants(&mut self) {
        // PHP version constants
        const PHP_VERSION_STR: &str = "8.5.1";
        const PHP_VERSION_ID_VALUE: i64 = 80501;
        const PHP_MAJOR: i64 = 8;
        const PHP_MINOR: i64 = 5;
        const PHP_RELEASE: i64 = 1;

        self.insert_builtin_constant(
            b"PHP_VERSION",
            Val::String(Rc::new(PHP_VERSION_STR.as_bytes().to_vec())),
        );
        self.insert_builtin_constant(b"PHP_VERSION_ID", Val::Int(PHP_VERSION_ID_VALUE));
        self.insert_builtin_constant(b"PHP_MAJOR_VERSION", Val::Int(PHP_MAJOR));
        self.insert_builtin_constant(b"PHP_MINOR_VERSION", Val::Int(PHP_MINOR));
        self.insert_builtin_constant(b"PHP_RELEASE_VERSION", Val::Int(PHP_RELEASE));
        self.insert_builtin_constant(b"PHP_EXTRA_VERSION", Val::String(Rc::new(Vec::new())));

        // System constants
        self.insert_builtin_constant(b"PHP_OS", Val::String(Rc::new(b"Darwin".to_vec())));
        self.insert_builtin_constant(b"PHP_SAPI", Val::String(Rc::new(b"cli".to_vec())));
        self.insert_builtin_constant(b"PHP_EOL", Val::String(Rc::new(b"\n".to_vec())));
        self.insert_builtin_constant(b"INF", Val::Float(f64::INFINITY));
        self.insert_builtin_constant(b"NAN", Val::Float(f64::NAN));
        self.insert_builtin_constant(b"M_E", Val::Float(std::f64::consts::E));
        self.insert_builtin_constant(b"M_PI", Val::Float(std::f64::consts::PI));

        // Array case constants
        self.insert_builtin_constant(b"CASE_LOWER", Val::Int(0));
        self.insert_builtin_constant(b"CASE_UPPER", Val::Int(1));

        // Path separator constants
        let dir_sep = std::path::MAIN_SEPARATOR.to_string().into_bytes();
        self.insert_builtin_constant(b"DIRECTORY_SEPARATOR", Val::String(Rc::new(dir_sep)));

        let path_sep_byte = if cfg!(windows) { b';' } else { b':' };
        self.insert_builtin_constant(b"PATH_SEPARATOR", Val::String(Rc::new(vec![path_sep_byte])));

        // Error reporting level constants
        self.insert_builtin_constant(b"E_ERROR", Val::Int(1));
        self.insert_builtin_constant(b"E_WARNING", Val::Int(2));
        self.insert_builtin_constant(b"E_PARSE", Val::Int(4));
        self.insert_builtin_constant(b"E_NOTICE", Val::Int(8));
        self.insert_builtin_constant(b"E_CORE_ERROR", Val::Int(16));
        self.insert_builtin_constant(b"E_CORE_WARNING", Val::Int(32));
        self.insert_builtin_constant(b"E_COMPILE_ERROR", Val::Int(64));
        self.insert_builtin_constant(b"E_COMPILE_WARNING", Val::Int(128));
        self.insert_builtin_constant(b"E_USER_ERROR", Val::Int(256));
        self.insert_builtin_constant(b"E_USER_WARNING", Val::Int(512));
        self.insert_builtin_constant(b"E_USER_NOTICE", Val::Int(1024));
        self.insert_builtin_constant(b"E_STRICT", Val::Int(2048));
        self.insert_builtin_constant(b"E_RECOVERABLE_ERROR", Val::Int(4096));
        self.insert_builtin_constant(b"E_DEPRECATED", Val::Int(8192));
        self.insert_builtin_constant(b"E_USER_DEPRECATED", Val::Int(16384));
        self.insert_builtin_constant(b"E_ALL", Val::Int(32767));
    }

    pub fn insert_builtin_constant(&mut self, name: &[u8], value: Val) {
        let sym = self.interner.intern(name);
        self.constants.insert(sym, value);
    }
}

/// Builder for constructing EngineContext with extensions
///
/// # Example
/// ```ignore
/// let engine = EngineBuilder::new()
///     .with_core_extensions()
///     .build()?;
/// ```
pub struct EngineBuilder {
    extensions: Vec<Box<dyn Extension>>,
}

impl EngineBuilder {
    /// Create a new empty builder
    pub fn new() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    /// Add an extension to the builder
    pub fn with_extension<E: Extension + 'static>(mut self, ext: E) -> Self {
        self.extensions.push(Box::new(ext));
        self
    }

    /// Add core extensions (standard builtins)
    ///
    /// This includes all core PHP functionality: core functions, classes, interfaces,
    /// exceptions, and the date/time extension.
    pub fn with_core_extensions(mut self) -> Self {
        self.extensions
            .push(Box::new(super::core_extension::CoreExtension));
        self.extensions
            .push(Box::new(super::date_extension::DateExtension));
        self.extensions
            .push(Box::new(super::hash_extension::HashExtension));
        #[cfg(feature = "mysqli")]
        self.extensions
            .push(Box::new(super::mysqli_extension::MysqliExtension));
        self.extensions
            .push(Box::new(super::json_extension::JsonExtension));
        #[cfg(feature = "openssl")]
        self.extensions
            .push(Box::new(super::openssl_extension::OpenSSLExtension));
        #[cfg(feature = "pdo")]
        self.extensions
            .push(Box::new(super::pdo_extension::PdoExtension));
        #[cfg(feature = "pthreads")]
        self.extensions
            .push(Box::new(super::pthreads_extension::PthreadsExtension));
        self.extensions
            .push(Box::new(super::zlib_extension::ZlibExtension));
        self.extensions
            .push(Box::new(super::mb_extension::MbStringExtension));
        self
    }

    /// Build the EngineContext
    ///
    /// This will:
    /// 1. Create an empty registry
    /// 2. Register all extensions (calling MINIT for each)
    /// 3. Return the configured EngineContext
    pub fn build(self) -> Result<Arc<EngineContext>, String> {
        let mut registry = ExtensionRegistry::new();

        // Register all extensions
        for ext in self.extensions {
            registry.register_extension(ext)?;
        }

        Ok(Arc::new(EngineContext { registry }))
    }
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Call RSHUTDOWN for all extensions when request ends
impl Drop for RequestContext {
    fn drop(&mut self) {
        // Call RSHUTDOWN for all extensions in reverse order (LIFO)
        // Clone Arc to separate lifetimes and avoid borrow checker conflict
        let engine = Arc::clone(&self.engine);
        engine.registry.request_shutdown_all(self);
    }
}

/// Call MSHUTDOWN for all extensions when engine shuts down
impl Drop for EngineContext {
    fn drop(&mut self) {
        // Call MSHUTDOWN for all extensions in reverse order (LIFO)
        self.registry.module_shutdown_all();
    }
}
