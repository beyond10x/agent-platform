# Roadmap

The governed status of each item lives in `.engineering/planning/`; this page gives the delivery
order and does not duplicate lifecycle state.

1. **Management walking slice.** Trusted development authentication, tenant-scoped agents,
   immutable revisions, deterministic one-to-one Connector tool projection, idempotent task intake,
   and trigger definitions.
2. **Developer surface.** Generate OpenAPI from the actual route and DTO model, serve it at
   `/openapi.json`, and embed a curated documentation build in the service under `/docs/` without a
   GitHub Pages deployment, Node toolchain or runtime site dependency.
3. **Durable authority.** PostgreSQL, transactional idempotency, Identity's audience-bound verifier,
   delegated execution authority and append-only run evidence.
4. **Execution.** Worker leases around the already embedded Harness adapter, live Connector
   describe/invoke, approval evidence, cancellation and outcome-unknown reconciliation.
5. **Delivery.** Schedule dispatch, authenticated replay-resistant webhooks and Connector event
   triggers, all producing ordinary Tasks.
6. **Product adoption.** A one-way adapter from babelforce `ai-agent-platform` committed definitions
   to generic revisions, followed by shadow comparison and gradual text/effectful cutover. Voice and
   other product channels remain downstream until a generic session contract is deliberately added.
