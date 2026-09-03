---
format: aep.planning-md/1
id: story:agentide-v2-hosted-protocol
kind: story
status: implemented
title: Adopt AgentIDE v2 hosted protocol
summary: Validate and execute the released sealed hosted coding-turn boundary.
relations:
- derived_from: story:hosted-coding-turn
- serves: vision:O5
scope:
- confidence: inferred
  path: CHANGELOG.md
- confidence: inferred
  path: Cargo.lock
- confidence: inferred
  path: Cargo.toml
- confidence: inferred
  path: crates/agent-platform-harness/Cargo.toml
- confidence: inferred
  path: crates/agent-platform-harness/src/lib.rs
- confidence: inferred
  path: crates/agent-platform-openapi/src/lib.rs
revision: 7
---
# Adopt AgentIDE v2 hosted protocol

## Outcome

Agent Platform consumes the released AgentIDE 0.2.0 and Workspace 0.2.8 boundaries so every coding turn validates the sealed actor view, context digest, attachment provenance, current inventory, and independent coordination revision before model execution.

## Acceptance

- Direct AgentIDE and Harness dependencies resolve to released exact commits.
- Workspace actor views are validated before their context and inventory enter Harness.
- Coding-turn fixtures use complete v2 attachment provenance and sealed context packs.
- The repository gate passes and the change is released.

## Scope

AgentIDE, Workspace, and Harness dependency pins plus the hosted coding-turn adapter and fixtures.
