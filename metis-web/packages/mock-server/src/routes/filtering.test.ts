// BigInt cannot be serialized by JSON.stringify by default.
(BigInt.prototype as unknown as { toJSON: () => number }).toJSON = function () {
  return Number(this);
};

import { describe, it, expect, beforeEach } from "vitest";
import { Store } from "../store.js";
import { createIssueRoutes } from "./issues.js";
import { createPatchRoutes } from "./patches.js";
import { createSessionRoutes } from "./sessions.js";
import { createDocumentRoutes } from "./documents.js";
import type { Issue, Patch, Task, Document, Status } from "@metis/api";

function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return {
    type: "task",
    title: "",
    description: "Default issue description",
    creator: "testuser",
    status: "open",
    progress: "",
    dependencies: [],
    patches: [],
    todo_list: [],
    ...overrides,
  };
}

function makePatch(overrides: Partial<Patch> = {}): Patch {
  return {
    title: "Default patch title",
    description: "Default patch description",
    diff: "--- a/file\n+++ b/file\n",
    status: "Open",
    is_automatic_backup: false,
    creator: "testuser",
    reviews: [],
    service_repo_name: "test/repo",
    ...overrides,
  };
}

function makeTask(overrides: Partial<Task> = {}): Task {
  return {
    prompt: "Default task prompt",
    context: { type: "none" },
    creator: "testuser",
    status: "pending" as Status,
    creation_time: new Date().toISOString(),
    ...overrides,
  };
}

function makeDocument(overrides: Partial<Document> = {}): Document {
  return {
    title: "Default document title",
    body_markdown: "Default body",
    ...overrides,
  };
}

describe("Issue list filtering", () => {
  let store: Store;
  let app: ReturnType<typeof createIssueRoutes>;

  beforeEach(() => {
    store = new Store();
    app = createIssueRoutes(store);
  });

  async function listIssues(params: Record<string, string> = {}) {
    const qs = new URLSearchParams(params).toString();
    const url = qs ? `http://localhost/v1/issues?${qs}` : "http://localhost/v1/issues";
    const res = await app.request(url);
    return res.json();
  }

  it("returns all issues when no filters provided", async () => {
    store.create("issues", "i-1", makeIssue({ description: "First" }), "issue");
    store.create("issues", "i-2", makeIssue({ description: "Second" }), "issue");
    const data = await listIssues();
    expect(data.issues).toHaveLength(2);
  });

  it("filters by status", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open" }), "issue");
    store.create("issues", "i-2", makeIssue({ status: "closed" }), "issue");
    store.create("issues", "i-3", makeIssue({ status: "open" }), "issue");
    const data = await listIssues({ status: "open" });
    expect(data.issues).toHaveLength(2);
    expect(data.issues.every((i: { issue: { status: string } }) => i.issue.status === "open")).toBe(true);
  });

  it("filters by assignee", async () => {
    store.create("issues", "i-1", makeIssue({ assignee: "alice" }), "issue");
    store.create("issues", "i-2", makeIssue({ assignee: "bob" }), "issue");
    store.create("issues", "i-3", makeIssue({ assignee: "alice" }), "issue");
    const data = await listIssues({ assignee: "alice" });
    expect(data.issues).toHaveLength(2);
  });

  it("filters by issue_type", async () => {
    store.create("issues", "i-1", makeIssue({ type: "task" }), "issue");
    store.create("issues", "i-2", makeIssue({ type: "bug" }), "issue");
    store.create("issues", "i-3", makeIssue({ type: "task" }), "issue");
    const data = await listIssues({ issue_type: "task" });
    expect(data.issues).toHaveLength(2);
    expect(data.issues.every((i: { issue: { type: string } }) => i.issue.type === "task")).toBe(true);
  });

  it("filters by q (case-insensitive substring on description)", async () => {
    store.create("issues", "i-1", makeIssue({ description: "Fix the login bug" }), "issue");
    store.create("issues", "i-2", makeIssue({ description: "Add new feature" }), "issue");
    store.create("issues", "i-3", makeIssue({ description: "Another LOGIN issue" }), "issue");
    const data = await listIssues({ q: "login" });
    expect(data.issues).toHaveLength(2);
  });

  it("combines multiple filters with AND logic", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open", assignee: "alice" }), "issue");
    store.create("issues", "i-2", makeIssue({ status: "closed", assignee: "alice" }), "issue");
    store.create("issues", "i-3", makeIssue({ status: "open", assignee: "bob" }), "issue");
    const data = await listIssues({ status: "open", assignee: "alice" });
    expect(data.issues).toHaveLength(1);
  });

  it("returns empty array when no issues match", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open" }), "issue");
    const data = await listIssues({ status: "closed" });
    expect(data.issues).toHaveLength(0);
  });
});

