//! Shared OpenAPI-document helpers for the client/server emitters.
//!
//! The IR is a [`SpecModule`] whose `document` is the parsed
//! OpenAPI 3.0.x JSON — these helpers walk `paths` → method →
//! operation without enforcing full spec validity (the document was
//! produced by `lib-http-openapi` or hand-written; emitters are
//! tolerant and skip non-operation entries like `parameters`/
//! `summary` path-items).

use handl_plugin_codegen::SpecModule;
use serde_json::Value;
use std::collections::BTreeSet;

/// HTTP methods recognised on a path item, in emission order.
pub const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Path-item keys that are not operations.
const NON_OPERATIONS: [&str; 4] = ["parameters", "summary", "description", "servers"];

/// One (path, method, operation) triple flattened out of the doc.
pub struct Operation<'a> {
    /// OpenAPI path template, e.g. `/users/{id}`.
    pub path: &'a str,
    /// Lowercase method (`get`, `post`, …).
    pub method: &'a str,
    /// The operation object.
    pub op: &'a Value,
}

/// Iterate every operation in document order (paths sorted by serde's
/// map order — `lib-http-openapi` emits sorted paths; hand-written
/// docs keep insertion order).
pub fn operations(module: &SpecModule) -> Vec<Operation<'_>> {
    let mut out = Vec::new();
    let Some(paths) = module.document.get("paths").and_then(Value::as_object) else {
        return out;
    };
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for method in METHODS {
            if NON_OPERATIONS.contains(&method) {
                continue;
            }
            if let Some(op) = item.get(method).filter(|v| v.is_object()) {
                out.push(Operation { path, method, op });
            }
        }
        // `x-` extensions and unknown keys are ignored.
    }
    out
}

/// The operation's declared name: `operationId` when present, else a
/// generated `<method>_<sanitized-path>`.
pub fn operation_name(op: &Operation<'_>) -> String {
    if let Some(id) = op
        .op
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return id.to_string();
    }
    let sanitized: String = op
        .path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('_');
    format!(
        "{}_{}",
        op.method,
        if sanitized.is_empty() {
            "root"
        } else {
            sanitized
        }
    )
}

/// A handl-identifier-safe version of an operation name (for `export`
/// bindings): non-alnum → `_`, leading digit → `_` prefix, dedup'd by
/// the caller via [`unique_names`].
pub fn handl_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(if i == 0 && c.is_ascii_digit() { '_' } else { c });
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "op".to_string()
    } else {
        out
    }
}

/// Assign collision-free idents: first occurrence keeps the name,
/// later ones get `_2`, `_3`, … (same convention `lib-http-openapi`
/// uses for operationIds).
pub fn unique_names(names: &[String]) -> Vec<String> {
    let mut used = BTreeSet::new();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    names
        .iter()
        .map(|n| {
            let base = handl_ident(n);
            let count = counts.entry(base.clone()).or_insert(0);
            loop {
                *count += 1;
                let candidate = if *count == 1 {
                    base.clone()
                } else {
                    format!("{base}_{count}")
                };
                if used.insert(candidate.clone()) {
                    return candidate;
                }
            }
        })
        .collect()
}

/// The `doc` record for a server-side route leaf: operation metadata
/// copied verbatim under the keys `lib-http-openapi` recognises
/// (`summary`, `description`, `tags`, `operationId`, `deprecated`)
/// plus `params`/`requestBody`/`responses`/`security` for downstream
/// consumers. Returns `None` when the operation carries none of them.
pub fn operation_doc(op: &Operation<'_>) -> Option<Value> {
    const DOC_KEYS: [&str; 9] = [
        "summary",
        "description",
        "tags",
        "operationId",
        "deprecated",
        "params",
        "requestBody",
        "responses",
        "security",
    ];
    let mut doc = serde_json::Map::new();
    // `parameters` is spelled `params` in the route-doc vocabulary.
    if let Some(params) = op.op.get("parameters") {
        doc.insert("params".into(), params.clone());
    }
    for key in DOC_KEYS {
        if let Some(v) = op.op.get(key) {
            doc.insert(key.into(), v.clone());
        }
    }
    if doc.is_empty() {
        None
    } else {
        Some(Value::Object(doc))
    }
}

/// Split an operation's `parameters` into `(path, query)` buckets by
/// `in:`. Entries without `in:` (or with `header`/`cookie`) are
/// skipped — the client emitter only models path+query.
pub fn parameter_buckets<'a>(op: &Operation<'a>) -> (Vec<&'a Value>, Vec<&'a Value>) {
    let mut path = Vec::new();
    let mut query = Vec::new();
    if let Some(params) = op.op.get("parameters").and_then(Value::as_array) {
        for p in params {
            match p.get("in").and_then(Value::as_str) {
                Some("path") => path.push(p),
                Some("query") => query.push(p),
                _ => {}
            }
        }
    }
    (path, query)
}

/// Whether the operation declares a request body.
pub fn has_request_body(op: &Operation<'_>) -> bool {
    op.op.get("requestBody").is_some_and(|v| !v.is_null())
}
