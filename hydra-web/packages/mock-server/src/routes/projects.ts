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
const DEFAULT_PROJECT_KEY = "default";

// Mirrors `hydra-server/src/domain/projects.rs::default_project()`. Keep the
// status list, flag semantics, and colors aligned with the real backend so
// project-less issues resolve to the same five wire strings the frontend
// expects ("Open", "In progress", "Closed", "Dropped", "Failed").
function defaultProject(): Project {
  return {
    key: DEFAULT_PROJECT_KEY,
    name: "Default",
    statuses: [
      {
        key: "open",
        label: "Open",
        icon: "circle",
        color: "#3498db",
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
      },
      {
        key: "in-progress",
        label: "In progress",
        icon: "circle-dot",
        color: "#f1c40f",
        unblocks_parents: false,
        unblocks_dependents: false,
        cascades_to_children: false,
      },
      {
        key: "closed",
        label: "Closed",
        icon: "check-circle",
        color: "#2ecc71",
        unblocks_parents: true,
        unblocks_dependents: true,
        cascades_to_children: false,
      },
      {
        key: "dropped",
        label: "Dropped",
        icon: "x-circle",
        color: "#795548",
        unblocks_parents: true,
        unblocks_dependents: false,
        cascades_to_children: true,
      },
      {
        key: "failed",
        label: "Failed",
        icon: "alert-circle",
        color: "#e74c3c",
        unblocks_parents: true,
        unblocks_dependents: false,
        cascades_to_children: true,
      },
    ],
    default_status_key: "open",
    creator: "system",
    deleted: false,
  };
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

function toStatusesResponse(project: Project): ProjectStatusesResponse {
  return {
    statuses: project.statuses.map((s: StatusDefinition) => ({ ...s })),
    default_status_key: project.default_status_key,
  };
}

export function createProjectRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/projects/default/statuses — synthesized DefaultProject options
  // for issues with no `project_id`. Registered before the parameterized
  // `:projectId/statuses` route so Hono dispatches "default" here instead of
  // treating it as a stored ProjectId.
  app.get("/v1/projects/default/statuses", (c) => {
    return c.json(toStatusesResponse(defaultProject()));
  });

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
    if (projectId === DEFAULT_PROJECT_KEY) {
      // Mirror the real backend: the default project is read-only and not
      // fetchable by ID. Callers should use the /statuses endpoint.
      return c.json(
        {
          error:
            "the default project is read-only; use GET /v1/projects/default/statuses to fetch its status list",
        },
        400,
      );
    }
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
