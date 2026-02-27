import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import type {
  Issue,
  UpsertIssueRequest,
  UpsertIssueResponse,
  IssueVersionRecord,
  ListIssuesResponse,
  ListIssueVersionsResponse,
  IssueSummaryRecord,
  IssueSummary,
  AddTodoItemRequest,
  ReplaceTodoListRequest,
  SetTodoItemStatusRequest,
  TodoListResponse,
  TodoItem,
} from "@metis/api";

const COLLECTION = "issues";
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
    description: issue.description.split("\n")[0].slice(0, 200),
    creator: issue.creator,
    status: issue.status,
    assignee: issue.assignee,
    dependencies: issue.dependencies,
    patches: issue.patches,
    todo_list: issue.todo_list,
    deleted: issue.deleted,
  };
  return {
    issue_id: issueId,
    version: BigInt(version),
    timestamp,
    issue: summary,
    creation_time: creationTime,
  };
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
    const items = store.list<Issue>(COLLECTION, includeDeleted);
    const issues: IssueSummaryRecord[] = items.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListIssuesResponse = { issues };
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
