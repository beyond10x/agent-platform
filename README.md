# agent-platform

`agent-platform` is the authenticated, multi-tenant service for managing and running AI agents.
It owns stable agents, immutable revisions, agent-specific capability mappings, asynchronous tasks,
run evidence and triggers while composing the existing foundation:

The source and GitHub releases are public. The crates remain deployment components rather than
registry-published packages.

- [Harness](https://github.com/beyond10x/harness) owns the model/tool loop.
- [Connectors](https://github.com/beyond10x/connectors) owns external operations, credentials,
  grants, invocation and connector audit.
- Identity owns tenant and principal truth. Production requests are verified and exchanged through
  its official private client; a loopback-only development verifier remains available for local use.
- Substrate owns confined execution and llmgw will own production model routing.

The current walking slice implements authenticated durable management of agents and revisions,
deterministic Connector-operation to Harness-tool projection, idempotent task intake, and schedule or
webhook trigger definitions. User-bound Tasks run through an attempt-scoped Connector lease and
stream ordered execution evidence over SSE. The adapter pins Harness 0.11 at the reviewed
hosted-context and approval-checkpoint revision. When `--state-path` is
configured, exact human approval checkpoints and decisions remain resumable with freshly verified
attempt credentials; other interrupted attempts close with an explicit `execution_interrupted`
result. Live Connector invocation is supported for configured hosted workers; trigger delivery
remains a separate AEP story.

## Run the development service

```bash
export AGENT_PLATFORM_DEV_BEARER_TOKEN='replace-this-loopback-token'
cargo run --locked -p agent-platform -- serve
```

The listener defaults to `127.0.0.1:8090`. Authenticated routes expect
`Authorization: Bearer $AGENT_PLATFORM_DEV_BEARER_TOKEN`. Use `--connector-catalog` with a synthetic
or operator-owned JSON array of Connector operation descriptions to enable capability-profile
creation in the walking slice.

For a synthetic local projection:

```bash
cargo run --locked -p agent-platform -- serve \
  --state-path .local/agent-platform-state.json \
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
| `GET/POST /v1/capability-profiles` | list or compile personal profiles and shared tenant templates into Harness tools |
| `PATCH /v1/capability-profiles/{profile_id}` | compare-and-swap capability posture changes |
| `GET/POST /v1/tasks` | list or idempotently admit asynchronous work |
| `GET /v1/tasks/{task_id}` | read pinned task state |
| `GET /v1/tasks/{task_id}/events` | stream ordered events for the admitted attempt as SSE |
| `GET /v1/tasks/{task_id}/approvals` | list exact durable calls awaiting continuation |
| `POST /v1/tasks/{task_id}/approvals/{approval_id}` | persist an exact decision and resume with freshly verified attempt authority |
| `GET/POST /v1/triggers` | list or define schedule/webhook task sources |
| `GET /livez` | unauthenticated process liveness |
| `GET /openapi.json` | public deterministic OpenAPI 3.1 document |
| `GET /docs/` | public Rust-built documentation website embedded in the binary |

State remains process-local unless `--state-path` is configured. With it, agents, revisions,
capability profiles, tasks, evidence, approval continuations and triggers survive restarts; only an
exact persisted approval checkpoint is resumable, while other in-flight work terminates explicitly.
Production Identity and Connector custody are external services; model credentials are redeemed by
Harness only at the provider request boundary.

New capability profiles default to `personal`: only their verified creator can list, edit, bind, or
execute them. An explicit `tenant` audience creates a shared operator template. Profiles persisted
before 0.6.0 retain their prior shared behavior and load as tenant templates.

## Development

Rust 1.97 or newer and the `protocol` CLI are required.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
protocol artifact validate --strict
cargo run --locked -p agent-platform -- openapi --digest
```

The AEP-governed work record is under `.engineering/planning/`; see `docs/roadmap.md` for the delivery
sequence.

## Releases

Versions use bare semantic tags such as `0.2.0`; releases are described in `CHANGELOG.md`.

<!-- b10x-docs:start -->
## Documentation

[Agent Platform documentation](https://beyond10x.github.io/docs/agent-platform/) · [Start](https://beyond10x.github.io/) · [Ecosystem](https://beyond10x.github.io/ecosystem/) · [Impact](https://beyond10x.github.io/changes/) · [Releases](https://beyond10x.github.io/releases/)
<!-- b10x-docs:end -->
