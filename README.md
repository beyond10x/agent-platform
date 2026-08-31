# agent-platform

`agent-platform` is the authenticated, multi-tenant service for managing and running AI agents.
It owns stable agents, immutable revisions, agent-specific capability mappings, asynchronous tasks,
run evidence and triggers while composing the existing foundation:

This repository and its release artifacts are private.

- [Harness](https://github.com/beyond10x/harness) owns the model/tool loop.
- [Connectors](https://github.com/beyond10x/connectors) owns external operations, credentials,
  grants, invocation and connector audit.
- Identity owns tenant and principal truth; the current walking slice supplies only a loopback
  development verifier.
- Substrate owns confined execution and llmgw will own production model routing.

The first walking slice implements authenticated in-memory management of agents and revisions,
deterministic Connector-operation to Harness-tool projection, idempotent task intake, and schedule or
webhook trigger definitions. Its Harness adapter embeds Harness 0.8.0 and has a model-free
tool-round-trip test; the HTTP task path deliberately stops at `accepted`. It does not claim durable
worker execution, production Identity, live Connector invocation, approvals or trigger delivery;
those are separate AEP stories.

## Run the development service

```console
export AGENT_PLATFORM_DEV_BEARER_TOKEN='replace-this-loopback-token'
cargo run --locked -p agent-platform -- serve
```

The listener defaults to `127.0.0.1:8090`. Authenticated routes expect
`Authorization: Bearer $AGENT_PLATFORM_DEV_BEARER_TOKEN`. Use `--connector-catalog` with a synthetic
or operator-owned JSON array of Connector operation descriptions to enable capability-profile
creation in the walking slice.

For a synthetic local projection:

```console
cargo run --locked -p agent-platform -- serve \
  --connector-catalog examples/synthetic-connector-catalog.json
```

Exposing the development verifier beyond loopback requires the visibly insecure
`--allow-insecure-dev-listener` flag. It is not production authentication.

The same process serves the public, binary-embedded documentation at
`http://127.0.0.1:8090/docs/` and its generated OpenAPI 3.1 contract at
`http://127.0.0.1:8090/openapi.json`. Neither route exposes tenant data or private planning records.

## HTTP surface

All `/v1` routes require the bearer token. Request authority is derived before the JSON body is
materialized.

| route | purpose |
|---|---|
| `GET/POST /v1/agents` | list or create stable agent identities |
| `GET /v1/agents/{agent_id}` | read one tenant-owned agent |
| `GET/POST /v1/agents/{agent_id}/revisions` | list or append immutable revisions |
| `POST /v1/agents/{agent_id}/activate` | compare-and-swap the active revision |
| `GET/POST /v1/capability-profiles` | list or compile Connector mappings into Harness tools |
| `GET/POST /v1/tasks` | list or idempotently admit asynchronous work |
| `GET /v1/tasks/{task_id}` | read pinned task state |
| `GET/POST /v1/triggers` | list or define schedule/webhook task sources |
| `GET /livez` | unauthenticated process liveness |
| `GET /openapi.json` | public deterministic OpenAPI 3.1 document |
| `GET /docs/` | public Rust-built documentation website embedded in the binary |

State is intentionally in memory in this slice and is lost on restart.

## Development

Rust 1.97 or newer and the `protocol` CLI are required.

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
protocol artifact validate --strict
cargo run --locked -p agent-platform -- openapi --digest
```

The AEP-governed work record is under `.engineering/planning/`; see `docs/roadmap.md` for the delivery
sequence.

## Releases

Versions use bare semantic tags such as `0.2.0`; private releases are described in `CHANGELOG.md`.
