# Changelog

All notable operator-visible changes to this private service are recorded here.

## [Unreleased]

## [0.6.6] — 2026-09-03

- Consume the exact AgentIDE 0.2.1 and Workspace 0.2.9 context handoff so DevCenter browser drafts,
  authenticated sealing, Workspace actor-view refresh, and Harness tool binding use one Rust type
  graph while retaining pre-turn `ActorView` validation.

## [0.6.5] — 2026-09-03

- Consume AgentIDE 0.2.0, Workspace 0.2.8, and Harness 0.11.1 as exact released dependencies for
  sealed actor views, context digests, attachment provenance, session-bound dispatch, and durable
  approval continuation.
- Validate the complete actor view before any hosted context or inventory is bound into a Harness
  model turn, and keep malformed coordination state out of model input.

## [0.6.4] — 2026-09-03

- Align the hosted coding boundary with AgentIDE 0.1.3 and Workspace 0.2.7 so actor views and
  deliberate prompt attachments share one released protocol type graph.
- Allow hosted deployments to supply the Identity, Connectors, Workspace, and durable state inputs
  through explicit `AGENT_PLATFORM_*` environment variables while preserving the corresponding CLI
  flags.
- Add a typed `coding_session_turn` conversation envelope that persists only Workspace and
  AgentIDE session references plus deliberate prompt attachments; actor context and intent
  inventory remain dynamically derived by Workspace for each model turn.
- Execute hosted coding turns through the official Workspace client, bind each current AgentIDE
  intent inventory as Harness tools, and persist context/inventory revision events alongside the
  task journal without exposing the user's transient Identity session to task state or model input.
- Delegate private runtime construction, signing, visibility enforcement and release announcement to
  Devcenter's repository-owned promotion workflow instead of publishing from the public source
  repository's package authority.

## [0.6.3] — 2026-09-03

- Prepare the fail-closed capability fixes for promotion through Devcenter's private runtime
  package and parallelize the architecture builds.

## [0.6.2] — 2026-09-03

- Make a capability-profile `deny` terminal before Connector description and approval handling, so
  a provider-required approval can never re-enable a denied agent tool.
- Refuse unconstrained Connector input placeholders before profile activation instead of allowing
  the model provider to reject every later task with an opaque route error.
- Replace blocking, process-local approval waiters with Harness 0.11 durable checkpoints. Agent
  Platform persists the checkpoint, exact immutable revision, compiled Connector toolset, decision,
  and single-worker claim in tenant-scoped task state; restarts clear only the transient claim and
  leave the exact approval resumable with freshly verified attempt authority.

## [0.6.1] — 2026-09-03

- Scope task discovery, detail, approval, and event streams to the verified task owner, enabling
  durable personal chat history without exposing another tenant member's prompts or outputs.
- Expose typed task listing through the client used by product BFFs.

## [0.6.0] — 2026-09-03

- Add explicit personal and tenant capability-profile audiences. Personal profiles are discoverable,
  editable, bindable to agent revisions, and executable only by their creating principal; existing
  persisted profiles remain tenant templates for backward compatibility.
- Move the Identity and Connectors client boundaries to Identity 0.5.0 and Connectors 0.5.0 so the
  execution plane consumes the released multi-provider authority and principal-owned connector
  contracts.

## [0.5.4] — 2026-09-02

- Bootstrap the repository-linked private `agent-platform-server` package with the organization
  App's package authority, rather than a repository token whose public-repository inheritance made
  the prior package public and caused the release guard to refuse it.

## [0.5.3] — 2026-09-02

- Bind the private `ghcr.io/beyond10x/agent-platform-service` runtime package to this repository
  through OCI source metadata and bootstrap its architectures sequentially, so each short-lived
  release job receives repository-inherited package access without widening package visibility.

## [0.5.2] — 2026-09-02

- Retire the irreversibly public `agent-platform-runtime` identity and attempt publication through
  a new private package; no release was announced when that package lacked repository-inherited
  access.

## [0.5.1] — 2026-09-02

- Execute generated Connector tool calls with a separate attempt-bound invoke credential, suspend
  exact calls that require human approval, and resume them only with task-, attempt-, call-, and
  input-bound single-use evidence.
- Expose authenticated pending-approval and resolution APIs carrying the immutable, non-secret
  Connector owner context required for Devcenter to issue the proof without trusting browser
  coordinates.

## [0.5.0] — 2026-09-02

- Capability-profile creation and compare-and-swap updates can compile an exact credential-free
  Connector operation snapshot observed under the caller's current grants, so user-specific
  connection identities no longer depend on a process-static deployment catalogue.
- The release pipeline refuses to announce a release unless the repository-owned runtime package
  has its one-time administrative visibility setting fixed to private.

## [0.4.2] — 2026-09-02

- Bound the runtime image to a repository-owned package identity so release jobs do not depend on
  mutable access inherited from the legacy package.

## [0.4.1] — 2026-09-02

- Added a tag-gated, default-branch-bound multi-architecture release pipeline that publishes,
  signs and records the exact private image digest.

## [0.4.0] — 2026-09-02

- Added credential-free durable state snapshots and explicit restart recovery for agents,
  revisions, capability profiles, tasks, evidence and triggers through `--state-path`.
- Added compare-and-swap capability-profile updates with per-capability `allow`,
  `approval_required` and `deny` posture.
- Preserved model-provider failure classes as stable task failure codes so credential,
  availability, rate-limit and policy failures are no longer collapsed into one error.

## [0.3.1] — 2026-09-01

- Added the pinned, non-root OCI build used for private Kubernetes deployments. Private source
  access is supplied only as a BuildKit secret during compilation.

## [0.3.0] — 2026-09-01

- Added production Identity session verification and exact audience/scope exchange through the
  official Identity client; tenant and subject authority remain outside Agent Platform.
- Added attempt-bound user model execution: task admission creates an exact Connector credential
  lease, Harness redeems it only at the provider boundary, and the application layer never receives
  credential material.
- Added ordered, tenant-scoped task execution events over server-sent events, including text deltas,
  terminal output, and named failures with bounded lag recovery.
- Added the private typed `agent-platform-client` crate for Devcenter and other trusted service
  compositions.
- Kept this walking slice explicitly process-local; durable dispatch, restart recovery, Connector
  operation execution, approvals, and trigger delivery remain later milestones.

## [0.2.0] — 2026-08-31

- Added one shared API route catalog consumed by the Axum router and deterministic OpenAPI 3.1
  projection, with schema coverage for every request, response and problem document.
- Exposed the exact generated contract publicly at `/openapi.json` with no-store and nosniff
  headers, plus `agent-platform openapi` and its digest mode for release verification.
- Added a responsive public documentation website built entirely by Rust, embedded into the service
  binary and served under `/docs/` with no Pages, Node, CDN or mutable runtime site directory.
- Added executable route/catalog agreement, OpenAPI determinism and embedded-asset security tests.

## [0.1.0] — 2026-08-31

- Established the private `agent-platform` repository, architectural boundaries and AEP-governed
  delivery record.
- Added authenticated, tenant-scoped in-memory management for stable agents and immutable revisions,
  including compare-and-swap activation.
- Added deterministic projection of Connector operation descriptions into conservative Harness tool
  specifications and a model-free embedded Harness round-trip test.
- Added idempotent asynchronous Task admission and schedule/webhook trigger definitions through a
  bounded Axum API.
- Added a loopback-only development bearer verifier; production Identity, durable persistence, live
  Connector invocation, task workers and trigger delivery remain explicitly outside this release.
