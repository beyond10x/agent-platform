---
format: aep.planning-md/1
id: story:dependency-pins-current
kind: story
status: draft
title: Bring the external dependency pins to current tags and replace the workspace branch pin
summary: connectors is pinned v0.5.6 against v0.11.0 and agentide 0.2.1 against 0.3.5, and workspace is taken by branch = main, which is not a pin.
tags:
- dependencies
- pins
relations:
- serves: vision:O5
revision: 4
---
## Acceptance

- `Cargo.toml:48` takes `connectors-client` at the **head of the v1 line**, `tag = "v0.7.2"`. Done
  2026-09-15; `cargo check -p agent-platform-auth --all-targets` is clean with no source change.
  It must **not** be moved to `v0.8.0` or later as if that were a version bump: those tags are the
  v2 lineage under the same namespace (atlas ADR 0051, accepted 2026-09-15). v1 will publish no
  further tag, so `v0.7.2` is the last v1 pin that can exist.
- `Cargo.toml:37-38` take `agentide-contracts` and `agentide-harness` at `0.3.5`, the current tag.
  Done 2026-09-15; `cargo check -p agent-platform-core -p agent-platform-auth -p
  agent-platform-harness --all-targets` is clean with no source change.
- **Still open — the substantive remainder.** The v1 to v2 migration for `connectors-client` is
  carried out as its own change against `v0.11.0` or later, replacing the `HostedClient`,
  `SubscriptionLease`, `RedeemedSubscription` and `operation` surface that
  `crates/agent-platform-auth/src/lib.rs` is written against, with the delta absorbed in this
  repository's source rather than by relaxing a pin.
- **Still open.** `Cargo.toml:39-40` take `workspace-client` and `workspace-core` by `tag` or `rev` —
  no `branch = "main"` remains anywhere in the manifest. Retiring it also collapses the two linked
  copies of `agentide-contracts` (`0.3.5` from `:37-38`, `0.3.4` via `workspace-client`'s rev pin).
- The gate passes against the new pins.
- `AGENTS.md` § Pins records each pin's actual state and why any held pin is held.

## Measured gap, 2026-09-15

| line | dependency | pinned | current | note |
| --- | --- | --- | --- | --- |
| `Cargo.toml:48` | `connectors-client` | `tag = "v0.7.2"` | `v0.11.0` | head of the v1 line. `v0.11.0` is the v2 lineage, not six minor versions of v1 |
| `Cargo.toml:37-38` | `agentide-contracts`, `agentide-harness` | `tag = "0.3.5"` | `0.3.5` | current |
| `Cargo.toml:39-40` | `workspace-client`, `workspace-core` | `branch = "main"` | latest tag `0.2.24`; `main` = `2c25863` today | a branch is not a pin — the build is not reproducible |
| `Cargo.toml:47` | `identity-client` | `tag = "0.5.6"` | `0.5.6` | current; nothing to do |
| `Cargo.toml:44-46` | `harness-*` | `rev = "0f2edfef"` | — | exact rev by design, see the comment at `:42-43` |

Currents read with `git ls-remote --tags` on 2026-09-15 against each origin.

The original gap note recorded `connectors-client` as "six minor versions" behind `v0.11.0`. That
reading was wrong, and the number was the whole of it: `beyond10x/connectors` carries two lineages in
one tag namespace — `v0.2.0`–`v0.7.2` are v1, `v0.8.0`–`v0.11.0` are a clean-room rewrite that took
the repository identity (atlas ADR 0051, accepted 2026-09-15). Comparing the two numbers measures
nothing. The v1 pin was six v1 releases behind the v1 head and is now at it; crossing to v2 is a
separate migration, sized separately.

Recorded by the beyond10x org-state review 2026-09-15-run2, lane `10-long-tail` F8, ledger
`ORG-0107`.

## Why this is release-shaped and was not done with the note

Two of the three moves turned out to be manifest edits after all. `connectors-client` `v0.5.6` to
`v0.7.2` and `agentide-*` `0.2.1` to `0.3.5` each check clean with no source change — the predicted
API delta was not there, because the six-version figure was measuring across two lineages rather
than along one.

What is genuinely release-shaped is what is left. The v1 to v2 migration replaces the client surface
`agent-platform-auth` is built on, and replacing the `workspace-*` branch pin changes what the build
resolves for every consumer. Both end in a gate run and this repository's own release.

## Scope

- `Cargo.toml` (`:37-40`, `:48`)
- `Cargo.lock`
- whatever source the connectors and agentide API deltas touch under `crates/`
- `AGENTS.md` § Pins
