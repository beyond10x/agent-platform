---
format: aep.planning-md/1
id: story:connector-tool-projection
kind: story
status: implemented
title: Compile Connector operations into Harness tools
summary: Produce deterministic one-to-one agent tools without weakening source operation facts.
relations:
- decomposes: epic:capability-projection
- serves: vision:O1
- serves: vision:O5
revision: 5
---
## Context

Connector catalogs are deliberately granular and vendor-shaped; publishing that surface directly to a model wastes context and obscures the agent's actual job.

## Acceptance

A version-one mapping selects exactly one described Connector operation and may assign only a tool name plus contextual description. The compiler copies the exact input schema, preserves required approval, conservatively maps read/mutating/destructive effects into Harness risk and idempotency, refuses duplicate tool names and unknown operations, and produces a deterministic digest over source plus mapping. Credentials, destinations and vendor responses are absent.

## Implementation record — 2026-08-31

Implemented the deterministic v1 projection in `agent-platform-connectors`: one mapping selects one described operation, preserves its input schema and approval posture, maps effects conservatively into Harness envelopes, rejects ambiguity and duplicate tool names, and hashes the compiled result. `agent-platform-harness` embeds Harness 0.8.0 and proves the compiled tool-to-Connector invocation round trip without a live model or vendor.
