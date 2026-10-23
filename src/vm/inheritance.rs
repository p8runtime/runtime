//! Class linking: the checks PHP runs when a class is declared, with its
//! messages. A violation is a compile-time fatal error at the offending
//! declaration's line.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_inheritance.c - do_inheritance_check_on_method,
//! zend_do_perform_implementation_check, zend_perform_covariant_type_check,
//! zend_get_function_declaration, zend_verify_abstract_class

use crate::builtins::signature::{
    self, SIG_ABSTRACT, SIG_FINAL, SIG_PRIVATE, SIG_PROTECTED, SIG_STATIC, Sig, T_ARRAY, T_BOOL,
    T_CALLABLE, T_FALSE, T_FLOAT, T_INT, T_MIXED, T_NEVER, T_NULL, T_OBJECT, T_STATIC, T_STRING,
    T_TRUE, T_VOID,
};
use crate::compiler::chunk::{ReturnType, UserFunc};
use crate::core::value::{Handle, Symbol, Val, Visibility};
use crate::vm::engine::{ErrorLevel, VM, VmError};
use std::rc::Rc;

/// A declared type, normalized: built-in type bits plus class names (an
/// intersection as `A&B`), with `self`/`parent` resolved.
#[derive(Debug, Clone, Default)]
struct LinkType {
    mask: u32,
    classes: Vec<String>,
}

impl LinkType {
    fn display(&self) -> String {
        let classes: Vec<&str> = self.classes.iter().map(String::as_str).collect();
        signature::type_to_string(self.mask, &classes)
    }
}

/// One side of an inheritance check: a user method or a built-in one.
#[derive(Clone)]
enum Method {
    User {
        func: Rc<UserFunc>,
        /// Declaring class.
        scope: Symbol,
        name: Symbol,
        visibility: Visibility,
        is_static: bool,
        is_abstract: bool,
    },
    Internal {
        sig: &'static Sig,
    },
}

/// A parameter, for either kind of method.
struct ParamView {
    name: String,
    ty: Option<LinkType>,
    by_ref: bool,
    variadic: bool,
    default: Option<String>,
}

enum Status {
    Ok,
    Error,
    /// Only a tentative return type differs: a deprecation.
    Warning,
}

impl Method {
    fn is_static(&self) -> bool {
        match self {
            Method::User { is_static, .. } => *is_static,
            Method::Internal { sig } => sig.flags & SIG_STATIC != 0,
        }
    }
    fn is_abstract(&self) -> bool {
        match self {
            Method::User { is_abstract, .. } => *is_abstract,
            Method::Internal { sig } => sig.flags & SIG_ABSTRACT != 0,
        }
    }
    fn is_final(&self) -> bool {
        match self {
            Method::User { func, .. } => func.is_final,
            Method::Internal { sig } => sig.flags & SIG_FINAL != 0,
        }
    }
    fn visibility(&self) -> Visibility {
        match self {
            Method::User { visibility, .. } => *visibility,
            Method::Internal { sig } if sig.flags & SIG_PRIVATE != 0 => Visibility::Private,
            Method::Internal { sig } if sig.flags & SIG_PROTECTED != 0 => Visibility::Protected,
            Method::Internal { .. } => Visibility::Public,
        }
    }
    fn returns_ref(&self) -> bool {
        match self {
            Method::User { func, .. } => func.chunk.returns_ref,
            Method::Internal { sig } => sig.returns_ref,
        }
    }
}

fn visibility_rank(v: Visibility) -> u8 {
    match v {
        Visibility::Public => 0,
        Visibility::Protected => 1,
        Visibility::Private => 2,
    }
}

fn visibility_name(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
    }
}

