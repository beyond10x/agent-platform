---
format: aep.planning-md/1
id: story:shared-docs-toolchain
kind: story
status: draft
title: Align documentation tooling and the CI compiler requirement
scope:
- confidence: cited
  path: .github/workflows/b10x-docs-bundle.yml
- confidence: cited
  path: .github/workflows/gate.yml
revision: 3
---
## Outcome

Align the passive documentation bundle producer with the reviewed shared runtime and make the existing CI gate use a compiler meeting the workspace's declared Rust requirement.

## Acceptance

Atlas-generated b10x-docs-bundle.yml pins Docs System 1d4c0262911761118ffdd7037890f541a0688714 without changing selected sources or permissions. The Gate workflow uses Rust 1.98.1, satisfying the existing 1.98 manifest requirement, and passes formatting, strict Clippy and locked tests. Shared source checks and exact dependency admission pass before main integration.

## Evidence and scope

The initial producer-only candidate failed Gate run 35191944384 before compilation because the workflow selected Rust 1.97.1 for crates requiring 1.98. This is a CI pin repair, not a runtime or dependency migration. Scope is .github/workflows/b10x-docs-bundle.yml, .github/workflows/gate.yml and this planning record. The coordinated Website rollout owns the publication requirement; application behavior is unchanged.

The touched Gate workflow also pins its Rust setup action to the exact immutable revision used by the coordinated documentation workflows. The local workflow pin guard correctly refused retaining the previous mutable stable action reference; the repair changes the action reference instead of adding an exception.
