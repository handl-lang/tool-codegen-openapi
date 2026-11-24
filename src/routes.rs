//! Spec direction: a handl module exporting a route map (or flat
//! entry list) is evaluated through the `hnl` binary + the
//! `lib-http-openapi` transform (`openapi/1` / `openapi_entries/1`),
//! and the resulting OpenAPI 3.0.x document becomes the IR
//! ([`SpecModule`] with `format: "json"`).
//!
//! # Why a probe script
//!
//! `lib-http-openapi` is pure handl — the transform itself lives in
//! `.hnl` sources resolved through `.handl.toml` `[dependencies]`.
//! Rather than re-implementing the pattern bridging (`:param`,
//! `*splat`, `(/optional)`, `?k=v`, doc passthrough, operationId
//! dedup) in Rust, the source writes a tiny probe module plus a
//! `.handl.toml` into a scratch dir and runs `hnl run probe.hnl`.
//! `hnl run` prints the tail expression's value; a record value
//! serializes to pretty JSON on stdout, which is parsed back into
//! the IR. The probe is regenerated per parse — it is a build
//! artifact of the request, never committed.
//!
//! # Toolchain discovery
//!
//! `hnl` must provide the `run` command (the `ext-eval` cdylib).
//! Resolution order: `OpenApiConfig::hnl` / `$HANDL_HNL`, then
//! `~/.local/bin/hnl`, `~/.cargo/shared-target/debug/hnl`,
//! `../host-cli/target/debug/hnl` relative to this crate, then
//! `hnl` on `PATH`. `HANDL_EXT_DIR` is honoured by `hnl` itself.

use handl_plugin_codegen::{CodegenError, EmitContext, Source, SpecModule};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::OpenApiConfig;

/// Default timeout for one `hnl run` probe (typecheck + eval of the
/// routes module can be slow on first run — the language snapshot
/// cache amortizes it).
const PROBE_TIMEOUT: Duration = Duration::from_secs(300);

/// Process-global probe sequence — see [`OpenApiRoutesSource`].
static PROBE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `Source<SpecModule>` for the `openapi` direction: handl route map
/// → OpenAPI document.
pub struct OpenApiRoutesSource {
    config: OpenApiConfig,
    /// Scratch root for probe dirs; `None` = `std::env::temp_dir()`.
    /// Exposed for tests that want a deterministic location.
    probe_root: Option<PathBuf>,
}

impl OpenApiRoutesSource {
    pub fn new(config: OpenApiConfig) -> Self {
        Self {
            config,
            probe_root: None,
        }
    }

    /// Override the scratch root (tests).
    pub fn with_probe_root(mut self, dir: impl Into<PathBuf>) -> Self {
        self.probe_root = Some(dir.into());
        self
    }

    /// Convenience wrapper mirroring `Source::parse` for direct/test
    /// use: parse the module at `path`.
    pub fn parse_module(&self, path: &Path) -> Result<SpecModule, CodegenError> {
        self.parse(&EmitContext::new().with_spec_path(path))
    }
}

impl Source<SpecModule> for OpenApiRoutesSource {
    fn parse(&self, req: &EmitContext) -> Result<SpecModule, CodegenError> {
        let module_path = self
            .config
            .module
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| req.spec_path.clone())
            .ok_or_else(|| {
                CodegenError::Parse(
                    "openapi source requires a routes module: pass the .hnl \
                     path as the spec argument or set `module:` in the config"
                        .to_string(),
                )
            })?;
        let module_path = if module_path.is_absolute() {
            module_path
        } else {
            // Relative `module:` with no config dir to rebase onto —
            // resolve against the process cwd.
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(module_path)
        };
        if !module_path.is_file() {
            return Err(CodegenError::Parse(format!(
                "routes module not found: {}",
                module_path.display()
            )));
        }

        let hnl = find_hnl(self.config.hnl.as_deref())?;
        let openapi_lib = resolve_openapi_lib(self.config.openapi_lib.as_deref())?;
        let export = self
            .config
            .export
            .clone()
            .unwrap_or_else(|| "routes".to_string());
        let mode = self.config.mode.as_deref().unwrap_or("routes");
        let opts = opts_literal(&self.config);
        let call = match mode {
            "routes" => format!("oa.openapi({export}, {opts})"),
            "entries" => format!("oa.openapi_entries({export}, {opts})"),
            other => {
                return Err(CodegenError::Parse(format!(
                    "openapi mode must be \"routes\" or \"entries\", got {other:?}"
                )));
            }
        };

