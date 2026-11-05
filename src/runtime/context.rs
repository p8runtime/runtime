use crate::builtins::defaults::ConstValue;
use crate::compiler::chunk::UserFunc;
use crate::core::interner::Interner;
use crate::core::value::{Handle, Symbol, Val, Visibility};
use crate::runtime::extension::Extension;
use crate::runtime::registry::ExtensionRegistry;
use crate::runtime::resource_manager::ResourceManager;
use crate::vm::engine::VM;
use indexmap::IndexMap;
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub type NativeHandler = fn(&mut VM, args: &[Handle]) -> Result<Handle, String>;

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
}

impl Default for PhpConfig {
    fn default() -> Self {
        Self {
            error_reporting: 32767, // E_ALL
            max_execution_time: 30,
            timezone: "UTC".to_string(),
            working_dir: None,
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
    /// Order of declaration across the request (reflection lists methods in this order).
    pub decl_order: u64,
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
    /// Order of declaration across the request (shared with static properties).
    pub decl_order: u64,
    pub default_value: Val,
    pub visibility: Visibility,
    pub type_hint: Option<TypeHint>,
    pub is_readonly: bool,
    pub attributes: Vec<AttributeInstance>,
}

#[derive(Debug, Clone)]
pub struct StaticPropertyEntry {
    /// Order of declaration across the request (shared with instance properties).
    pub decl_order: u64,
    /// The property's slot in the VM heap, so references to it (`$r = &A::$p`) share it.
    pub handle: Handle,
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
    pub is_final: bool,
    pub is_enum: bool,
    pub enum_backed_type: Option<EnumBackedType>,
    pub interfaces: Vec<Symbol>,
    pub traits: Vec<Symbol>,
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

impl Default for EngineContext {
    fn default() -> Self {
        Self::new()
    }
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

        use crate::runtime::opcache_extension::OpcacheExtension;
        registry
            .register_extension(Box::new(OpcacheExtension))
            .expect("Failed to register Zend OPcache");

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
    /// Insertion-ordered, as PHP's symbol table is ($GLOBALS, get_defined_vars()).
    pub globals: indexmap::IndexMap<Symbol, Handle>,
    pub constants: HashMap<Symbol, Val>,
    /// Built-in constants PHP 8.4 marks #[\Deprecated]: reading one warns.
    pub deprecated_constants: HashSet<Symbol>,
    pub user_functions: HashMap<Symbol, Rc<UserFunc>>,
    pub classes: HashMap<Symbol, ClassDef>,
    /// Files compiled by include/require, in order (get_included_files()).
    pub included_files: indexmap::IndexSet<String>,
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
    /// setlocale() state (PHP 8 starts each request in the C locale).
    pub locale: crate::builtins::locale::LocaleState,
    /// putenv() changes, layered over the process environment (None = unset).
    /// The real environment is never modified: it is shared by every thread.
    pub env_overrides: HashMap<Vec<u8>, Option<Vec<u8>>>,
    /// php.ini directives (ini_get/ini_set).
    pub ini: crate::runtime::ini::IniSettings,
}

impl RequestContext {
    pub fn new(engine: Arc<EngineContext>) -> Self {
        Self::with_config(engine, PhpConfig::default())
    }

    pub fn with_config(engine: Arc<EngineContext>, config: PhpConfig) -> Self {
        let mut ctx = Self {
            engine: Arc::clone(&engine),
            config,
            globals: indexmap::IndexMap::new(),
            constants: HashMap::new(),
            deprecated_constants: HashSet::new(),
            user_functions: HashMap::new(),
            classes: HashMap::new(),
            included_files: indexmap::IndexSet::new(),
            autoloaders: Vec::new(),
            interner: Interner::new(),
            last_error: None,
            headers: Vec::new(),
            http_status: None,
            native_methods: HashMap::new(),
            next_resource_id: 1,
            extension_data: HashMap::new(),
            resource_manager: ResourceManager::new(),
            locale: Default::default(),
            env_overrides: HashMap::new(),
            ini: crate::runtime::ini::IniSettings::new(&engine.registry.get_extensions()),
        };

        // Float formatting follows this request's precision directives
        for name in ["precision", "serialize_precision"] {
            if let Some(value) = ctx.ini.get(name) {
                crate::core::value::FloatPrecision::set_ini(name, &value);
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
        // Phase 0: the constants real PHP defines in the loaded extensions
        let registry = &self.engine.registry;
        for (ext, name, value, deprecated) in crate::builtins::defaults::CONSTANTS {
            if !registry.extension_loaded(ext) {
                continue;
            }
            let value = match value {
                ConstValue::Int(i) => Val::Int(*i),
                ConstValue::Float(bits) => Val::Float(f64::from_bits(*bits)),
                ConstValue::Str(s) => Val::String(Rc::new(s.to_vec())),
                ConstValue::Bool(b) => Val::Bool(*b),
                ConstValue::Null => Val::Null,
            };
            let sym = self.interner.intern(name);
            self.constants.insert(sym, value);
            if deprecated.is_some() {
                self.deprecated_constants.insert(sym);
            }
        }

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
                    is_final: false,
                    is_enum: false,
                    enum_backed_type: None,
                    interfaces,
                    traits: Vec::new(),
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
        // The release whose php-src test suite this runtime is measured against.
        const PHP_VERSION_STR: &str = "8.4.27";
        const PHP_VERSION_ID_VALUE: i64 = 80427;
        const PHP_MAJOR: i64 = 8;
        const PHP_MINOR: i64 = 4;
        const PHP_RELEASE: i64 = 27;

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
        // The running binary, so scripts that spawn PHP_BINARY run this runtime
        let binary = std::env::current_exe()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.insert_builtin_constant(b"PHP_BINARY", Val::String(Rc::new(binary.into_bytes())));
        self.insert_builtin_constant(b"PHP_SAPI", Val::String(Rc::new(b"cli".to_vec())));
        self.insert_builtin_constant(b"PHP_EOL", Val::String(Rc::new(b"\n".to_vec())));
        let os_family: &[u8] = match std::env::consts::OS {
            "macos" => b"Darwin",
            "windows" => b"Windows",
            "linux" => b"Linux",
            "freebsd" | "openbsd" | "netbsd" | "dragonfly" => b"BSD",
            "solaris" | "illumos" => b"Solaris",
            _ => b"Unknown",
        };
        self.insert_builtin_constant(b"PHP_OS_FAMILY", Val::String(Rc::new(os_family.to_vec())));
        self.insert_builtin_constant(b"PHP_INT_MAX", Val::Int(i64::MAX));
        self.insert_builtin_constant(b"PHP_INT_MIN", Val::Int(i64::MIN));
        self.insert_builtin_constant(b"PHP_INT_SIZE", Val::Int(8));
        self.insert_builtin_constant(b"PHP_FLOAT_DIG", Val::Int(f64::DIGITS as i64));
        self.insert_builtin_constant(b"PHP_FLOAT_EPSILON", Val::Float(f64::EPSILON));
        self.insert_builtin_constant(b"PHP_FLOAT_MAX", Val::Float(f64::MAX));
        self.insert_builtin_constant(b"PHP_FLOAT_MIN", Val::Float(f64::MIN_POSITIVE));
        self.insert_builtin_constant(b"PHP_MAXPATHLEN", Val::Int(4096));
        self.insert_builtin_constant(b"PHP_FD_SETSIZE", Val::Int(1024));
        self.insert_builtin_constant(b"PHP_ZTS", Val::Bool(false));
        self.insert_builtin_constant(b"PHP_DEBUG", Val::Bool(false));
        self.insert_builtin_constant(b"ZEND_THREAD_SAFE", Val::Bool(false));
        self.insert_builtin_constant(b"ZEND_DEBUG_BUILD", Val::Bool(false));
        self.insert_builtin_constant(b"DEBUG_BACKTRACE_PROVIDE_OBJECT", Val::Int(1));
        self.insert_builtin_constant(b"DEBUG_BACKTRACE_IGNORE_ARGS", Val::Int(2));
        for (name, code) in [
            (&b"UPLOAD_ERR_OK"[..], 0),
            (b"UPLOAD_ERR_INI_SIZE", 1),
            (b"UPLOAD_ERR_FORM_SIZE", 2),
            (b"UPLOAD_ERR_PARTIAL", 3),
            (b"UPLOAD_ERR_NO_FILE", 4),
            (b"UPLOAD_ERR_NO_TMP_DIR", 6),
            (b"UPLOAD_ERR_CANT_WRITE", 7),
            (b"UPLOAD_ERR_EXTENSION", 8),
        ] {
            self.insert_builtin_constant(name, Val::Int(code));
        }
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
        self.extensions
            .push(Box::new(super::opcache_extension::OpcacheExtension));
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

impl RequestContext {
    /// An environment variable as this request sees it (putenv() included).
    pub fn env_var(&self, name: &[u8]) -> Option<Vec<u8>> {
        match self.env_overrides.get(name) {
            Some(value) => value.clone(),
            None => std::str::from_utf8(name)
                .ok()
                .and_then(std::env::var_os)
                .map(|v| v.to_string_lossy().into_owned().into_bytes()),
        }
    }

    /// The whole environment as this request sees it.
    pub fn env_vars(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut vars: Vec<(Vec<u8>, Vec<u8>)> = std::env::vars_os()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned().into_bytes(),
                    v.to_string_lossy().into_owned().into_bytes(),
                )
            })
            .filter(|(k, _)| !self.env_overrides.contains_key(k))
            .collect();
        vars.extend(
            self.env_overrides
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone()))),
        );
        vars
    }

    /// Give a child process this request's environment.
    pub fn apply_env(&self, cmd: &mut std::process::Command) {
        for (key, value) in &self.env_overrides {
            let key = String::from_utf8_lossy(key).into_owned();
            match value {
                Some(v) => {
                    cmd.env(key, String::from_utf8_lossy(v).into_owned());
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }
}
