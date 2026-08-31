---
format: aep.planning-md/1
id: epic:developer-surface
kind: epic
status: draft
title: Embedded developer surface
summary: Expose one generated API contract and an embedded documentation experience from the private service.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O5
revision: 1
---
## Context

A private service still needs an operator and integrator surface, but GitHub Pages would publish from the wrong boundary and create a second deployment. The running binary should expose the documentation matching its exact API generation.

## Outcome

The service generates one deterministic OpenAPI document from its route and DTO model, serves that document, and embeds a curated static documentation build under the same HTTP origin. Building documentation may use a pinned Node toolchain, but running the service has no Node or filesystem-site dependency.
