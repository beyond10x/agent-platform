# Changelog

All notable operator-visible changes to this private service are recorded here.

## [Unreleased]

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
