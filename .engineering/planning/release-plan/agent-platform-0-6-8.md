---
format: aep.planning-md/1
id: release-plan:agent-platform-0-6-8
kind: release-plan
status: implemented
title: Release Agent Platform 0.6.8
summary: Ship the principal-owned agent isolation fix and promote only Agent Platform into Devcenter.
relations:
- delivers: story:principal-owned-agent-isolation
revision: 3
---
## Intent

Publish the tested principal-owned agent isolation boundary as Agent Platform 0.6.8, let the owner repository trigger Devcenter's component promotion, and pin the resulting immutable runtime digest in the downstream deployment.

## Release checks

- The source tag points at the merged default-branch version commit.
- The Agent Platform release workflow and Devcenter component promotion succeed.
- The downstream deployment changes only the Agent Platform immutable image reference.
- The live Agent Platform workload is ready at the promoted digest and Devcenter remains healthy.