        // Scratch dir: `<root>/hnl-openapi-<pid>-<seq>` holding
        // `.handl.toml` + `probe.hnl`. Not journaled — it is request
        // scratch, like the MDN cache webidl keeps outside the tree.
        let seq = PROBE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = self.probe_root.clone().unwrap_or_else(std::env::temp_dir);
        let dir = root.join(format!("hnl-openapi-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| {
            CodegenError::Parse(format!("failed to create probe dir {}: {e}", dir.display()))
        })?;

        let manifest = probe_manifest(&openapi_lib, &self.config.deps);
        std::fs::write(dir.join(".handl.toml"), manifest)
            .map_err(|e| CodegenError::Parse(format!("failed to write probe manifest: {e}")))?;
        let probe = format!(
            "--- generated by handl-plugin-codegen-backend-openapi — do not edit\n\
             from \"http-openapi\" import * as oa\n\
             from \"{}\" import *\n\n\
             {}\n",
            module_path.display(),
            call,
        );
        std::fs::write(dir.join("probe.hnl"), probe)
            .map_err(|e| CodegenError::Parse(format!("failed to write probe script: {e}")))?;

        let stdout = run_hnl(&hnl, &dir)?;
        let trimmed = stdout.trim();
        let document: Value = serde_json::from_str(trimmed).map_err(|e| {
            CodegenError::Parse(format!(
                "openapi probe did not produce a JSON document: {e}\n\
                 --- hnl stdout (first 2 KiB) ---\n{}",
                &trimmed[..trimmed.len().min(2048)]
            ))
        })?;
        if !document.is_object() {
            return Err(CodegenError::Parse(
                "openapi probe produced a non-object document".to_string(),
            ));
        }

        let name = module_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("routes")
            .to_string();
        Ok(SpecModule {
            format: "json".to_string(),
            name,
            document,
            source: Some(module_path.display().to_string()),
            diagnostics: Vec::new(),
        })
    }
}

// ------------------------------------------------------------------
// toolchain + lib discovery
// ------------------------------------------------------------------

/// Candidate `hnl` binaries, in order.
fn hnl_candidates(configured: Option<&str>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(h) = configured {
        out.push(PathBuf::from(h));
    }
    if let Ok(h) = std::env::var("HANDL_HNL") {
        out.push(PathBuf::from(h));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        out.push(home.join(".local/bin/hnl"));
        out.push(home.join(".cargo/shared-target/debug/hnl"));
    }
    // Sibling `host-cli` checkout, relative to this crate.
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../host-cli/target/debug/hnl"));
    out.push(PathBuf::from("hnl")); // PATH fallback
    out
}

/// Locate an executable `hnl`. Exposed for tests/tooling.
pub fn find_hnl(configured: Option<&str>) -> Result<PathBuf, CodegenError> {
    for cand in hnl_candidates(configured) {
        if cand == Path::new("hnl") {
            // Bare name — check PATH via `hnl version`? Cheaper: trust
            // `Command::new` to resolve; verify by spawning below.
            return Ok(cand);
        }
        if cand.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if cand
                    .metadata()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    return Ok(cand);
                }
            }
            #[cfg(not(unix))]
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    Err(CodegenError::Parse(
        "hnl binary not found — set `hnl:` in openapi.yaml, $HANDL_HNL, \
         or install hnl (host-cli) with the ext-eval cdylib for `hnl run`"
            .to_string(),
    ))
}

/// Resolve the `lib-http-openapi` package dir: accepts the repo root
/// (`…/lib-http-openapi`, containing `src/index.hnl`), the `src/`
/// dir itself, or `src/index.hnl`. Returns the directory that holds
/// `index.hnl` — the `[dependencies]` path form `.handl.toml` uses.
fn resolve_openapi_lib(configured: Option<&str>) -> Result<PathBuf, CodegenError> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured {
        candidates.push(PathBuf::from(p));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../lib-http-openapi"));
    for cand in candidates {
        for dir in [
            cand.join("src"),
            cand.clone(),
            cand.join("src/index.hnl")
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
        ] {
            if dir.join("index.hnl").is_file() {
                // Canonicalize so sibling resolution and the emitted
                // manifest carry clean absolute paths (no `..`).
                return Ok(dir.canonicalize().unwrap_or(dir));
            }
        }
        // A direct path to index.hnl.
        if cand.is_file()
            && cand.file_name().is_some_and(|n| n == "index.hnl")
            && let Some(parent) = cand.parent()
        {
            return Ok(parent.to_path_buf());
        }
    }
    Err(CodegenError::Parse(
        "lib-http-openapi not found — set `openapi_lib:` in openapi.yaml \
         to the lib-http-openapi checkout"
            .to_string(),
    ))
}

