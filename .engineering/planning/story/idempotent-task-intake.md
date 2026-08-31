---
format: aep.planning-md/1
id: story:idempotent-task-intake
kind: story
status: implemented
title: Accept idempotent asynchronous tasks
summary: Pin admitted work to one tenant, agent revision and capability profile.
relations:
- decomposes: epic:task-execution
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
revision: 5
---
## Context

API retries and trigger redelivery must not create two pieces of work, and queued work must not follow a later agent activation silently.

## Acceptance

Task submission requires a bounded idempotency key, resolves the agent's active revision inside the tenant scope, stores actor/executor attribution and the pinned revision, and returns the original task for an equal retry. Reusing the key for different intent is a conflict. The walking slice reports accepted work honestly and does not claim Harness execution before a worker exists.

## Implementation record — 2026-08-31

Implemented bounded asynchronous task admission in `agent-platform-app` and `agent-platform-http`. Equal tenant/key retries return the original task; changed intent conflicts. Admission records actor, executor, delegation, request id, revision and capability profile and reports only `accepted`; durable execution remains a separate story.
