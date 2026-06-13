import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  Issue,
  ListProjectsResponse,
  Project,
  ProjectRecord,
  ProjectStatusesResponse,
  StatusDefinition,
  UpsertProjectRequest,
  UpsertProjectResponse,
  UpsertProjectStatusResponse,
} from "@hydra/api";

const COLLECTION = "projects";
const ISSUES_COLLECTION = "issues";

/**
 * Cascade-archive every non-archived issue matching `predicate`. Mirrors
 * the real server's cascade in `archive_project` / `archive_status`:
 * idempotent on already-archived issues. Returns the number of issues
 * that flipped.
 */
function cascadeArchiveIssues(
  store: Store,
  predicate: (issue: Issue) => boolean,
): number {
  const items = store.list<Issue>(ISSUES_COLLECTION);
  let count = 0;
  for (const { id, entry } of items) {
    if (!predicate(entry.data)) continue;
    store.delete<Issue>(ISSUES_COLLECTION, id, "issue");
    count += 1;
  }
  return count;
}

/**
 * True iff `value` matches the `[a-z]-...` shape reserved for
 * `HydraId` values. Mirrors `HydraId::is_id_or_reserved_shape` in
 * `hydra-common/src/ids.rs`.
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
 * pair. Mirrors `hydra-server/src/routes/projects.rs::resolve_project_ref`.
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
    statuses: orderedStatuses(project).map((s) => ({ ...s })),
  };
}

function orderedStatuses(project: Project): StatusDefinition[] {
  // Match the server's ORDER BY position, sequence — `position` is a
  // float wire field added by the per-status CRUD cutover; stable
  // sort preserves insertion order on ties.
  return [...project.statuses].sort((a, b) => {
    const pa = a.position ?? 0;
    const pb = b.position ?? 0;
    return pa - pb;
  });
}

export function createProjectRoutes(store: Store): Hono {
  const app = new Hono();

  // GET /v1/projects
  app.get("/v1/projects", (c) => {
    const items = store.list<Project>(COLLECTION);
    const projects: ProjectRecord[] = items.map(({ id, entry }) => ({
      project_id: id,
      version: entry.version,
      project: { ...entry.data, statuses: orderedStatuses(entry.data) },
    }));
    // Mirror the real server's `ORDER BY priority ASC, id ASC` (see
    // `hydra-server/src/store/memory_store.rs::list_projects`). Without
    // this, the board's drag-to-reorder PUT lands correctly but the GET
    // on reload returns insertion order, so the new order appears not
    // to persist when dev-testing against this mock. The `project_id`
    // tiebreak is what keeps the order stable across non-priority
    // updates ([[i-esgcpsmn]]).
    projects.sort(
      (a, b) =>
        a.project.priority - b.project.priority ||
        a.project_id.localeCompare(b.project_id),
    );
    const resp: ListProjectsResponse = { projects };
    return c.json(resp);
  });

  // POST /v1/projects — project-level fields only post-cutover.
  app.post("/v1/projects", async (c) => {
    const body = await c.req.json<UpsertProjectRequest>();
    const existing = findByKey(store, body.key);
    if (existing) {
      return c.json({ error: `a project with key '${body.key}' already exists` }, 400);
    }
    const id = generateProjectId();
    const project: Project = {
      key: body.key,
      name: body.name,
      statuses: [],
      creator: "mock-user",
      archived: false,
      prompt_path: body.prompt_path ?? undefined,
      priority: body.priority ?? 0,
    };
    const entry = store.create<Project>(COLLECTION, id, project, null);
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

  // POST /v1/projects/:projectRef/statuses — add a new status.
  app.post("/v1/projects/:projectRef/statuses", async (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const body = await c.req.json<StatusDefinition>();
    if (resolved.entry.data.statuses.some((s) => s.key === body.key)) {
      return c.json(
        { error: `status '${body.key}' already exists on project '${resolved.id}'` },
        400,
      );
    }
    const nextProject: Project = {
      ...resolved.entry.data,
      statuses: [...resolved.entry.data.statuses, body],
    };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    const resp: UpsertProjectStatusResponse = {
      project_id: resolved.id,
      version: entry.version,
      status: body,
    };
    return c.json(resp, 201);
  });

  // PUT /v1/projects/:projectRef/statuses/:statusKey — update (and
  // possibly rename) a status. A body whose `key` differs from
  // `:statusKey` is a rename in place.
  app.put("/v1/projects/:projectRef/statuses/:statusKey", async (c) => {
    const projectRef = c.req.param("projectRef");
    const statusKey = c.req.param("statusKey");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const body = await c.req.json<StatusDefinition>();
    const project = resolved.entry.data;
    const idx = project.statuses.findIndex((s) => s.key === statusKey);
    if (idx === -1) {
      return c.json(
        { error: `status '${statusKey}' does not exist on project '${resolved.id}'` },
        400,
      );
    }
    if (body.key !== statusKey && project.statuses.some((s) => s.key === body.key)) {
      return c.json(
        { error: `status '${body.key}' already exists on project '${resolved.id}'` },
        400,
      );
    }
    const nextStatuses = [...project.statuses];
    nextStatuses[idx] = body;
    const nextProject: Project = {
      ...project,
      statuses: nextStatuses,
    };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    const resp: UpsertProjectStatusResponse = {
      project_id: resolved.id,
      version: entry.version,
      status: body,
    };
    return c.json(resp);
  });

  // POST /v1/projects/:projectRef/statuses/:statusKey/archive — flip
  // `archived = true` on the status row in place. The row stays in
  // the project's status list so historical issues continue to
  // resolve through it.
  app.post("/v1/projects/:projectRef/statuses/:statusKey/archive", (c) => {
    const projectRef = c.req.param("projectRef");
    const statusKey = c.req.param("statusKey");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const project = resolved.entry.data;
    const idx = project.statuses.findIndex((s) => s.key === statusKey);
    if (idx === -1) {
      return c.json(
        { error: `status '${statusKey}' does not exist on project '${resolved.id}'` },
        400,
      );
    }
    const nextStatuses = project.statuses.map((s) =>
      s.key === statusKey ? { ...s, archived: true } : s,
    );
    const nextProject: Project = { ...project, statuses: nextStatuses };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    // Cascade-archive every non-archived issue in this (project, status).
    // `Issue.status` is a resolved `StatusDefinition`; match by `key`.
    cascadeArchiveIssues(
      store,
      (issue) =>
        issue.project_id === resolved.id && issue.status.key === statusKey,
    );
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  // POST /v1/projects/:projectRef/statuses/:statusKey/unarchive
  app.post("/v1/projects/:projectRef/statuses/:statusKey/unarchive", (c) => {
    const projectRef = c.req.param("projectRef");
    const statusKey = c.req.param("statusKey");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const project = resolved.entry.data;
    const idx = project.statuses.findIndex((s) => s.key === statusKey);
    if (idx === -1) {
      return c.json(
        { error: `status '${statusKey}' does not exist on project '${resolved.id}'` },
        400,
      );
    }
    const nextStatuses = project.statuses.map((s) =>
      s.key === statusKey ? { ...s, archived: false } : s,
    );
    const nextProject: Project = { ...project, statuses: nextStatuses };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
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
      project: { ...resolved.entry.data, statuses: orderedStatuses(resolved.entry.data) },
    };
    return c.json(record);
  });

  // PUT /v1/projects/:projectRef — update project-level fields only.
  app.put("/v1/projects/:projectRef", async (c) => {
    const projectRef = c.req.param("projectRef");
    const body = await c.req.json<UpsertProjectRequest>();
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    if (body.key !== resolved.entry.data.key) {
      const conflict = findByKey(store, body.key);
      if (conflict && conflict.id !== resolved.id) {
        return c.json({ error: `a project with key '${body.key}' already exists` }, 400);
      }
    }
    const nextProject: Project = {
      ...resolved.entry.data,
      key: body.key,
      name: body.name,
      prompt_path: body.prompt_path ?? undefined,
      priority: body.priority ?? resolved.entry.data.priority,
    };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  // POST /v1/projects/:projectRef/archive
  app.post("/v1/projects/:projectRef/archive", (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const nextProject: Project = { ...resolved.entry.data, archived: true };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    // Cascade-archive every non-archived issue in this project.
    cascadeArchiveIssues(store, (issue) => issue.project_id === resolved.id);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  // POST /v1/projects/:projectRef/unarchive
  app.post("/v1/projects/:projectRef/unarchive", (c) => {
    const projectRef = c.req.param("projectRef");
    const resolved = resolveProjectRef(store, projectRef);
    if (!resolved) {
      return c.json({ error: `project '${projectRef}' not found` }, 404);
    }
    const nextProject: Project = { ...resolved.entry.data, archived: false };
    const entry = store.update<Project>(COLLECTION, resolved.id, nextProject, null);
    const resp: UpsertProjectResponse = {
      project_id: resolved.id,
      version: entry.version,
    };
    return c.json(resp);
  });

  return app;
}
