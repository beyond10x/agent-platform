---
format: aep.planning-md/1
id: initiative:generic-agent-platform
kind: initiative
status: draft
title: Generic agent platform service
summary: One authenticated service for agent lifecycle, capability projection, tasks, runs and triggers.
relations:
- serves: vision:O1
- serves: vision:O5
- serves: vision:O6
revision: 1
---
# Initiative: Generic agent platform service

## Outcome

A standalone multi-tenant service lets authenticated people manage agents and submit work while Harness owns the agent loop, Connectors owns external capabilities and credentials, and Identity owns principal and tenant truth.

## Scope

Agent identities and immutable revisions, agent-specific capability mappings, asynchronous tasks and attempts, trigger definitions, trusted attribution, durable evidence, and the ports that compose Harness, Connectors and Identity.

## Success

A first-party client can complete the governed walking slice through the generic API, and the babelforce ai-agent-platform can later project its product-specific definitions into this service without the service learning babelforce-specific concepts.

## Not This

An identity provider, connector credential store, vendor API catalog, model gateway, voice product, product-specific authoring system, or second implementation of the Harness loop.