impl VM {
    /// Link a class once all its members are declared (FinalizeClass).
    /// Evaluate the property defaults of `class` and its ancestors that are
    /// constant expressions, each in its declaring class's scope, the first
    /// time the class is used (zend_update_class_constants: on `new` or a
    /// static property access, not at declaration).
    pub(crate) fn ensure_class_defaults(&mut self, class: Symbol) -> Result<(), VmError> {
        if !self.defaults_resolved.insert(class) {
            return Ok(());
        }
        let Some(def) = self.context.classes.get(&class) else {
            return Ok(());
        };
        if let Some(parent) = def.parent {
            self.ensure_class_defaults(parent)?;
        }
        let Some(def) = self.context.classes.get(&class) else {
            return Ok(());
        };
        let instance: Vec<(Symbol, Val)> = def
            .properties
            .iter()
            .filter(|(_, e)| crate::compiler::chunk::ConstExpr::of(&e.default_value).is_some())
            .map(|(n, e)| (*n, e.default_value.clone()))
            .collect();
        let statics: Vec<(Symbol, Handle)> = def
            .static_properties
            .iter()
            .filter(|(_, e)| crate::compiler::chunk::ConstExpr::of(&self.arena.get(e.handle).value).is_some())
            .map(|(n, e)| (*n, e.handle))
            .collect();
        for (name, thunk) in instance {
            let value = self.resolve_const_value(thunk, Some(class))?;
            if let Some(entry) = self.context.classes.get_mut(&class).and_then(|d| d.properties.get_mut(&name)) {
                entry.default_value = value;
            }
        }
        for (_, handle) in statics {
            let thunk = self.arena.get(handle).value.clone();
            let value = self.resolve_const_value(thunk, Some(class))?;
            self.arena.get_mut(handle).value = value;
        }
        Ok(())
    }

    pub(crate) fn link_class(&mut self, class: Symbol) -> Result<(), VmError> {
        let Some(def) = self.context.classes.get(&class).cloned() else {
            return Ok(());
        };

        if let Some(parent) = def.parent
            && self.context.classes.get(&parent).is_some_and(|p| p.is_final)
        {
            let msg = format!(
                "Class {} cannot extend final class {}",
                self.symbol_to_string(class),
                self.symbol_to_string(parent)
            );
            return Err(self.compile_error_here(&msg));
        }

        for &iface in &def.interfaces {
            let iface = self.canonical_class(iface);
            let iface_name = self.symbol_to_string(iface);
            match self.context.classes.get(&iface) {
                None => {
                    let msg = format!("Interface \"{}\" not found", iface_name);
                    return Err(self.throw_error(b"Error", &msg));
                }
                Some(idef) if !idef.is_interface => {
                    let msg = format!(
                        "{} cannot implement {} - it is not an interface",
                        self.symbol_to_string(class),
                        iface_name
                    );
                    return Err(self.compile_error_here(&msg));
                }
                Some(_) if !def.is_enum && matches!(iface_name.as_str(), "UnitEnum" | "BackedEnum") => {
                    let msg = format!(
                        "Non-enum class {} cannot implement interface {}",
                        self.symbol_to_string(class),
                        iface_name
                    );
                    return Err(self.compile_error_here(&msg));
                }
                Some(_) => {}
            }
        }

        // Methods this class declares, against what its parent declares.
        let mut own: Vec<_> = def
            .methods
            .iter()
            .filter(|(_, m)| m.declaring_class == class)
            .map(|(k, m)| (*k, m.clone()))
            .collect();
        own.sort_by_key(|(_, m)| m.decl_order);
        if let Some(parent) = def.parent {
            for (key, entry) in &own {
                if let Some(proto) = self.parent_method(parent, *key, entry.name) {
                    let child = self.user_method(class, entry);
                    self.check_inherited_method(class, &child, &proto, true)?;
                }
            }
        }

        // Interface methods, against the class's own or inherited methods.
        for iface in self.all_interfaces(class) {
            for proto in self.interface_methods(iface) {
                let proto_name = match &proto {
                    Method::User { name, .. } => self.symbol_to_string(*name),
                    Method::Internal { sig } => sig.name.rsplit("::").next().unwrap_or("").to_string(),
                };
                let key = self.context.interner.find(proto_name.to_ascii_lowercase().as_bytes());
                let Some(entry) = key.and_then(|k| def.methods.get(&k)) else {
                    continue;
                };
                let child = self.user_method(entry.declaring_class, entry);
                self.check_inherited_method(class, &child, &proto, true)?;
            }
        }

        if !def.is_abstract && !def.is_interface && !def.is_trait {
            self.verify_abstract_class(class)?;
        }
        Ok(())
    }

