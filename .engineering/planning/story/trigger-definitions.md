---
format: aep.planning-md/1
id: story:trigger-definitions
kind: story
status: implemented
title: Manage schedule and webhook trigger definitions
summary: Persist validated trigger intent that will later create ordinary tasks.
relations:
- decomposes: epic:triggered-work
- serves: vision:O1
- serves: vision:O5
revision: 5
---
## Context

Scheduled and inbound work must share task admission rather than becoming privileged alternate execution paths.

## Acceptance

An authenticated tenant member can create and list bounded schedule or webhook trigger definitions referencing an agent and a task template. Schedule definitions name timezone, misfire and overlap policy; webhook definitions name an input schema. Trigger records contain no reusable user session credential, and this slice does not claim delivery before a dispatcher exists.

## Implementation record — 2026-08-31

Implemented tenant-scoped create/list APIs and validation for schedule and webhook definitions in `agent-platform-core`, `agent-platform-app` and `agent-platform-http`. Definitions pin the current agent revision, include explicit schedule overlap/misfire/timezone or webhook input schema, and do not retain bearer credentials. Delivery remains a separate story.
