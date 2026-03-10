import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import { applyPagination } from "./pagination.js";
import type {
  Issue,
  Task,
  UpsertIssueRequest,
  UpsertIssueResponse,
  IssueVersionRecord,
  ListIssuesResponse,
  ListIssueVersionsResponse,
  IssueSummaryRecord,
  IssueSummary,
  SubtreeIssue,
  JobStatusSummary,
  AddTodoItemRequest,
  ReplaceTodoListRequest,
  SetTodoItemStatusRequest,
  TodoListResponse,
  TodoItem,
  IssueStatus,
  Status,
} from "@metis/api";
import { getLabelsForObject, resolveLabelNames } from "./labels.js";

const COLLECTION = "issues";
const JOBS_COLLECTION = "jobs";
const SSE_PREFIX = "issue";

function toVersionRecord(
  issueId: string,
  version: number,
  timestamp: string,
  issue: Issue,
  creationTime: string,
): IssueVersionRecord {
  return {
    issue_id: issueId,
    version: BigInt(version),
    timestamp,
    issue,
    creation_time: creationTime,
    labels: getLabelsForObject(issueId),
  };
}

function toSummaryRecord(
  issueId: string,
  version: number,
  timestamp: string,
  issue: Issue,
  creationTime: string,
): IssueSummaryRecord {
  const summary: IssueSummary = {
    type: issue.type,
    title: issue.title,
    description: issue.description.split("\n")[0].slice(0, 200),
    creator: issue.creator,
    status: issue.status,
    assignee: issue.assignee,
    progress: (issue.progress ?? "").slice(0, 200),
    dependencies: issue.dependencies,
    patches: issue.patches,
    todo_list: issue.todo_list,
    deleted: issue.deleted,
    labels: getLabelsForObject(issueId),
  };
  return {
    issue_id: issueId,
    version: BigInt(version),
    timestamp,
    issue: summary,
    creation_time: creationTime,
  };
}

function computeJobStatusSummary(store: Store, issueId: string): JobStatusSummary {
  const allJobs = store.list<Task>(JOBS_COLLECTION, false);
  const issueJobs = allJobs.filter(({ entry }) => entry.data.spawned_from === issueId);

  let running = 0;
  let failed = 0;
  let latestJob: { id: string; task: Task; timestamp: string } | null = null;

  for (const { id, entry } of issueJobs) {
    if (entry.data.status === "running" || entry.data.status === "pending") running++;
    if (entry.data.status === "failed") failed++;
    if (!latestJob || entry.timestamp > latestJob.timestamp) {
      latestJob = { id, task: entry.data, timestamp: entry.timestamp };
    }
  }

  return {
    total: issueJobs.length,
    running,
    failed,
    latest_job_id: latestJob?.id ?? null,
    latest_job_status: (latestJob?.task.status as Status) ?? null,
    latest_start_time: latestJob?.task.start_time ?? null,
    latest_end_time: latestJob?.task.end_time ?? null,
  };
}

function computeSubtree(store: Store, parentId: string): SubtreeIssue[] {
  const allIssues = store.list<Issue>(COLLECTION, false);
  const childrenMap = new Map<string, { id: string; issue: Issue }[]>();
  for (const { id, entry } of allIssues) {
    for (const dep of entry.data.dependencies) {
      if (dep.type === "child-of") {
        const siblings = childrenMap.get(dep.issue_id) ?? [];
        siblings.push({ id, issue: entry.data });
        childrenMap.set(dep.issue_id, siblings);
      }
    }
  }

  const allJobs = store.list<Task>(JOBS_COLLECTION, false);
  const activeJobIssues = new Set<string>();
  for (const { entry } of allJobs) {
    if (
      entry.data.spawned_from &&
      (entry.data.status === "running" || entry.data.status === "pending")
    ) {
      activeJobIssues.add(entry.data.spawned_from);
    }
  }

  function buildNode(issueId: string, issue: Issue): SubtreeIssue {
    const children = (childrenMap.get(issueId) ?? []).map(({ id, issue: childIssue }) =>
      buildNode(id, childIssue),
    );
    return {
      issue_id: issueId,
      status: issue.status as IssueStatus,
      has_active_task: activeJobIssues.has(issueId),
      assignee: issue.assignee,
      title: issue.title,
      children,
    };
  }

  const directChildren = childrenMap.get(parentId) ?? [];
  return directChildren.map(({ id, issue }) => buildNode(id, issue));
}


