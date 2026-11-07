//! The session's per-request state (php_ps_globals) and the helpers that
//! read the ini settings it is driven by.

use crate::files::Files;
use p8_runtime::ext::Ctx;
use p8_runtime::array::PhpArray;
use p8_runtime::value::Value;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

/// php_session_status (PHP_SESSION_DISABLED, PHP_SESSION_NONE, PHP_SESSION_ACTIVE).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    Disabled = 0,
    None = 1,
    Active = 2,
}

/// A storage module (ps_module).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Module {
    Files,
    User,
}

impl Module {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Module::Files => "files",
            Module::User => "user",
        }
    }

    /// _php_find_ps_module()
    pub(crate) fn find(name: &[u8]) -> Option<Module> {
        [Module::Files, Module::User].into_iter().find(|m| m.name().as_bytes().eq_ignore_ascii_case(name))
    }
}

/// A serialize handler (ps_serializer).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Serializer {
    PhpSerialize,
    Php,
    PhpBinary,
}

impl Serializer {
    /// _php_find_ps_serializer()
    pub(crate) fn find(name: &[u8]) -> Option<Serializer> {
        [("php_serialize", Serializer::PhpSerialize), ("php", Serializer::Php), ("php_binary", Serializer::PhpBinary)]
            .into_iter()
            .find(|(n, _)| n.as_bytes().eq_ignore_ascii_case(name))
            .map(|(_, s)| s)
    }
}

/// The callables a user save handler is made of (mod_user_names).
#[derive(Default, Clone)]
pub(crate) struct Handlers {
    pub open: Option<Value>,
    pub close: Option<Value>,
    pub read: Option<Value>,
    pub write: Option<Value>,
    pub destroy: Option<Value>,
    pub gc: Option<Value>,
    pub create_sid: Option<Value>,
    pub validate_sid: Option<Value>,
    pub update_timestamp: Option<Value>,
}

pub(crate) struct State {
    pub status: Status,
    /// PS(id)
    pub id: Option<Vec<u8>>,
    /// PS(mod), PS(default_mod)
    pub module: Option<Module>,
    pub default_mod: Option<Module>,
    /// PS(mod_data) of the files module
    pub files: Option<Files>,
    pub names: Handlers,
    /// PS(mod_user_class_name)
    pub class_name: Option<Vec<u8>>,
    pub user_implemented: bool,
    pub user_is_open: bool,
    pub in_save_handler: bool,
    /// session.save_handler is being set to "user" by session_set_save_handler().
    pub set_handler: bool,
    pub send_cookie: bool,
    pub define_sid: bool,
    /// PS(http_session_vars): the cell $_SESSION shares.
    pub vars: Option<Rc<RefCell<Value>>>,
    /// PS(session_vars): the data as read, for session.lazy_write.
    pub original: Option<Vec<u8>>,
    /// Where session_start() was called from.
    pub started: Option<(Rc<str>, u32)>,
    /// The request is ending (RSHUTDOWN).
    pub ending: bool,
}

pub(crate) type Shared = Rc<RefCell<State>>;

const KEY: &str = "session";

/// The request's state, if a session function has made it.
pub(crate) fn peek(ctx: &mut dyn Ctx) -> Option<Shared> {
    ctx.ext_data(KEY).as_ref().and_then(|b| b.downcast_ref::<Shared>()).cloned()
}

/// The request's state (php_rinit_session_globals() and php_rinit_session():
/// the modules come from the settings).
pub(crate) fn state(ctx: &mut dyn Ctx) -> Shared {
    if let Some(s) = peek(ctx) {
        return s;
    }
    let module = Module::find(&text(ctx, "session.save_handler"));
    let serializer = Serializer::find(&text(ctx, "session.serialize_handler"));
    let status = if module.is_none() || serializer.is_none() { Status::Disabled } else { Status::None };
    let s: Shared = Rc::new(RefCell::new(State {
        status,
        id: None,
        module,
        default_mod: None,
        files: None,
        names: Handlers::default(),
        class_name: None,
        user_implemented: false,
        user_is_open: false,
        in_save_handler: false,
        set_handler: false,
        send_cookie: false,
        define_sid: true,
        vars: None,
        original: None,
        started: None,
        ending: false,
    }));
    let boxed: Box<dyn Any> = Box::new(Rc::clone(&s));
    *ctx.ext_data(KEY) = Some(boxed);
    s
}

