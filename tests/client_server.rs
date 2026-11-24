//! Client/server directions: fixture OpenAPI spec → `.hnl`
//! artifacts through `OpenApiSpecSource`-equivalent `SpecModule` +
//! emitters, and the kernel lifecycle over `MemoryStore`.

use handl_plugin_codegen::{
    ApplyOptions, Change, EmitContext, Emitter, Engine, MemoryStore, RunMeta,
};
use handl_plugin_codegen_backend_openapi::{OpenApiClientEmitter, OpenApiServerEmitter};
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn load_spec() -> handl_plugin_codegen::SpecModule {
    // The `openapi-spec` source is registry-internal; load through the
    // same `SpecModule::load` it delegates to.
    handl_plugin_codegen::SpecModule::load(&fixture("openapi.json"), None)
        .expect("fixture spec parses")
}

fn emitted(
    emitter: &dyn Fn(&handl_plugin_codegen::SpecModule, &EmitContext) -> Vec<Change>,
    module: &handl_plugin_codegen::SpecModule,
) -> String {
    let changes = emitter(module, &EmitContext::new());
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        Change::Put {
            artifact, content, ..
        } => {
            assert!(artifact.as_str().ends_with(".hnl"));
            String::from_utf8(content.clone()).unwrap()
        }
        _ => panic!("expected Put"),
    }
}

#[test]
fn client_emits_fetch_arrows() {
    let module = load_spec();
    let src = emitted(
        &|m, r| OpenApiClientEmitter::new().materialize(m, r).unwrap(),
        &module,
    );
    // Imports + base_url
    assert!(src.contains("from \"fetch-c\" import fetch"));
    assert!(src.contains("from \"json\" import stringify, parse"));
    assert!(src.contains("base_url := \"\""));
    // One arrow per operation, named by operationId
    assert!(src.contains("export listUsers := (params, body)"));
    assert!(src.contains("export createUser := (params, body)"));
    assert!(src.contains("export getUser := (params, body)"));
    // Path param substitution + query params
    assert!(src.contains("str.replace(\"/users/{id}\", \"{id}\""));
    assert!(src.contains("\"limit=\" ++ str_of(params.limit)"));
    // requestBody → JSON init
    assert!(src.contains("body: stringify(body)"));
    assert!(src.contains("\"content-type\": \"application/json\""));
    // 2xx JSON responses get decode variants
    assert!(src.contains("export listUsers_json := (params, body)"));
    assert!(src.contains("export getUser_json := (params, body)"));
    assert!(!src.contains("export createUser_json"));
    // fetch call shape
    assert!(src.contains("fetch(url, init)"));
}

#[test]
fn client_config_base_url_and_name() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("openapi.yaml");
    std::fs::write(
        &cfg,
        "base_url: https://api.example.com\nclient_module: my_client\n",
    )
    .unwrap();
    let module = load_spec();
    let ctx = EmitContext::new().with_config("config", cfg.display().to_string());
    let changes = OpenApiClientEmitter::new()
        .materialize(&module, &ctx)
        .unwrap();
    match &changes[0] {
        Change::Put {
            artifact, content, ..
        } => {
            assert_eq!(artifact.as_str(), "my_client.hnl");
            let text = String::from_utf8(content.clone()).unwrap();
            assert!(text.contains("base_url := \"https://api.example.com\""));
        }
        _ => panic!("expected Put"),
    }
}

#[test]
fn server_emits_route_map_skeleton() {
    let module = load_spec();
    let src = emitted(
        &|m, r| OpenApiServerEmitter::new().materialize(m, r).unwrap(),
        &module,
    );
    assert!(src.contains("export routes := {"));
    // One leaf per operation with explicit path/method/name
    assert!(src.contains("\"method\": \"get\""));
    assert!(src.contains("\"method\": \"post\""));
    // {id} → :id route-pattern spelling
    assert!(src.contains("\"path\": \"/users/:id\""));
    assert!(src.contains("\"path\": \"/users\""));
    // doc passthrough: summary/operationId/params/responses
    assert!(src.contains("\"summary\": \"Show user\""));
    assert!(src.contains("\"operationId\": \"getUser\""));
    assert!(src.contains("\"responses\""));
    // 501 stub handler
    assert!(src.contains("\"status\": 501"));
    assert!(src.contains("not implemented: getUser"));
}

#[test]
fn server_client_lifecycle_is_cached() {
    let store = MemoryStore::new();
    let engine = Engine::new(&store);
    let module = load_spec();
    let meta = RunMeta::new("openapi-server", "0.1.0");
    let changes = OpenApiServerEmitter::new()
        .materialize(&module, &EmitContext::new())
        .unwrap();
    engine
        .apply(&changes, None, &meta, &ApplyOptions::default())
        .unwrap();
    let plan = engine.plan(&changes, None).unwrap();
    assert!(plan.is_clean(), "re-plan must be clean: {plan:?}");
    // The emitted .hnl is on the store.
    let snap = store.snapshot();
    assert!(snap.keys().any(|k| k.as_str().ends_with(".hnl")));
    // And it is valid UTF-8 handl source (smoke: balanced braces).
    let text = String::from_utf8(
        snap.iter()
            .find(|(k, _)| k.as_str().ends_with(".hnl"))
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert_eq!(text.matches('{').count(), text.matches('}').count());
}

#[test]
fn spec_source_load_via_registry_path() {
    // SpecModule::load accepts .json and .yaml — the `openapi-spec`
    // source delegates to it.
    let json = load_spec();
    assert_eq!(json.format, "json");
    assert_eq!(json.document["info"]["title"], "Fixture API");

    let dir = tempfile::tempdir().unwrap();
    let yaml = dir.path().join("api.yaml");
    std::fs::write(&yaml, "openapi: 3.0.3\ninfo:\n  title: Y\npaths: {}\n").unwrap();
    let m = handl_plugin_codegen::SpecModule::load(&yaml, None).unwrap();
    assert_eq!(m.format, "yaml");
    assert_eq!(m.document["info"]["title"], "Y");
}
