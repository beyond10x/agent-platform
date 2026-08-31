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
revision: 1
---
## Context

The repository and releases are private, so documentation must follow service access and deployment rather than GitHub Pages. Operators should not install Node or mount a mutable site directory merely to run the service.

## Acceptance

A curated documentation source explains concepts, API use, authentication and current limitations. Its pinned build consumes the generated OpenAPI asset, produces deterministic static files, and a Rust embedding step packages them into the service binary. `/docs/` serves those immutable assets with correct content types, cache policy, a single-page fallback only where intended, and no runtime Node or writable-filesystem dependency. The docs route is explicitly either authenticated or public-by-operator-policy before implementation; it never exposes private planning records.
