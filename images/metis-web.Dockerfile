# ── Stage 1: build ──────────────────────────────────────────────
FROM node:22-slim AS build

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

# Compile BFF server TypeScript to JavaScript
RUN pnpm --filter @metis/web exec tsc --project tsconfig.server.json

# ── Stage 2: runtime ───────────────────────────────────────────
FROM node:22-slim AS runtime

RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

# Copy only production dependencies
COPY metis-web/package.json metis-web/pnpm-workspace.yaml metis-web/pnpm-lock.yaml ./
COPY metis-web/packages/api/package.json ./packages/api/package.json
COPY metis-web/packages/ui/package.json ./packages/ui/package.json
COPY metis-web/packages/web/package.json ./packages/web/package.json

RUN pnpm install --frozen-lockfile --prod

# Copy built SPA assets (served by the BFF)
COPY --from=build /app/metis-web/packages/web/dist ./packages/web/dist

# Copy compiled BFF server
COPY --from=build /app/metis-web/packages/web/server-dist ./packages/web/server-dist

# Copy built @metis/api dist (needed as workspace dependency)
COPY --from=build /app/metis-web/packages/api/dist ./packages/api/dist

# Copy built @metis/ui dist (needed as workspace dependency)
COPY --from=build /app/metis-web/packages/ui/dist ./packages/ui/dist

ENV PORT=4000
ENV METIS_SERVER_URL=http://server.metis.svc.cluster.local
ENV NODE_ENV=production

EXPOSE 4000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD node -e "fetch('http://localhost:4000/health').then(r => { if (!r.ok) process.exit(1) })"

WORKDIR /app/packages/web

CMD ["node", "server-dist/index.js"]
