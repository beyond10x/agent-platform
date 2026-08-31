---
format: aep.planning-md/1
id: epic:capability-projection
kind: epic
status: draft
title: Connector capability projection
summary: Compile low-level Connector operations into narrow agent-facing Harness tools.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O1
- serves: vision:O5
revision: 1
---
# Epic: Connector capability projection

## Outcome

A tenant can describe an agent-specific mapping from current Connector operations to model-facing tools, preview the deterministic result, and never widen the source operation's schema or safety posture.

## Done When

The compiler validates source descriptions and mappings, emits Harness ToolSpec values with preserved effect/risk/approval facts, binds the result to source and mapping digests, and live invocation is independently reauthorized by Connectors.