    fn user_method(&self, scope: Symbol, entry: &crate::runtime::context::MethodEntry) -> Method {
        let in_interface = self
            .context
            .classes
            .get(&entry.declaring_class)
            .is_some_and(|d| d.is_interface);
        Method::User {
            func: entry.func.clone(),
            scope,
            name: entry.name,
            visibility: entry.visibility,
            is_static: entry.is_static,
            is_abstract: entry.is_abstract || in_interface,
        }
    }

    /// Whether a built-in class above `class` already implements `iface`
    /// (built-in classes are trusted to).
    fn internal_ancestor_implements(&self, class: Symbol, iface: Symbol) -> bool {
        let mut current = self.context.classes.get(&class).and_then(|d| d.parent);
        while let Some(c) = current {
            let name = self.context.interner.lookup(c).unwrap_or(b"");
            if self.context.engine.registry.get_class(name).is_some() && self.is_subclass_of(c, iface) {
                return true;
            }
            current = self.context.classes.get(&c).and_then(|d| d.parent);
        }
        false
    }

    /// The method `key` as the parent class has it: declared by the parent or
    /// inherited, user-defined or built in.
    fn parent_method(&self, parent: Symbol, key: Symbol, name: Symbol) -> Option<Method> {
        if let Some(entry) = self.context.classes.get(&parent).and_then(|d| d.methods.get(&key)) {
            return Some(self.user_method(entry.declaring_class, entry));
        }
        let method_name = self.symbol_to_string(name);
        self.walk_inheritance_chain(parent, |_, cls| {
            signature::lookup(format!("{}::{}", self.symbol_to_string(cls), method_name).as_bytes())
        })
        .map(|sig| Method::Internal { sig })
    }

    /// Every interface the class implements, directly or through its
    /// parents and other interfaces, each once.
    fn all_interfaces(&self, class: Symbol) -> Vec<Symbol> {
        let mut out: Vec<Symbol> = Vec::new();
        let mut stack: Vec<Symbol> = Vec::new();
        let mut current = Some(class);
        while let Some(c) = current {
            let Some(def) = self.context.classes.get(&c) else { break };
            stack.extend(def.interfaces.iter().rev().copied());
            current = def.parent;
        }
        while let Some(iface) = stack.pop() {
            let iface = self.canonical_class(iface);
            if out.contains(&iface) {
                continue;
            }
            out.push(iface);
            if let Some(def) = self.context.classes.get(&iface) {
                stack.extend(def.interfaces.iter().rev().copied());
            }
        }
        out
    }

    /// The methods an interface declares itself.
    fn interface_methods(&self, iface: Symbol) -> Vec<Method> {
        let Some(def) = self.context.classes.get(&iface) else {
            return Vec::new();
        };
        let mut user: Vec<_> = def.methods.values().filter(|m| m.declaring_class == iface).collect();
        if !user.is_empty() {
            user.sort_by_key(|m| m.decl_order);
            return user.into_iter().map(|m| self.user_method(iface, m)).collect();
        }
        let prefix = format!("{}::", self.symbol_to_string(iface)).to_ascii_lowercase();
        signature::all()
            .iter()
            .filter(|s| s.name.to_ascii_lowercase().starts_with(&prefix))
            .map(|sig| Method::Internal { sig })
            .collect()
    }