/// The `route` and `json` dependency dirs, derived as siblings of the
/// openapi lib (`../lib-route/src`, `../lib-json/src` relative to the
/// lib repo root). `json` is usually prelude-resolvable, but pinning
/// it explicitly keeps the probe hermetic when the prelude's `json`
/// is the codec token rather than the stringify/parse module.
fn sibling_lib(lib_repo_root: &Path, name: &str) -> Option<PathBuf> {
    let dir = lib_repo_root.parent()?.join(name).join("src");
    dir.join("index.hnl").is_file().then_some(dir)
}

/// Build the probe's `.handl.toml` `[dependencies]` table.
///
/// `openapi_lib` is the dir containing `index.hnl`; its repo root is
/// the parent of `src/`. `route`/`json` siblings are added when they
/// exist on disk; `config.deps` entries override/add arbitrary names.
fn probe_manifest(openapi_lib: &Path, extra: &HashMap<String, String>) -> String {
    let mut deps: BTreeMap<String, String> = BTreeMap::new();
    // openapi_lib may be `<repo>/src` or `<repo>` — normalise to the
    // dir whose parent is the repo root when it is named `src`.
    let repo_root = if openapi_lib.file_name().is_some_and(|n| n == "src") {
        openapi_lib.parent().unwrap_or(openapi_lib)
    } else {
        openapi_lib
    };
    deps.insert(
        "http-openapi".to_string(),
        openapi_lib.display().to_string(),
    );
    for name in ["lib-route", "lib-json"] {
        if let Some(dir) = sibling_lib(repo_root, name) {
            // The import name drops the `lib-` prefix.
            let import_name = name.strip_prefix("lib-").unwrap_or(name);
            deps.insert(import_name.to_string(), dir.display().to_string());
        }
    }
    for (k, v) in extra {
        deps.insert(k.clone(), v.clone());
    }
    // `deps` is a HashMap — sort for a deterministic manifest.
    let deps: BTreeMap<_, _> = deps.into_iter().collect();
    let mut out = String::from("[dependencies]\n");
    for (k, v) in deps {
        out.push_str(&format!(
            "{k} = {{ path = \"{}\" }}\n",
            v.replace('\\', "\\\\")
        ));
    }
    out
}

// ------------------------------------------------------------------
// hnl invocation
// ------------------------------------------------------------------

/// Run `hnl run probe.hnl` inside `dir`; return stdout. stderr is
/// folded into the error on failure.
fn run_hnl(hnl: &Path, dir: &Path) -> Result<String, CodegenError> {
    let probe = dir.join("probe.hnl");
    let started = Instant::now();
    let mut child = Command::new(hnl)
        .arg("run")
        .arg(&probe)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodegenError::Parse(format!("failed to spawn `{}`: {e}", hnl.display())))?;

    let deadline = started + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CodegenError::Parse(format!(
                        "hnl run probe timed out after {}s",
                        PROBE_TIMEOUT.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(CodegenError::Parse(format!("hnl run wait failed: {e}")));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| CodegenError::Parse(format!("failed to read hnl output: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CodegenError::Parse(format!(
        "hnl run failed (exit {:?}) in {}\n{}",
        output.status.code(),
        dir.display(),
        stderr.trim()
    )))
}

// ------------------------------------------------------------------
// opts literal
// ------------------------------------------------------------------

/// The `opts` record passed as the second argument to
/// `openapi`/`openapi_entries`, rendered as a handl record literal.
/// Only config-declared keys are emitted.
fn opts_literal(config: &OpenApiConfig) -> String {
    let mut m = Map::new();
    if let Some(v) = &config.info {
        m.insert("info".into(), v.clone());
    }
    if let Some(v) = &config.servers {
        m.insert("servers".into(), v.clone());
    }
    if let Some(v) = &config.components {
        m.insert("components".into(), v.clone());
    }
    if let Some(v) = &config.methods {
        m.insert(
            "methods".into(),
            Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    if let Some(v) = &config.sep {
        m.insert("sep".into(), Value::String(v.clone()));
    }
    if let Some(v) = &config.leaf_keys {
        m.insert(
            "leaf_keys".into(),
            Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    if let Some(v) = &config.spec_keys {
        m.insert(
            "spec_keys".into(),
            Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
    value_to_hnl(&Value::Object(m))
}

/// Render a JSON value as a handl literal — shared by the probe opts
/// record and the server emitter's `doc` annotations.
///
/// Records emit `"key": value` pairs (quoted keys are valid handl
/// record syntax — `lib-fetch` uses them); strings/numbers/bools/
/// null/arrays map directly. Anything else is a bug in the caller.
pub(crate) fn value_to_hnl(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => hnl_string(s),
        Value::Array(xs) => {
            let inner: Vec<String> = xs.iter().map(value_to_hnl).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Object(m) => {
            let inner: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", hnl_string(k), value_to_hnl(v)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
    }
}

/// A handl double-quoted string literal body.
pub(crate) fn hnl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
