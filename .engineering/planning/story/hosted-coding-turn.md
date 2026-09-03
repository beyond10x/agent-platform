---
format: aep.planning-md/1
id: story:hosted-coding-turn
kind: story
status: implemented
title: Execute hosted AgentIDE coding turns
summary: Refresh actor-specific Workspace context and tools for every Harness turn while preserving durable approval recovery.
relations:
- decomposes: epic:task-execution
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
scope:
- confidence: cited
  path: Cargo.toml
- confidence: cited
  path: crates/agent-platform-core
- confidence: cited
  path: crates/agent-platform-harness
- confidence: cited
  path: crates/agent-platform-worker
revision: 5
---
# Story: Execute hosted AgentIDE coding turns

## Outcome

A coding task runs through Harness using a freshly derived Workspace ActorView and the exact current AgentIDE tool inventory while task state retains only durable references, revisions, approvals, and evidence.

## Acceptance

- Each turn refreshes server-derived actor context, bounded prompt attachments, and the digest-sealed current tool inventory.
- Approval checkpoints survive worker restarts and resume only the exact approved or denied plan.
- Child-agent authority is the intersection of the parent delegation and current bounded grant.
- AgentIDE contract dependencies resolve to one released revision across Agent Platform and its DevCenter consumer.
- No source buffer, terminal scrollback, user credential, or parallel file store enters Agent Platform persistence.

## Scope

Hosted coding task input, Workspace context provider, Harness tool adapter, durable checkpoints, task events, and released protocol pins.
