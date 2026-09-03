---
format: aep.planning-md/1
id: story:agentide-0-2-1-alignment
kind: story
status: implemented
title: Align execution with AgentIDE 0.2.1
summary: Keep hosted coding turns on one renderer-to-model context type graph.
relations:
- derived_from: story:agentide-v2-hosted-protocol
- serves: vision:O5
scope:
- confidence: inferred
  path: .engineering/planning/journal.jsonl
- confidence: inferred
  path: .engineering/planning/story/agentide-0-2-1-alignment.md
- confidence: inferred
  path: CHANGELOG.md
- confidence: inferred
  path: Cargo.lock
- confidence: inferred
  path: Cargo.toml
- confidence: inferred
  path: crates/agent-platform-openapi/src/lib.rs
revision: 7
---
# Align execution with AgentIDE 0.2.1

## Context

DevCenter seals renderer drafts before submitting a coding-session turn, Workspace refreshes the resulting actor view, and Agent Platform validates that view before binding Harness tools. All three consumers must resolve the same AgentIDE crate identity and Workspace revision.

## Acceptance

Agent Platform consumes the exact AgentIDE 0.2.1 and Workspace 0.2.9 candidates, preserves pre-turn ActorView validation, and passes its complete repository gate for DevCenter composition.

## Scope

Exact dependency pins, lock state, release metadata, and governed evidence.
