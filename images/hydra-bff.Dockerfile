# ── Stage 1: Build React SPA ─────────────────────────────────────
FROM node:22-slim AS spa-build

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app/hydra-web

# Copy workspace config and lockfile first for layer caching
COPY hydra-web/package.json hydra-web/pnpm-workspace.yaml hydra-web/pnpm-lock.yaml ./
COPY hydra-web/packages/api/package.json ./packages/api/package.json
COPY hydra-web/packages/ui/package.json ./packages/ui/package.json
COPY hydra-web/packages/web/package.json ./packages/web/package.json

RUN pnpm install --frozen-lockfile

# Copy source files
COPY hydra-web/tsconfig.base.json ./tsconfig.base.json
COPY hydra-web/packages/api/ ./packages/api/
COPY hydra-web/packages/ui/ ./packages/ui/
COPY hydra-web/packages/web/ ./packages/web/

# Build packages in dependency order: api → ui → web
RUN pnpm --filter @hydra/api build && pnpm --filter @hydra/ui build && pnpm --filter @hydra/web build

# ── Stage 2: Cargo chef planner ──────────────────────────────────
FROM rust:1.88.0 AS planner
RUN cargo install cargo-chef

WORKDIR /app
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: Build Rust binary ───────────────────────────────────
FROM rust:1.88.0 AS builder
RUN cargo install cargo-chef

WORKDIR /app

# Build dependencies (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build the full project
COPY . .
RUN cargo build --bin hydra-bff-server --release

# ── Stage 4: Runtime ─────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/hydra-bff-server /usr/local/bin/hydra-bff-server
COPY --from=spa-build /app/hydra-web/packages/web/dist /app/dist

ENV RUST_LOG=info
ENV PORT=4000
ENV COOKIE_SECURE=true
ENV FRONTEND_ASSETS_DIR=/app/dist
ENV UPSTREAM_URL=http://server.hydra.svc.cluster.local

EXPOSE 4000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:4000/health || exit 1

ENTRYPOINT ["hydra-bff-server"]
