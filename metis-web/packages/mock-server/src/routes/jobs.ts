import { Hono } from "hono";
import { stream } from "hono/streaming";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { applyPagination } from "./pagination.js";
import { DEV_USERNAME } from "../auth.js";
import type {
  Task,
  CreateJobRequest,
  CreateJobResponse,
  JobVersionRecord,
  ListJobsResponse,
  ListJobVersionsResponse,
  JobSummaryRecord,
  JobSummary,
  KillJobResponse,
  JobStatusUpdate,
  SetJobStatusResponse,
  WorkerContext,
  Status,
} from "@metis/api";

const COLLECTION = "jobs";
const SSE_PREFIX = "job";

function toVersionRecord(
  jobId: string,
  version: number,
  timestamp: string,
  task: Task,
): JobVersionRecord {
  return {
    job_id: jobId,
    version: BigInt(version),
    timestamp,
    task,
  };
}

function toSummaryRecord(
  jobId: string,
  version: number,
  timestamp: string,
  task: Task,
): JobSummaryRecord {
  const summary: JobSummary = {
    prompt: task.prompt.slice(0, 100),
    spawned_from: task.spawned_from,
    creator: task.creator,
    status: task.status,
    error: task.error,
    deleted: task.deleted,
    creation_time: task.creation_time,
    start_time: task.start_time,
    end_time: task.end_time,
  };
  return {
    job_id: jobId,
    version: BigInt(version),
    timestamp,
    task: summary,
  };
}

export function createJobRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/jobs
  app.post("/v1/jobs", async (c) => {
    const body = await c.req.json<CreateJobRequest>();
    const id = generateId("job");
    const now = new Date().toISOString();
    const task: Task = {
      prompt: body.prompt,
      context: body.context,
      spawned_from: body.issue_id,
      creator: DEV_USERNAME,
      image: body.image,
      env_vars: body.variables,
      status: "pending" as Status,
      creation_time: now,
    };
    store.create<Task>(COLLECTION, id, task, SSE_PREFIX);
    const resp: CreateJobResponse = { job_id: id };
    return c.json(resp, 201);
  });

  // GET /v1/jobs
  app.get("/v1/jobs", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const q = c.req.query("q");
    const spawnedFrom = c.req.query("spawned_from");
    const status = c.req.query("status");
    const limitParam = c.req.query("limit");
    const cursor = c.req.query("cursor") ?? null;

    const items = store.list<Task>(COLLECTION, includeDeleted);

    let filtered = items;
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        entry.data.prompt.toLowerCase().includes(lower),
      );
    }
    if (spawnedFrom) {
      filtered = filtered.filter(({ entry }) => entry.data.spawned_from === spawnedFrom);
    }
    if (status) {
      filtered = filtered.filter(({ entry }) => entry.data.status === status);
    }

    const limit = limitParam ? Number(limitParam) : null;
    const withTimestamp = filtered.map(({ id, entry }) => ({
      id,
      entry,
      timestamp: entry.timestamp,
    }));
    const { page, nextCursor } = applyPagination(withTimestamp, limit, cursor);

    const jobs: JobSummaryRecord[] = page.map(({ id, entry }) =>
      toSummaryRecord(id, entry.version, entry.timestamp, entry.data),
    );
    const resp: ListJobsResponse = { jobs, next_cursor: nextCursor };
    return c.json(resp);
  });

  // GET /v1/jobs/:id
  app.get("/v1/jobs/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Task>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data));
  });

  // GET /v1/jobs/:id/versions/:version
  app.get("/v1/jobs/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Task>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `job '${id}' version ${version} not found` }, 404);
    }
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data));
  });

  // DELETE /v1/jobs/:id — kill job
  // The real server sends a kill signal to K8s but the job stays "running"
  // until the pod actually terminates. We simulate this by not updating
  // the store immediately, so refetches still return "running".
  app.delete("/v1/jobs/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Task>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    const resp: KillJobResponse = { job_id: id, status: "failed" };
    return c.json(resp);
  });

  // GET /v1/jobs/:id/logs
  app.get("/v1/jobs/:id/logs", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Task>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    const watch = c.req.query("watch") === "true";
    if (watch) {
      return stream(c, async (s) => {
        c.header("Content-Type", "text/event-stream");
        c.header("Cache-Control", "no-cache");
        c.header("Connection", "keep-alive");
        await s.write(`data: [mock] Job ${id} log line 1\n\n`);
        await s.write(`data: [mock] Job ${id} log line 2\n\n`);
        await s.write(`data: [mock] Job ${id} complete\n\n`);
      });
    }
    return c.text(`[mock] Job ${id} log output\n[mock] Job completed successfully\n`);
  });

  // POST /v1/jobs/:id/status
  app.post("/v1/jobs/:id/status", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<JobStatusUpdate>();
    const entry = store.get<Task>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    let newStatus: Status;
    const updates: Partial<Task> = {};
    if (body.status === "complete") {
      newStatus = "complete";
      updates.last_message = body.last_message;
      updates.end_time = new Date().toISOString();
    } else if (body.status === "failed") {
      newStatus = "failed";
      updates.error = { job_engine_error: { reason: body.reason } };
      updates.end_time = new Date().toISOString();
    } else {
      newStatus = "unknown";
    }
    const updated: Task = { ...entry.data, ...updates, status: newStatus };
    store.update<Task>(COLLECTION, id, updated, SSE_PREFIX);
    const resp: SetJobStatusResponse = { job_id: id, status: newStatus };
    return c.json(resp);
  });

  // GET /v1/jobs/:id/context
  app.get("/v1/jobs/:id/context", (c) => {
    const id = c.req.param("id");
    const entry = store.get<Task>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    const task = entry.data;
    const resp: WorkerContext = {
      request_context: task.context.type === "git_repository"
        ? { type: "git_repository", url: task.context.url, rev: task.context.rev }
        : { type: "none" },
      prompt: task.prompt,
      model: task.model,
      variables: task.env_vars ?? {},
    };
    return c.json(resp);
  });

  // GET /v1/jobs/:id/versions
  app.get("/v1/jobs/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Task>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `job '${id}' not found` }, 404);
    }
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data),
    );
    const resp: ListJobVersionsResponse = { versions };
    return c.json(resp);
  });

  return app;
}
