//! `openapi-json` emitter: the OpenAPI document IR → `openapi.json`.
//!
//! One artifact, deterministic pretty-printed JSON. Unchanged input
//! produces byte-identical output, so the kernel's fingerprint diff
//! makes repeat plans no-ops — that is the entire point of routing
//! through `Engine`/`FsStore` rather than writing the file directly.

use handl_plugin_codegen::{Change, CodegenError, EmitContext, Emitter, SpecModule};

/// Emitter rendering [`SpecModule`] (an OpenAPI document) to
/// `openapi.json`.
pub struct OpenApiJsonEmitter {
    /// Artifact path within the store; `openapi.json` by default,
    /// overridable per-request via `EmitContext.config["output-name"]`.
    artifact: String,
}

impl OpenApiJsonEmitter {
    pub fn new() -> Self {
        Self {
            artifact: "openapi.json".to_string(),
        }
    }
}

impl Default for OpenApiJsonEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Emitter<SpecModule> for OpenApiJsonEmitter {
    fn materialize(
        &self,
        module: &SpecModule,
        req: &EmitContext,
    ) -> Result<Vec<Change>, CodegenError> {
        if !module.document.is_object() {
            return Err(CodegenError::Emit(
                "openapi-json emitter requires an object document".to_string(),
            ));
        }
        let name = req
            .config
            .get("output-name")
            .cloned()
            .unwrap_or_else(|| self.artifact.clone());
        let mut text = serde_json::to_string_pretty(&module.document)
            .map_err(|e| CodegenError::Emit(format!("failed to serialize openapi doc: {e}")))?;
        text.push('\n');
        Ok(vec![Change::text(name, text)])
    }

    fn verify(&self, _module: &SpecModule, changes: &[Change]) -> Result<(), CodegenError> {
        // Exactly one Put, non-empty — the honest-decline gate for a
        // degenerate "empty document" emit.
        match changes {
            [Change::Put { content, .. }] if !content.is_empty() => Ok(()),
            _ => Err(CodegenError::Emit(
                "openapi-json emitter produced an unexpected change set".to_string(),
            )),
        }
    }
}
