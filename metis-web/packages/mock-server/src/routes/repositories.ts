import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  Repository,
  RepositoryRecord,
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
  UpsertRepositoryResponse,
  ListRepositoriesResponse,
  DeleteRepositoryResponse,
} from "@hydra/api";

const COLLECTION = "repositories";

export function createRepositoryRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/repositories
  app.get("/v1/repositories", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const items = store.list<Repository>(COLLECTION, includeDeleted);
    const repositories: RepositoryRecord[] = items.map(({ id, entry }) => ({
      name: id,
      repository: entry.data,
    }));
    const resp: ListRepositoriesResponse = { repositories };
    return c.json(resp);
  });

  // POST /v1/repositories
  app.post("/v1/repositories", async (c) => {
    const body = await c.req.json<CreateRepositoryRequest>();
    const repo: Repository = {
      remote_url: body.remote_url,
      default_branch: body.default_branch,
      default_image: body.default_image,
      deleted: body.deleted,
      patch_workflow: body.patch_workflow,
    };
    store.create<Repository>(COLLECTION, body.name, repo, null);
    const record: RepositoryRecord = { name: body.name, repository: repo };
    const resp: UpsertRepositoryResponse = { repository: record };
    return c.json(resp, 201);
  });

  // PUT /v1/repositories/:org/:repo
  app.put("/v1/repositories/:org/:repo", async (c) => {
    const org = c.req.param("org");
    const repo = c.req.param("repo");
    const name = `${org}/${repo}`;
    const body = await c.req.json<UpdateRepositoryRequest>();
    const existing = store.get<Repository>(COLLECTION, name);
    if (!existing) {
      return c.json({ error: `repository '${name}' not found` }, 404);
    }
    const updated: Repository = {
      remote_url: body.remote_url,
      default_branch: body.default_branch,
      default_image: body.default_image,
      deleted: body.deleted,
      patch_workflow: body.patch_workflow,
    };
    store.update<Repository>(COLLECTION, name, updated, null);
    const record: RepositoryRecord = { name, repository: updated };
    const resp: UpsertRepositoryResponse = { repository: record };
    return c.json(resp);
  });

  // DELETE /v1/repositories/:org/:repo
  app.delete("/v1/repositories/:org/:repo", (c) => {
    const org = c.req.param("org");
    const repo = c.req.param("repo");
    const name = `${org}/${repo}`;
    const entry = store.delete<Repository>(COLLECTION, name, null);
    const record: RepositoryRecord = { name, repository: entry.data };
    const resp: DeleteRepositoryResponse = { repository: record };
    return c.json(resp);
  });

  return app;
}