    /// do_inheritance_check_on_method: `child` (in `class`) overriding or
    /// implementing `proto`.
    fn check_inherited_method(
        &mut self,
        class: Symbol,
        child: &Method,
        proto: &Method,
        check_visibility: bool,
    ) -> Result<(), VmError> {
        let Method::User { func: child_func, scope: child_scope, name: child_name, .. } = child else {
            return Ok(());
        };
        if proto.visibility() == Visibility::Private && !proto.is_abstract() {
            return Ok(());
        }
        let file = child_func.chunk.file_path.clone().unwrap_or_default();
        let line = child_func.decl_line;
        let method_name = self.symbol_to_string(*child_name);
        let proto_scope = self.method_scope_name(proto);

        if proto.is_final() {
            let msg = format!("Cannot override final method {}::{}()", proto_scope, method_name);
            return Err(self.compile_error_at(&msg, file, line));
        }
        if child.is_static() != proto.is_static() {
            let msg = if child.is_static() {
                format!(
                    "Cannot make non static method {}::{}() static in class {}",
                    proto_scope,
                    method_name,
                    self.symbol_to_string(*child_scope)
                )
            } else {
                format!(
                    "Cannot make static method {}::{}() non static in class {}",
                    proto_scope,
                    method_name,
                    self.symbol_to_string(*child_scope)
                )
            };
            return Err(self.compile_error_at(&msg, file, line));
        }
        let proto_abstract = proto.is_abstract() || self.method_from_interface(proto);
        if child.is_abstract() && !proto_abstract {
            let msg = format!(
                "Cannot make non abstract method {}::{}() abstract in class {}",
                proto_scope,
                method_name,
                self.symbol_to_string(*child_scope)
            );
            return Err(self.compile_error_at(&msg, file, line));
        }
        // A constructor only has a prototype when the parent's is abstract
        // or comes from an interface.
        if method_name.eq_ignore_ascii_case("__construct") && !proto_abstract {
            return Ok(());
        }
        if check_visibility && visibility_rank(child.visibility()) > visibility_rank(proto.visibility()) {
            let msg = format!(
                "Access level to {}::{}() must be {} (as in class {}){}",
                self.symbol_to_string(*child_scope),
                method_name,
                visibility_name(proto.visibility()),
                proto_scope,
                if proto.visibility() == Visibility::Public { "" } else { " or weaker" }
            );
            return Err(self.compile_error_at(&msg, file, line));
        }
        match self.implementation_check(child, proto, class) {
            Status::Ok => Ok(()),
            Status::Error => {
                let msg = format!(
                    "Declaration of {} must be compatible with {}",
                    self.method_declaration(child, class),
                    self.method_declaration(proto, class)
                );
                Err(self.compile_error_at(&msg, file, line))
            }
            Status::Warning => {
                let suppressed = child_func.attributes.iter().any(|a| {
                    self.symbol_to_string(a.name).trim_start_matches('\\').eq_ignore_ascii_case("ReturnTypeWillChange")
                }) || self.method_has_attribute(*child_scope, *child_name, "ReturnTypeWillChange");
                if !suppressed {
                    let msg = format!(
                        "Return type of {} should either be compatible with {}, or the #[\\ReturnTypeWillChange] attribute should be used to temporarily suppress the notice",
                        self.method_declaration(child, class),
                        self.method_declaration(proto, class)
                    );
                    self.report_error_at(ErrorLevel::Deprecated, &msg, file, line);
                }
                Ok(())
            }
        }
    }

    fn method_has_attribute(&self, class: Symbol, method: Symbol, attr: &str) -> bool {
        let Some(key) = self
            .context
            .interner
            .find(self.symbol_to_string(method).to_ascii_lowercase().as_bytes())
        else {
            return false;
        };
        self.context
            .classes
            .get(&class)
            .and_then(|d| d.methods.get(&key))
            .is_some_and(|m| {
                m.attributes.iter().any(|a| {
                    self.symbol_to_string(a.name).trim_start_matches('\\').eq_ignore_ascii_case(attr)
                })
            })
    }

    fn method_from_interface(&self, m: &Method) -> bool {
        match m {
            Method::User { scope, .. } => self.context.classes.get(scope).is_some_and(|d| d.is_interface),
            Method::Internal { sig } => {
                let class = sig.name.split("::").next().unwrap_or("");
                self.context
                    .interner
                    .find(class.as_bytes())
                    .and_then(|s| self.context.classes.get(&s))
                    .is_some_and(|d| d.is_interface)
            }
        }
    }

    fn method_scope_name(&self, m: &Method) -> String {
        match m {
            Method::User { scope, .. } => self.symbol_to_string(*scope),
            Method::Internal { sig } => sig.name.split("::").next().unwrap_or("").to_string(),
        }
    }

