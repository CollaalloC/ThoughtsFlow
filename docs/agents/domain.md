# Domain documentation

This is a single-context repository. Read `CONTEXT.md` for product vocabulary and `docs/adr/` for accepted decisions before changing a module.

Keep Orca as Agent lifecycle authority and preserve exact workspace, Mission, Dispatch and operation ownership. Context Receipts are immutable evidence. Read-only projections and UI scheduling must not silently rewrite these facts.

Document a new domain term or a durable architectural choice only when the implementation needs it. Record bounded implementation plans and verification in `docs/`; do not turn every helper into a domain concept.
