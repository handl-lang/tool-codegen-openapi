# tool-codegen-openapi

`handl-plugin-codegen-backend-openapi` — OpenAPI backend for the Handl
codegen pipeline: handl route maps → OpenAPI 3.0.x documents, and
OpenAPI documents → handl client (`fetch-c`) and server
(`route.route_map`) bindings.

## Layout

- `src/config.rs` — `OpenApiConfig` (`openapi.yaml`): module/export/
  mode, `openapi_lib`, `deps`, `hnl`, OpenAPI `opts` passthrough
  (`info`/`servers`/`components`/`methods`/`sep`/`leaf_keys`/
  `spec_keys`), and client/server naming (`base_url`,
  `client_module`, `server_module`).
- `src/routes.rs` — `OpenApiRoutesSource`: generates a probe `.hnl` +
  `.handl.toml` in a scratch dir, runs `hnl run`, parses the printed
  JSON document into `SpecModule`. Also `find_hnl` toolchain
  discovery and the shared JSON→handl literal renderer.
- `src/spec.rs` — OpenAPI document walkers: operation flattening,
  operationId → ident naming/dedup, `doc` extraction, parameter
  buckets.
- `src/emit_json.rs` — `openapi-json` emitter: `SpecModule` →
  `openapi.json` (deterministic pretty JSON; unchanged input is a
  no-op plan through the kernel).
- `src/emit_client.rs` — `hnl-openapi-client` emitter: spec →
  `<client_module>.hnl` with `fetch`/`stringify`/`parse`/`str`
  imports, path-param substitution, query assembly, JSON request
  bodies, and `_json` response-decode variants.
- `src/emit_server.rs` — `hnl-openapi-server` emitter: spec →
  `<server_module>.hnl` exporting `routes`, a `route_map`-compatible
  record of `{ handler, method, path, name, doc }` leaves with 501
  stubs; `{param}` templates become `:param`.
- `src/lib.rs` — `register()` wiring the three targets
  (`openapi`, `openapi-client`, `openapi-server`) into
  `CodegenRegistry`.

## Design notes

- **IR is `SpecModule`** — the OpenAPI document as
  `serde_json::Value`. The `spec` direction's transform lives in
  `lib-http-openapi` (handl), reached via the `hnl run` probe rather
  than re-implemented in Rust; client/server consume the same IR, so
  spec→client and spec→server round-trip through one document shape.
- **Purity**: `Source::parse` writes only into a per-request scratch
  dir (`$TMPDIR/hnl-openapi-<pid>-<seq>`), never the output tree;
  all artifacts flow through `Emitter::materialize` → `Change` →
  `Engine` (journaled, fingerprinted, revertible).
- **Eval tests are toolchain-gated**: `hnl run` requires the
  `ext-eval` cdylib; `tests/spec_direction.rs` probes once and skips
  when absent.
