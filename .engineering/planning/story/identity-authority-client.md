---
format: aep.planning-md/1
id: story:identity-authority-client
kind: story
status: draft
title: Adopt a shared Identity authority client
summary: Resolve audience-bound opaque credentials into a neutral verified authority contract.
relations:
- decomposes: epic:authenticated-control-plane
- serves: vision:O1
revision: 1
---
## Context

AEP service and agent-platform need the same authentication concept, while their realm/workspace and agent/task authorization policies remain different.

## Acceptance

After Identity's audience registry ships, an Identity-owned credential-free authority contract and bounded verifier client supply issuer, audience, tenant, authority, optional executor/delegation, scopes and expiry. This service maps those facts into its own authorization and retains no bearer value.
