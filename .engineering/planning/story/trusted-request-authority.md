---
format: aep.planning-md/1
id: story:trusted-request-authority
kind: story
status: implemented
title: Construct trusted request authority
summary: Authenticate before body decoding and derive tenant, actor and request metadata server-side.
relations:
- decomposes: epic:authenticated-control-plane
- serves: vision:O1
revision: 5
---
## Context

A management request must not assert its tenant, actor, executor, request id or receive time, and raw credential bytes must not reach application code.

## Acceptance

A credential verifier is the only port that receives Authorization. Authenticated routes run verification before JSON materialization; the application receives a credential-free authority with tenant, subject, optional executor/delegation and scopes. Cross-tenant selection is structurally impossible, and the loopback development verifier cannot bind a reachable listener without an explicit insecure override.

## Implementation record — 2026-08-31

Implemented in `agent-platform-auth`, `agent-platform-http` and the service binary. Authorization is consumed by the verifier before JSON extraction; application requests carry only verified tenant/authority/scopes plus a server-created request id and receive time. Tenant state is selected before resource lookup, and non-loopback development listeners require an explicit insecure override.
