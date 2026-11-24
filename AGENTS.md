<!-- myco:agent-awareness -->

## Agent awareness

- Session start / any work discussion: run `myco status --prompt`
  before proposing next steps — know the board first.
- User describes new work → offer to capture it (`myco todo add`,
  `issue new`, `proposal new`). Don't wait to be asked.
- Picking up a task: `myco resume <ID>` is the briefing.
- Wrapping up: `myco handoff <ID>` anything resumable; keep statuses
  honest (claimed → in-progress → blocked/done).

<!-- /myco:agent-awareness -->

# Agent guide — tool-codegen-openapi

OpenAPI backend for the Handl codegen pipeline (`handl-plugin-codegen-backend-openapi`, P-008 / T-144). Three directions over the `SpecModule` IR (OpenAPI doc as `serde_json::Value`):

- `openapi` — handl route map → `openapi.json` (spec; evaluates the module via an `hnl run` probe into `lib-http-openapi`).
- `openapi-client` — spec → `.hnl` `fetch-c` client bindings.
- `openapi-server` — spec → `.hnl` `route.route_map` skeleton.

## Conventions

- Kernel traits come from `handl-plugin-codegen` (re-exports of `hnl-tool-codegen`): `Source<I>`/`Emitter<I>`/`Change`/`Engine`/`FsStore`/`MemoryStore`. No direct `hnl-tool-codegen` dep.
- Emitters are pure: `materialize` returns `Vec<Change>`, never writes. All fs effects go through `Engine`.
- The `hnl run` probe is request scratch under `$TMPDIR` — never write into the output tree from `Source::parse`.
- Artifact ordering/dedup is the kernel's job; emitters emit one deterministic artifact per module.
- `value_to_hnl`/`hnl_string` (routes.rs) render JSON → handl literals; reuse them for any emitted `.hnl` data.

## Verification

```sh
cargo xtask gate        # fmt -> clippy -D warnings -> test -> doc
```

Eval-dependent tests skip when `hnl run` (ext-eval cdylib) is unavailable.
