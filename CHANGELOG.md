# Changelog

All notable operator-visible changes to this private service are recorded here.

## [Unreleased]

## [0.2.0] — 2026-08-31

- Added one shared API route catalog consumed by the Axum router and deterministic OpenAPI 3.1
  projection, with schema coverage for every request, response and problem document.
- Exposed the exact generated contract publicly at `/openapi.json` with no-store and nosniff
  headers, plus `agent-platform openapi` and its digest mode for release verification.
- Added a responsive public documentation website built entirely by Rust, embedded into the service
  binary and served under `/docs/` with no Pages, Node, CDN or mutable runtime site directory.
- Added executable route/catalog agreement, OpenAPI determinism and embedded-asset security tests.

## [0.1.0] — 2026-08-31

- Established the private `agent-platform` repository, architectural boundaries and AEP-governed
  delivery record.
- Added authenticated, tenant-scoped in-memory management for stable agents and immutable revisions,
  including compare-and-swap activation.
- Added deterministic projection of Connector operation descriptions into conservative Harness tool
  specifications and a model-free embedded Harness round-trip test.
- Added idempotent asynchronous Task admission and schedule/webhook trigger definitions through a
  bounded Axum API.
- Added a loopback-only development bearer verifier; production Identity, durable persistence, live
  Connector invocation, task workers and trigger delivery remain explicitly outside this release.
