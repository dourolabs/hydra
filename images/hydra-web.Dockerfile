# ── Stage 1: build ──────────────────────────────────────────────
FROM node:22-slim AS build

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

# Compile BFF server TypeScript to JavaScript
RUN pnpm --filter @hydra/web exec tsc --project tsconfig.server.json

# ── Stage 2: runtime ───────────────────────────────────────────
FROM node:22-slim AS runtime

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

# Copy only production dependencies
COPY hydra-web/package.json hydra-web/pnpm-workspace.yaml hydra-web/pnpm-lock.yaml ./
COPY hydra-web/packages/api/package.json ./packages/api/package.json
COPY hydra-web/packages/ui/package.json ./packages/ui/package.json
COPY hydra-web/packages/web/package.json ./packages/web/package.json

RUN pnpm install --frozen-lockfile --prod

# Copy built SPA assets (served by the BFF)
COPY --from=build /app/hydra-web/packages/web/dist ./packages/web/dist

# Copy compiled BFF server
COPY --from=build /app/hydra-web/packages/web/server-dist ./packages/web/server-dist

# Copy built @hydra/api dist (needed as workspace dependency)
COPY --from=build /app/hydra-web/packages/api/dist ./packages/api/dist

# Copy built @hydra/ui dist (needed as workspace dependency)
COPY --from=build /app/hydra-web/packages/ui/dist ./packages/ui/dist

ENV PORT=4000
ENV METIS_SERVER_URL=http://server.metis.svc.cluster.local
ENV NODE_ENV=production

EXPOSE 4000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD node -e "fetch('http://localhost:4000/health').then(r => { if (!r.ok) process.exit(1) })"

WORKDIR /app/packages/web

CMD ["node", "server-dist/index.js"]
