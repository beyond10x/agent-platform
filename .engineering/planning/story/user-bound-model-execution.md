---
format: aep.planning-md/1
id: story:user-bound-model-execution
kind: story
status: implemented
title: Execute a user-bound model task through Harness
summary: Reduce a live session to an attempt lease and stream credential-safe task evidence.
relations:
- decomposes: epic:task-execution
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
revision: 4
---
## Context

The first authenticated product journey needs a useful task result before the PostgreSQL worker and service-bound model route are complete. A user-bound model credential remains owned by Connectors and must never become application state.

## Acceptance

An authenticated task submission creates an exact attempt, exchanges the live user session for the Connectors lease scope, and stores neither credential. The worker receives only a finite, expiring, attempt-bound lease; Harness redeems it at each provider call. Task events expose accepted, running, text delta, completion, and credential-safe failure states through a tenant-scoped streaming endpoint. Process restart may interrupt this walking-slice execution and records that limitation without claiming durable execution.
