# hydra-web

Web interface for Hydra. This is a pnpm monorepo containing three packages:

- **`@hydra/api`** — Typed API client with auto-generated TypeScript types from hydra-server Rust structs
- **`@hydra/ui`** — React component library with a dark terminal-inspired theme (JetBrains Mono font, `#0a0a0a` background, `#00cc66` green accent)
- **`@hydra/web`** — React 19 SPA frontend + Hono BFF (Backend-for-Frontend) server

The BFF server proxies authenticated API requests to hydra-server and serves the React SPA's static assets.

## Prerequisites

- Node.js 22+
- pnpm (install via `corepack enable`)
- Rust toolchain (only needed for regenerating TypeScript types from Rust structs)

## Getting started

```bash
cd hydra-web
pnpm install
```

## Development

Run three processes for full local development:

| Command | Description |
|---|---|
| `pnpm dev` | React dev server on port 3000 (Vite, proxies `/api` and `/auth` to `localhost:4000`) |
| `pnpm dev:server` | BFF server on port 4000 (requires `HYDRA_SERVER_URL` to point to a running hydra-server) |
| `pnpm -r dev:demo` | Component library demo on port 3001 |

## Building

```bash
pnpm build       # Build all three packages (api → ui → web)
pnpm lint        # Run ESLint
pnpm test        # Run Vitest unit tests
pnpm typecheck   # TypeScript type checking across all packages
```

## Regenerating TypeScript types

The `@hydra/api` package contains TypeScript type definitions auto-generated from Rust structs in `hydra-common` using [ts-rs](https://github.com/Aleph-Alpha/ts-rs). These generated files are committed to the repository so that `@hydra/api` can be built without a Rust toolchain.

When Rust API types change in `hydra-common`, regenerate the TypeScript types:

```bash
cd hydra-web
pnpm generate-types
```

This runs `cargo test -p hydra-common --features ts export_bindings` to export TypeScript definitions to `packages/api/src/generated/`, then formats them with Prettier.

CI verifies that generated types are up-to-date by regenerating them and checking for uncommitted diffs.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `HYDRA_SERVER_URL` | `http://server.hydra.svc.cluster.local` | URL of the hydra-server API |
| `PORT` | `4000` | Port the BFF server listens on |
| `NODE_ENV` | (unset in dev) | Set to `production` in Docker for secure cookies |

## Authentication

Users provide a Hydra API token via `POST /auth/login`. The BFF validates the token against hydra-server's `/v1/whoami` endpoint and, on success, stores it in an HttpOnly secure cookie. All subsequent `/api/*` requests are proxied to hydra-server with the token attached as a `Bearer` authorization header. Users can log out via `POST /auth/logout`, which clears the cookie.

## Project structure

```
hydra-web/
├── packages/
│   ├── api/
│   │   └── src/
│   │       ├── generated/     # Auto-generated TypeScript types from Rust (ts-rs)
│   │       ├── client.ts      # HydraApiClient — typed fetch client for hydra-server
│   │       ├── errors.ts      # ApiError class
│   │       ├── sse.ts         # SSE event stream helper
│   │       └── types.ts       # Re-exports from generated types
│   ├── ui/
│   │   └── src/
│   │       ├── components/    # Reusable React components
│   │       ├── theme/         # Global CSS and theme tokens
│   │       └── demo/          # Standalone demo app for the component library
│   └── web/
│       ├── src/               # React SPA source
│       └── server/            # Hono BFF server (auth, API proxy, static serving)
├── package.json
├── pnpm-workspace.yaml
├── tsconfig.base.json
└── vitest.config.ts
```

## Deployment

See [DEPLOYMENT.md](./DEPLOYMENT.md) for Kubernetes deployment instructions.
