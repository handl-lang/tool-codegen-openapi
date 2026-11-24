//! `hnl-openapi-client` emitter: OpenAPI document → handl client
//! bindings over `fetch-c`.
//!
//! One artifact `<client_module>.hnl` (default `openapi_client.hnl`)
//! exporting one arrow per operation:
//!
//! ```hnl
//! export get_user := (params, body) ~> do { ... fetch(url, init) }
//! export get_user_json := (params, body) ~> do { ... parse(r.body) }
//! ```
//!
//! - Path templates `{id}` are substituted from `params` via
//!   `str.replace` + `type.to_string`.
//! - Declared `in: query` parameters append `?k=v&…` (present-only;
//!   absent params are skipped via `hash_map.has`).
//! - A `requestBody` becomes `init.body = stringify(body)` with a
//!   JSON content-type header.
//! - `<name>_json` variants are emitted for operations with a 2xx
//!   JSON response and decode `r.body` via `json.parse`.
//!
//! The emitted module needs `fetch-c`, `json`, `str` resolvable —
//! `json`/`str` are prelude names; `fetch-c` must be added to the
//! project's `.handl.toml` `[dependencies]` (noted in the header).

use handl_plugin_codegen::{Change, CodegenError, EmitContext, Emitter, SpecModule};
use serde_json::Value;
use std::fmt::Write as _;

use crate::config::OpenApiConfig;
use crate::routes::hnl_string;
use crate::spec::{self, Operation};

/// Emitter rendering an OpenAPI [`SpecModule`] to a `.hnl` client.
pub struct OpenApiClientEmitter;

impl OpenApiClientEmitter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenApiClientEmitter {
    fn default() -> Self {
        Self::new()
    }
}

/// `params.<name>` vs `hash_map.get_or` — record field access only
/// works for identifier-shaped keys.
fn param_access(name: &str) -> String {
    let ident_ok = name
        .chars()
        .enumerate()
        .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if ident_ok {
        format!("params.{name}")
    } else {
        format!("hash_map.get_or(params, {}, null)", hnl_string(name))
    }
}

/// The path-expression: template with `{p}` → `str.replace(…, "{p}",
/// str_of(<access>))` applied inside-out.
fn path_expr(op: &Operation<'_>, (path_params, _): (&[&Value], &[&Value])) -> String {
    let mut expr = hnl_string(op.path);
    for p in path_params {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        let placeholder = format!("{{{name}}}");
        if !op.path.contains(&placeholder) {
            continue;
        }
        expr = format!(
            "str.replace({expr}, {}, str_of({}))",
            hnl_string(&placeholder),
            param_access(name)
        );
    }
    expr
}

/// The query-pair accumulation statements for `in: query` params.
fn query_stmts(query_params: &[&Value]) -> String {
    let mut out = String::new();
    for p in query_params {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        let _ = writeln!(
            out,
            "  pairs := if hash_map.has(params, {}) {{ [...pairs, {} ++ str_of({})] }} else {{ pairs }}",
            hnl_string(name),
            hnl_string(&format!("{name}=")),
            param_access(name),
        );
    }
    out
}

/// The `init` record literal for one operation.
fn init_expr(op: &Operation<'_>) -> String {
    let method = op.method.to_ascii_uppercase();
    if spec::has_request_body(op) {
        format!(
            "{{ method: {}, headers: {{ \"content-type\": \"application/json\" }}, body: stringify(body) }}",
            hnl_string(&method)
        )
    } else {
        format!("{{ method: {} }}", hnl_string(&method))
    }
}

/// Whether the op has a 2xx/`default` JSON response (drives the
/// `_json` decode variant).
fn expects_json(op: &Operation<'_>) -> bool {
    let Some(responses) = op.op.get("responses").and_then(Value::as_object) else {
        return false;
    };
    responses.iter().any(|(status, r)| {
        let ok_status = status == "default"
            || status
                .parse::<u16>()
                .map(|c| (200..300).contains(&c))
                .unwrap_or(false);
        ok_status
            && r.get("content")
                .and_then(Value::as_object)
                .is_some_and(|content| {
                    content
                        .keys()
                        .any(|mt| mt == "application/json" || mt.ends_with("+json"))
                })
    })
}

/// The doc comment for an emitted binding.
fn op_doc_comment(op: &Operation<'_>, ident: &str) -> String {
    let mut lines = vec![format!(
        "--- {ident} — {method} {path}",
        method = op.method.to_ascii_uppercase(),
        path = op.path
    )];
    if let Some(s) = op.op.get("summary").and_then(Value::as_str) {
        lines.push(format!("--- {s}"));
    }
    if let Some(d) = op.op.get("description").and_then(Value::as_str) {
        for l in d.lines() {
            lines.push(format!("--- {l}"));
        }
    }
    lines.join("\n")
}

impl Emitter<SpecModule> for OpenApiClientEmitter {
    fn materialize(
        &self,
        module: &SpecModule,
        req: &EmitContext,
    ) -> Result<Vec<Change>, CodegenError> {
        let config = OpenApiConfig::for_request(req.config.get("config").map(std::path::Path::new))
            .map_err(CodegenError::Emit)?;
        let ops = spec::operations(module);
        if ops.is_empty() {
            return Err(CodegenError::Emit(
                "openapi spec contains no operations — nothing to emit".to_string(),
            ));
        }
        let raw_names: Vec<String> = ops.iter().map(spec::operation_name).collect();
        let idents = spec::unique_names(&raw_names);

        let module_name = config
            .client_module
            .clone()
            .unwrap_or_else(|| "openapi_client".to_string());
        let title = module
            .document
            .get("info")
            .and_then(|i| i.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("OpenAPI client");

        let mut out = String::new();
        let _ = writeln!(
            out,
            "--- {title} — generated by handl-plugin-codegen-backend-openapi; do not edit\n\
             ---\n\
             --- Deps (.handl.toml [dependencies]): fetch-c = {{ path = \"…/lib-fetch-c/src\" }}\n\
             --- (`json`/`str`/`hash_map`/`type` are prelude names.)\n\n\
             from \"fetch-c\" import fetch\n\
             from \"json\" import stringify, parse\n\
             from \"str\" import * as str\n\n\
             --- Base URL for every call — set to the service origin.\n\
             base_url := {}\n\n\
             str_of := (v) ~> if is_str(v) {{ v }} else {{ type.to_string(v) }}\n",
            hnl_string(config.base_url.as_deref().unwrap_or("")),
        );

        for (op, ident) in ops.iter().zip(idents.iter()) {
            let (path_params, query_params) = spec::parameter_buckets(op);
            let buckets = (path_params.as_slice(), query_params.as_slice());
            let _ = writeln!(out, "{}", op_doc_comment(op, ident));
            let _ = writeln!(
                out,
                "export {ident} := (params, body) ~> do {{\n  path := {}\n  pairs := []\n{}\n  query := if pairs == [] {{ \"\" }} else {{ \"?\" ++ str.join(pairs, \"&\") }}\n  url := base_url ++ path ++ query\n  init := {}\n  fetch(url, init)\n}}\n",
                path_expr(op, buckets),
                query_stmts(query_params.as_slice()),
                init_expr(op),
            );
            if expects_json(op) {
                let _ = writeln!(
                    out,
                    "export {ident}_json := (params, body) ~> do {{\n  r := {ident}(params, body)\n  if r.body == \"\" {{ null }} else {{ parse(r.body) }}\n}}\n",
                );
            }
        }

        Ok(vec![Change::text(format!("{module_name}.hnl"), out)])
    }
}
