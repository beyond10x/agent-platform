---
format: aep.planning-md/1
id: story:service-bound-model-route
kind: story
status: draft
title: Route service-bound agents without borrowing a user subscription
summary: Agent revisions select an approved deployment model route while user-bound credentials remain Connector-owned and attempt-scoped.
relations:
- derived_from: epic:task-execution
- serves: vision:O1
- serves: vision:O5
revision: 1
---
## Outcome

A service-bound agent can execute unattended work through an operator-approved model route without
borrowing, copying or silently falling back to a human subscription credential.

## Context

Devcenter composes both user-initiated and trigger-initiated agents. A user may explicitly connect a
subscription-capable provider through Connectors for eligible interactive work, but scheduled and
service-bound agents need deployment-owned inference such as an llmgw, Bedrock or OpenRouter route.
Harness owns the loop and the model adapter; Agent Platform owns the immutable revision and task
attempt that must pin which class of route is allowed.

## Acceptance

- An immutable agent revision declares a model-route reference and whether it is user-bound or
  service-bound; it never contains credential bytes or a vendor secret reference.
- Task admission resolves the route inside the tenant and pins its identity/generation into the
  attempt evidence before Harness starts.
- A service-bound attempt cannot select a user credential, even when the submitting user has one;
  missing service routing is a named refusal rather than an implicit provider fallback.
- A user-bound route obtains only a just-in-time, attempt-bounded lease through the owning Connector
  seam and records no credential material in task, log or transcript state.
- Harness receives a provider-neutral model invocation configuration and returns attributable usage,
  refusal and route-generation evidence.
- Conformance tests cover revoked/stale routes, cross-tenant references, absent user leases, absent
  service routes and retrying an attempt after route rotation.

## Out of Scope

Owning provider credentials, implementing provider SDKs, choosing production vendors, billing or
model policy. Connectors and the configured model gateway retain those responsibilities.
