import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  ListProjectsResponse,
  Project,
  ProjectRecord,
  ProjectStatusesResponse,
  StatusDefinition,
  UpsertProjectRequest,
  UpsertProjectResponse,
} from "@hydra/api";

const COLLECTION = "projects";

/**
 * True iff `value` matches the `[a-z]-...` shape reserved for
 * `HydraId` values. Mirrors `HydraId::is_id_or_reserved_shape` in
 * `hydra-common/src/ids.rs:140`. Used to dispatch path-segment
 * lookups: id-shape paths go through `store.get()` directly; anything
 * else routes through the key lookup.
 */
function isIdOrReservedShape(value: string): boolean {
  return value.length >= 2 && /^[a-z]-/.test(value);
}

function generateProjectId(): string {
  const suffix = Math.random().toString(36).slice(2, 8);
  return `j-${suffix}`;
}

function findByKey(store: Store, key: string): { id: string; project: Project } | null {
  for (const { id, entry } of store.list<Project>(COLLECTION)) {
    if (entry.data.key === key) return { id, project: entry.data };
  }
  return null;
}

/**
 * Resolve a path-segment project reference to a concrete `(id, Project)`
 * pair. The id branch hits `store.get` directly; the key branch falls
 * back to the in-memory key scan. Mirrors the server-side resolver in
 * `hydra-server/src/routes/projects.rs::resolve_project_ref` so the
 * mock server's 404 surface matches real-server behavior end-to-end.
 */
function resolveProjectRef(
  store: Store,
  projectRef: string,
): { id: string; entry: { version: number; data: Project } } | null {
  if (isIdOrReservedShape(projectRef)) {
    const entry = store.get<Project>(COLLECTION, projectRef);
    return entry ? { id: projectRef, entry } : null;
  }
  const byKey = findByKey(store, projectRef);
  if (!byKey) return null;
  const entry = store.get<Project>(COLLECTION, byKey.id);
  return entry ? { id: byKey.id, entry } : null;
}

function toStatusesResponse(project: Project): ProjectStatusesResponse {
  return {
    statuses: project.statuses.map((s: StatusDefinition) => ({ ...s })),
  };
}

export function createProjectRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/projects
  app.get("/v1/projects", (c) => {
    const items = store.list<Project>(COLLECTION);
    const projects: ProjectRecord[] = items.map(({ id, entry }) => ({
      project_id: id,
      version: entry.version,
      project: entry.data,
    }));
    const resp: ListProjectsResponse = { projects };
    return c.json(resp);
  });

  // POST /v1/projects
  app.post("/v1/projects", async (c) => {
    const body = await c.req.json<UpsertProjectRequest>();
    const existing = findByKey(store, body.project.key);
    if (existing) {
      return c.json({ error: `a project with key '${body.project.key}' already exists` }, 400);
    }
    const id = generateProjectId();
    const entry = store.create<Project>(COLLECTION, id, body.project, null);
    const resp: UpsertProjectResponse = {
      project_id: id,
      version: entry.version,
    };
    return c.json(resp, 201);
  });

  // GET /v1/projects/:projectRef/statuses
  app.get("/v1/projects/:projectRef/statuses", (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    return c.json(toStatusesResponse(resolved.entry.data));
  });

  // GET /v1/projects/:projectRef
  app.get("/v1/projects/:projectRef", (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const record: ProjectRecord = {
      project_id: resolved.id,
      version: resolved.entry.version,
      project: resolved.entry.data,
    };
    return c.json(record);
  });

  // PUT /v1/projects/:projectRef
  app.put("/v1/projects/:projectRef", async (c) => {
    const projectRef = c.req.param("projectRef");
    const body = await c.req.json<UpsertProjectRequest>();
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    if (body.project.key !== resolved.entry.data.key) {
      const conflict = findByKey(store, body.project.key);
      if (conflict && conflict.id !== resolved.id) {
        return c.json({ error: `a project with key '${body.project.key}' already exists` }, 400);
      }
    }
    const entry = store.update<Project>(COLLECTION, resolved.id, body.project, null);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  // DELETE /v1/projects/:projectRef
  app.delete("/v1/projects/:projectRef", (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const entry = store.delete<Project>(COLLECTION, resolved.id, null);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  return app;
}
