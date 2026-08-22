use super::*;

/// If `stmt` is `export <decl>`, return the wrapped declaration so that
/// collectors and the statement checker can treat exported declarations the
/// same as top-level declarations.
pub(in crate::phpx::typeck::check) fn export_decl_stmt<'a>(stmt: &'a Stmt<'a>) -> &'a Stmt<'a> {
    match stmt {
        Stmt::Export {
            item: ExportItem::Decl(decl),
            ..
        } => decl,
        _ => stmt,
    }
}

pub(in crate::phpx::typeck::check) fn member_span(member: &ClassMember) -> Span {
    match member {
        ClassMember::Property { span, .. }
        | ClassMember::PropertyHook { span, .. }
        | ClassMember::Method { span, .. }
        | ClassMember::Const { span, .. }
        | ClassMember::TraitUse { span, .. }
        | ClassMember::Embed { span, .. }
        | ClassMember::Case { span, .. } => *span,
    }
}

pub(in crate::phpx::typeck::check) fn enum_backed_primitive(
    ty: &AstType<'_>,
) -> Option<PrimitiveType> {
    match ty {
        AstType::Simple(token) => match token.kind {
            TokenKind::TypeInt => Some(PrimitiveType::Number),
            TokenKind::TypeString => Some(PrimitiveType::String),
            _ => None,
        },
        _ => None,
    }
}

pub(in crate::phpx::typeck::check) fn relation_model_from_field_type(
    field_type: Option<&Type>,
    kind: &str,
) -> Option<String> {
    let field_type = field_type?;
    if kind == "hasMany" {
        match field_type {
            Type::Applied { base, args } if base.eq_ignore_ascii_case("array") => {
                if args.len() == 1 {
                    if let Type::Struct(name) = &args[0] {
                        return Some(name.clone());
                    }
                }
            }
            _ => {}
        }
        return None;
    }

    match field_type {
        Type::Struct(name) => Some(name.clone()),
        _ => None,
    }
}

pub(in crate::phpx::typeck::check) fn token_text(source: &[u8], span: Span) -> String {
    let start = span.start;
    let end = span.end.min(source.len());
    String::from_utf8_lossy(&source[start..end]).to_string()
}

pub(in crate::phpx::typeck::check) fn capitalize_jsx_name(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    let mut out = String::new();
    out.push(first.to_ascii_uppercase());
    out.push_str(chars.as_str());
    out
}

pub(in crate::phpx::typeck::check) fn is_builtin_variable(name: &str) -> bool {
    matches!(
        name,
        "GLOBALS"
            | "_SERVER"
            | "_GET"
            | "_POST"
            | "_FILES"
            | "_COOKIE"
            | "_SESSION"
            | "_REQUEST"
            | "_ENV"
            | "this"
    )
}

pub(in crate::phpx::typeck::check) fn nearest_name<'a, I>(
    needle: &str,
    candidates: I,
) -> Option<&'a str>
where
    I: Iterator<Item = &'a str>,
{
    let mut best: Option<(&'a str, usize)> = None;
    for candidate in candidates {
        let dist = levenshtein(needle, candidate);
        if dist > 2 {
            continue;
        }
        match best {
            Some((_, best_dist)) if dist >= best_dist => {}
            _ => best = Some((candidate, dist)),
        }
    }
    best.map(|(name, _)| name)
}

pub(in crate::phpx::typeck::check) fn levenshtein(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.chars().count();
    }
    if b.is_empty() {
        return a.chars().count();
    }
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0usize; b_chars.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = if ca == *cb { 0 } else { 1 };
            let del = prev[j + 1] + 1;
            let ins = curr[j] + 1;
            let sub = prev[j] + cost;
            curr[j + 1] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

pub(in crate::phpx::typeck::check) fn parse_type_field_name(source: &[u8], span: Span) -> String {
    let raw = token_text(source, span);
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_type_string(inner, first == b'"');
        }
    }
    raw
}

pub(in crate::phpx::typeck::check) fn unescape_type_string(
    value: &str,
    double_quoted: bool,
) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

pub(in crate::phpx::typeck::check) fn substitute_type(
    ty: &Type,
    mapping: &HashMap<String, Type>,
) -> Type {
    match ty {
        Type::TypeParam(name) => mapping
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::TypeParam(name.clone())),
        Type::Union(types) => {
            let out = types
                .iter()
                .map(|t| substitute_type(t, mapping))
                .collect::<Vec<_>>();
            Type::Union(out)
        }
        Type::ObjectShape(fields) => {
            let mut out = BTreeMap::new();
            for (name, field) in fields.iter() {
                out.insert(
                    name.clone(),
                    ObjectField {
                        ty: substitute_type(&field.ty, mapping),
                        optional: field.optional,
                        is_mut: field.is_mut,
                    },
                );
            }
            Type::ObjectShape(out)
        }
        Type::Applied { base, args } => Type::Applied {
            base: base.clone(),
            args: args.iter().map(|t| substitute_type(t, mapping)).collect(),
        },
        _ => ty.clone(),
    }
}

pub(in crate::phpx::typeck::check) fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int"
            | "integer"
            | "number"
            | "float"
            | "double"
            | "bool"
            | "boolean"
            | "string"
            | "null"
            | "array"
            | "object"
            | "mixed"
            | "void"
            | "never"
            | "false"
            | "true"
            | "iterable"
            | "callable"
            | "option"
            | "result"
    )
}

