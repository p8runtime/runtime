use anyhow::{Context, anyhow};
use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    net::SocketAddr,
    path::{Component, Path as StdPath, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::spawn_blocking;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::I32Exit;
use wasmtime_wasi::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

#[derive(Clone)]
struct AppState {
    doc_root: PathBuf,
    mode: ExecutionMode,
}
#[derive(Debug, Error)]
enum AppError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("execution failed: {0}")]
    Execution(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Execution(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

#[derive(Debug)]
struct ExecutionResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

#[derive(Clone)]
struct WasmRuntime {
    engine: Engine,
}

#[derive(Clone)]
enum ExecutionMode {
    Native {
        binary: PathBuf,
    },
    Wasm {
        wasm_path: PathBuf,
        runtime: Arc<WasmRuntime>,
    },
}

impl WasmRuntime {
    fn new() -> anyhow::Result<Self> {
        let config = Config::new();
        let engine = Engine::new(&config)?;
        Ok(Self { engine })
    }

    fn execute(
        &self,
        module_path: &std::path::Path,
        script_path: &std::path::Path,
        doc_root: &std::path::Path,
    ) -> anyhow::Result<ExecutionResult> {
        let wasm_bytes = std::fs::read(module_path)
            .with_context(|| format!("Failed to read WASM module {}", module_path.display()))?;

        let module = Module::from_binary(&self.engine, &wasm_bytes)
            .context("Failed to compile WASM module")?;

        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |ctx: &mut WasiP1Ctx| ctx)?;

        let program_name = module_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("php");

        let relative_script = script_path
            .strip_prefix(doc_root)
            .unwrap_or(script_path)
            .to_string_lossy()
            .to_string();

        let mut wasm_args = vec![program_name.to_string()];
        wasm_args.push(relative_script.clone());

        let stdout_pipe = MemoryOutputPipe::new(1024 * 1024);
        let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

        let mut builder = WasiCtxBuilder::new();
        builder.stdout(stdout_pipe.clone());
        builder.stderr(stderr_pipe.clone());
        let stdin_pipe = MemoryInputPipe::new(Vec::new());
        builder.stdin(stdin_pipe);

        for arg in &wasm_args {
            builder.arg(arg);
        }

        builder.env("SCRIPT_FILENAME", &script_path.to_string_lossy());
        builder.env("REQUEST_METHOD", "GET");
        builder.env("REDIRECT_STATUS", "200");
        builder.env("GATEWAY_INTERFACE", "CGI/1.1");
        builder.env("SERVER_PROTOCOL", "HTTP/1.1");
        builder.env("SERVER_SOFTWARE", "php-router");
        builder.env("QUERY_STRING", "");
        builder.env("PATH_INFO", "");
        builder.env("PATH_TRANSLATED", "");

        builder.preopened_dir(doc_root, ".", DirPerms::all(), FilePerms::all())?;
        builder.preopened_dir(
            std::env::temp_dir(),
            "/tmp",
            DirPerms::all(),
            FilePerms::all(),
        )?;

        let wasi = builder.build_p1();

        let mut store = Store::new(&self.engine, wasi);
        let instance = linker.instantiate(&mut store, &module)?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("Failed to find _start function")?;

        let result = start.call(&mut store, ());

        let exit_code = match result {
            Ok(_) => 0,
            Err(e) => {
                if let Some(exit) = e.downcast_ref::<I32Exit>() {
                    exit.0
                } else {
                    warn!("wasmtime execution failed: {:?}", e);
                    1
                }
            }
        };

        let stdout =
            String::from_utf8_lossy(&stdout_pipe.try_into_inner().unwrap_or_default()).to_string();
        let stderr =
            String::from_utf8_lossy(&stderr_pipe.try_into_inner().unwrap_or_default()).to_string();

        Ok(ExecutionResult {
            stdout,
            stderr,
            exit_code,
        })
    }
}

impl ExecutionMode {
    fn execute(
        &self,
        script_path: &std::path::Path,
        doc_root: &std::path::Path,
    ) -> anyhow::Result<ExecutionResult> {
        match self {
            ExecutionMode::Native { binary } => execute_native(binary, script_path, doc_root),
            ExecutionMode::Wasm { wasm_path, runtime } => {
                runtime.execute(wasm_path, script_path, doc_root)
            }
        }
    }
}

fn execute_native(
    binary: &StdPath,
    script_path: &StdPath,
    doc_root: &StdPath,
) -> anyhow::Result<ExecutionResult> {
    let mut cmd = Command::new(binary);
    cmd.arg(script_path)
        .current_dir(doc_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    configure_cgi_env(&mut cmd, script_path);

    let output = cmd.output().context(format!(
        "Failed to run native PHP binary: {}",
        binary.display()
    ))?;

    let exit_code = output
        .status
        .code()
        .unwrap_or_else(|| if output.status.success() { 0 } else { 1 });

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(ExecutionResult {
        stdout,
        stderr,
        exit_code,
    })
}

fn configure_cgi_env(cmd: &mut Command, script_path: &StdPath) {
    cmd.env("SCRIPT_FILENAME", script_path.to_string_lossy().to_string());
    cmd.env("REQUEST_METHOD", "GET");
    cmd.env("REDIRECT_STATUS", "200");
    cmd.env("GATEWAY_INTERFACE", "CGI/1.1");
    cmd.env("SERVER_PROTOCOL", "HTTP/1.1");
    cmd.env("SERVER_SOFTWARE", "php-router");
    cmd.env("QUERY_STRING", "");
    cmd.env("PATH_INFO", "");
    cmd.env("PATH_TRANSLATED", "");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let doc_root = std::env::var("PHP_ROUTER_DOC_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap().join("tests/php/test-001"));
    let doc_root = std::fs::canonicalize(&doc_root)
        .with_context(|| format!("Document root not found: {}", doc_root.display()))?;

    let current_dir = std::env::current_dir().unwrap_or_else(|_| doc_root.clone());
    let mode = resolve_router_mode(&current_dir)?;

    let state = Arc::new(AppState { doc_root, mode });

    let app = Router::new()
        .route("/health", get(health))
        .route("/*file", get(handle_file_route))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8541));
    info!("Starting php-router on http://{}", addr);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind router port: {}", e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    Ok(())
}

async fn health() -> impl IntoResponse {
    "php router ready"
}

async fn handle_file_route(
    State(state): State<Arc<AppState>>,
    Path(file_path): Path<String>,
) -> Result<Response, AppError> {
    let target = if file_path.is_empty() {
        "index.php".to_string()
    } else {
        file_path
    };
    execute_script(state, target).await
}

async fn execute_script(state: Arc<AppState>, incoming: String) -> Result<Response, AppError> {
    let path = PathBuf::from(incoming);

    if path.is_absolute() {
        return Err(AppError::BadRequest(
            "absolute paths are not allowed".into(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AppError::BadRequest(
            "parent directory access forbidden".into(),
        ));
    }

    let full_path = state.doc_root.join(&path);
    let canonical_path = full_path
        .canonicalize()
        .map_err(|_| AppError::NotFound(path.to_string_lossy().into_owned()))?;

    if !canonical_path.starts_with(&state.doc_root) {
        return Err(AppError::Forbidden("outside document root".into()));
    }
    if !canonical_path.exists() {
        return Err(AppError::NotFound(path.to_string_lossy().into_owned()));
    }

    if canonical_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        != "php"
    {
        return Err(AppError::BadRequest("only .php files are supported".into()));
    }

    let mode = state.mode.clone();
    let doc_root = state.doc_root.clone();

    let exec_result = spawn_blocking(move || mode.execute(&canonical_path, &doc_root))
        .await
        .map_err(|e| AppError::Execution(format!("task join error: {}", e)))?
        .map_err(|e| AppError::Execution(e.to_string()))?;

    if !exec_result.stderr.is_empty() {
        warn!("execution stderr: {}", exec_result.stderr);
    }

    let status = if exec_result.exit_code == 0 {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    let filtered_output = sanitize_response_output(&exec_result.stdout);
    let body = filtered_output.into_bytes();
    let response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(body.into())
        .map_err(|e| AppError::Execution(format!("failed to build response: {}", e)))?;

    Ok(response)
}

fn resolve_router_mode(current_dir: &StdPath) -> anyhow::Result<ExecutionMode> {
    let mode = std::env::var("PHP_ROUTER_MODE").unwrap_or_else(|_| "native".into());
    match mode.to_ascii_lowercase().as_str() {
        "native" | "" => {
            let binary = resolve_native_binary(current_dir)?;
            Ok(ExecutionMode::Native { binary })
        }
        "wasm" => {
            let wasm_path = resolve_wasm_module(current_dir)?;
            let runtime = Arc::new(WasmRuntime::new()?);
            Ok(ExecutionMode::Wasm { wasm_path, runtime })
        }
        other => Err(anyhow!("Unknown PHP_ROUTER_MODE: {}", other)),
    }
}

fn resolve_native_binary(current_dir: &StdPath) -> anyhow::Result<PathBuf> {
    let release = current_dir.join("target/release/php");
    let debug = current_dir.join("target/debug/php");

    let candidate = if let Ok(var) = std::env::var("PHP_NATIVE_BIN") {
        PathBuf::from(var)
    } else if release.exists() {
        release
    } else if debug.exists() {
        debug
    } else {
        return Err(anyhow!(
            "Native php binary not found; build `cargo build --bin php` or set PHP_NATIVE_BIN"
        ));
    };

    std::fs::canonicalize(&candidate)
        .with_context(|| format!("Native PHP binary not found: {}", candidate.display()))
}

fn resolve_wasm_module(current_dir: &StdPath) -> anyhow::Result<PathBuf> {
    let release = current_dir.join("target/wasm32-wasip1/release/php-wasm.wasm");
    let debug = current_dir.join("target/wasm32-wasip1/debug/php-wasm.wasm");

    let candidate = if let Ok(var) = std::env::var("PHP_WASM_PATH") {
        PathBuf::from(var)
    } else if release.exists() {
        release
    } else if debug.exists() {
        debug
    } else {
        return Err(anyhow!(
            "WASM module not found; run `cargo build --bin php-wasm --features wasm-target --release`"
        ));
    };

    std::fs::canonicalize(&candidate)
        .with_context(|| format!("WASM module not found: {}", candidate.display()))
}

fn sanitize_response_output(output: &str) -> String {
    let mut lines = Vec::new();
    for line in output.lines() {
        if line.starts_with("[PthreadsExtension]") {
            continue;
        }
        lines.push(line);
    }

    let mut result = lines.join("\n");
    if output.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}