describe("Patch list filtering", () => {
  let store: Store;
  let app: ReturnType<typeof createPatchRoutes>;

  beforeEach(() => {
    store = new Store();
    app = createPatchRoutes(store);
  });

  async function listPatches(params: Record<string, string> = {}) {
    const qs = new URLSearchParams(params).toString();
    const url = qs ? `http://localhost/v1/patches?${qs}` : "http://localhost/v1/patches";
    const res = await app.request(url);
    return res.json();
  }

  it("returns all patches when no filters provided", async () => {
    store.create("patches", "p-1", makePatch({ title: "First" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "Second" }), "patch");
    const data = await listPatches();
    expect(data.patches).toHaveLength(2);
  });

  it("filters by q (case-insensitive substring on title)", async () => {
    store.create("patches", "p-1", makePatch({ title: "Fix authentication bug" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "Add new feature" }), "patch");
    store.create("patches", "p-3", makePatch({ title: "AUTH improvements" }), "patch");
    const data = await listPatches({ q: "auth" });
    expect(data.patches).toHaveLength(2);
  });

  it("filters by single status", async () => {
    store.create("patches", "p-1", makePatch({ status: "Open" }), "patch");
    store.create("patches", "p-2", makePatch({ status: "Closed" }), "patch");
    store.create("patches", "p-3", makePatch({ status: "Merged" }), "patch");
    const data = await listPatches({ status: "Open" });
    expect(data.patches).toHaveLength(1);
  });

  it("filters by multiple statuses (comma-separated)", async () => {
    store.create("patches", "p-1", makePatch({ status: "Open" }), "patch");
    store.create("patches", "p-2", makePatch({ status: "Closed" }), "patch");
    store.create("patches", "p-3", makePatch({ status: "Merged" }), "patch");
    const data = await listPatches({ status: "Open,Closed" });
    expect(data.patches).toHaveLength(2);
  });

  it("filters by branch_name", async () => {
    store.create("patches", "p-1", makePatch({ branch_name: "feature/foo" }), "patch");
    store.create("patches", "p-2", makePatch({ branch_name: "feature/bar" }), "patch");
    store.create("patches", "p-3", makePatch({ branch_name: "feature/foo" }), "patch");
    const data = await listPatches({ branch_name: "feature/foo" });
    expect(data.patches).toHaveLength(2);
  });

  it("combines filters with AND logic", async () => {
    store.create("patches", "p-1", makePatch({ title: "Fix auth bug", status: "Open" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "Fix auth issue", status: "Closed" }), "patch");
    store.create("patches", "p-3", makePatch({ title: "Add feature", status: "Open" }), "patch");
    const data = await listPatches({ q: "auth", status: "Open" });
    expect(data.patches).toHaveLength(1);
  });
});

describe("Session list filtering", () => {
  let store: Store;
  let app: ReturnType<typeof createSessionRoutes>;

  beforeEach(() => {
    store = new Store();
    app = createSessionRoutes(store);
  });

  async function listSessions(params: Record<string, string> = {}) {
    const qs = new URLSearchParams(params).toString();
    const url = qs ? `http://localhost/v1/sessions?${qs}` : "http://localhost/v1/sessions";
    const res = await app.request(url);
    return res.json();
  }

  it("returns all sessions when no filters provided", async () => {
    store.create("sessions", "t-1", makeTask({ prompt: "First" }), "job");
    store.create("sessions", "t-2", makeTask({ prompt: "Second" }), "job");
    const data = await listSessions();
    expect(data.jobs).toHaveLength(2);
  });

  it("filters by spawned_from", async () => {
    store.create("sessions", "t-1", makeTask({ spawned_from: "i-abc123" }), "job");
    store.create("sessions", "t-2", makeTask({ spawned_from: "i-def456" }), "job");
    store.create("sessions", "t-3", makeTask({ spawned_from: "i-abc123" }), "job");
    const data = await listSessions({ spawned_from: "i-abc123" });
    expect(data.jobs).toHaveLength(2);
    expect(data.jobs.every((j: { task: { spawned_from: string } }) => j.task.spawned_from === "i-abc123")).toBe(true);
  });

  it("filters by status", async () => {
    store.create("sessions", "t-1", makeTask({ status: "running" as Status }), "job");
    store.create("sessions", "t-2", makeTask({ status: "pending" as Status }), "job");
    store.create("sessions", "t-3", makeTask({ status: "running" as Status }), "job");
    const data = await listSessions({ status: "running" });
    expect(data.jobs).toHaveLength(2);
    expect(data.jobs.every((j: { task: { status: string } }) => j.task.status === "running")).toBe(true);
  });

  it("filters by q (case-insensitive substring on prompt)", async () => {
    store.create("sessions", "t-1", makeTask({ prompt: "Deploy the application" }), "job");
    store.create("sessions", "t-2", makeTask({ prompt: "Run tests" }), "job");
    store.create("sessions", "t-3", makeTask({ prompt: "deploy staging" }), "job");
    const data = await listSessions({ q: "deploy" });
    expect(data.jobs).toHaveLength(2);
  });

  it("combines filters with AND logic", async () => {
    store.create("sessions", "t-1", makeTask({ spawned_from: "i-abc", status: "running" as Status }), "job");
    store.create("sessions", "t-2", makeTask({ spawned_from: "i-abc", status: "complete" as Status }), "job");
    store.create("sessions", "t-3", makeTask({ spawned_from: "i-def", status: "running" as Status }), "job");
    const data = await listSessions({ spawned_from: "i-abc", status: "running" });
    expect(data.jobs).toHaveLength(1);
  });
});

