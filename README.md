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

The cross-platform framework decision, fallback criteria, and target product
architecture are documented in
[`跨平台技术路线与产品技术架构调研.md`](./跨平台技术路线与产品技术架构调研.md).

The beachhead market, positioning, ICP, competitive evidence, expansion path,
and six-week validation gates are documented in
[`产品市场定位与切入策略调研.md`](./产品市场定位与切入策略调研.md).

## Project MCP servers

All 12 TikHub social-platform MCP servers are declared only for this repository
in `.codex/config.toml`; no entry is added to `~/.codex/config.toml`. Before
starting Codex from this project, expose the bare TikHub token as
`TIKHUB_API_KEY`; keep the token itself in a local environment or secret
manager, never in Git. The source `.omp/` directory is intentionally ignored
because its local configuration contains credentials.

Restart Codex and open a new task after changing MCP configuration so the
project-scoped servers and environment variable are loaded.