export function createIssueRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/issues
  app.post("/v1/issues", async (c) => {
    const body = await c.req.json<UpsertIssueRequest>();
    const id = generateId("issue");
    const issue: Issue = {
      ...body.issue,
      todo_list: body.issue.todo_list ?? [],
      dependencies: body.issue.dependencies ?? [],
      patches: body.issue.patches ?? [],
    };
    const entry = store.create<Issue>(COLLECTION, id, issue, SSE_PREFIX);

    // Resolve label_names: create missing labels and associate them with the issue
    if (body.label_names && body.label_names.length > 0) {
      resolveLabelNames(store, body.label_names, id);
    }

    const resp: UpsertIssueResponse = {
      issue_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp, 201);
  });

  // PUT /v1/issues/:id
  app.put("/v1/issues/:id", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<UpsertIssueRequest>();
    const issue: Issue = {
      ...body.issue,
      todo_list: body.issue.todo_list ?? [],
      dependencies: body.issue.dependencies ?? [],
      patches: body.issue.patches ?? [],
    };
    const entry = store.update<Issue>(COLLECTION, id, issue, SSE_PREFIX);
    const resp: UpsertIssueResponse = {
      issue_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp);
  });

  // GET /v1/issues/:id
  app.get("/v1/issues/:id", (c) => {
    const id = c.req.param("id");
    const includeDeleted = c.req.query("include_deleted") === "true";
    const entry = store.get<Issue>(COLLECTION, id, includeDeleted);
    if (!entry) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  // GET /v1/issues/:id/versions/:version
  app.get("/v1/issues/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Issue>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `issue '${id}' version ${version} not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  // GET /v1/issues
  app.get("/v1/issues", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const issueType = c.req.query("issue_type");
    const status = c.req.query("status");
    const assignee = c.req.query("assignee");
    const q = c.req.query("q");
    const labels = c.req.query("labels");
    const limitParam = c.req.query("limit");
    const cursor = c.req.query("cursor") ?? null;
    const includeSubtree = c.req.query("include_subtree") === "true";
    const includeJobStatus = c.req.query("include_job_status") === "true";

    const items = store.list<Issue>(COLLECTION, includeDeleted);

    let filtered = items;
    if (issueType) {
      filtered = filtered.filter(({ entry }) => entry.data.type === issueType);
    }
    if (status) {
      filtered = filtered.filter(({ entry }) => entry.data.status === status);
    }
    if (assignee) {
      filtered = filtered.filter(({ entry }) => entry.data.assignee === assignee);
    }
    if (labels) {
      const labelIds = new Set(labels.split(",").map((l) => l.trim()));
      filtered = filtered.filter(({ id }) => {
        const issueLabels = getLabelsForObject(id);
        return issueLabels.some((l) => labelIds.has(l.label_id));
      });
    }
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        entry.data.title.toLowerCase().includes(lower) ||
        entry.data.description.toLowerCase().includes(lower),
      );
    }

    const limit = limitParam ? Number(limitParam) : null;
    const withTimestamp = filtered.map(({ id, entry }) => ({
      id,
      entry,
      timestamp: entry.timestamp,
    }));
    const { page, nextCursor } = applyPagination(withTimestamp, limit, cursor);

    const issues: IssueSummaryRecord[] = page.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      const record = toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
      if (includeJobStatus) {
        record.jobs_summary = computeJobStatusSummary(store, id);
      }
      if (includeSubtree) {
        record.subtree = computeSubtree(store, id);
      }
      return record;
    });
    const resp: ListIssuesResponse = { issues, next_cursor: nextCursor };
    return c.json(resp);
  });

  // GET /v1/issues/:id/versions
  app.get("/v1/issues/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Issue>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data, creationTime),
    );
    const resp: ListIssueVersionsResponse = { versions };
    return c.json(resp);
  });

  // DELETE /v1/issues/:id
  app.delete("/v1/issues/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.delete<Issue>(COLLECTION, id, SSE_PREFIX);
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  // POST /v1/issues/:id/todo-items — add a todo item
  app.post("/v1/issues/:id/todo-items", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<AddTodoItemRequest>();
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const todoList = [...(existing.data.todo_list ?? []), { description: body.description, is_done: body.is_done }];
    const updated: Issue = { ...existing.data, todo_list: todoList };
    store.update<Issue>(COLLECTION, id, updated, SSE_PREFIX);
    const resp: TodoListResponse = { issue_id: id, todo_list: todoList };
    return c.json(resp);
  });

  // PUT /v1/issues/:id/todo-items — replace todo list
  app.put("/v1/issues/:id/todo-items", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<ReplaceTodoListRequest>();
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const todoList: TodoItem[] = body.todo_list;
    const updated: Issue = { ...existing.data, todo_list: todoList };
    store.update<Issue>(COLLECTION, id, updated, SSE_PREFIX);
    const resp: TodoListResponse = { issue_id: id, todo_list: todoList };
    return c.json(resp);
  });

  // POST /v1/issues/:id/todo-items/:index — set todo item status
  app.post("/v1/issues/:id/todo-items/:index", async (c) => {
    const id = c.req.param("id");
    const index = Number(c.req.param("index"));
    const body = await c.req.json<SetTodoItemStatusRequest>();
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const todoList = [...(existing.data.todo_list ?? [])];
    if (index < 0 || index >= todoList.length) {
      return c.json({ error: `todo item index ${index} out of range` }, 422);
    }
    todoList[index] = { ...todoList[index], is_done: body.is_done };
    const updated: Issue = { ...existing.data, todo_list: todoList };
    store.update<Issue>(COLLECTION, id, updated, SSE_PREFIX);
    const resp: TodoListResponse = { issue_id: id, todo_list: todoList };
    return c.json(resp);
  });

  return app;
}
