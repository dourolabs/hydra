import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { DEV_USERNAME } from "../auth.js";
import type {
  Patch,
  UpsertPatchRequest,
  UpsertPatchResponse,
  PatchVersionRecord,
  ListPatchesResponse,
  ListPatchVersionsResponse,
  PatchSummaryRecord,
  PatchSummary,
  ReviewSummary,
} from "@metis/api";

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
    created_by: patch.created_by,
    creator: patch.creator,
    review_summary: reviewSummary,
    service_repo_name: patch.service_repo_name,
    github: patch.github,
    branch_name: patch.branch_name,
    base_branch: patch.base_branch,
    deleted: patch.deleted,
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
    const patch: Patch = {
      ...body.patch,
      creator: body.patch.creator || DEV_USERNAME,
      reviews: body.patch.reviews ?? [],
    };
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
    const patch: Patch = {
      ...body.patch,
      reviews: body.patch.reviews ?? [],
    };
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
    const includeDeleted = c.req.query("include_deleted") === "true";
    const q = c.req.query("q");
    const statusParam = c.req.query("status");
    const branchName = c.req.query("branch_name");

    const items = store.list<Patch>(COLLECTION, includeDeleted);

    let filtered = items;
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        entry.data.title.toLowerCase().includes(lower),
      );
    }
    if (statusParam) {
      const statuses = statusParam.split(",");
      filtered = filtered.filter(({ entry }) =>
        statuses.includes(entry.data.status),
      );
    }
    if (branchName) {
      filtered = filtered.filter(({ entry }) => entry.data.branch_name === branchName);
    }

    const patches: PatchSummaryRecord[] = filtered.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListPatchesResponse = { patches };
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
