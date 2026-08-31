---
format: aep.planning-md/1
id: story:embedded-documentation-site
kind: story
status: draft
title: Embed and serve the documentation site
summary: Compile curated service documentation into the binary and serve it under /docs/ without GitHub Pages.
relations:
- decomposes: epic:developer-surface
- serves: vision:O5
- depends_on: story:generated-openapi-contract
revision: 2
---
## Context

The repository and releases are private, so documentation must follow service access and deployment rather than GitHub Pages. Operators should not install Node or mount a mutable site directory merely to run the service.

## Acceptance

A curated public documentation source explains concepts, API use, authentication and current limitations. A Rust build step consumes that source and the generated OpenAPI asset, produces deterministic static files, and embeds them into the service binary. `/docs/` serves those immutable assets publicly with correct content types, cache policy and a single-page fallback only where intended; `/openapi.json` is the API-reference source. The build and runtime require no Node process, CDN, writable site directory or network access, and the projection excludes private planning records, deployment configuration, tenant data and credentials.