describe("Document list filtering", () => {
  let store: Store;
  let app: ReturnType<typeof createDocumentRoutes>;

  beforeEach(() => {
    store = new Store();
    app = createDocumentRoutes(store);
  });

  async function listDocuments(params: Record<string, string> = {}) {
    const qs = new URLSearchParams(params).toString();
    const url = qs ? `http://localhost/v1/documents?${qs}` : "http://localhost/v1/documents";
    const res = await app.request(url);
    return res.json();
  }

  it("returns all documents when no filters provided", async () => {
    store.create("documents", "d-1", makeDocument({ title: "First" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Second" }), "document");
    const data = await listDocuments();
    expect(data.documents).toHaveLength(2);
  });

  it("filters by q matching title (case-insensitive)", async () => {
    store.create("documents", "d-1", makeDocument({ title: "README" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "API Guide" }), "document");
    store.create("documents", "d-3", makeDocument({ title: "readme notes" }), "document");
    const data = await listDocuments({ q: "readme" });
    expect(data.documents).toHaveLength(2);
  });

  it("filters by q matching path (case-insensitive)", async () => {
    store.create("documents", "d-1", makeDocument({ title: "Doc A", path: "docs/readme.md" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Doc B", path: "src/index.ts" }), "document");
    const data = await listDocuments({ q: "readme" });
    expect(data.documents).toHaveLength(1);
  });

  it("filters by q matching either title or path", async () => {
    store.create("documents", "d-1", makeDocument({ title: "README", path: "docs/intro.md" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Guide", path: "docs/readme.md" }), "document");
    store.create("documents", "d-3", makeDocument({ title: "Other", path: "src/main.ts" }), "document");
    const data = await listDocuments({ q: "readme" });
    expect(data.documents).toHaveLength(2);
  });

  it("filters by created_by", async () => {
    store.create("documents", "d-1", makeDocument({ created_by: "t-xyz" }), "document");
    store.create("documents", "d-2", makeDocument({ created_by: "t-abc" }), "document");
    store.create("documents", "d-3", makeDocument({ created_by: "t-xyz" }), "document");
    const data = await listDocuments({ created_by: "t-xyz" });
    expect(data.documents).toHaveLength(2);
  });

  it("combines q and created_by with AND logic", async () => {
    store.create("documents", "d-1", makeDocument({ title: "README", created_by: "t-xyz" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "README", created_by: "t-abc" }), "document");
    store.create("documents", "d-3", makeDocument({ title: "Guide", created_by: "t-xyz" }), "document");
    const data = await listDocuments({ q: "readme", created_by: "t-xyz" });
    expect(data.documents).toHaveLength(1);
  });

  it("combines with existing path_prefix filter", async () => {
    store.create("documents", "d-1", makeDocument({ title: "Doc A", path: "docs/readme.md", created_by: "t-xyz" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Doc B", path: "docs/guide.md", created_by: "t-xyz" }), "document");
    store.create("documents", "d-3", makeDocument({ title: "Doc C", path: "src/readme.md", created_by: "t-xyz" }), "document");
    const data = await listDocuments({ path_prefix: "docs/", created_by: "t-xyz" });
    expect(data.documents).toHaveLength(2);
  });
});