    fn params_of(&self, m: &Method, class: Symbol) -> Vec<ParamView> {
        match m {
            Method::User { func, scope, .. } => func
                .params
                .iter()
                .map(|p| ParamView {
                    name: self.symbol_to_string(p.name),
                    ty: p.param_type.as_ref().map(|t| self.link_type(t, *scope, class)),
                    by_ref: p.by_ref,
                    variadic: p.is_variadic,
                    default: p.default_display.as_deref().map(str::to_string),
                })
                .collect(),
            Method::Internal { sig } => sig
                .params
                .iter()
                .map(|p| ParamView {
                    name: p.name.to_string(),
                    ty: (p.mask != 0 || !p.classes.is_empty()).then(|| LinkType {
                        mask: p.mask,
                        classes: p.classes.iter().map(|c| c.to_string()).collect(),
                    }),
                    by_ref: p.by_ref,
                    variadic: p.variadic,
                    default: p.default.map(|d| d.to_string()),
                })
                .collect(),
        }
    }

    fn return_type_of(&self, m: &Method, class: Symbol) -> Option<(LinkType, bool)> {
        match m {
            Method::User { func, scope, .. } => {
                func.return_type.as_ref().map(|t| (self.link_type(t, *scope, class), false))
            }
            Method::Internal { sig } => (sig.ret_mask != 0 || !sig.ret_classes.is_empty()).then(|| {
                (
                    LinkType {
                        mask: sig.ret_mask,
                        classes: sig.ret_classes.iter().map(|c| c.to_string()).collect(),
                    },
                    sig.tentative,
                )
            }),
        }
    }

    /// Required parameters: up to the last one without a default.
    fn required_args(params: &[ParamView]) -> usize {
        params
            .iter()
            .rposition(|p| p.default.is_none() && !p.variadic)
            .map_or(0, |i| i + 1)
    }

    /// zend_do_perform_implementation_check.
    fn implementation_check(&self, child: &Method, proto: &Method, class: Symbol) -> Status {
        let fe = self.params_of(child, class);
        let pr = self.params_of(proto, class);
        if Self::required_args(&pr) < Self::required_args(&fe) {
            return Status::Error;
        }
        if proto.returns_ref() && !child.returns_ref() {
            return Status::Error;
        }
        let pr_variadic = pr.last().is_some_and(|p| p.variadic);
        let fe_variadic = fe.last().is_some_and(|p| p.variadic);
        if pr_variadic && !fe_variadic {
            return Status::Error;
        }
        let n = pr.len().max(fe.len());
        for i in 0..n {
            let p = pr.get(i).or_else(|| pr.last().filter(|p| p.variadic));
            let f = fe.get(i).or_else(|| fe.last().filter(|p| p.variadic));
            let Some(p) = p else { continue };
            let Some(f) = f else { return Status::Error };
            // Parameter types are contravariant; an untyped or mixed child
            // parameter accepts anything.
            if let Some(ft) = &f.ty
                && ft.mask & T_MIXED == 0
            {
                match &p.ty {
                    None => return Status::Error,
                    Some(pt) if !self.is_subtype(pt, ft) => return Status::Error,
                    _ => {}
                }
            }
            if f.by_ref != p.by_ref {
                return Status::Error;
            }
        }
        if let Some((proto_ret, tentative)) = self.return_type_of(proto, class) {
            match self.return_type_of(child, class) {
                None => return if tentative { Status::Warning } else { Status::Error },
                Some((child_ret, _)) if !self.is_subtype(&child_ret, &proto_ret) => {
                    return if tentative { Status::Warning } else { Status::Error };
                }
                _ => {}
            }
        }
        Status::Ok
    }

    /// zend_perform_covariant_type_check: whether `sub` is a subtype of `sup`.
    fn is_subtype(&self, sub: &LinkType, sup: &LinkType) -> bool {
        if sup.mask & T_MIXED != 0 {
            return sub.mask & T_VOID == 0;
        }
        if sub.mask & T_MIXED != 0 {
            return false;
        }
        if sub.mask & T_NEVER != 0 && sub.mask == T_NEVER && sub.classes.is_empty() {
            return true;
        }
        let bits = [
            T_STATIC, T_CALLABLE, T_OBJECT, T_ARRAY, T_STRING, T_INT, T_FLOAT, T_BOOL, T_FALSE,
            T_TRUE, T_VOID, T_NULL,
        ];
        for bit in bits {
            if sub.mask & bit == 0 {
                continue;
            }
            let ok = match bit {
                T_TRUE | T_FALSE => sup.mask & (bit | T_BOOL) != 0,
                T_BOOL => sup.mask & T_BOOL != 0 || sup.mask & (T_TRUE | T_FALSE) == T_TRUE | T_FALSE,
                T_STATIC => sup.mask & (T_STATIC | T_OBJECT) != 0,
                _ => sup.mask & bit != 0,
            };
            if !ok {
                return false;
            }
        }
        sub.classes.iter().all(|c| {
            sup.mask & T_OBJECT != 0
                || (sup.mask & T_CALLABLE != 0 && c.eq_ignore_ascii_case("Closure"))
                || sup.classes.iter().any(|d| self.class_type_subtype(c, d))
        })
    }

