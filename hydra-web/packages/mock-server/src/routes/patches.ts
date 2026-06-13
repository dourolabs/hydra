import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { DEV_USERNAME } from "../auth.js";
import type {
  Patch,
  Principal,
  Review,
  UpsertPatch,
  UpsertPatchRequest,
  UpsertPatchResponse,
  UpsertReviewRequest,
  PatchVersionRecord,
  ListPatchesResponse,
  ListPatchVersionsResponse,
  PatchSummaryRecord,
  PatchSummary,
  ReviewSummary,
} from "@hydra/api";

/**
 * Phase 5b of `/designs/actor-system-overhaul.md` (§6): the
 * mock-server's review-author stamping mirrors the production
 * server's behaviour. For each incoming `UpsertReviewRequest`:
 *
 *   * if the tuple `(contents, is_approved, submitted_at)` matches
 *     an existing review on the prior patch version, preserve the
 *     stored author;
 *   * otherwise stamp the typed `Principal::User { name: DEV_USERNAME }`
 *     — the mock harness has a single authenticated dev user, so
 *     mirroring its identity onto new reviews lines up with the
 *     real auth flow.
 */
function stampReviewAuthors(
  incoming: UpsertReviewRequest[],
  prior: Review[],
): Review[] {
  const defaultAuthor: Principal = { User: { name: DEV_USERNAME } };
  return incoming.map((req) => {
    const match = prior.find(
      (existing) =>
        existing.contents === req.contents &&
        existing.is_approved === req.is_approved &&
        existing.submitted_at === req.submitted_at,
    );
    return {
      contents: req.contents,
      is_approved: req.is_approved,
      author: match ? match.author : defaultAuthor,
      submitted_at: req.submitted_at,
    };
  });
}

/**
 * Translate the wire-shape `UpsertPatch` (with author-less
 * `UpsertReviewRequest`s) into the canonical stored `Patch` shape
 * (with author-bearing `Review`s). Used by both POST (no prior
 * reviews) and PUT (prior reviews loaded from the store).
 */
function upsertPatchToPatch(
  upsert: UpsertPatch,
  prior: Review[],
): Patch {
  return {
    title: upsert.title,
    description: upsert.description,
    diff: upsert.diff,
    status: upsert.status,
    is_automatic_backup: upsert.is_automatic_backup,
    creator: upsert.creator,
    reviews: stampReviewAuthors(upsert.reviews ?? [], prior),
    service_repo_name: upsert.service_repo_name,
    github: upsert.github,
    archived: upsert.archived ?? false,
    branch_name: upsert.branch_name,
    commit_range: upsert.commit_range,
    base_branch: upsert.base_branch,
  };
}

const COLLECTION = "patches";
const SSE_PREFIX = "patch";

function toVersionRecord(
  patchId: string,
  version: number,
  timestamp: string,
  patch: Patch,
  creationTime: string,
): PatchVersionRecord {
  return {
    patch_id: patchId,
    version: BigInt(version),
    timestamp,
    patch,
    creation_time: creationTime,
  };
}

function toSummaryRecord(
  patchId: string,
  version: number,
  timestamp: string,
  patch: Patch,
  creationTime: string,
): PatchSummaryRecord {
  const reviewSummary: ReviewSummary = {
    count: patch.reviews.length,
    approved: patch.reviews.some((r) => r.is_approved),
  };
  const summary: PatchSummary = {
    title: patch.title,
    status: patch.status,
    is_automatic_backup: patch.is_automatic_backup,
    creator: patch.creator,
    review_summary: reviewSummary,
    service_repo_name: patch.service_repo_name,
    github: patch.github,
    branch_name: patch.branch_name,
    base_branch: patch.base_branch,
    archived: patch.archived,
  };
  return {
    patch_id: patchId,
    version: BigInt(version),
    timestamp,
    patch: summary,
    creation_time: creationTime,
  };
}

