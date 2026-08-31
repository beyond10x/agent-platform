---
format: aep.planning-md/1
id: epic:task-execution
kind: epic
status: draft
title: Durable tasks and Harness execution
summary: Submit asynchronous work and execute it through Harness under an immutable revision and toolset.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
revision: 1
---
# Epic: Durable tasks and Harness execution

## Outcome

Every submitted intention becomes an idempotent tenant-scoped Task whose attempts run through Harness against an immutable agent revision, capability profile and authority snapshot.

## Done When

Task admission, worker leasing, execution, cancellation, approvals, Connector audit references, usage and terminal outcomes survive process failure without replaying an effect whose outcome is unknown.