    /// A class (or intersection `A&B`) type within another.
    fn class_type_subtype(&self, sub: &str, sup: &str) -> bool {
        if sup.contains('&') {
            return sup.split('&').all(|d| self.class_type_subtype(sub, d));
        }
        if sub.contains('&') {
            return sub.split('&').any(|c| self.class_type_subtype(c, sup));
        }
        if sub.eq_ignore_ascii_case(sup) {
            return true;
        }
        let (Some(c), Some(d)) = (
            self.context.interner.find(sub.as_bytes()),
            self.context.interner.find(sup.as_bytes()),
        ) else {
            return false;
        };
        let c = self.canonical_class(c);
        self.context.classes.contains_key(&c) && self.is_subclass_of(c, d)
    }

    /// A user type as LinkType; `self`/`parent` name `scope`'s classes.
    fn link_type(&self, t: &ReturnType, scope: Symbol, class: Symbol) -> LinkType {
        let mut out = LinkType::default();
        self.add_link_type(t, scope, class, &mut out);
        out
    }

    fn add_link_type(&self, t: &ReturnType, scope: Symbol, class: Symbol, out: &mut LinkType) {
        let _ = class;
        match t {
            ReturnType::Int => out.mask |= T_INT,
            ReturnType::Float => out.mask |= T_FLOAT,
            ReturnType::String => out.mask |= T_STRING,
            ReturnType::Bool => out.mask |= T_BOOL,
            ReturnType::Array => out.mask |= T_ARRAY,
            ReturnType::Object => out.mask |= T_OBJECT,
            ReturnType::Void => out.mask |= T_VOID,
            ReturnType::Never => out.mask |= T_NEVER,
            ReturnType::Mixed => out.mask |= T_MIXED | T_NULL,
            ReturnType::Null => out.mask |= T_NULL,
            ReturnType::True => out.mask |= T_TRUE,
            ReturnType::False => out.mask |= T_FALSE,
            ReturnType::Callable => out.mask |= T_CALLABLE,
            ReturnType::Iterable => {
                out.mask |= T_ARRAY;
                out.classes.push("Traversable".into());
            }
            ReturnType::Static => out.mask |= T_STATIC,
            ReturnType::Named(sym) => out.classes.push(self.resolve_type_name(*sym, scope)),
            ReturnType::Nullable(inner) => {
                self.add_link_type(inner, scope, class, out);
                out.mask |= T_NULL;
            }
            ReturnType::Union(items) => {
                for item in items {
                    self.add_link_type(item, scope, class, out);
                }
            }
            ReturnType::Intersection(items) => {
                let names: Vec<String> = items
                    .iter()
                    .filter_map(|i| match i {
                        ReturnType::Named(sym) => Some(self.resolve_type_name(*sym, scope)),
                        _ => None,
                    })
                    .collect();
                out.classes.push(names.join("&"));
            }
        }
    }

    /// `self` and `parent` in a type, as the class names they stand for.
    fn resolve_type_name(&self, sym: Symbol, scope: Symbol) -> String {
        let name = self.symbol_to_string(sym);
        let name = name.trim_start_matches('\\');
        if name.eq_ignore_ascii_case("self") {
            return self.symbol_to_string(scope);
        }
        if name.eq_ignore_ascii_case("parent")
            && let Some(parent) = self.context.classes.get(&scope).and_then(|d| d.parent)
        {
            return self.symbol_to_string(parent);
        }
        let canonical = self.canonical_class(sym);
        if self.context.classes.contains_key(&canonical) {
            return self.symbol_to_string(canonical);
        }
        name.to_string()
    }

