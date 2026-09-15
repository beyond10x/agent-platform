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

A first adopter's `ai-agent-platform` is a downstream product. Its flows, knowledge,
voice, A2A channels, manager specialists, console, quotas and SDK facade do not enter this domain.

## Visibility

This repository is public. Documentation is available from the repository and is also served by the
service from an embedded, curated build; it is not published through GitHub Pages. Crates are not
published to a registry.

## Invariants

1. Raw credential bytes reach only a credential verifier. Application and persistence receive a
   credential-free verified authority.
2. Tenant, actor, executor, request id and receive time are server-derived. Request bodies cannot
   assert them.
3. Every store operation is tenant-scoped by construction. Human-owned agents, their revisions,
   tasks, and triggers are additionally scoped to the verified authority subject; tenant membership
   never implies access to another principal's agent. A global lookup followed by filtering is not
   accepted.
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
13. The API route catalog is consumed by both Axum and the deterministic OpenAPI projection.
    Handwritten parallel path or payload inventories are defects.
14. `/docs/` and `/openapi.json` are public, curated service metadata embedded in the binary. Their
    Rust-only build never projects planning records, tenant data, credentials or deployment config.

## Pins

**Verified 2026-09-15** with `git ls-remote --tags` against each origin, in this checkout.

| line | dependency | pinned | current | state |
| --- | --- | --- | --- | --- |
| `Cargo.toml:48` | `connectors-client` | `tag = "v0.7.2"` | `v0.11.0` | **head of the v1 line.** `v0.11.0` is a different lineage — see below |
| `Cargo.toml:37-38` | `agentide-contracts`, `agentide-harness` | `tag = "0.3.5"` | `0.3.5` | current |
| `Cargo.toml:39-40` | `workspace-client`, `workspace-core` | `branch = "main"` | latest tag `0.2.24`; `main` resolves to `2c25863` today | **not a pin** — the build is not reproducible |
| `Cargo.toml:47` | `identity-client` | `tag = "0.5.6"` | `0.5.6` | current |
| `Cargo.toml:44-46` | `harness-*` | `rev = "0f2edfef"` | — | exact by design, see `:42-43` |

### `connectors` is two lineages sharing one tag namespace

`beyond10x/connectors` publishes two lineages under one set of tags: **`v0.2.0`–`v0.7.2` are v1**,
and **`v0.8.0`–`v0.11.0` are v2**, a clean-room rewrite that took the repository identity. Its
default branch `next` is v2's. Atlas ADR 0051, accepted 2026-09-15.

So `v0.11.0` is **not "six minor versions ahead" of a v1 pin** — it is a different implementation of
the component, and a diff of the version numbers says nothing about the work. Moving `:48` onto
`v0.8.0` or later is a **lineage migration**: it replaces the `HostedClient`, `SubscriptionLease`,
`RedeemedSubscription` and `operation` surface that `agent-platform-auth` is written against
(`crates/agent-platform-auth/src/lib.rs:11-12,107-108,128,374-393,507`). That migration is the real
remaining work and is tracked by `story:dependency-pins-current`. It is not a manifest edit and must
not be attempted as one.

`:48` is therefore held at `v0.7.2` **deliberately**, not through neglect. v1 will cut no further tag
under this identity — `v0.8.0` is already published and belongs to v2 — so `v0.7.2` is the last v1
pin that can ever exist.

### Still open

`workspace-*` is taken by branch, which is not a pin: `main` moves, so the build is not reproducible,
and replacing it changes what every consumer's build resolves. Tracked by the same story. Do not add
a new `branch = ` dependency.

Two copies of `agentide-contracts` are linked today: `0.3.5` from `:37-38`, and `0.3.4` pulled in by
`workspace-client` at `rev = "081761e3"` (`Cargo.lock`). Their types are distinct to the compiler.
Retiring the `workspace-*` branch pin is what collapses them; until then, do not pass a contracts
type between the two paths.

## AEP planning

`.engineering/planning/` is changed only through `protocol artifact`. Before its first mutation in a
session, run `protocol artifact list`; after a batch, run `protocol artifact validate --strict` and
report its output. Do not hand-edit planning frontmatter or bodies.

**Artifact ids are immutable, including their literals.** There is no verb that renames an id, and
removing a file makes `validate --strict` report a deletion rather than a rename. One archived story
therefore carries an adopter's name in its id and filename and will keep carrying it; its body says
so and names its successor, `story:first-adopter-projection-adapter`, which holds the same Context
and Acceptance under a generic id. Plan and cite the successor. Do not delete the archived file, and
do not treat the literal in it as a defect to fix here.

## Gate

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
protocol artifact validate --strict
```

Preserve unrelated work. Do not commit or push unless the operator asks.

<!-- b10x-docs-operations:start -->
## Public documentation operations

This repository owns the public source and presentation allowlist in `b10x.docs.yaml`. The generated credential-free `.github/workflows/b10x-docs-bundle.yml` passively packages only those declared files for the exact successful `main` commit; it must never run repository code. Atlas selects the latest successful bundle with every other catalog source, and Website plus Docs System own rendering, shared components, search, and feeds. Do not add a standalone docs deployer or put App credentials in this public repository. If Atlas catalogs a former Pages workflow, that file remains repository-owned validation: preserve its bespoke checks while keeping exact read-only permissions, an unconditional pull-request trigger, and no deployment primitives. Project Pages at `/agent-platform/` is only the generated stable redirect façade in `.github/workflows/b10x-docs-pages.yml`; content-only publication never rebuilds it.

From the complete organization workspace, verify the contract with a clean Atlas checkout at the current remote `main`. Set `B10X_ATLAS_CHECKOUT` to a managed Atlas worktree when the primary checkout is dirty or stale; never infer command availability from the primary alone.

```bash
atlas_checkout="${B10X_ATLAS_CHECKOUT:-atlas}"
atlas_head="$(git -C "$atlas_checkout" rev-parse HEAD)"
atlas_main="$(git -C "$atlas_checkout" ls-remote origin refs/heads/main | awk '{print $1}')"
test -z "$(git -C "$atlas_checkout" status --porcelain)"
test "$atlas_head" = "$atlas_main"
cargo run --manifest-path "$atlas_checkout/Cargo.toml" --locked -q -- \
  --store "$atlas_checkout/catalog/store" docs reconcile --workspace . --check
```

Keep internal plans, stories, ADRs, decisions, worklogs, security material, and research out of the public allowlist unless a repository authority explicitly declares them public.
<!-- b10x-docs-operations:end -->

<!-- b10x-release-operations:start -->
## Release completion

An ordinary release completes after this repository's exact tag, required source checks,
published release and required artifacts are verified. A pushed tag with unfinished checks or
uploads is queued; report it as released only after those requirements succeed.

Atlas reconciliation and public documentation publication run asynchronously. Do not wait for
Atlas or Website, update Website source locks or bootstrap snapshots, promote consumer pins,
release shared docs tooling, or redeploy documentation façades as part of an ordinary source
release. Report documentation as pending unless its publication was actually verified. A background
documentation failure does not invalidate a successful source release.

Keep this repository's provenance, correctness, security, compatibility and artifact verification
requirements. Shared rendering, routing or delivery-control changes still require their relevant
integration gates. A release request does not authorize deployment or downstream releases.
Repositories without a release unit retain their existing publication policy. This completion
boundary supersedes older instructions that attach synchronous documentation ceremony to each
source release.
<!-- b10x-release-operations:end -->
