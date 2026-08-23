use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Official stdlib packages that may declare `host.kinds` at their compile root.
/// Published names may be `@deka/<name>`; the grant uses the bare name.
const OFFICIAL_PACKAGES: &[&str] = &[
    "crypto",
    "fs",
    "tcp",
    "tls",
    "db",
    "redis",
    "neo4j",
    "time",
    "concurrency",
    "vault",
];

fn canonical_stdlib_name(name: &str) -> &str {
    name.strip_prefix("@deka/").unwrap_or(name)
}

fn result_bytes() -> Type {
    Type::Applied {
        base: "Result".to_string(),
        args: vec![Type::Primitive(PrimitiveType::Bytes), Type::Unknown],
    }
}

fn result_bool() -> Type {
    Type::Applied {
        base: "Result".to_string(),
        args: vec![Type::Primitive(PrimitiveType::Bool), Type::Unknown],
    }
}

struct HostOp {
    kind: &'static str,
    action: &'static str,
    params: &'static [PrimitiveType],
    ret: fn() -> Type,
    is_async: bool,
}

/// Keep in sync with isolate `__deka_host` allowlist
/// (`crates/pool/src/isolate_pool/worker_execution.rs`).
const CATALOG: &[HostOp] = &[
    HostOp {
        kind: "crypto",
        action: "random_bytes",
        params: &[PrimitiveType::Number],
        ret: result_bytes,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "digest",
        params: &[PrimitiveType::String, PrimitiveType::Bytes],
        ret: result_bytes,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "hmac",
        params: &[
            PrimitiveType::String,
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
        ],
        ret: result_bytes,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "secure_compare",
        params: &[PrimitiveType::Bytes, PrimitiveType::Bytes],
        ret: result_bool,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "aes_256_gcm_encrypt",
        params: &[
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
        ],
        ret: result_bytes,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "aes_256_gcm_decrypt",
        params: &[
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
            PrimitiveType::Bytes,
        ],
        ret: result_bytes,
        is_async: false,
    },
    HostOp {
        kind: "crypto",
        action: "bcrypt_verify",
        params: &[PrimitiveType::String, PrimitiveType::String],
        ret: result_bool,
        is_async: false,
    },
    HostOp {
        kind: "fs",
        action: "read_file",
        params: &[PrimitiveType::String],
        ret: result_bytes,
        is_async: true,
    },
];

fn lookup_op(kind: &str, action: &str) -> Option<&'static HostOp> {
    CATALOG
        .iter()
        .find(|op| op.kind == kind && op.action == action)
}

fn official_kinds_for(package_name: &str) -> Option<&'static [&'static str]> {
    match canonical_stdlib_name(package_name) {
        "crypto" => Some(&["crypto"]),
        "fs" => Some(&["fs"]),
        "tcp" => Some(&["net"]),
        "tls" => Some(&["tls"]),
        "db" => Some(&["db"]),
        "redis" => Some(&["redis"]),
        "neo4j" => Some(&["neo4j"]),
        "time" => Some(&["time"]),
        "concurrency" => Some(&["concurrency"]),
        "vault" => Some(&["vault"]),
        _ => None,
    }
}

struct PackageManifest {
    path: PathBuf,
    name: String,
    host_kinds: Option<Vec<String>>,
}

fn parse_manifest(path: &Path) -> Option<PackageManifest> {
    let raw = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let name = json.get("name")?.as_str()?.to_string();
    let host_kinds = json
        .get("host")
        .and_then(|host| host.get("kinds"))
        .and_then(|kinds| kinds.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        });
    Some(PackageManifest {
        path: path.to_path_buf(),
        name,
        host_kinds,
    })
}

