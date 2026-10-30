#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's pdo extension: the driver-agnostic PDO core. PDO, PDOStatement,
//! PDOException and PDORow, the fetch modes, error modes and attributes of
//! php-src's ext/pdo, over a [`driver::Driver`] trait that `pdo_sqlite`
//! (p8-ext-pdo-sqlite) and `pdo_pgsql` (p8-ext-pdo-pgsql) implement and
//! `pdo_mysql` will, on p8-mysqlnd.
//!
//! Not implemented: persistent connections (the attribute is kept and
//! ignored), scrollable cursors, multiple rowsets, PDO::FETCH_SERIALIZE's
//! unserialize, the `pdo.dsn.*` ini DSN aliases and the `uri:` DSN.
//!
//! Reference: $PHP_SRC_PATH/ext/pdo (pdo_dbh.c, pdo_stmt.c, pdo_sql_parser.re)

pub mod consts;
pub mod dbh;
pub mod driver;
pub mod parser;
pub mod sqlstate;
pub mod state;
pub mod stmt;

use p8_runtime::ext::{Builtin, ClassSpec, Constant, Ctx, Extension, Registry};
use p8_runtime::object::{CLASS_FINAL, CLASS_NOT_SERIALIZABLE, CLASS_UNCLONEABLE, Native, Object, Visibility};
use p8_runtime::value::Value;
use std::rc::Rc;

pub use dbh::{call_driver_method, register_driver};
pub use state::{Dbh, dbh_of, dispatch_notices};

pub const EXTENSION: Extension = Extension { name: "pdo", register };

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    registry.add_data("PDO", || Rc::new(state::PdoData::default()));
    registry.add_data("PDOStatement", || Rc::new(stmt::StmtData::default()));
    registry.add_data("PDOStatementIterator", stmt::make_iter);
    registry.add_module("PDO");
}

static BUILTINS: &[Builtin] = &[Builtin { name: "pdo_drivers", run: dbh::pdo_drivers, by_ref: &[] }];

fn row_construct(ctx: &mut dyn Ctx, _: &Rc<Object>, _: &mut [Value]) -> Result<Value, p8_runtime::ext::Throw> {
    Err(state::exception(ctx, "You may not create a PDORow manually".into(), Value::Int(0), None))
}

macro_rules! m {
    ($name:literal, $f:path) => {
        ($name, Some(Native::Method($f)))
    };
}

static CLASSES: &[ClassSpec] = &[
    ClassSpec {
        name: "PDOException",
        parent: Some("RuntimeException"),
        interfaces: &[],
        flags: 0,
        properties: &[("errorInfo", Constant::Null, Visibility::Public)],
        methods: &[],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "PDO",
        parent: None,
        interfaces: &[],
        flags: CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[
            m!("__construct", dbh::m_construct),
            ("connect", Some(Native::Static(dbh::m_connect))),
            m!("beginTransaction", dbh::m_begin),
            m!("commit", dbh::m_commit),
            m!("errorCode", dbh::m_error_code),
            m!("errorInfo", dbh::m_error_info),
            m!("exec", dbh::m_exec),
            m!("getAttribute", dbh::m_get_attribute),
            ("getAvailableDrivers", Some(Native::Static(dbh::m_get_available_drivers))),
            m!("inTransaction", dbh::m_in_transaction),
            m!("lastInsertId", dbh::m_last_insert_id),
            m!("prepare", dbh::m_prepare),
            m!("query", dbh::m_query),
            m!("quote", dbh::m_quote),
            m!("rollBack", dbh::m_rollback),
            m!("setAttribute", dbh::m_set_attribute),
            // The driver methods PHP 8.4 still reaches through PDO itself.
            m!("sqliteCreateFunction", dbh::m_sqlite_create_function),
            m!("sqliteCreateAggregate", dbh::m_sqlite_create_aggregate),
            m!("sqliteCreateCollation", dbh::m_sqlite_create_collation),
            m!("pgsqlCopyFromArray", dbh::m_pgsql_copy_from_array),
            m!("pgsqlCopyFromFile", dbh::m_pgsql_copy_from_file),
            m!("pgsqlCopyToArray", dbh::m_pgsql_copy_to_array),
            m!("pgsqlCopyToFile", dbh::m_pgsql_copy_to_file),
            m!("pgsqlLOBCreate", dbh::m_pgsql_lob_create),
            m!("pgsqlLOBOpen", dbh::m_pgsql_lob_open),
            m!("pgsqlLOBUnlink", dbh::m_pgsql_lob_unlink),
            m!("pgsqlGetNotify", dbh::m_pgsql_get_notify),
            m!("pgsqlGetPid", dbh::m_pgsql_get_pid),
        ],
        constants: consts::PDO_CONSTANTS,
        debug_info: None,
    },
    ClassSpec {
        name: "PDOStatement",
        parent: None,
        interfaces: &["IteratorAggregate"],
        flags: CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[("queryString", Constant::Null, Visibility::Public)],
        methods: &[
            m!("bindColumn", stmt::m_bind_column),
            m!("bindParam", stmt::m_bind_param),
            m!("bindValue", stmt::m_bind_value),
            m!("closeCursor", stmt::m_close_cursor),
            m!("columnCount", stmt::m_column_count),
            m!("debugDumpParams", stmt::m_debug_dump_params),
            m!("errorCode", stmt::m_error_code),
            m!("errorInfo", stmt::m_error_info),
            m!("execute", stmt::m_execute),
            m!("fetch", stmt::m_fetch),
            m!("fetchAll", stmt::m_fetch_all),
            m!("fetchColumn", stmt::m_fetch_column),
            m!("fetchObject", stmt::m_fetch_object),
            m!("getAttribute", stmt::m_get_attribute),
            m!("getColumnMeta", stmt::m_get_column_meta),
            m!("nextRowset", stmt::m_next_rowset),
            m!("rowCount", stmt::m_row_count),
            m!("setAttribute", stmt::m_set_attribute),
            m!("setFetchMode", stmt::m_set_fetch_mode),
            m!("getIterator", stmt::m_get_iterator),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "PDOStatementIterator",
        parent: None,
        interfaces: &["Iterator"],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[],
        methods: &[
            m!("current", stmt::it_current),
            m!("key", stmt::it_key),
            m!("next", stmt::it_next),
            m!("valid", stmt::it_valid),
            m!("rewind", stmt::it_rewind),
        ],
        constants: &[],
        debug_info: None,
    },
    ClassSpec {
        name: "PDORow",
        parent: None,
        interfaces: &["ArrayAccess"],
        flags: CLASS_FINAL | CLASS_UNCLONEABLE | CLASS_NOT_SERIALIZABLE,
        properties: &[("queryString", Constant::Null, Visibility::Public)],
        methods: &[
            m!("__construct", row_construct),
            m!("offsetExists", stmt::row_offset_exists),
            m!("offsetGet", stmt::row_offset_get),
            m!("offsetSet", stmt::row_offset_set),
            m!("offsetUnset", stmt::row_offset_unset),
        ],
        constants: &[],
        debug_info: None,
    },
];
