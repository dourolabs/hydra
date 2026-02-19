# metis-web

Web interface for Metis. This is a pnpm monorepo containing two packages:

- **`@metis/ui`** — React component library with a dark terminal-inspired theme (JetBrains Mono font, `#0a0a0a` background, `#00cc66` green accent)
- **`@metis/web`** — React 19 SPA frontend + Hono BFF (Backend-for-Frontend) server

The BFF server proxies authenticated API requests to metis-server and serves the React SPA's static assets.

## Prerequisites

- Node.js 22+
- pnpm (install via `corepack enable`)

## Getting started

```bash
cd metis-web
pnpm install
```

## Development

Run three processes for full local development:

| Command | Description |
|---|---|
| `pnpm dev` | React dev server on port 3000 (Vite, proxies `/api` and `/auth` to `localhost:4000`) |
| `pnpm dev:server` | BFF server on port 4000 (requires `METIS_SERVER_URL` to point to a running metis-server) |
| `pnpm -r dev:demo` | Component library demo on port 3001 |

## Building

```bash
pnpm build       # Build both packages (UI library first, then web app)
pnpm lint        # Run ESLint
pnpm test        # Run Vitest unit tests
pnpm typecheck   # TypeScript type checking
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `METIS_SERVER_URL` | `http://server.metis.svc.cluster.local` | URL of the metis-server API |
| `PORT` | `4000` | Port the BFF server listens on |
| `NODE_ENV` | (unset in dev) | Set to `production` in Docker for secure cookies |

## Authentication

Users provide a Metis API token via `POST /auth/login`. The BFF validates the token against metis-server's `/v1/whoami` endpoint and, on success, stores it in an HttpOnly secure cookie. All subsequent `/api/*` requests are proxied to metis-server with the token attached as a `Bearer` authorization header. Users can log out via `POST /auth/logout`, which clears the cookie.

## Project structure

```
metis-web/
├── packages/
│   ├── ui/
│   │   └── src/
│   │       ├── components/   # Reusable React components
│   │       ├── theme/        # Global CSS and theme tokens
│   │       └── demo/         # Standalone demo app for the component library
│   └── web/
│       ├── src/              # React SPA source
│       └── server/           # Hono BFF server (auth, API proxy, static serving)
├── package.json
├── pnpm-workspace.yaml
├── tsconfig.base.json
└── vitest.config.ts
```

## Deployment

See [DEPLOYMENT.md](./DEPLOYMENT.md) for Kubernetes deployment instructions.
