---
format: aep.planning-md/1
id: story:durable-worker-execution
kind: story
status: draft
title: Execute durable tasks through Harness and Connectors
summary: Lease accepted tasks, compile current authority and run Harness with Connector-backed tools.
relations:
- decomposes: epic:task-execution
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
revision: 1
---
## Context

Accepted task state is not execution. Production work needs durable leases, current delegation and Connector revalidation, Harness events, approvals, cancellation and outcome-unknown recovery.

## Acceptance

A PostgreSQL-backed worker leases one accepted task, revalidates its execution authority and capability descriptions, runs the pinned revision through Harness, records every event and Connector audit reference, and reaches one terminal state without replaying an unknown effect. This story remains separate from the management walking slice.