fn nearest_deka_json(file_path: &Path) -> Option<PathBuf> {
    let mut dir = file_path.parent()?;
    loop {
        let candidate = dir.join("deka.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

fn ancestor_deka_json(from_dir: &Path) -> Option<PathBuf> {
    let mut dir = from_dir.parent()?;
    loop {
        let candidate = dir.join("deka.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

fn under_modules_dir(file_path: &Path) -> bool {
    file_path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|name| name == "ds_modules" || name == "php_modules")
    })
}

pub(in crate::phpx::typeck::check) enum BridgeGrant {
    Allowed,
    Denied { message: String },
}

pub(in crate::phpx::typeck::check) fn resolve_grant(
    file_path: Option<&Path>,
    kind: &str,
) -> BridgeGrant {
    match workspace_grant(file_path, kind) {
        BridgeGrant::Allowed => BridgeGrant::Allowed,
        workspace_denied => match digest_grant(file_path, kind) {
            BridgeGrant::Allowed => BridgeGrant::Allowed,
            digest_denied => {
                if file_path.is_some_and(under_modules_dir) {
                    digest_denied
                } else {
                    workspace_denied
                }
            }
        },
    }
}

fn workspace_grant(file_path: Option<&Path>, kind: &str) -> BridgeGrant {
    let Some(file_path) = file_path else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    let Some(manifest_path) = nearest_deka_json(file_path) else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    let Some(manifest) = parse_manifest(&manifest_path) else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };

    let is_workspace_root = ancestor_deka_json(manifest_path.parent().unwrap_or(&manifest.path))
        .is_none()
        && !under_modules_dir(file_path);

    if let Some(kinds) = &manifest.host_kinds {
        if !is_workspace_root {
            return BridgeGrant::Denied {
                message: "bridge is only allowed in a host-granted stdlib package".to_string(),
            };
        }
        if !OFFICIAL_PACKAGES.contains(&canonical_stdlib_name(&manifest.name)) {
            return BridgeGrant::Denied {
                message: "host.kinds is only valid on an official stdlib package".to_string(),
            };
        }
        if !kinds.iter().any(|k| k == kind) {
            return BridgeGrant::Denied {
                message: format!(
                    "host grant for {} does not include kind {}",
                    manifest.name, kind
                ),
            };
        }
        if let Some(allowed) = official_kinds_for(&manifest.name) {
            if !allowed.contains(&kind) {
                return BridgeGrant::Denied {
                    message: format!(
                        "host grant for {} does not include kind {}",
                        manifest.name, kind
                    ),
                };
            }
        }
        return BridgeGrant::Allowed;
    }

    BridgeGrant::Denied {
        message: "bridge is only allowed in a host-granted stdlib package".to_string(),
    }
}

fn catalog_type(kind: &str, action: &str) -> Result<(&'static HostOp, Type), String> {
    let Some(op) = lookup_op(kind, action) else {
        if CATALOG.iter().any(|op| op.kind == kind) {
            return Err(format!("unknown bridge action '{kind}.{action}'"));
        }
        return Err(format!("unknown bridge kind '{kind}'"));
    };
    let mut ret = (op.ret)();
    if op.is_async {
        ret = Type::Applied {
            base: "Promise".to_string(),
            args: vec![ret],
        };
    }
    Ok((op, ret))
}

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn check_bridge_expr(
        &mut self,
        kind: &[u8],
        action: &[u8],
        args: &'a [crate::parser::ast::Arg<'a>],
        span: crate::parser::span::Span,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        mut_env: &mut HashSet<String>,
    ) -> Type {
        let kind = String::from_utf8_lossy(kind).into_owned();
        let action = String::from_utf8_lossy(action).into_owned();

        let (op, ret) = match catalog_type(&kind, &action) {
            Ok(found) => found,
            Err(message) => {
                self.errors.push(TypeError {
                    severity: Severity::Error,
                    span,
                    message,
                });
                return Type::Unknown;
            }
        };

        match resolve_grant(self.file_path.as_deref(), &kind) {
            BridgeGrant::Allowed => {}
            BridgeGrant::Denied { message } => {
                self.errors.push(TypeError {
                    severity: Severity::Error,
                    span,
                    message,
                });
                return Type::Unknown;
            }
        }

        if args.len() != op.params.len() {
            self.errors.push(TypeError {
                severity: Severity::Error,
                span,
                message: format!(
                    "bridge {kind}.{action} expects {} argument(s), got {}",
                    op.params.len(),
                    args.len()
                ),
            });
        }

        for (i, arg) in args.iter().enumerate() {
            let got = self.check_expr(arg.value, env, explicit, mut_env);
            if let Some(expected) = op.params.get(i) {
                let expected_ty = Type::Primitive(expected.clone());
                if !matches!(got, Type::Unknown) && got != expected_ty {
                    // number literals are Number; allow Unknown from incomplete checking
                    if !is_assignable_bridge(&got, &expected_ty) {
                        self.errors.push(TypeError {
                            severity: Severity::Error,
                            span: arg.span,
                            message: format!(
                                "bridge {kind}.{action} argument {} has type {}, expected {}",
                                i + 1,
                                got.name(),
                                expected_ty.name()
                            ),
                        });
                    }
                }
            }
        }

        ret
    }
}

