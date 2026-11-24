//! OpenAPI backend for the Handl codegen pipeline (P-008 / T-144).
//!
//! Three directions, all typed over [`SpecModule`] (a parsed
//! OpenAPI 3.0.x document as `serde_json::Value`):
//!
//! | target           | source            | emitter               | direction |
//! |------------------|-------------------|------------------------|-----------|
//! | `openapi`        | `openapi-routes`  | `openapi-json`         | spec      |
//! | `openapi-client` | `openapi-spec`    | `hnl-openapi-client`   | client    |
//! | `openapi-server` | `openapi-spec`    | `hnl-openapi-server`   | server    |
//!
//! - **spec** — a handl module exporting a route map (`export routes
//!   := { photos: { ":id": { get: h, doc: {…} } } }`) or flat entry
//!   list is evaluated by the `hnl` binary through a generated probe
//!   script that calls `lib-http-openapi`'s `openapi/1` /
//!   `openapi_entries/1`; the resulting document is emitted as
//!   `openapi.json` through the kernel lifecycle (fingerprint-cached,
//!   journaled — unchanged input is a no-op plan).
//! - **client** — an OpenAPI spec file → `<client_module>.hnl` with
//!   one `fetch-c` arrow per operation.
//! - **server** — an OpenAPI spec file → `<server_module>.hnl` with a
//!   `route.route_map`-compatible `routes` record of 501-stub leaves
//!   carrying `doc` annotations.
//!
//! Config: `openapi.yaml` (`OpenApiConfig`) via `--config`.

mod config;
mod emit_client;
mod emit_json;
mod emit_server;
mod routes;
mod spec;

pub use config::OpenApiConfig;
pub use emit_client::OpenApiClientEmitter;
pub use emit_json::OpenApiJsonEmitter;
pub use emit_server::OpenApiServerEmitter;
pub use routes::{OpenApiRoutesSource, find_hnl};

use handl_plugin_codegen::{
    AnySource, CodegenEmitterDef, CodegenRegistry, CodegenRequest, CodegenSourceDef, CodegenTarget,
    SpecModule, erase_emitter, erase_source,
};
use std::path::Path;

/// Register the three `openapi*` codegen targets on `reg`
/// (ADR-0128 §0.1): sources `openapi-routes`/`openapi-spec`, emitters
/// `openapi-json`/`hnl-openapi-client`/`hnl-openapi-server`, resolved
/// by name — this crate never depends on sibling emitters.
pub fn register(reg: &mut CodegenRegistry) {
    reg.register_source(CodegenSourceDef {
        name: "openapi-routes",
        make: make_routes_source,
    });
    reg.register_source(CodegenSourceDef {
        name: "openapi-spec",
        make: make_spec_source,
    });
    reg.register_emitter(CodegenEmitterDef {
        name: "openapi-json",
        make: |_| Ok(erase_emitter(OpenApiJsonEmitter::new())),
    });
    reg.register_emitter(CodegenEmitterDef {
        name: "hnl-openapi-client",
        make: |_| Ok(erase_emitter(OpenApiClientEmitter::new())),
    });
    reg.register_emitter(CodegenEmitterDef {
        name: "hnl-openapi-server",
        make: |_| Ok(erase_emitter(OpenApiServerEmitter::new())),
    });
    for (name, source, emitter) in [
        ("openapi", "openapi-routes", "openapi-json"),
        ("openapi-client", "openapi-spec", "hnl-openapi-client"),
        ("openapi-server", "openapi-spec", "hnl-openapi-server"),
    ] {
        reg.register_target(CodegenTarget {
            name,
            source,
            emitter,
            source_filters: false,
            docs: None,
            spec_input: None,
        });
    }
}

fn config_for(req: &CodegenRequest<'_>) -> Result<OpenApiConfig, String> {
    OpenApiConfig::for_request(req.config)
}

fn make_routes_source(req: &CodegenRequest<'_>) -> Result<Box<dyn AnySource>, String> {
    let config = config_for(req)?;
    Ok(erase_source(OpenApiRoutesSource::new(config)))
}

/// `openapi-spec` source: a yaml/json OpenAPI document →
/// [`SpecModule`] via the reference loader.
struct OpenApiSpecSource;

impl handl_plugin_codegen::Source<SpecModule> for OpenApiSpecSource {
    fn parse(
        &self,
        req: &handl_plugin_codegen::EmitContext,
    ) -> Result<SpecModule, handl_plugin_codegen::CodegenError> {
        let path: &Path = req.spec_path.as_deref().ok_or_else(|| {
            handl_plugin_codegen::CodegenError::Parse(
                "openapi-spec source requires EmitContext::spec_path".to_string(),
            )
        })?;
        let mut module = SpecModule::load(path, req.source_url.clone())?;
        // Light sanity check — the emitters walk `paths`; a spec with
        // no `paths` object still emits (empty skeleton is legal for
        // server stubs of a spec-in-progress) but flag it.
        if module.document.get("paths").is_none() {
            module
                .diagnostics
                .push("spec has no `paths` object".to_string());
        }
        Ok(module)
    }
}

fn make_spec_source(_req: &CodegenRequest<'_>) -> Result<Box<dyn AnySource>, String> {
    Ok(erase_source(OpenApiSpecSource))
}
