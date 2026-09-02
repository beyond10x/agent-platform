---
format: aep.planning-md/1
id: story:todo-connector-execution
kind: story
status: implemented
title: Run Todo through governed Connector approvals
summary: Mint attempt-scoped invoke authority, suspend exact approved calls, and resume with one-use Connector evidence.
relations:
- implements: epic:task-execution
- serves: vision:O1
- serves: vision:O5
revision: 5
---
# Story: Run Todo through governed Connector approvals

## Outcome

A person or delegated agent can execute a compiled Todo capability through Connectors, while every effectful call waits for an exact human decision and resumes only with one-use approval evidence.

## Context

Agent Platform already compiles Connector descriptions into Harness toolsets, but execution previously used a refusing adapter and a deny-all approval port. This story completes the task-execution seam without moving Todo domain behavior into Agent Platform.

## Acceptance

- Identity produces a model-subscription lease and a separate, attempt-bound `connectors.invoke` credential; neither secret is retained in task state or exposed to Harness.
- Harness invokes the exact compiled operation, connection, description lease and JSON input through the hosted Connectors client.
- An approval-required call changes the task to `awaiting_approval`, is visible through authenticated task approval APIs, and blocks its execution worker until resolved.
- Approval resolution is scoped to the exact tenant, task, attempt and call, and approved evidence is consumed by the following invocation exactly once.
- The OpenAPI projection and Rust client include the approval endpoints, and the strict repository gate passes.
- No realm is accepted in an Agent Platform URL, request body, header, or client argument.

## Out of Scope

Todo's generated domain runtime, its persistence adapter, Devcenter presentation, and deployment composition remain owned by their respective repositories.

## Open Questions

None.
