---
format: aep.planning-md/1
id: epic:triggered-work
kind: epic
status: draft
title: Triggered agent work
summary: Turn schedules, webhooks and connector events into ordinary admitted tasks.
relations:
- decomposes: initiative:generic-agent-platform
- serves: vision:O1
- serves: vision:O5
revision: 1
---
# Epic: Triggered agent work

## Outcome

A tenant can define a revocable trigger that produces ordinary Tasks through the same admission path as manual submission.

## Done When

Schedules have explicit timezone, misfire and overlap policy; webhooks have bounded authenticated replay-resistant delivery; every firing is idempotent and revalidates its delegated execution authority.
