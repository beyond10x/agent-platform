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
revision: 1
---
## Acceptance

- `Cargo.toml:48` takes `connectors-client` at a current connectors tag (`v0.11.0` or later).
- `Cargo.toml:37-38` take `agentide-contracts` and `agentide-harness` at a current agentide tag
  (`0.3.5` or later).
- `Cargo.toml:39-40` take `workspace-client` and `workspace-core` by `tag` or `rev` — no
  `branch = "main"` remains anywhere in the manifest.
- The gate passes against the new pins, with the API deltas absorbed in this repository's source
  rather than by relaxing a pin.
- `AGENTS.md` § Pins no longer records these three as held.

## Measured gap, 2026-09-15

| line | dependency | pinned | current | note |
| --- | --- | --- | --- | --- |
| `Cargo.toml:48` | `connectors-client` | `tag = "v0.5.6"` | `v0.11.0` | six minor versions |
| `Cargo.toml:37-38` | `agentide-contracts`, `agentide-harness` | `tag = "0.2.1"` | `0.3.5` | one minor, four patches |
| `Cargo.toml:39-40` | `workspace-client`, `workspace-core` | `branch = "main"` | latest tag `0.2.24`; `main` = `2c25863` today | a branch is not a pin — the build is not reproducible |
| `Cargo.toml:47` | `identity-client` | `tag = "0.5.6"` | `0.5.6` | current; nothing to do |
| `Cargo.toml:44-46` | `harness-*` | `rev = "0f2edfef"` | — | exact rev by design, see the comment at `:42-43` |

Currents read with `git ls-remote --tags` on 2026-09-15. Recorded by the beyond10x org-state review
2026-09-15-run2, lane `10-long-tail` F8, ledger `ORG-0107`.

## Why this is release-shaped and was not done with the note

Six minor versions of connectors and a minor of agentide are an API delta, not a number change, and
replacing the workspace branch pin changes what the build resolves for every consumer. It ends in a
gate run and this repository's own release. The fix pass that recorded the gap was
documentation-only and explicitly barred from pin bumps.

## Scope

- `Cargo.toml` (`:37-40`, `:48`)
- `Cargo.lock`
- whatever source the connectors and agentide API deltas touch under `crates/`
- `AGENTS.md` § Pins
