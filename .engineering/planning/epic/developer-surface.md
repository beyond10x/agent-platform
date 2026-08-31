---
format: aep.planning-md/1
id: epic:developer-surface
kind: epic
status: implemented
title: Embedded developer surface
summary: Expose one generated API contract and an embedded documentation experience from the private service.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O5
revision: 6
---
## Context

A private service still needs an operator and integrator surface, but GitHub Pages would publish from the wrong boundary and create a second deployment. The running binary should expose the documentation matching its exact API generation.

## Outcome

The service generates one deterministic OpenAPI document from its route and DTO model, serves that document publicly at `/openapi.json`, and embeds a curated public static documentation build under the same HTTP origin at `/docs/`. The entire build and embedding path is Rust; running the service has no external site process, runtime toolchain or mutable filesystem dependency. Neither public surface contains tenant data, configuration, credentials, planning records or private operational detail.

## Implementation record — 2026-08-31

Delivered as the version 0.2.0 developer surface: one executable route catalog, deterministic OpenAPI, a CLI projection/digest, and a responsive Rust-built site embedded in the same private service binary. Both public surfaces exclude tenant and planning data and share executable drift tests with the authenticated API.
