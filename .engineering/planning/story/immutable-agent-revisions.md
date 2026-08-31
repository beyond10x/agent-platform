---
format: aep.planning-md/1
id: story:immutable-agent-revisions
kind: story
status: implemented
title: Manage immutable agent revisions
summary: Create tenant-owned agents, append revisions and activate them with compare-and-swap.
relations:
- decomposes: epic:authenticated-control-plane
- serves: vision:O5
revision: 5
---
## Context

Editing a live agent in place makes a running task change underneath itself and erases which configuration produced an outcome.

## Acceptance

An authenticated tenant member can create and list agents, append a validated immutable revision, and activate an existing revision only when the expected active revision matches. Another tenant receives no row, and a task can pin the exact active revision it admitted.

## Implementation record — 2026-08-31

Implemented tenant-scoped agent CRUD, append-only validated revisions, and compare-and-swap activation in `agent-platform-core`, `agent-platform-app` and `agent-platform-http`. Accepted tasks pin the active revision and its capability profile.
