# tool-codegen-openapi

`handl-plugin-codegen-backend-openapi` — OpenAPI backend for the Handl
codegen pipeline (P-008 / T-144). Part of the `tool-codegen-*` family:
three directions over the OpenAPI 3.0.x document IR (`SpecModule`).

| target           | input                        | output              |
|------------------|------------------------------|---------------------|
| `openapi`        | handl module exporting a route map (`export routes := {…}`) or flat entry list | `openapi.json` |
| `openapi-client` | OpenAPI spec (yaml/json)     | `<client_module>.hnl` — `fetch-c` arrows per operation |
| `openapi-server` | OpenAPI spec (yaml/json)     | `<server_module>.hnl` — `route.route_map` skeleton, 501 stubs |

The `spec` direction evaluates the routes module through the `hnl`
binary (`hnl run` + a generated probe that calls
`lib-http-openapi`'s `openapi/1`/`openapi_entries/1`) — the pattern
bridging (`:param`, `*splat`, `(/optional)`, `?k=v`, doc passthrough,
operationId dedup) stays in handl where it is already implemented and
tested. The probe's `.handl.toml` is generated per request; `route`,
`json`, and `http-openapi` resolve as sibling `lib-*` checkouts
(overridable via `openapi_lib:`/`deps:` in `openapi.yaml`).

## Configuration

`openapi.yaml` (all fields optional):

```yaml
module: src/routes.hnl        # default: the spec argument
export: routes                # export carrying the map/entries
mode: routes                  # routes | entries
openapi_lib: ../lib-http-openapi
info: { title: My API, version: 1.0.0 }
servers: [{ url: https://api.example.com }]
base_url: https://api.example.com   # client direction
client_module: my_client
server_module: my_server
deps:                          # extra probe .handl.toml deps
  http-router: ../lib-http-router/src
hnl: /path/to/hnl              # default: $HANDL_HNL / ~/.local/bin/hnl / …
```

## Development

```bash
cargo xtask gate        # fmt --check → clippy -D warnings → test → doc
```

`cargo xtask` wraps the repo's `xtask/` crate; `ci.sh` is a local gate
entry; the bupha gate is `workflows/gate.yaml` (rust campaign). The eval-dependent tests skip cleanly when `hnl run` (the
`ext-eval` cdylib) is unavailable; emitter and lifecycle tests always
run.

## Docs

- [docs/index.md](docs/index.md) — architecture and direction notes.
