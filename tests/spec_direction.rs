//! Spec direction end-to-end: `tests/fixtures/routes.hnl` →
//! `OpenApiRoutesSource` (hnl probe + lib-http-openapi) →
//! `SpecModule` → `OpenApiJsonEmitter` → `openapi.json`, reconciled
//! through the kernel `Engine` over `MemoryStore`/`FsStore`.
//!
//! The eval half needs a working `hnl run` (the `ext-eval` cdylib).
//! Tests detect the toolchain once and skip cleanly when absent —
//! the emitter/lifecycle halves always run.

use handl_plugin_codegen::{
    ApplyOptions, EmitContext, Emitter, Engine, FsStore, MemoryStore, RunMeta,
};
use handl_plugin_codegen_backend_openapi::{
    OpenApiConfig, OpenApiJsonEmitter, OpenApiRoutesSource, find_hnl,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Whether `hnl run` can execute a trivial file — the ext-eval
/// cdylib must be discoverable. Probed once per test process.
fn hnl_run_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        let Some(hnl) = find_hnl(None).ok() else {
            eprintln!("skip: no hnl binary found");
            return false;
        };
        let dir = std::env::temp_dir().join(format!("hnl-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("t.hnl");
        if std::fs::write(&f, "1\n").is_err() {
            return false;
        }
        match Command::new(&hnl)
            .arg("run")
            .arg(&f)
            .current_dir(&dir)
            .output()
        {
            Ok(o) if o.status.success() => true,
            Ok(o) => {
                eprintln!(
                    "skip: `hnl run` unavailable: {}",
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                );
                false
            }
            Err(e) => {
                eprintln!("skip: cannot spawn {}: {e}", hnl.display());
                false
            }
        }
    })
}

fn parse_routes() -> handl_plugin_codegen::SpecModule {
    let src = OpenApiRoutesSource::new(OpenApiConfig::default());
    src.parse_module(&fixture("routes.hnl"))
        .expect("routes probe should produce an OpenAPI document")
}

#[test]
fn spec_direction_produces_openapi_document() {
    if !hnl_run_available() {
        return;
    }
    let module = parse_routes();
    assert_eq!(module.format, "json");
    let doc = &module.document;
    assert_eq!(doc["openapi"], "3.0.3");
    assert_eq!(doc["info"]["title"], "API");

    let paths = doc["paths"].as_object().expect("paths object");
    // :param bridging
    let users = paths["/users/{id}"].as_object().unwrap();
    let get = users["get"].as_object().unwrap();
    assert_eq!(get["summary"], "Show user");
    assert_eq!(get["operationId"], "getUser");
    assert_eq!(get["tags"], serde_json::json!(["users"]));
    // doc.params merged with generated path param
    let params = get["params"].as_array().unwrap();
    assert!(
        params
            .iter()
            .any(|p| p["name"] == "id" && p["in"] == "path")
    );
    assert!(
        params
            .iter()
            .any(|p| p["name"] == "verbose" && p["in"] == "query")
    );
    // splat → {rest} + x-handl-pattern
    let files = paths["/files/{rest}"].as_object().unwrap();
    assert_eq!(files["get"]["x-handl-pattern"], "/files/*rest");
    // doc passthrough on method-map leaf
    assert_eq!(paths["/health"]["get"]["summary"], "Health check");
    // explicit path + ?k=v constraint → query param
    let search = paths["/search"].as_object().unwrap();
    assert_eq!(search["get"]["deprecated"], true);
    let q = search["get"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "q")
        .expect("?q=test constraint becomes a query param")
        .clone();
    assert_eq!(q["in"], "query");
    assert_eq!(q["schema"]["enum"], serde_json::json!(["test"]));
}

#[test]
fn spec_direction_entries_mode() {
    if !hnl_run_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let entries = dir.path().join("entries.hnl");
    std::fs::write(
        &entries,
        "h := (_req) ~> null\n\
         export entries := [\n  \
           { path: \"/ping\", method: \"get\", handler: h,\n    doc: { summary: \"Ping\" } }\n]\n",
    )
    .unwrap();
    let cfg = OpenApiConfig {
        export: Some("entries".into()),
        mode: Some("entries".into()),
        ..Default::default()
    };
    let src = OpenApiRoutesSource::new(cfg);
    let module = src.parse_module(&entries).unwrap();
    assert_eq!(module.document["paths"]["/ping"]["get"]["summary"], "Ping");
}

#[test]
fn emitter_produces_openapi_json() {
    let doc = serde_json::json!({
        "openapi": "3.0.3",
        "info": { "title": "T", "version": "0" },
        "paths": {}
    });
    let module = handl_plugin_codegen::SpecModule {
        format: "json".into(),
        name: "t".into(),
        document: doc,
        source: None,
        diagnostics: vec![],
    };
    let changes = OpenApiJsonEmitter::new()
        .materialize(&module, &EmitContext::new())
        .unwrap();
    assert_eq!(changes.len(), 1);
    let text = match &changes[0] {
        handl_plugin_codegen::Change::Put { content, .. } => {
            String::from_utf8(content.clone()).unwrap()
        }
        _ => panic!("expected Put"),
    };
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["openapi"], "3.0.3");
}

#[test]
fn lifecycle_second_plan_is_clean() {
    // MemoryStore: plan → apply → plan must be a no-op — the
    // fingerprint/content-hash diff, not a timestamp, decides.
    let store = MemoryStore::new();
    let engine = Engine::new(&store);
    let module = handl_plugin_codegen::SpecModule {
        format: "json".into(),
        name: "t".into(),
        document: serde_json::json!({ "openapi": "3.0.3", "paths": {} }),
        source: None,
        diagnostics: vec![],
    };
    let changes = OpenApiJsonEmitter::new()
        .materialize(&module, &EmitContext::new())
        .unwrap();

    let plan1 = engine.plan(&changes, None).unwrap();
    assert_eq!(plan1.count(handl_plugin_codegen::PlanAction::Create), 1);

    let meta = RunMeta::new("openapi", "0.1.0");
    engine
        .apply(&changes, None, &meta, &ApplyOptions::default())
        .unwrap();

    // Second materialize of the same IR → identical bytes → clean plan.
    let changes2 = OpenApiJsonEmitter::new()
        .materialize(&module, &EmitContext::new())
        .unwrap();
    let plan2 = engine.plan(&changes2, None).unwrap();
    assert!(plan2.is_clean(), "second plan must be a no-op: {plan2:?}");

    // FsStore + tempfile: the artifact lands on disk.
    let dir = tempfile::tempdir().unwrap();
    let fs = FsStore::new(dir.path());
    let fs_engine = Engine::new(&fs);
    fs_engine
        .apply(&changes, None, &meta, &ApplyOptions::default())
        .unwrap();
    let written = std::fs::read_to_string(dir.path().join("openapi.json")).unwrap();
    assert!(written.contains("\"openapi\": \"3.0.3\""));
    let plan3 = fs_engine.plan(&changes, None).unwrap();
    assert!(plan3.is_clean());
}