    /// zend_get_function_declaration: `& A::f(int $x, $y = 'abc'): void`.
    fn method_declaration(&self, m: &Method, class: Symbol) -> String {
        let mut out = String::new();
        if m.returns_ref() {
            out.push_str("& ");
        }
        match m {
            Method::User { scope, name, .. } => {
                out.push_str(&self.symbol_to_string(*scope));
                out.push_str("::");
                out.push_str(&self.symbol_to_string(*name));
            }
            Method::Internal { sig } => out.push_str(sig.name),
        }
        out.push('(');
        let params = self.params_of(m, class);
        let required = Self::required_args(&params);
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if let Some(t) = &p.ty {
                out.push_str(&t.display());
                out.push(' ');
            }
            if p.by_ref {
                out.push('&');
            }
            if p.variadic {
                out.push_str("...");
            }
            out.push('$');
            out.push_str(&p.name);
            if i >= required && !p.variadic {
                out.push_str(" = ");
                out.push_str(p.default.as_deref().unwrap_or("<default>"));
            }
        }
        out.push(')');
        if let Some((ret, _)) = self.return_type_of(m, class) {
            out.push_str(": ");
            out.push_str(&ret.display());
        }
        out
    }

    /// zend_verify_abstract_class: a concrete class may not keep abstract
    /// methods. Lists up to three of them, as PHP does.
    fn verify_abstract_class(&mut self, class: Symbol) -> Result<(), VmError> {
        let Some(def) = self.context.classes.get(&class) else {
            return Ok(());
        };
        let mut missing: Vec<(u64, String)> = def
            .methods
            .values()
            .filter(|m| m.is_abstract)
            .map(|m| {
                (
                    m.decl_order,
                    format!("{}::{}", self.symbol_to_string(m.declaring_class), self.symbol_to_string(m.name)),
                )
            })
            .collect();
        // Interface methods nothing implements.
        for iface in self.all_interfaces(class) {
            if self.internal_ancestor_implements(class, iface) {
                continue;
            }
            for proto in self.interface_methods(iface) {
                let name = match &proto {
                    Method::User { name, .. } => self.symbol_to_string(*name),
                    Method::Internal { sig } => sig.name.rsplit("::").next().unwrap_or("").to_string(),
                };
                let implemented = self
                    .context
                    .interner
                    .find(name.to_ascii_lowercase().as_bytes())
                    .is_some_and(|k| self.context.classes.get(&class).is_some_and(|d| d.methods.contains_key(&k)))
                    || self.find_native_method_named(class, &name);
                let label = format!("{}::{}", self.method_scope_name(&proto), name);
                if !implemented && !missing.iter().any(|(_, l)| l.eq_ignore_ascii_case(&label)) {
                    missing.push((u64::MAX, label));
                }
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort_by_key(|(order, _)| *order);
        let n = missing.len();
        let mut listed: Vec<String> = missing.iter().take(3).map(|(_, l)| l.clone()).collect();
        if n > 3 {
            listed.push("...".into());
        }
        let msg = format!(
            "Class {} contains {} abstract method{} and must therefore be declared abstract or implement the remaining methods ({})",
            self.symbol_to_string(class),
            n,
            if n == 1 { "" } else { "s" },
            listed.join(", ")
        );
        Err(self.compile_error_here(&msg))
    }

    fn find_native_method_named(&self, class: Symbol, name: &str) -> bool {
        [name.to_string(), name.to_ascii_lowercase()].iter().any(|n| {
            self.context
                .interner
                .find(n.as_bytes())
                .is_some_and(|m| self.find_native_method(class, m).is_some())
        })
    }

    /// A compile-time fatal error at `file:line`: shown, and the script ends.
    pub(crate) fn compile_error_at(&mut self, message: &str, file: String, line: u32) -> VmError {
        self.report_error_at(ErrorLevel::Error, message, file, line);
        VmError::Terminated
    }

    /// A compile-time fatal error at the line running now (the declaration).
    fn compile_error_here(&mut self, message: &str) -> VmError {
        let (file, line) = self.current_location();
        self.compile_error_at(message, file, line)
    }
}