impl State {
    /// IF_SESSION_VARS(): whether $_SESSION is the array the session keeps.
    pub(crate) fn has_vars(&self) -> bool {
        self.vars.as_ref().is_some_and(|c| c.borrow().is_array())
    }

    /// The session's variables, if they are an array.
    pub(crate) fn vars(&self) -> Option<Rc<PhpArray>> {
        self.vars.as_ref()?.borrow().cloned_array()
    }

    /// Change the session's variables in place (SEPARATE_ARRAY); false when
    /// they are not an array.
    pub(crate) fn edit_vars(&self, f: impl FnOnce(&mut PhpArray)) -> bool {
        let Some(cell) = &self.vars else { return false };
        match cell.borrow_mut().array_mut() {
            Some(a) => {
                f(a);
                true
            }
            None => false,
        }
    }

    /// PS(mod_data) or a user handler that is open.
    pub(crate) fn has_mod_data(&self) -> bool {
        self.files.is_some() || self.user_implemented
    }
}

// ---- the settings ----

/// A setting's value as text (empty for a setting without one).
pub(crate) fn text(ctx: &dyn Ctx, name: &str) -> Vec<u8> {
    match ctx.ini_get(name.as_bytes()) {
        Some(v) if v.is_str() => v.as_str().expect("a string").as_bytes().to_vec(),
        Some(v) if !v.is_null() => v.to_php_string().map(|s| s.as_bytes().to_vec()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// C's atol(): optional whitespace and sign, then digits.
pub(crate) fn atol(s: &[u8]) -> i64 {
    let s = s.iter().position(|c| !c.is_ascii_whitespace() && *c != 0x0b).map_or(&s[s.len()..], |i| &s[i..]);
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).fold(0i64, |n, &c| n.saturating_mul(10).saturating_add(i64::from(c - b'0')));
    if negative { -n } else { n }
}

pub(crate) fn int(ctx: &dyn Ctx, name: &str) -> i64 {
    atol(&text(ctx, name))
}

/// zend_ini_parse_bool()
pub(crate) fn parse_bool(v: &[u8]) -> bool {
    let lower = v.to_ascii_lowercase();
    matches!(lower.as_slice(), b"true" | b"yes" | b"on") || atol(v) != 0
}

pub(crate) fn flag(ctx: &dyn Ctx, name: &str) -> bool {
    parse_bool(&text(ctx, name))
}

/// "name(): " for a diagnostic raised in a function, "Unknown: " where no
/// function is running (php_verror()).
pub(crate) fn origin(ctx: &mut dyn Ctx) -> String {
    let name = ctx.builtin_name();
    if !name.is_empty() {
        format!("{name}()")
    } else if ctx.location().0.is_empty() {
        // the host's settings are being made
        "PHP Startup".to_string()
    } else if peek(ctx).is_some_and(|s| s.borrow().ending) {
        "PHP Request Shutdown".to_string()
    } else {
        "Unknown".to_string()
    }
}

pub(crate) fn warn(ctx: &mut dyn Ctx, message: &str) {
    let message = format!("{}: {message}", origin(ctx));
    ctx.warn(&message);
}

pub(crate) fn notice(ctx: &mut dyn Ctx, message: &str) {
    let message = format!("{}: {message}", origin(ctx));
    ctx.notice(&message);
}

pub(crate) fn deprecated(ctx: &mut dyn Ctx, message: &str) {
    let message = format!("{}: {message}", origin(ctx));
    ctx.deprecated(&message);
}

/// SG(headers_sent)
pub(crate) fn headers_sent(ctx: &dyn Ctx) -> bool {
    ctx.output_started().is_some()
}

/// strerror's text and the number, as PHP words "%s (%d)".
pub(crate) fn errno(e: &std::io::Error) -> String {
    let code = e.raw_os_error().unwrap_or(0);
    let shown = e.to_string();
    let message = shown.split(" (os error").next().unwrap_or(&shown);
    format!("{message} ({code})")
}