pub(in crate::phpx::typeck::check) fn remove_null(ty: &Type) -> Type {
    match ty {
        Type::Union(types) => {
            let mut out: Vec<Type> = types
                .iter()
                .filter(|t| !matches!(t, Type::Primitive(PrimitiveType::Null)))
                .cloned()
                .collect();
            if out.is_empty() {
                Type::Unknown
            } else if out.len() == 1 {
                out.remove(0)
            } else {
                Type::Union(out)
            }
        }
        Type::Primitive(PrimitiveType::Null) => Type::Unknown,
        _ => ty.clone(),
    }
}

pub(in crate::phpx::typeck::check) fn keep_only_null(ty: &Type) -> Type {
    match ty {
        Type::Primitive(PrimitiveType::Null) => Type::Primitive(PrimitiveType::Null),
        Type::Union(types) => {
            if types
                .iter()
                .any(|t| matches!(t, Type::Primitive(PrimitiveType::Null)))
            {
                Type::Primitive(PrimitiveType::Null)
            } else {
                ty.clone()
            }
        }
        _ => ty.clone(),
    }
}

pub(in crate::phpx::typeck::check) fn object_key_name(key: ObjectKey, source: &[u8]) -> String {
    match key {
        ObjectKey::Ident(token) => token_text(source, token.span),
        ObjectKey::String(token) => {
            let raw = token_text(source, token.span);
            parse_string_key(&raw)
        }
    }
}

// Keep in sync with `decode_string_key` in
// runtime/crates/deka_js/src/lib.rs and runtime/crates/deka_lsp/src/lib.rs.
// All three strip the matching quote pair and decode the same escape set on
// ObjectKey::String tokens.
pub(in crate::phpx::typeck::check) fn parse_string_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_string_key(inner, first == b'"');
        }
    }
    raw.to_string()
}

pub(in crate::phpx::typeck::check) fn unescape_string_key(
    value: &str,
    double_quoted: bool,
) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

pub(in crate::phpx::typeck::check) fn resolve_wasm_stub(
    specifier: &str,
    file_path: &Path,
    modules_root: &Path,
) -> Result<PathBuf, String> {
    let current_dir = file_path
        .parent()
        .ok_or_else(|| "wasm import requires a parent directory".to_string())?;
    let is_project_alias = specifier.starts_with("@/");
    let base_dir = if specifier.starts_with('.') {
        current_dir.to_path_buf()
    } else if is_project_alias {
        modules_root
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| "wasm import alias '@/...' requires project root".to_string())?
    } else {
        modules_root.to_path_buf()
    };

    let spec_path = if let Some(rest) = specifier.strip_prefix("@/") {
        rest
    } else {
        specifier
    };

    let root_path = normalize_path(base_dir.join(spec_path));
    let allowed_root = if is_project_alias {
        modules_root.parent().unwrap_or(modules_root)
    } else {
        modules_root
    };
    if !root_path.starts_with(allowed_root) {
        let scope = if is_project_alias {
            "project root"
        } else {
            "php_modules/"
        };
        return Err(format!(
            "wasm import must resolve inside {} ({}).",
            scope, specifier
        ));
    }

    let manifest_path = root_path.join("deka.json");
    if !manifest_path.is_file() {
        return Err(format!(
            "Missing wasm module manifest for '{}' (expected {}).",
            specifier,
            manifest_path.display()
        ));
    }

    let stub_spec = read_stub_path(&manifest_path)?;
    let stub_path = match stub_spec {
        Some(path) => {
            let stub_path = PathBuf::from(path);
            if stub_path.is_absolute() {
                stub_path
            } else {
                root_path.join(stub_path)
            }
        }
        None => root_path.join("module.d.phpx"),
    };

    if !stub_path.is_file() {
        return Err(format!(
            "Missing wasm type stubs for '{}' (expected {}).",
            specifier,
            stub_path.display()
        ));
    }

    Ok(stub_path)
}

pub(in crate::phpx::typeck::check) fn read_stub_path(
    manifest_path: &Path,
) -> Result<Option<String>, String> {
    let raw = fs::read_to_string(manifest_path).map_err(|err| {
        format!(
            "Failed to read wasm manifest {}: {}",
            manifest_path.display(),
            err
        )
    })?;
    let json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "Failed to parse wasm manifest {}: {}",
            manifest_path.display(),
            err
        )
    })?;
    let stubs = json
        .get("stubs")
        .and_then(|val| val.as_str())
        .or_else(|| json.get("stub").and_then(|val| val.as_str()));
    Ok(stubs.map(|value| value.to_string()))
}

pub(in crate::phpx::typeck::check) fn find_modules_root(file_path: &Path) -> Option<PathBuf> {
    let mut dir = file_path.parent()?;
    loop {
        if dir.file_name().and_then(|name| name.to_str()) == Some("php_modules") {
            return Some(dir.to_path_buf());
        }
        let candidate = dir.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    None
}

pub(in crate::phpx::typeck::check) fn normalize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
            Component::Normal(_) => out.push(component.as_os_str()),
        }
    }
    out
}

pub(in crate::phpx::typeck::check) fn path_has_php_modules_bridge(path: &Path) -> bool {
    let mut saw_php_modules = false;
    for component in path.components() {
        let Component::Normal(seg) = component else {
            continue;
        };
        let seg = seg.to_string_lossy();
        if !saw_php_modules {
            if seg == "php_modules" {
                saw_php_modules = true;
            }
            continue;
        }
        return seg == "core" || seg == "internals";
    }
    false
}
