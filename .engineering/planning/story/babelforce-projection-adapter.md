---
format: aep.planning-md/1
id: story:babelforce-projection-adapter
kind: story
status: archived
title: Project a first adopter's agent definitions into generic revisions (superseded)
summary: Compile committed downstream definitions without importing product vocabulary.
relations:
- decomposes: epic:downstream-adoption
- serves: vision:O5
- serves: vision:O6
revision: 5
---
Archived: superseded by `story:first-adopter-projection-adapter`, which carries the same Context and Acceptance under an id that does not name the adopter.

This id and the filename derived from it are **immutable**, and the literal in them is not a defect
that can be fixed here. The planning CLI has no verb that renames an artifact id, and removing the
file makes `validate --strict` report a deletion rather than a rename — so correcting the name in
place is not available, and the record stays as written.

The corrected successor is `story:first-adopter-projection-adapter`. Plan, cite and relate that one.
Do not delete this file.
