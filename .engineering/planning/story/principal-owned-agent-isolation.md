---
format: aep.planning-md/1
id: story:principal-owned-agent-isolation
kind: story
status: implemented
title: Keep agents private to their creating principal
summary: Enforce owner scoping for agent enumeration and every agent-rooted operation.
relations:
- decomposes: epic:authenticated-control-plane
- serves: vision:O1
scope:
- confidence: cited
  path: AGENTS.md
- confidence: cited
  path: crates/agent-platform-app/src/lib.rs
- confidence: cited
  path: crates/agent-platform-http/src/lib.rs
revision: 5
---
## Outcome

An authenticated principal can enumerate and operate only agents that principal created; another principal in the same tenant receives the same not-found response as for an unknown identifier.

## Acceptance

- Agent listing returns only agents whose `created_by` equals the verified authority subject.
- Get, revision creation/listing/activation, task admission, and trigger creation refuse another principal’s agent as not found.
- Task idempotency is scoped to the verified authority subject so another principal cannot observe, collide with, or recover an owner’s task.
- Trigger listing returns only triggers whose authority subject equals the verified authority subject.
- Same-tenant, distinct-principal application and HTTP tests prove enumeration and direct-ID isolation while the owner journey remains functional.
- Existing agents remain owned by their persisted `created_by` subject; no ownership migration or tenant-wide sharing is inferred.

## Out of Scope

Explicit team sharing is a future typed authorization feature; it is not represented by tenant membership.
