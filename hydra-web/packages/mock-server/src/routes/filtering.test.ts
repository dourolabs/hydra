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
import type {
  Issue,
  Patch,
  Session,
  Document,
  Status,
  StatusDefinition,
  StatusKey,
} from "@hydra/api";

function placeholderStatus(key: StatusKey): StatusDefinition {
  return {
    key,
    label: "",
    color: "#888888",
    unblocks_parents: false,
    unblocks_dependents: false,
    cascades_to_children: false,
  };
}

function makeIssue(
  overrides: Omit<Partial<Issue>, "status"> & { status?: StatusKey } = {},
): Issue {
  const { status, ...rest } = overrides;
  return {
    type: "task",
    title: "",
    description: "Default issue description",
    creator: "testuser",
    status: placeholderStatus(status ?? "open"),
    project_id: "j-defaul",
    progress: "",
    dependencies: [],
    patches: [],
    ...rest,
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

function makeSession(
  overrides: Partial<Session> & { prompt?: string } = {},
): Session {
  // `prompt` was a top-level Session field pre-PR-2; it now lives on
  // `agent_config.system_prompt`. Accept it as a convenience override and
  // funnel it into the agent_config so call sites stay terse.
  const { prompt, ...rest } = overrides;
  const mode: Session["mode"] = rest.mode ?? { type: "headless" };
  const agentConfig: Session["agent_config"] = rest.agent_config ?? {
    system_prompt: prompt ?? "Default task prompt",
  };
  return {
    creator: "testuser",
    agent_config: agentConfig,
    mount_spec: { working_dir: "repo", mounts: [] },
    status: "pending" as Status,
    creation_time: new Date().toISOString(),
    ...rest,
    mode,
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
    expect(
      data.issues.every(
        (i: { issue: { status: { key: string } } }) => i.issue.status.key === "open",
      ),
    ).toBe(true);
  });

  it("filters by assignee", async () => {
    // Phase 4b: assignee is a typed Principal; the query-string
    // filter is the canonical path form (`users/<name>`).
    store.create(
      "issues",
      "i-1",
      makeIssue({ assignee: { User: { name: "alice" } } }),
      "issue",
    );
    store.create(
      "issues",
      "i-2",
      makeIssue({ assignee: { User: { name: "bob" } } }),
      "issue",
    );
    store.create(
      "issues",
      "i-3",
      makeIssue({ assignee: { User: { name: "alice" } } }),
      "issue",
    );
    const data = await listIssues({ assignee: "users/alice" });
    expect(data.issues).toHaveLength(2);
  });

  it("filters by issue_type", async () => {
    store.create("issues", "i-1", makeIssue({ type: "task" }), "issue");
    store.create("issues", "i-2", makeIssue({ type: "bug" }), "issue");
    store.create("issues", "i-3", makeIssue({ type: "task" }), "issue");
    const data = await listIssues({ issue_type: "task" });
    expect(data.issues).toHaveLength(2);
    expect(data.issues.every((i: { issue: { type: string } }) => i.issue.type === "task")).toBe(
      true,
    );
  });

  it("filters by q (case-insensitive substring on description)", async () => {
    store.create("issues", "i-1", makeIssue({ description: "Fix the login bug" }), "issue");
    store.create("issues", "i-2", makeIssue({ description: "Add new feature" }), "issue");
    store.create("issues", "i-3", makeIssue({ description: "Another LOGIN issue" }), "issue");
    const data = await listIssues({ q: "login" });
    expect(data.issues).toHaveLength(2);
  });

  it("combines multiple filters with AND logic", async () => {
    store.create(
      "issues",
      "i-1",
      makeIssue({ status: "open", assignee: { User: { name: "alice" } } }),
      "issue",
    );
    store.create(
      "issues",
      "i-2",
      makeIssue({ status: "closed", assignee: { User: { name: "alice" } } }),
      "issue",
    );
    store.create(
      "issues",
      "i-3",
      makeIssue({ status: "open", assignee: { User: { name: "bob" } } }),
      "issue",
    );
    const data = await listIssues({ status: "open", assignee: "users/alice" });
    expect(data.issues).toHaveLength(1);
  });

  it("returns empty array when no issues match", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open" }), "issue");
    const data = await listIssues({ status: "closed" });
    expect(data.issues).toHaveLength(0);
  });

  // Parity with prod: `SearchIssuesQuery::ids` is `Vec<IssueId>`, so a
  // mixed-prefix CSV must 400. Previously the mock filtered as strings and
  // silently dropped non-issue ids, masking the filter-by-conversation bug.
  it("rejects ids with non-`i-` prefix (400)", async () => {
    store.create("issues", "i-1", makeIssue(), "issue");
    const qs = new URLSearchParams({ ids: "i-1,p-foo" }).toString();
    const res = await app.request(`http://localhost/v1/issues?${qs}`);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/p-foo/);
  });

  it("accepts a CSV of `i-` prefixed ids unchanged", async () => {
    store.create("issues", "i-1", makeIssue(), "issue");
    store.create("issues", "i-2", makeIssue(), "issue");
    const data = await listIssues({ ids: "i-1,i-2" });
    expect(data.issues).toHaveLength(2);
  });

  // Parity with the backend `?status=` filter ([[p-urywauam]]): per-project
  // StatusKey strings (e.g. `inbox`) must pass through unchanged, not be
  // silently coerced to the legacy five-enum domain.
  it("filters by per-project status key (e.g. inbox)", async () => {
    store.create("issues", "i-1", makeIssue({ status: "inbox" }), "issue");
    store.create("issues", "i-2", makeIssue({ status: "open" }), "issue");
    store.create("issues", "i-3", makeIssue({ status: "inbox" }), "issue");
    const data = await listIssues({ status: "inbox" });
    expect(data.issues).toHaveLength(2);
    expect(
      data.issues.every(
        (i: { issue: { status: { key: string } } }) => i.issue.status.key === "inbox",
      ),
    ).toBe(true);
  });

  it("filters by multi-value status (OR-union)", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open" }), "issue");
    store.create("issues", "i-2", makeIssue({ status: "in-progress" }), "issue");
    store.create("issues", "i-3", makeIssue({ status: "closed" }), "issue");
    store.create("issues", "i-4", makeIssue({ status: "dropped" }), "issue");
    const data = await listIssues({ status: "open,in-progress" });
    expect(data.issues).toHaveLength(2);
    const statuses = data.issues
      .map((i: { issue: { status: { key: string } } }) => i.issue.status.key)
      .sort();
    expect(statuses).toEqual(["in-progress", "open"]);
  });

  it("trims whitespace in CSV status entries", async () => {
    store.create("issues", "i-1", makeIssue({ status: "open" }), "issue");
    store.create("issues", "i-2", makeIssue({ status: "in-progress" }), "issue");
    store.create("issues", "i-3", makeIssue({ status: "closed" }), "issue");
    const data = await listIssues({ status: "open, in-progress" });
    expect(data.issues).toHaveLength(2);
  });

  it("filters by project_id and excludes issues in other projects", async () => {
    store.create(
      "issues",
      "i-1",
      makeIssue({ project_id: "engineering-v2" }),
      "issue",
    );
    store.create(
      "issues",
      "i-2",
      makeIssue({ project_id: "design" }),
      "issue",
    );
    // Default project (no override) — must NOT match the engineering-v2 filter.
    store.create("issues", "i-3", makeIssue(), "issue");
    store.create(
      "issues",
      "i-4",
      makeIssue({ project_id: "engineering-v2" }),
      "issue",
    );
    const data = await listIssues({ project_id: "engineering-v2" });
    expect(data.issues).toHaveLength(2);
    expect(
      data.issues.every(
        (i: { issue: { project_id: string } }) => i.issue.project_id === "engineering-v2",
      ),
    ).toBe(true);
  });

  it("AND-composes multi-status, project_id, and assignee", async () => {
    store.create(
      "issues",
      "i-1",
      makeIssue({
        status: "inbox",
        project_id: "engineering-v2",
        assignee: { User: { name: "alice" } },
      }),
      "issue",
    );
    store.create(
      "issues",
      "i-2",
      makeIssue({
        status: "open",
        project_id: "engineering-v2",
        assignee: { User: { name: "alice" } },
      }),
      "issue",
    );
    // Wrong assignee — filtered out by assignee.
    store.create(
      "issues",
      "i-3",
      makeIssue({
        status: "open",
        project_id: "engineering-v2",
        assignee: { User: { name: "bob" } },
      }),
      "issue",
    );
    // Wrong project — filtered out by project_id.
    store.create(
      "issues",
      "i-4",
      makeIssue({
        status: "open",
        project_id: "design",
        assignee: { User: { name: "alice" } },
      }),
      "issue",
    );
    // Wrong status — filtered out by status.
    store.create(
      "issues",
      "i-5",
      makeIssue({
        status: "closed",
        project_id: "engineering-v2",
        assignee: { User: { name: "alice" } },
      }),
      "issue",
    );
    const data = await listIssues({
      status: "inbox,open",
      project_id: "engineering-v2",
      assignee: "users/alice",
    });
    expect(data.issues).toHaveLength(2);
    const returnedIds = data.issues
      .map((i: { issue_id: string }) => i.issue_id)
      .sort();
    expect(returnedIds).toEqual(["i-1", "i-2"]);
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
    store.create(
      "patches",
      "p-2",
      makePatch({ title: "Fix auth issue", status: "Closed" }),
      "patch",
    );
    store.create("patches", "p-3", makePatch({ title: "Add feature", status: "Open" }), "patch");
    const data = await listPatches({ q: "auth", status: "Open" });
    expect(data.patches).toHaveLength(1);
  });

  it("returns total_count when count=true", async () => {
    store.create("patches", "p-1", makePatch({ title: "First" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "Second" }), "patch");
    store.create("patches", "p-3", makePatch({ title: "Third" }), "patch");
    const data = await listPatches({ count: "true" });
    expect(data.total_count).toBe(3);
  });

  it("returns total_count for the filtered set, not the page", async () => {
    store.create("patches", "p-1", makePatch({ status: "Open" }), "patch");
    store.create("patches", "p-2", makePatch({ status: "Open" }), "patch");
    store.create("patches", "p-3", makePatch({ status: "Closed" }), "patch");
    const data = await listPatches({ count: "true", status: "Open", limit: "0" });
    expect(data.patches).toHaveLength(0);
    expect(data.total_count).toBe(2);
  });

  it("supports limit=0 returning no patches but a valid total_count", async () => {
    store.create("patches", "p-1", makePatch({ title: "A" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "B" }), "patch");
    const data = await listPatches({ count: "true", limit: "0" });
    expect(data.patches).toHaveLength(0);
    expect(data.total_count).toBe(2);
  });

  it("omits total_count when count is not set", async () => {
    store.create("patches", "p-1", makePatch({ title: "First" }), "patch");
    const data = await listPatches();
    expect(data.total_count).toBeUndefined();
  });

  it("filters by ids (comma-separated)", async () => {
    store.create("patches", "p-1", makePatch({ title: "First" }), "patch");
    store.create("patches", "p-2", makePatch({ title: "Second" }), "patch");
    store.create("patches", "p-3", makePatch({ title: "Third" }), "patch");
    const data = await listPatches({ ids: "p-1,p-3" });
    expect(data.patches).toHaveLength(2);
    const returnedIds = data.patches.map((p: { patch_id: string }) => p.patch_id).sort();
    expect(returnedIds).toEqual(["p-1", "p-3"]);
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
    store.create("sessions", "t-1", makeSession({ prompt: "First" }), "session");
    store.create("sessions", "t-2", makeSession({ prompt: "Second" }), "session");
    const data = await listSessions();
    expect(data.sessions).toHaveLength(2);
  });

  it("filters by spawned_from", async () => {
    store.create("sessions", "t-1", makeSession({ spawned_from: "i-abc123" }), "session");
    store.create("sessions", "t-2", makeSession({ spawned_from: "i-def456" }), "session");
    store.create("sessions", "t-3", makeSession({ spawned_from: "i-abc123" }), "session");
    const data = await listSessions({ spawned_from: "i-abc123" });
    expect(data.sessions).toHaveLength(2);
    expect(
      data.sessions.every(
        (j: { session: { spawned_from: string } }) => j.session.spawned_from === "i-abc123",
      ),
    ).toBe(true);
  });

  it("filters by status", async () => {
    store.create("sessions", "t-1", makeSession({ status: "running" as Status }), "session");
    store.create("sessions", "t-2", makeSession({ status: "pending" as Status }), "session");
    store.create("sessions", "t-3", makeSession({ status: "running" as Status }), "session");
    const data = await listSessions({ status: "running" });
    expect(data.sessions).toHaveLength(2);
    expect(
      data.sessions.every((j: { session: { status: string } }) => j.session.status === "running"),
    ).toBe(true);
  });

  it("filters by q (case-insensitive substring on prompt)", async () => {
    store.create("sessions", "t-1", makeSession({ prompt: "Deploy the application" }), "session");
    store.create("sessions", "t-2", makeSession({ prompt: "Run tests" }), "session");
    store.create("sessions", "t-3", makeSession({ prompt: "deploy staging" }), "session");
    const data = await listSessions({ q: "deploy" });
    expect(data.sessions).toHaveLength(2);
  });

  it("combines filters with AND logic", async () => {
    store.create(
      "sessions",
      "t-1",
      makeSession({ spawned_from: "i-abc", status: "running" as Status }),
      "session",
    );
    store.create(
      "sessions",
      "t-2",
      makeSession({ spawned_from: "i-abc", status: "complete" as Status }),
      "session",
    );
    store.create(
      "sessions",
      "t-3",
      makeSession({ spawned_from: "i-def", status: "running" as Status }),
      "session",
    );
    const data = await listSessions({ spawned_from: "i-abc", status: "running" });
    expect(data.sessions).toHaveLength(1);
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
    store.create(
      "documents",
      "d-1",
      makeDocument({ title: "Doc A", path: "docs/readme.md" }),
      "document",
    );
    store.create(
      "documents",
      "d-2",
      makeDocument({ title: "Doc B", path: "src/index.ts" }),
      "document",
    );
    const data = await listDocuments({ q: "readme" });
    expect(data.documents).toHaveLength(1);
  });

  it("filters by q matching either title or path", async () => {
    store.create(
      "documents",
      "d-1",
      makeDocument({ title: "README", path: "docs/intro.md" }),
      "document",
    );
    store.create(
      "documents",
      "d-2",
      makeDocument({ title: "Guide", path: "docs/readme.md" }),
      "document",
    );
    store.create(
      "documents",
      "d-3",
      makeDocument({ title: "Other", path: "src/main.ts" }),
      "document",
    );
    const data = await listDocuments({ q: "readme" });
    expect(data.documents).toHaveLength(2);
  });

  it("filters by ids (comma-separated)", async () => {
    store.create("documents", "d-1", makeDocument({ title: "First" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Second" }), "document");
    store.create("documents", "d-3", makeDocument({ title: "Third" }), "document");
    const data = await listDocuments({ ids: "d-1,d-3" });
    expect(data.documents).toHaveLength(2);
    const returnedIds = data.documents
      .map((d: { document_id: string }) => d.document_id)
      .sort();
    expect(returnedIds).toEqual(["d-1", "d-3"]);
  });

  it("returns total_count when count=true", async () => {
    store.create("documents", "d-1", makeDocument({ title: "First" }), "document");
    store.create("documents", "d-2", makeDocument({ title: "Second" }), "document");
    const data = await listDocuments({ count: "true" });
    expect(data.total_count).toBe(2);
  });

  it("paginates with limit + cursor and preserves ids filter across pages", async () => {
    // Regression for i-pfowwitt: when the client paginates a relation-scoped
    // query, the `ids` filter must be honored on every page — not just the
    // first. This test seeds 3 in-set documents + 2 out-of-set documents,
    // pages through the in-set with limit=2, and asserts the second page only
    // contains the remaining in-set document (never the out-of-set ones).
    for (let i = 1; i <= 3; i++) {
      store.create(
        "documents",
        `d-in-${i}`,
        makeDocument({ title: `In ${i}` }),
        "document",
      );
    }
    store.create("documents", "d-out-1", makeDocument({ title: "Out 1" }), "document");
    store.create("documents", "d-out-2", makeDocument({ title: "Out 2" }), "document");

    const idsParam = "d-in-1,d-in-2,d-in-3";

    const page1 = await listDocuments({ ids: idsParam, limit: "2" });
    expect(page1.documents).toHaveLength(2);
    expect(
      page1.documents.every((d: { document_id: string }) =>
        d.document_id.startsWith("d-in-"),
      ),
    ).toBe(true);
    expect(page1.next_cursor).toBeTruthy();

    const page2 = await listDocuments({
      ids: idsParam,
      limit: "2",
      cursor: page1.next_cursor,
    });
    expect(page2.documents).toHaveLength(1);
    expect(page2.documents[0].document_id.startsWith("d-in-")).toBe(true);
    expect(page2.next_cursor).toBeFalsy();

    // Union of the two pages must equal the in-set, with no out-of-set leaks.
    const allReturned = [...page1.documents, ...page2.documents]
      .map((d: { document_id: string }) => d.document_id)
      .sort();
    expect(allReturned).toEqual(["d-in-1", "d-in-2", "d-in-3"]);
  });
});

// Regression coverage for i-pfowwitt: when the Chat Related tab's Load More
// fires `fetchNextPage`, the paginated query must continue to honor the
// `ids` filter (the relation scope). These tests assert the mock backend
// keeps that contract for issues and patches as well — both surfaces are
// driven by the same chat related artifacts hook.
describe("Paginated relation scoping", () => {
  it("issues: cursor pagination keeps the ids filter on every page", async () => {
    const store = new Store();
    const app = createIssueRoutes(store);
    for (let i = 1; i <= 3; i++) {
      store.create("issues", `i-in-${i}`, makeIssue({ description: `In ${i}` }), "issue");
    }
    store.create("issues", "i-out-1", makeIssue({ description: "Out 1" }), "issue");
    store.create("issues", "i-out-2", makeIssue({ description: "Out 2" }), "issue");

    const idsParam = "i-in-1,i-in-2,i-in-3";
    async function listIssues(params: Record<string, string>) {
      const qs = new URLSearchParams(params).toString();
      const res = await app.request(`http://localhost/v1/issues?${qs}`);
      return res.json();
    }

    const page1 = await listIssues({ ids: idsParam, limit: "2" });
    expect(page1.issues).toHaveLength(2);
    expect(page1.next_cursor).toBeTruthy();

    const page2 = await listIssues({
      ids: idsParam,
      limit: "2",
      cursor: page1.next_cursor,
    });
    const allReturned = [...page1.issues, ...page2.issues]
      .map((i: { issue_id: string }) => i.issue_id)
      .sort();
    expect(allReturned).toEqual(["i-in-1", "i-in-2", "i-in-3"]);
    expect(
      allReturned.every((id: string) => id.startsWith("i-in-")),
    ).toBe(true);
  });

  it("patches: cursor pagination keeps the ids filter on every page", async () => {
    const store = new Store();
    const app = createPatchRoutes(store);
    for (let i = 1; i <= 3; i++) {
      store.create("patches", `p-in-${i}`, makePatch({ title: `In ${i}` }), "patch");
    }
    store.create("patches", "p-out-1", makePatch({ title: "Out 1" }), "patch");
    store.create("patches", "p-out-2", makePatch({ title: "Out 2" }), "patch");

    const idsParam = "p-in-1,p-in-2,p-in-3";
    async function listPatches(params: Record<string, string>) {
      const qs = new URLSearchParams(params).toString();
      const res = await app.request(`http://localhost/v1/patches?${qs}`);
      return res.json();
    }

    const page1 = await listPatches({ ids: idsParam, limit: "2" });
    expect(page1.patches).toHaveLength(2);
    expect(page1.next_cursor).toBeTruthy();

    const page2 = await listPatches({
      ids: idsParam,
      limit: "2",
      cursor: page1.next_cursor,
    });
    const allReturned = [...page1.patches, ...page2.patches]
      .map((p: { patch_id: string }) => p.patch_id)
      .sort();
    expect(allReturned).toEqual(["p-in-1", "p-in-2", "p-in-3"]);
    expect(
      allReturned.every((id: string) => id.startsWith("p-in-")),
    ).toBe(true);
  });
});
