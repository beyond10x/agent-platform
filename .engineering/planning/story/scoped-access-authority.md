---
format: aep.planning-md/1
id: story:scoped-access-authority
kind: story
status: archived
title: Verify scoped Identity access authority
summary: Authorize Agent Platform requests from exact-audience short-lived access credentials.
relations:
- decomposes: epic:authenticated-control-plane
- serves: vision:O1
revision: 3
---
## Outcome

Agent Platform accepts only an exact-audience Identity access credential and derives permissions from its canonical scope set.

## Acceptance

- Production verification resolves `/v1/access-authority`, not a forwarded browser session.
- Tenant, actor, groups, expiry, and scopes come from Identity authority and the bearer is not retained.
- Route authorization uses the resolved scopes rather than a deployment-wide grant of every scope.
- Wrong-audience, expired, malformed, and insufficiently scoped credentials fail closed.

## Out of Scope

Service-bound agents and durable trigger delegation.

## Decision

Archived before implementation. Agent Platform remains the authorization boundary and interprets generic verified Identity sessions using its own policy. Identity stays agnostic: it does not compile Agent Platform audiences, scopes, capabilities, or resource vocabulary into its API or data model.
