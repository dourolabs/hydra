# metis-web

Web interface for Metis. This is a pnpm monorepo containing three packages:

- **`@metis/api`** — Typed API client with auto-generated TypeScript types from metis-server Rust structs
- **`@metis/ui`** — React component library with a dark terminal-inspired theme (JetBrains Mono font, `#0a0a0a` background, `#00cc66` green accent)
- **`@metis/web`** — React 19 SPA frontend

The frontend is served by the Rust BFF (`metis-bff-server`), which proxies authenticated API requests to metis-server.

## Prerequisites

- Node.js 22+
- pnpm (install via `corepack enable`)
- Rust toolchain (only needed for regenerating TypeScript types from Rust structs)

## Getting started

```bash
cd metis-web
pnpm install
```

## Development

Run these processes for local development:

| Command | Description |
|---|---|
| `pnpm dev` | React dev server on port 3000 (Vite, proxies `/api` and `/auth` to `localhost:4000`) |
| `pnpm -r dev:demo` | Component library demo on port 3001 |

## Building

```bash
pnpm build       # Build all three packages (api → ui → web)
pnpm lint        # Run ESLint
pnpm test        # Run Vitest unit tests
pnpm typecheck   # TypeScript type checking across all packages
```

## Regenerating TypeScript types

The `@metis/api` package contains TypeScript type definitions auto-generated from Rust structs in `metis-common` using [ts-rs](https://github.com/Aleph-Alpha/ts-rs). These generated files are committed to the repository so that `@metis/api` can be built without a Rust toolchain.

When Rust API types change in `metis-common`, regenerate the TypeScript types:

```bash
cd metis-web
pnpm generate-types
```

This runs `cargo test -p metis-common --features ts export_bindings` to export TypeScript definitions to `packages/api/src/generated/`, then formats them with Prettier.

CI verifies that generated types are up-to-date by regenerating them and checking for uncommitted diffs.

## Project structure

```
metis-web/
├── packages/
│   ├── api/
│   │   └── src/
│   │       ├── generated/     # Auto-generated TypeScript types from Rust (ts-rs)
│   │       ├── client.ts      # MetisApiClient — typed fetch client for metis-server
│   │       ├── errors.ts      # ApiError class
│   │       ├── sse.ts         # SSE event stream helper
│   │       └── types.ts       # Re-exports from generated types
│   ├── ui/
│   │   └── src/
│   │       ├── components/    # Reusable React components
│   │       ├── theme/         # Global CSS and theme tokens
│   │       └── demo/          # Standalone demo app for the component library
│   └── web/
│       └── src/               # React SPA source
├── package.json
├── pnpm-workspace.yaml
├── tsconfig.base.json
└── vitest.config.ts
```

## Deployment

The frontend is deployed as part of the `metis-bff-server` Rust binary. See `images/metis-bff.Dockerfile`.
