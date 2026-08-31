---
format: aep.planning-md/1
id: epic:downstream-adoption
kind: epic
status: draft
title: Downstream product adoption
summary: Let the babelforce ai-agent-platform project product-specific definitions into the generic service.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O5
- serves: vision:O6
revision: 1
---
# Epic: Downstream product adoption

## Outcome

The babelforce ai-agent-platform consumes this service for generic agent lifecycle and work execution while retaining its product-specific authoring, knowledge, channels, voice, console and SDK facade.

## Done When

A one-way compiler projects a committed downstream AgentDef to an immutable generic revision, the existing API remains compatible through its facade, and no babelforce-specific noun enters this repository's domain or wire.
