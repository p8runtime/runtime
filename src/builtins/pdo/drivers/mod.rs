//! PDO Drivers
//!
//! This module contains all PDO driver implementations and the driver registry.

pub mod mysql;
pub mod oci;
pub mod pgsql;
pub mod sqlite;

use super::driver::PdoDriver;
use super::types::PdoError;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn strip_driver_prefix<'a>(dsn: &'a str, driver: &str) -> &'a str {
    let dsn = dsn.trim();
    let driver = driver.trim();
    if dsn.len() > driver.len() {
        let (prefix, rest) = dsn.split_at(driver.len());
        if prefix.eq_ignore_ascii_case(driver) && rest.starts_with(':') {
            return &rest[1..];
        }
    }
    dsn
}

pub(crate) fn parse_semicolon_kv(s: &str) -> impl Iterator<Item = (&str, &str)> {
    s.split(';').filter_map(|part| {
        let mut it = part.splitn(2, '=');
        let key = it.next()?.trim();
        let value = it.next()?.trim();
        if key.is_empty() {
            None
        } else {
            Some((key, value))
        }
    })
}

/// Registry of PDO drivers
#[derive(Debug)]
pub struct DriverRegistry {
    drivers: HashMap<String, Arc<dyn PdoDriver>>,
}

impl DriverRegistry {
    /// Create a new driver registry with all built-in drivers
    pub fn new() -> Self {
        let mut registry = Self {
            drivers: HashMap::new(),
        };

        // Register built-in drivers
        registry.register(Box::new(sqlite::SqliteDriver));
        registry.register(Box::new(mysql::MysqlDriver));
        registry.register(Box::new(pgsql::PgsqlDriver));
        registry.register(Box::new(oci::OciDriver));

        registry
    }

    /// Register a driver
    fn register(&mut self, driver: Box<dyn PdoDriver>) {
        let driver: Arc<dyn PdoDriver> = driver.into();
        // Canonicalize keys so lookup can be cheaply case-insensitive.
        self.drivers
            .insert(driver.name().to_ascii_lowercase(), driver);
    }

    /// Get a driver by name (case-insensitive)
    pub fn get(&self, name: &str) -> Option<&dyn PdoDriver> {
        let name = name.trim();
        if name.as_bytes().iter().any(|b| b.is_ascii_uppercase()) {
            let lower = name.to_ascii_lowercase();
            self.drivers.get(&lower).map(|d| &**d)
        } else {
            self.drivers.get(name).map(|d| &**d)
        }
    }

    /// Get a driver by name (case-insensitive) as an Arc for cheap cloning.
    pub fn get_arc(&self, name: &str) -> Option<Arc<dyn PdoDriver>> {
        let name = name.trim();
        if name.as_bytes().iter().any(|b| b.is_ascii_uppercase()) {
            let lower = name.to_ascii_lowercase();
            self.drivers.get(&lower).cloned()
        } else {
            self.drivers.get(name).cloned()
        }
    }

    /// List all available drivers
    pub fn list_drivers(&self) -> Vec<&str> {
        let mut drivers: Vec<_> = self.drivers.keys().map(|s| s.as_str()).collect();
        drivers.sort_unstable();
        drivers
    }

    /// Parse a DSN string into driver name and connection string
    /// Format: "driver:connection_string"
    pub fn parse_dsn(dsn: &str) -> Result<(&str, &str), PdoError> {
        let dsn = dsn.trim();
        if dsn.is_empty() {
            return Err(PdoError::InvalidParameter(
                "Invalid DSN format: empty DSN".to_string(),
            ));
        }

        if let Some(colon_pos) = dsn.find(':') {
            let driver = dsn[..colon_pos].trim();
            if driver.is_empty() {
                return Err(PdoError::InvalidParameter(
                    "Invalid DSN format: empty driver name".to_string(),
                ));
            }
            let connection_str = &dsn[colon_pos + 1..];
            Ok((driver, connection_str))
        } else {
            Err(PdoError::InvalidParameter(
                "Invalid DSN format: expected 'driver:connection_string'".to_string(),
            ))
        }
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_registry_initialization() {
        let registry = DriverRegistry::new();
        assert!(registry.get("sqlite").is_some());
    }

    #[test]
    fn test_parse_dsn() {
        let (driver, conn_str) = DriverRegistry::parse_dsn("sqlite:/tmp/test.db").unwrap();
        assert_eq!(driver, "sqlite");
        assert_eq!(conn_str, "/tmp/test.db");

        let (driver, conn_str) =
            DriverRegistry::parse_dsn("mysql:host=localhost;dbname=test").unwrap();
        assert_eq!(driver, "mysql");
        assert_eq!(conn_str, "host=localhost;dbname=test");

        assert!(DriverRegistry::parse_dsn("invalid").is_err());
    }

    #[test]
    fn test_case_insensitive_lookup() {
        let registry = DriverRegistry::new();
        assert!(registry.get("SQLite").is_some());
        assert!(registry.get("SQLITE").is_some());
        assert!(registry.get("sqlite").is_some());
    }
}
