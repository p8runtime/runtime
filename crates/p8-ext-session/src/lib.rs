#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's session extension: the session_* functions, `$_SESSION`, the
//! `files` and `user` save handlers (SessionHandlerInterface and
//! SessionHandler), the `php`, `php_binary` and `php_serialize`
//! serializers, the session.* settings, the id cookie and the cache
//! headers, and the write when the request ends. What is observable
//! follows php-src's ext/session. Not implemented: session upload progress
//! (the session.upload_progress.* settings are kept and do nothing), the
//! URL rewriter (session.use_trans_sid), and the mm and memcached save
//! handlers PHP does not ship in the CLI.
//!
//! Reference: $PHP_SRC_PATH/ext/session/session.c, mod_files.c,
//! mod_user.c, mod_user_class.c

mod classes;
mod life;
mod files;
mod funcs;
mod ini;
mod state;
mod user;

use p8_runtime::ext::{Constant, Extension, Registry};

pub const EXTENSION: Extension = Extension { name: "session", register };

/// The longest session id PHP takes.
pub(crate) const PS_MAX_SID_LENGTH: usize = 256;

fn register(registry: &mut Registry) {
    registry.add_functions(funcs::BUILTINS);
    // They reach $_SESSION and the request's variables, and call save
    // handlers, which are PHP code.
    registry.mark_reenters(&funcs::names());
    // Its two signatures are told apart, and individual callbacks
    // deprecated, before the callbacks are checked (session.c).
    registry.own_callbacks(&["session_set_save_handler"]);
    registry.add_classes(classes::CLASSES);
    registry.add_constant(b"PHP_SESSION_DISABLED", Constant::Int(0));
    registry.add_constant(b"PHP_SESSION_NONE", Constant::Int(1));
    registry.add_constant(b"PHP_SESSION_ACTIVE", Constant::Int(2));
    ini::register(registry);
    registry.add_request_start(life::request_start);
    registry.add_request_end(life::request_end);
    registry.add_module("session");
}
