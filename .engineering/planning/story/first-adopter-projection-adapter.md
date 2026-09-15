---
format: aep.planning-md/1
id: story:first-adopter-projection-adapter
kind: story
status: draft
title: Project a first adopter's agent definitions into generic revisions
summary: Compile committed downstream definitions without importing product vocabulary.
relations:
- decomposes: epic:downstream-adoption
- serves: vision:O5
- serves: vision:O6
revision: 1
---
## Context

The existing first adopter's ai-agent-platform already owns product-specific flows, knowledge, voice, channels, console and SDK contracts. Moving everything at once would merge product and platform boundaries.

## Acceptance

A downstream adapter projects a committed first-adopter AgentDef into this service's normalized immutable revision and capability profile, preserves stable correlation, and can shadow-compare reach and safety facts before cutover. No adopter-specific field or dependency enters this repository.