export function createPatchRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/patches
  app.post("/v1/patches", async (c) => {
    const body = await c.req.json<UpsertPatchRequest>();
    const id = generateId("patch");
    const upsert: UpsertPatch = {
      ...body.patch,
      creator: body.patch.creator || DEV_USERNAME,
    };
    // POST has no prior reviews; every incoming review is "new" and
    // therefore stamped with the dev user's typed Principal.
    const patch = upsertPatchToPatch(upsert, []);
    const entry = store.create<Patch>(COLLECTION, id, patch, SSE_PREFIX);
    const resp: UpsertPatchResponse = {
      patch_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp, 201);
  });

  // PUT /v1/patches/:id
  app.put("/v1/patches/:id", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<UpsertPatchRequest>();
    // Preserve existing review authors on a PUT by matching the
    // incoming tuple against the prior stored patch's reviews.
    const prior = store.get<Patch>(COLLECTION, id)?.data.reviews ?? [];
    const patch = upsertPatchToPatch(body.patch, prior);
    const entry = store.update<Patch>(COLLECTION, id, patch, SSE_PREFIX);
    const resp: UpsertPatchResponse = {
      patch_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp);
  });

  // GET /v1/patches/:id
  app.get("/v1/patches/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Patch>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `patch '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  // GET /v1/patches/:id/versions/:version
  app.get("/v1/patches/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Patch>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `patch '${id}' version ${version} not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  // GET /v1/patches
  app.get("/v1/patches", (c) => {
    const includeDeleted = c.req.query("include_archived") === "true";
    const ids = c.req.query("ids");
    const q = c.req.query("q");
    const statusParam = c.req.query("status");
    const branchName = c.req.query("branch_name");
    const limitParam = c.req.query("limit");
    const cursorParam = c.req.query("cursor");
    const countParam = c.req.query("count");

    const items = store.list<Patch>(COLLECTION, includeDeleted);

    let filtered = items;
    if (ids) {
      const idSet = new Set(ids.split(",").map((s) => s.trim()));
      filtered = filtered.filter(({ id }) => idSet.has(id));
    }
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) => entry.data.title.toLowerCase().includes(lower));
    }
    if (statusParam) {
      const statuses = statusParam.split(",");
      filtered = filtered.filter(({ entry }) => statuses.includes(entry.data.status));
    }
    if (branchName) {
      filtered = filtered.filter(({ entry }) => entry.data.branch_name === branchName);
    }

    // Sort by last-update time descending (most recently updated first) for stable pagination
    filtered.sort((a, b) => {
      return b.entry.timestamp.localeCompare(a.entry.timestamp);
    });

    const totalCount = filtered.length;

    // Apply cursor-based pagination
    if (cursorParam) {
      const cursorIndex = filtered.findIndex(({ id }) => id === cursorParam);
      if (cursorIndex !== -1) {
        filtered = filtered.slice(cursorIndex + 1);
      }
    }

    let nextCursor: string | null = null;
    if (limitParam !== undefined && limitParam !== null) {
      const limit = Number(limitParam);
      if (Number.isFinite(limit) && limit >= 0 && filtered.length > limit) {
        nextCursor = filtered[limit - 1]?.id ?? null;
        filtered = filtered.slice(0, limit);
      }
    }

    const patches: PatchSummaryRecord[] = filtered.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListPatchesResponse = {
      patches,
      next_cursor: nextCursor,
      total_count: countParam === "true" ? BigInt(totalCount) : undefined,
    };
    return c.json(resp);
  });

  // GET /v1/patches/:id/versions
  app.get("/v1/patches/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Patch>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `patch '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data, creationTime),
    );
    const resp: ListPatchVersionsResponse = { versions };
    return c.json(resp);
  });

  // DELETE /v1/patches/:id
  app.delete("/v1/patches/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.delete<Patch>(COLLECTION, id, SSE_PREFIX);
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  return app;
}
