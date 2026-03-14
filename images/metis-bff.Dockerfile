# ── Stage 1: Build React SPA ─────────────────────────────────────
FROM node:22-slim AS spa-build

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app/metis-web

# Copy workspace config and lockfile first for layer caching
COPY metis-web/package.json metis-web/pnpm-workspace.yaml metis-web/pnpm-lock.yaml ./
COPY metis-web/packages/api/package.json ./packages/api/package.json
COPY metis-web/packages/ui/package.json ./packages/ui/package.json
COPY metis-web/packages/web/package.json ./packages/web/package.json

RUN pnpm install --frozen-lockfile

# Copy source files
COPY metis-web/tsconfig.base.json ./tsconfig.base.json
COPY metis-web/packages/api/ ./packages/api/
COPY metis-web/packages/ui/ ./packages/ui/
COPY metis-web/packages/web/ ./packages/web/

# Build packages in dependency order: api → ui → web
RUN pnpm --filter @metis/api build && pnpm --filter @metis/ui build && pnpm --filter @metis/web build

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
RUN cargo build --bin metis-bff-server --release

# ── Stage 4: Runtime ─────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/metis-bff-server /usr/local/bin/metis-bff-server
COPY --from=spa-build /app/metis-web/packages/web/dist /app/dist

ENV RUST_LOG=info
ENV PORT=4000
ENV COOKIE_SECURE=true
ENV FRONTEND_ASSETS_DIR=/app/dist
ENV UPSTREAM_URL=http://server.metis.svc.cluster.local

EXPOSE 4000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:4000/health || exit 1

ENTRYPOINT ["metis-bff-server"]
