---
format: aep.planning-md/1
id: story:agent-conversation-lifecycle
kind: story
status: active
title: Manage agents, conversations and capability profiles durably
relations:
- informed_by: story:principal-owned-agent-isolation
- informed_by: story:immutable-agent-revisions
- serves: vision:O1
- serves: vision:O5
scope:
- confidence: cited
  path: crates/agent-platform-api/src/lib.rs
- confidence: cited
  path: crates/agent-platform-app
- confidence: cited
  path: crates/agent-platform-client/src/lib.rs
- confidence: cited
  path: crates/agent-platform-connectors/src/lib.rs
- confidence: cited
  path: crates/agent-platform-core/src/lib.rs
- confidence: cited
  path: crates/agent-platform-harness/src/lib.rs
- confidence: cited
  path: crates/agent-platform-http/src/lib.rs
- confidence: cited
  path: crates/agent-platform-openapi/src/lib.rs
- confidence: cited
  path: ess/system
revision: 6
---
## Outcome

An authenticated owner can edit and retire agents, create, rename, clear and delete distinct conversations, and retire unneeded capability profiles through durable service operations. Devcenter can consume these operations before publication using local candidate images.

## Domain

`ess/system/domains/control.yaml` records the Agent identity derived from the existing generated OpenAPI contract and the Conversation identity, ownership and persisted fields. Retirement retains immutable task and revision evidence. A conversation belongs to one agent and creator; it has no meaning independent of that retained agent record.

## Acceptance

- Agent edits atomically compare the expected active revision, validate the new name and revision, create an immutable revision, and activate it without changing already-running work.
- Agent retirement is durable and removes it from normal discovery and new task or trigger admission. Active tasks refuse retirement explicitly; existing revisions and terminal task evidence remain available to the owner.
- Distinct durable conversations isolate visible history and model context. Every new turn derives prior context from the selected conversation on the server, with bounded history, unchanged task idempotency, and owner checks. Concurrent active turns in the same conversation refuse.
- Conversation rename and clear use compare-and-swap. Clear atomically replaces the visible conversation with an empty one; delete hides it durably. Both refuse while a task is active. Existing ungrouped main-agent tasks migrate into one legacy conversation without changing task input or evidence.
- Profile retirement refuses while used by a current agent revision or active task; retained immutable revisions remain auditable. Empty deny-all profiles are valid.
- Cross-tenant and same-tenant different-owner access, refusal atomicity, old-state reload, durable delete and clear, context isolation and API/client/OpenAPI route parity have regression coverage.
- Local browser acceptance proves these operations against candidate images before release. Generated capability schema defects remain rejected with actionable details, never silently granted or coerced into guessed types.

## Scope

Core lifecycle requests and conversation data, application persistence and admission, authenticated route catalog and HTTP handlers, official client and generated OpenAPI, Harness conversation assembly, the bounded ESS ownership model, and corresponding tests. Provider credential handling remains owned by Connectors.

## Validation so far

The complete Rust formatting, clippy with warnings denied, and workspace test gates pass for the initial lifecycle implementation. Application regression tests exercise isolated server-derived conversation context, exact task retries, owner and tenant isolation, active-work refusal, atomic revision editing, retained terminal evidence, legacy-history migration, empty profile creation, profile-in-use refusal and persistent clear/delete/retirement. The deterministic OpenAPI digest was updated from the changed typed route catalog. The shared model-input-schema check is exported from core so downstream profile UX can preflight the same contract instead of guessing types or masking a model refusal. Composed local browser verification remains pending; this story stays active.
