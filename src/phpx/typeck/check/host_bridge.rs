use super::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Official stdlib packages that may declare `host.kinds` at their compile root.
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

fn result_bytes() -> Type {
    Type::Applied {
        base: "Result".to_string(),
        args: vec![Type::Primitive(PrimitiveType::Bytes), Type::Unknown],
    }
}

struct HostOp {
    kind: &'static str,
    action: &'static str,
    params: &'static [PrimitiveType],
    ret: fn() -> Type,
    is_async: bool,
}

const CATALOG: &[HostOp] = &[
    HostOp {
        kind: "crypto",
        action: "random_bytes",
        params: &[PrimitiveType::Number],
        ret: result_bytes,
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
    match package_name {
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

pub(in crate::phpx::typeck::check) fn workspace_grant(
    file_path: Option<&Path>,
    kind: &str,
) -> BridgeGrant {
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
        if !OFFICIAL_PACKAGES.contains(&manifest.name.as_str()) {
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

        match workspace_grant(self.file_path.as_deref(), &kind) {
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
