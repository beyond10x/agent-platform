# AGENTS.md — agent-platform

The change contract for this repository. Read it before changing source or planning records.

## Serves

- **O1 — governed reach.** Every external effect is attributable to verified authority and remains
  inside the active agent revision, compiled capability profile and current Connector Grant.
- **O5 — the generic agent platform.** Authenticated tenants manage agents, capabilities, tasks,
  runs and triggers through a product-neutral API.
- **O6 — self-improvement, built into all of it.** Immutable revisions and durable run evidence make
  changes comparable and reversible.

## Boundary

This repository owns the multi-tenant agent control and execution plane: stable agent identities,
immutable revisions, agent-specific capability mappings, tasks, attempts, triggers and their
evidence. Harness owns the agent loop. Connectors owns providers, integrations, connections,
credentials, grants, operation descriptions, invocation and connector audit. Identity owns tenant
and principal truth. Substrate owns confinement and llmgw owns production model routing.

Babelforce's `ai-agent-platform` is a downstream product and future adopter. Its flows, knowledge,
voice, A2A channels, manager specialists, console, quotas and SDK facade do not enter this domain.

## Visibility

This repository and its release artifacts are private. Documentation is served by the service from
an embedded, curated build; it is not published through GitHub Pages. Crates are not published.

## Invariants

1. Raw credential bytes reach only a credential verifier. Application and persistence receive a
   credential-free verified authority.
2. Tenant, actor, executor, request id and receive time are server-derived. Request bodies cannot
   assert them.
3. Every store operation is tenant-scoped by construction. A global lookup followed by filtering is
   not accepted.
4. Agent revisions are immutable. Activation is a compare-and-swap decision and running work pins
   an exact revision.
5. A capability mapping only narrows a Connector operation. It never manufactures authority,
   weakens effects/risk/approval, handles credentials or selects a destination.
6. Search and describe grant nothing. Connectors revalidates its current Connection, Grant,
   description lease and approval at invocation.
7. Task idempotency equality covers caller-controlled intent. Reusing a key for different intent is
   a conflict, never a second task.
8. Triggers produce ordinary Tasks through the same admission path. They hold revocable delegation
   references, never user session credentials.
9. Development bearer authentication is loopback-only unless an explicit insecure listener override
   is named and warned.
10. No crate imports Connectors or Identity implementation source. Released contracts and official
    clients are the seams.
11. Anything that runs is Rust. Shell is orchestration only; command-line surfaces use `clap`.
12. Credentials, customer data, private transcripts and production configuration never enter this
    repository.

## AEP planning

`.engineering/planning/` is changed only through `protocol artifact`. Before its first mutation in a
session, run `protocol artifact list`; after a batch, run `protocol artifact validate --strict` and
report its output. Do not hand-edit planning frontmatter or bodies.

## Gate

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
protocol artifact validate --strict
```

Preserve unrelated work. Do not commit or push unless the operator asks.
