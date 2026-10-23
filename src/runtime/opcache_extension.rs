use crate::builtins::opcache;
use crate::runtime::extension::{Extension, ExtensionInfo, ExtensionResult};
use crate::runtime::registry::ExtensionRegistry;

/// Zend OPcache: a Zend extension in PHP (get_loaded_extensions(true)).
pub struct OpcacheExtension;

/// The name PHP gives it; extension_loaded('opcache') is false in PHP too.
pub const NAME: &str = "Zend OPcache";

impl Extension for OpcacheExtension {
    fn info(&self) -> ExtensionInfo {
        ExtensionInfo {
            name: NAME,
            version: "8.4.27",
            dependencies: &[],
        }
    }

    fn module_init(&self, registry: &mut ExtensionRegistry) -> ExtensionResult {
        registry.register_function(b"opcache_get_status", opcache::php_opcache_get_status);
        registry.register_function(b"opcache_get_configuration", opcache::php_opcache_get_configuration);
        registry.register_function(b"opcache_compile_file", opcache::php_opcache_compile_file);
        registry.register_function(b"opcache_is_script_cached", opcache::php_opcache_is_script_cached);
        registry.register_function(b"opcache_invalidate", opcache::php_opcache_invalidate);
        registry.register_function(b"opcache_reset", opcache::php_opcache_reset);
        registry.register_function(b"opcache_jit_blacklist", opcache::php_opcache_jit_blacklist);
        ExtensionResult::Success
    }
}