fn is_assignable_bridge(got: &Type, expected: &Type) -> bool {
    got == expected || matches!(got, Type::Unknown)
}

#[derive(Debug, Clone)]
struct LoadedGrant {
    digest: String,
    kinds: Vec<String>,
}

fn digest_grant(file_path: Option<&Path>, kind: &str) -> BridgeGrant {
    let Some(file_path) = file_path else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    if !under_modules_dir(file_path) {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    }
    let Some(manifest_path) = nearest_deka_json(file_path) else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    let Some(root) = manifest_path.parent() else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    let Ok(digest) = package_fs_digest(root) else {
        return BridgeGrant::Denied {
            message: "bridge is only allowed in a host-granted stdlib package".to_string(),
        };
    };
    let grants = load_grant_table_from(Some(file_path));
    let package_name = parse_manifest(&manifest_path)
        .map(|m| m.name)
        .unwrap_or_else(|| "package".to_string());
    let Some(grant) = grants.iter().find(|g| g.digest == digest) else {
        return BridgeGrant::Denied {
            message: format!("package {package_name} digest does not match a host grant"),
        };
    };
    if !grant.kinds.iter().any(|k| k == kind) {
        return BridgeGrant::Denied {
            message: format!("host grant does not include kind {kind}"),
        };
    }
    BridgeGrant::Allowed
}

fn load_grant_table_from(file_path: Option<&Path>) -> Vec<LoadedGrant> {
    let mut grants = Vec::new();
    if let Ok(raw) = std::env::var("DEKA_HOST_GRANTS") {
        grants.extend(parse_grant_json(&raw));
    }
    if let Ok(path) = std::env::var("DEKA_HOST_GRANTS_FILE") {
        if let Ok(raw) = fs::read_to_string(path) {
            grants.extend(parse_grant_json(&raw));
        }
    }
    if let Some(file_path) = file_path {
        if let Some(path) = nearest_host_grants_file(file_path) {
            if let Ok(raw) = fs::read_to_string(path) {
                grants.extend(parse_grant_json(&raw));
            }
        }
    }
    grants
}

fn nearest_host_grants_file(file_path: &Path) -> Option<PathBuf> {
    let mut dir = file_path.parent()?;
    loop {
        let candidate = dir.join("host-grants.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

fn parse_grant_json(raw: &str) -> Vec<LoadedGrant> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let digest = item.get("digest")?.as_str()?.trim().to_string();
            if digest.is_empty() {
                return None;
            }
            let kinds = item
                .get("kinds")?
                .as_array()?
                .iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>();
            if kinds.is_empty() {
                return None;
            }
            Some(LoadedGrant { digest, kinds })
        })
        .collect()
}

/// Filesystem-graph digest, same shape as `modules_php::integrity` fs_graph.
pub(crate) fn package_fs_digest(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "failed to normalize integrity path")?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        hasher.update(rel_str.as_bytes());
        hasher.update(b"\0");

        let mut file = File::open(&path)
            .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
        let mut buf = [0u8; 8192];
        loop {
            let read = file
                .read(&mut buf)
                .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        hasher.update(b"\n");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|err| format!("failed to read {}: {}", current.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read entry: {}", err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read entry type: {}", err))?;
        if file_type.is_dir() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel_str == ".git"
                || rel_str == ".cache"
                || rel_str == "node_modules"
                || rel_str == "target"
                || rel_str.starts_with(".git/")
                || rel_str.starts_with(".cache/")
                || rel_str.starts_with("node_modules/")
                || rel_str.starts_with("target/")
            {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel_str != ".DS_Store" {
                out.push(path);
            }
        }
    }
    Ok(())
}
