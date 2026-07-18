# ThoughsFlow UI Prototype

> Throwaway interaction prototype for deciding what a local-first AI branching
> workspace should feel like. This is not production application code.

The same sample workspace is rendered in three structurally different ways:

- `?variant=focus` - a reading-first route with branching as progressive disclosure;
- `?variant=canvas` - a spatial, technical-cartography canvas;
- `?variant=trace` - a context-proof and request-snapshot workbench.

The bottom prototype switcher, and its left/right keyboard shortcuts, only render
in Vite development mode.

## Run

```bash
npm install
npm run dev
```

Open <http://127.0.0.1:5173/?variant=focus>.

## Verify

```bash
npm run build
```

The product rationale, interaction model, visual direction, and validation plan
are documented in [`PRODUCT_BLUEPRINT.md`](./PRODUCT_BLUEPRINT.md).
