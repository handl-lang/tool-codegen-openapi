//! `openapi.yaml` configuration shared by the three directions.
//!
//! The file is optional — every field has a default. Paths inside it
//! (`module`, `openapi_lib`, `deps` values) are resolved relative to
//! the config file's directory, matching the webidl `includes:`
//! convention.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// OpenAPI backend configuration (`.sdkgen.yaml`-style).
#[derive(Debug, Default, Deserialize)]
pub struct OpenApiConfig {
    // -- spec direction: handl route map -> OpenAPI document ---------
    /// Handl module exporting the route map. Defaults to the codegen
    /// request's spec path (`hnl codegen openapi src/routes.hnl`).
    #[serde(default)]
    pub module: Option<String>,
    /// Export name carrying the route map / entry list.
    /// Default `"routes"`.
    #[serde(default)]
    pub export: Option<String>,
    /// `"routes"` (nested `{ name: leaf | subtree }` map, default) or
    /// `"entries"` (flat `[{ path, method, ... }]` list) — selects
    /// `openapi/1` vs `openapi_entries/1` in `lib-http-openapi`.
    #[serde(default)]
    pub mode: Option<String>,
    /// Path to the `lib-http-openapi` checkout (the dir containing
    /// `src/index.hnl`, or `src/` itself). Default: the sibling
    /// `../lib-http-openapi` next to this crate.
    #[serde(default)]
    pub openapi_lib: Option<String>,
    /// Extra `[dependencies]` entries for the generated probe's
    /// `.handl.toml` — name → package dir, exactly like a project's
    /// own manifest. Needed when the routes module imports packages
    /// outside the prelude (`http-router`, `fetch-c`, …). Values are
    /// resolved relative to the config file's directory.
    #[serde(default)]
    pub deps: HashMap<String, String>,
    /// `hnl` binary override. Default discovery: `$HANDL_HNL`,
    /// `~/.local/bin/hnl`, `~/.cargo/shared-target/debug/hnl`,
    /// `../host-cli/target/debug/hnl` (sibling checkout), then `hnl`
    /// on `PATH`.
    #[serde(default)]
    pub hnl: Option<String>,

    // -- OpenAPI `opts` passthrough (spec direction) ------------------
    /// Merged over `{ title: "API", version: "0.0.0" }` — becomes the
    /// document's `info`.
    #[serde(default)]
    pub info: Option<Value>,
    /// Passed through to the document's top-level `servers`.
    #[serde(default)]
    pub servers: Option<Value>,
    /// Passed through to the document's top-level `components`.
    #[serde(default)]
    pub components: Option<Value>,
    /// Verb list forwarded to `route_map` / used to expand `"*"`
    /// methods (openapi/1 only).
    #[serde(default)]
    pub methods: Option<Vec<String>>,
    /// `sep`/`leaf_keys`/`spec_keys` forwarded to `route_map`
    /// (openapi/1 only).
    #[serde(default)]
    pub sep: Option<String>,
    #[serde(default)]
    pub leaf_keys: Option<Vec<String>>,
    #[serde(default)]
    pub spec_keys: Option<Vec<String>>,

    // -- client direction ---------------------------------------------
    /// Base URL prepended to every generated client call
    /// (`base_url := "…"` in the emitted module).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Output module name for the client emitter
    /// (default `openapi_client`).
    #[serde(default)]
    pub client_module: Option<String>,
    /// Output module name for the server emitter
    /// (default `openapi_server`).
    #[serde(default)]
    pub server_module: Option<String>,
}

impl OpenApiConfig {
    /// Load an `openapi.yaml`-style config file. Absent file is *not*
    /// an error here — callers decide (the registry passes
    /// `req.config` only when `--config` was given; direct users may
    /// pass any path).
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config {}: {}", path.display(), e))?;
        serde_yaml::from_str(&s).map_err(|e| format!("failed to parse config: {e}"))
    }

    /// The config for a request: `--config` path when given, else
    /// defaults. Relative `module`/`openapi_lib`/`deps` values are
    /// re-based onto the config file's directory.
    pub fn for_request(config_path: Option<&Path>) -> Result<Self, String> {
        let mut cfg = match config_path {
            Some(p) => Self::load(p)?,
            None => Self::default(),
        };
        if let Some(dir) = config_path.and_then(Path::parent) {
            cfg.rebase(dir);
        }
        Ok(cfg)
    }

    /// Rewrite relative path-ish fields onto `base`.
    fn rebase(&mut self, base: &Path) {
        let join = |v: &Option<String>| -> Option<String> {
            v.as_ref().map(|s| {
                let p = Path::new(s);
                if p.is_absolute() {
                    s.clone()
                } else {
                    base.join(p).display().to_string()
                }
            })
        };
        self.module = join(&self.module);
        self.openapi_lib = join(&self.openapi_lib);
        self.deps = self
            .deps
            .iter()
            .map(|(k, v)| {
                let p = Path::new(v);
                let v = if p.is_absolute() {
                    v.clone()
                } else {
                    base.join(p).display().to_string()
                };
                (k.clone(), v)
            })
            .collect();
    }
}
