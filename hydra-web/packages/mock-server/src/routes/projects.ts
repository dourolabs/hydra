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

  // GET /v1/projects/:projectId/statuses
  app.get("/v1/projects/:projectId/statuses", (c) => {
    const projectId = c.req.param("projectId");
    const entry = store.get<Project>(COLLECTION, projectId);
    if (!entry) {
      return c.json({ error: `project '${projectId}' not found` }, 404);
    }
    return c.json(toStatusesResponse(entry.data));
  });

  // GET /v1/projects/:projectId
  app.get("/v1/projects/:projectId", (c) => {
    const projectId = c.req.param("projectId");
    const entry = store.get<Project>(COLLECTION, projectId);
    if (!entry) {
      return c.json({ error: `project '${projectId}' not found` }, 404);
    }
    const record: ProjectRecord = {
      project_id: projectId,
      version: entry.version,
      project: entry.data,
    };
    return c.json(record);
  });

  // PUT /v1/projects/:projectId
  app.put("/v1/projects/:projectId", async (c) => {
    const projectId = c.req.param("projectId");
    const body = await c.req.json<UpsertProjectRequest>();
    const existing = store.get<Project>(COLLECTION, projectId);
    if (!existing) {
      return c.json({ error: `project '${projectId}' not found` }, 404);
    }
    if (body.project.key !== existing.data.key) {
      const conflict = findByKey(store, body.project.key);
      if (conflict && conflict.id !== projectId) {
        return c.json({ error: `a project with key '${body.project.key}' already exists` }, 400);
      }
    }
    const entry = store.update<Project>(COLLECTION, projectId, body.project, null);
    const resp: UpsertProjectResponse = {
      project_id: projectId,
      version: entry.version,
    };
    return c.json(resp);
  });

  // DELETE /v1/projects/:projectId
  app.delete("/v1/projects/:projectId", (c) => {
    const projectId = c.req.param("projectId");
    const existing = store.get<Project>(COLLECTION, projectId);
    if (!existing) {
      return c.json({ error: `project '${projectId}' not found` }, 404);
    }
    const entry = store.delete<Project>(COLLECTION, projectId, null);
    const resp: UpsertProjectResponse = {
      project_id: projectId,
      version: entry.version,
    };
    return c.json(resp);
  });

  return app;
}
