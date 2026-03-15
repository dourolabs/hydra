import { describe, it, expect, beforeAll, afterAll, beforeEach } from "vitest";
import { MetisApiClient, ApiError } from "@metis/api";
import type {
  UpsertIssueRequest,
  CreateSessionRequest,
  UpsertPatchRequest,
  UpsertDocumentRequest,
  UpsertAgentRequest,
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
} from "@metis/api";
import { startMockServer, type MockServerHandle } from "../index.js";

let server: MockServerHandle;
let client: MetisApiClient;
let baseUrl: string;
const originalFetch = globalThis.fetch;

beforeAll(async () => {
  server = await startMockServer({ port: 0 });
  baseUrl = `http://localhost:${server.port}`;
  client = new MetisApiClient({ baseUrl });

  // Inject Authorization header into all requests since MetisApiClient
  // relies on a BFF proxy for auth in production.
  globalThis.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
    const headers = new Headers(init?.headers);
    if (!headers.has("Authorization")) {
      headers.set("Authorization", "Bearer dev-token-12345");
    }
    return originalFetch(input, { ...init, headers });
  };
});

afterAll(async () => {
  globalThis.fetch = originalFetch;
  await server?.close();
});

/** POST /v1/dev/reset to restore seed data. */
async function resetServer() {
  await originalFetch(`${baseUrl}/v1/dev/reset`, {
    method: "POST",
    headers: { Authorization: "Bearer dev-token-12345" },
  });
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------
describe("Health", () => {
  it("GET /health returns 200 with status ok, no auth required", async () => {
    const resp = await originalFetch(`${baseUrl}/health`);
    expect(resp.status).toBe(200);
    const body = await resp.json();
    expect(body).toEqual({ status: "ok" });
  });
});

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------
describe("Issues", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const issuePayload: UpsertIssueRequest = {
    issue: {
      type: "task",
      title: "",
      description: "Contract test issue",
      creator: "dev-user",
      progress: "",
      status: "open",
      assignee: "alice",
      dependencies: [],
      patches: [],
      todo_list: [],
    },
    session_id: null,
  };

  it("round-trip: create → get → list → update → get → delete → 404", async () => {
    // Create
    const created = await client.createIssue(issuePayload);
    expect(created.issue_id).toBeTruthy();
    expect(created.version).toBeDefined();

    const issueId = created.issue_id;

    // Get
    const fetched = await client.getIssue(issueId);
    expect(fetched.issue_id).toBe(issueId);
    expect(fetched.issue.description).toBe("Contract test issue");
    expect(fetched.issue.status).toBe("open");
    expect(fetched.issue.assignee).toBe("alice");
    expect(fetched.creation_time).toBeTruthy();

    // List — should contain our issue
    const list = await client.listIssues();
    const found = list.issues.find((i) => i.issue_id === issueId);
    expect(found).toBeDefined();
    expect(found!.issue.status).toBe("open");

    // Update
    const updatedPayload: UpsertIssueRequest = {
      issue: {
        ...issuePayload.issue,
        status: "in-progress",
        progress: "Working on it",
      },
      session_id: null,
    };
    const updateResp = await client.updateIssue(issueId, updatedPayload);
    expect(updateResp.issue_id).toBe(issueId);
    expect(Number(updateResp.version)).toBeGreaterThan(Number(created.version));

    // Get after update
    const refetched = await client.getIssue(issueId);
    expect(refetched.issue.status).toBe("in-progress");
    expect(refetched.issue.progress).toBe("Working on it");

    // Delete
    const deleted = await client.deleteIssue(issueId);
    expect(deleted.issue_id).toBe(issueId);
    expect(deleted.issue.deleted).toBe(true);

    // Get after delete → 404
    try {
      await client.getIssue(issueId);
      expect.unreachable("Should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      expect((err as ApiError).status).toBe(404);
    }

    // Can still fetch with includeDeleted
    const deletedFetch = await client.getIssue(issueId, true);
    expect(deletedFetch.issue.deleted).toBe(true);
  });

  it("versions: create → update → list versions", async () => {
    const created = await client.createIssue(issuePayload);
    const issueId = created.issue_id;

    await client.updateIssue(issueId, {
      issue: { ...issuePayload.issue, status: "closed" },
      session_id: null,
    });

    // List all versions
    const versions = await client.listIssueVersions(issueId);
    expect(versions.versions.length).toBeGreaterThanOrEqual(2);
    expect(versions.versions[0].issue.status).toBe("open");
    expect(versions.versions[1].issue.status).toBe("closed");

    // Get specific version
    const v1 = await client.getIssueVersion(issueId, 1);
    expect(v1.issue.status).toBe("open");
  });

  it("list filtering by status", async () => {
    await client.createIssue(issuePayload);
    await client.createIssue({
      issue: { ...issuePayload.issue, status: "closed" },
      session_id: null,
    });

    const openIssues = await client.listIssues({ status: "open" });
    for (const issue of openIssues.issues) {
      expect(issue.issue.status).toBe("open");
    }

    const closedIssues = await client.listIssues({ status: "closed" });
    for (const issue of closedIssues.issues) {
      expect(issue.issue.status).toBe("closed");
    }
  });

  it("todo items: add, replace, set status", async () => {
    const created = await client.createIssue(issuePayload);
    const issueId = created.issue_id;

    // Add todo item
    const added = await client.addTodoItem(issueId, {
      description: "First task",
      is_done: false,
    });
    expect(added.issue_id).toBe(issueId);
    expect(added.todo_list).toHaveLength(1);
    expect(added.todo_list[0].description).toBe("First task");
    expect(added.todo_list[0].is_done).toBe(false);

    // Add another
    const added2 = await client.addTodoItem(issueId, {
      description: "Second task",
      is_done: false,
    });
    expect(added2.todo_list).toHaveLength(2);

    // Set todo item status
    const toggled = await client.setTodoItemStatus(issueId, 0, {
      is_done: true,
    });
    expect(toggled.todo_list[0].is_done).toBe(true);
    expect(toggled.todo_list[1].is_done).toBe(false);

    // Replace entire todo list
    const replaced = await client.replaceTodoList(issueId, {
      todo_list: [{ description: "Replaced task", is_done: true }],
    });
    expect(replaced.todo_list).toHaveLength(1);
    expect(replaced.todo_list[0].description).toBe("Replaced task");
  });
});

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------
describe("Sessions", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const sessionPayload: CreateSessionRequest = {
    prompt: "Contract test session prompt",
    context: {
      type: "git_repository",
      url: "https://github.com/test/repo.git",
      rev: "main",
    },
    issue_id: null,
  };

  it("round-trip: create → get → list → versions → kill", async () => {
    // Create
    const created = await client.createSession(sessionPayload);
    expect(created.session_id).toBeTruthy();
    const sessionId = created.session_id;

    // Get
    const fetched = await client.getSession(sessionId);
    expect(fetched.session_id).toBe(sessionId);
    expect(fetched.session.prompt).toBe("Contract test session prompt");
    expect(fetched.session.status).toBe("pending");

    // List
    const list = await client.listSessions();
    const found = list.sessions.find((j) => j.session_id === sessionId);
    expect(found).toBeDefined();

    // Versions
    const versions = await client.listSessionVersions(sessionId);
    expect(versions.versions.length).toBeGreaterThanOrEqual(1);

    // Get specific version
    const v1 = await client.getSessionVersion(sessionId, 1);
    expect(v1.session.status).toBe("pending");

    // Kill (DELETE) — returns intended terminal status but the session
    // stays "running" in the store until the pod actually terminates.
    const killed = await client.killSession(sessionId);
    expect(killed.session_id).toBe(sessionId);
    expect(killed.status).toBe("failed");
  });

  it("set session status: complete and failed", async () => {
    const created = await client.createSession(sessionPayload);
    const sessionId = created.session_id;

    // Set to complete
    const completed = await client.setSessionStatus(sessionId, {
      status: "complete",
      last_message: "All done",
    });
    expect(completed.status).toBe("complete");

    const afterComplete = await client.getSession(sessionId);
    expect(afterComplete.session.status).toBe("complete");
    expect(afterComplete.session.last_message).toBe("All done");
    expect(afterComplete.session.end_time).toBeTruthy();
  });

  it("get session context", async () => {
    const created = await client.createSession(sessionPayload);
    const ctx = await client.getSessionContext(created.session_id);
    expect(ctx.prompt).toBe("Contract test session prompt");
    expect(ctx.request_context.type).toBe("git_repository");
  });

  it("get session logs", async () => {
    const created = await client.createSession(sessionPayload);
    const resp = await client.getSessionLogs(created.session_id);
    const text = await resp.text();
    expect(text).toContain("[mock]");
  });
});

// ---------------------------------------------------------------------------
// Patches
// ---------------------------------------------------------------------------
describe("Patches", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const patchPayload: UpsertPatchRequest = {
    patch: {
      title: "Contract test patch",
      description: "Test patch description",
      diff: "diff --git a/test.ts b/test.ts\n+hello",
      status: "Open",
      is_automatic_backup: false,
      creator: "dev-user",
      reviews: [],
      service_repo_name: "acme/web-app",
      branch_name: "test-branch",
      base_branch: "main",
    },
  };

  it("round-trip: create → get → list → update → delete → 404", async () => {
    // Create
    const created = await client.createPatch(patchPayload);
    expect(created.patch_id).toBeTruthy();
    const patchId = created.patch_id;

    // Get
    const fetched = await client.getPatch(patchId);
    expect(fetched.patch_id).toBe(patchId);
    expect(fetched.patch.title).toBe("Contract test patch");
    expect(fetched.patch.status).toBe("Open");
    expect(fetched.creation_time).toBeTruthy();

    // List
    const list = await client.listPatches();
    const found = list.patches.find((p) => p.patch_id === patchId);
    expect(found).toBeDefined();

    // Update
    const updateResp = await client.updatePatch(patchId, {
      patch: { ...patchPayload.patch, status: "Merged", title: "Updated title" },
    });
    expect(updateResp.patch_id).toBe(patchId);

    // Verify update
    const refetched = await client.getPatch(patchId);
    expect(refetched.patch.status).toBe("Merged");
    expect(refetched.patch.title).toBe("Updated title");

    // Delete
    const deleted = await client.deletePatch(patchId);
    expect(deleted.patch.deleted).toBe(true);

    // 404 after delete
    await expect(client.getPatch(patchId)).rejects.toThrow(ApiError);
  });

  it("versions: create → update → list versions", async () => {
    const created = await client.createPatch(patchPayload);
    const patchId = created.patch_id;

    await client.updatePatch(patchId, {
      patch: { ...patchPayload.patch, title: "V2 title" },
    });

    const versions = await client.listPatchVersions(patchId);
    expect(versions.versions.length).toBeGreaterThanOrEqual(2);

    const v1 = await client.getPatchVersion(patchId, 1);
    expect(v1.patch.title).toBe("Contract test patch");
  });

  it("list filtering by status", async () => {
    await client.createPatch(patchPayload);
    await client.createPatch({
      patch: { ...patchPayload.patch, status: "Closed" },
    });

    const open = await client.listPatches({ status: ["Open"] });
    for (const p of open.patches) {
      expect(p.patch.status).toBe("Open");
    }
  });
});

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------
describe("Documents", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const docPayload: UpsertDocumentRequest = {
    document: {
      title: "Contract test document",
      body_markdown: "# Test\n\nThis is a test document.",
      path: "/test/contract-test-doc",
    },
  };

  it("round-trip: create → get → list → update → delete → 404", async () => {
    // Create
    const created = await client.createDocument(docPayload);
    expect(created.document_id).toBeTruthy();
    const docId = created.document_id;

    // Get
    const fetched = await client.getDocument(docId);
    expect(fetched.document_id).toBe(docId);
    expect(fetched.document.title).toBe("Contract test document");
    expect(fetched.document.body_markdown).toContain("# Test");
    expect(fetched.creation_time).toBeTruthy();

    // List
    const list = await client.listDocuments();
    const found = list.documents.find((d) => d.document_id === docId);
    expect(found).toBeDefined();

    // Update
    const updateResp = await client.updateDocument(docId, {
      document: { ...docPayload.document, title: "Updated doc title" },
    });
    expect(updateResp.document_id).toBe(docId);

    // Verify update
    const refetched = await client.getDocument(docId);
    expect(refetched.document.title).toBe("Updated doc title");

    // Delete
    const deleted = await client.deleteDocument(docId);
    expect(deleted.document.deleted).toBe(true);

    // 404
    await expect(client.getDocument(docId)).rejects.toThrow(ApiError);
  });

  it("versions: create → update → list versions", async () => {
    const created = await client.createDocument(docPayload);
    const docId = created.document_id;

    await client.updateDocument(docId, {
      document: { ...docPayload.document, title: "V2" },
    });

    const versions = await client.listDocumentVersions(docId);
    expect(versions.versions.length).toBeGreaterThanOrEqual(2);

    const v1 = await client.getDocumentVersion(docId, 1);
    expect(v1.document.title).toBe("Contract test document");
  });

  it("getDocumentByPath", async () => {
    await client.createDocument(docPayload);

    const byPath = await client.getDocumentByPath("/test/contract-test-doc");
    expect(byPath.document.title).toBe("Contract test document");
  });
});

// ---------------------------------------------------------------------------
// Repositories
// ---------------------------------------------------------------------------
describe("Repositories", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("round-trip: create → list → update → delete", async () => {
    const createReq: CreateRepositoryRequest = {
      name: "test-org/test-repo",
      remote_url: "https://github.com/test-org/test-repo.git",
      default_branch: "main",
      default_image: "node:20-slim",
    };

    // Create
    const created = await client.createRepository(createReq);
    expect(created.repository.name).toBe("test-org/test-repo");
    expect(created.repository.repository.remote_url).toBe(
      "https://github.com/test-org/test-repo.git",
    );

    // List
    const list = await client.listRepositories();
    const found = list.repositories.find((r) => r.name === "test-org/test-repo");
    expect(found).toBeDefined();

    // Update
    const updateReq: UpdateRepositoryRequest = {
      remote_url: "https://github.com/test-org/test-repo-v2.git",
      default_branch: "develop",
      default_image: "node:22-slim",
    };
    const updated = await client.updateRepository("test-org/test-repo", updateReq);
    expect(updated.repository.repository.remote_url).toBe(
      "https://github.com/test-org/test-repo-v2.git",
    );

    // Delete
    const deleted = await client.deleteRepository("test-org/test-repo");
    expect(deleted.name).toBe("test-org/test-repo");
  });
});

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------
describe("Agents", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const agentPayload: UpsertAgentRequest = {
    name: "test-agent",
    prompt: "You are a test agent.",
    prompt_path: "",
    max_tries: 5,
    max_simultaneous: 3,
    is_assignment_agent: false,
  };

  it("round-trip: create → get → list → update → delete", async () => {
    // Create
    const created = await client.createAgent(agentPayload);
    expect(created.agent.name).toBe("test-agent");

    // Get
    const fetched = await client.getAgent("test-agent");
    expect(fetched.agent.prompt).toBe("You are a test agent.");
    expect(fetched.agent.max_tries).toBe(5);
    expect(fetched.agent.max_simultaneous).toBe(3);

    // List
    const list = await client.listAgents();
    const found = list.agents.find((a) => a.name === "test-agent");
    expect(found).toBeDefined();

    // Update
    const updated = await client.updateAgent("test-agent", {
      ...agentPayload,
      prompt: "Updated prompt",
      max_tries: 10,
    });
    expect(updated.agent.prompt).toBe("Updated prompt");
    expect(updated.agent.max_tries).toBe(10);

    // Delete
    const deleted = await client.deleteAgent("test-agent");
    expect(deleted.agent.name).toBe("test-agent");

    // 404 after delete
    await expect(client.getAgent("test-agent")).rejects.toThrow(ApiError);
  });
});

// ---------------------------------------------------------------------------
// Merge Queues
// ---------------------------------------------------------------------------
describe("Merge Queues", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("get empty queue → enqueue → get with patch", async () => {
    const queue = await client.getMergeQueue("acme/web-app", "main");
    expect(queue.patches).toEqual([]);

    const enqueued = await client.enqueueMergePatch(
      "acme/web-app",
      "main",
      "p-test-001",
    );
    expect(enqueued.patches).toContain("p-test-001");

    const refetched = await client.getMergeQueue("acme/web-app", "main");
    expect(refetched.patches).toContain("p-test-001");
  });
});

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------
describe("Auth", () => {
  it("whoami returns dev-user", async () => {
    const who = await client.whoami();
    expect(who.actor.type).toBe("user");
    expect(who.actor).toEqual({ type: "user", username: "dev-user" });
  });

  it("getUserInfo returns user summary", async () => {
    const user = await client.getUserInfo("alice");
    expect(user.username).toBe("alice");
    expect(user.github_user_id).toBeDefined();
  });

  it("getGithubToken returns a token", async () => {
    const token = await client.getGithubToken();
    expect(token).toBe("ghp_mock_token_for_dev");
  });

  it("request without Bearer token returns 401", async () => {
    // Make a raw fetch without the auth header
    const resp = await originalFetch(`${baseUrl}/v1/issues`, {
      method: "GET",
    });
    expect(resp.status).toBe(401);
    const body = await resp.json();
    expect(body.error).toContain("Authorization");
  });
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------
describe("Error handling", () => {
  it("404 for nonexistent entity", async () => {
    try {
      await client.getIssue("i-nonexistent");
      expect.unreachable("Should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError);
      expect((err as ApiError).status).toBe(404);
      expect((err as ApiError).message).toContain("not found");
    }
  });

  it("X-Mock-Error header returns simulated error", async () => {
    const resp = await originalFetch(`${baseUrl}/v1/issues`, {
      method: "GET",
      headers: {
        Authorization: "Bearer dev-token-12345",
        "X-Mock-Error": "503",
      },
    });
    expect(resp.status).toBe(503);
    const body = await resp.json();
    expect(body.error).toBe("simulated server error");
  });
});

// ---------------------------------------------------------------------------
// Reset endpoint
// ---------------------------------------------------------------------------
describe("Reset endpoint", () => {
  it("create entity → reset → entity gone from list", async () => {
    // Start from seed data
    await resetServer();
    const before = await client.listIssues();
    const seedCount = before.issues.length;

    // Create a new issue
    const created = await client.createIssue({
      issue: {
        type: "task",
        title: "",
        description: "Ephemeral issue for reset test",
        creator: "dev-user",
        progress: "",
        status: "open",
        dependencies: [],
        patches: [],
      },
      session_id: null,
    });

    // Verify it's in the list
    const afterCreate = await client.listIssues();
    expect(afterCreate.issues.length).toBe(seedCount + 1);

    // Reset
    await resetServer();

    // Verify it's gone
    const afterReset = await client.listIssues();
    expect(afterReset.issues.length).toBe(seedCount);
    const found = afterReset.issues.find(
      (i) => i.issue_id === created.issue_id,
    );
    expect(found).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// SSE Events
// ---------------------------------------------------------------------------
describe("SSE Events", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("connect → receive connected → create entity → receive entity event", async () => {
    // Use raw fetch to test SSE since EventSource may not be available in test env
    const controller = new AbortController();
    const resp = await originalFetch(`${baseUrl}/v1/events`, {
      method: "GET",
      headers: { Authorization: "Bearer dev-token-12345" },
      signal: controller.signal,
    });
    expect(resp.status).toBe(200);

    const reader = resp.body!.getReader();
    const decoder = new TextDecoder();

    // Read chunks until we get the connected event
    let buffer = "";
    const events: Array<{ event: string; data: string; id?: string }> = [];

    function parseSSEBuffer(buf: string): {
      parsed: Array<{ event: string; data: string; id?: string }>;
      remaining: string;
    } {
      const parsed: Array<{ event: string; data: string; id?: string }> = [];
      const blocks = buf.split("\n\n");
      const remaining = blocks.pop() ?? "";

      for (const block of blocks) {
        if (!block.trim()) continue;
        let event = "";
        let data = "";
        let id: string | undefined;
        for (const line of block.split("\n")) {
          if (line.startsWith("event:")) event = line.slice(6).trim();
          else if (line.startsWith("data:")) data = line.slice(5).trim();
          else if (line.startsWith("id:")) id = line.slice(3).trim();
        }
        if (event || data) {
          parsed.push({ event, data, id });
        }
      }
      return { parsed, remaining };
    }

    // Read until we have the connected event
    async function readUntil(
      predicate: (evts: typeof events) => boolean,
      maxBytes = 65536,
    ) {
      let bytesRead = 0;
      while (!predicate(events) && bytesRead < maxBytes) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        bytesRead += value.length;
        const { parsed, remaining } = parseSSEBuffer(buffer);
        buffer = remaining;
        events.push(...parsed);
      }
    }

    // Wait for connected event
    await readUntil((evts) => evts.some((e) => e.event === "connected"));
    const connected = events.find((e) => e.event === "connected");
    expect(connected).toBeDefined();
    const connectedData = JSON.parse(connected!.data);
    expect(connectedData.current_seq).toBeDefined();

    // Now create an issue to trigger an entity event
    const eventsBeforeCreate = events.length;
    await client.createIssue({
      issue: {
        type: "task",
        title: "",
        description: "SSE test issue",
        creator: "dev-user",
        progress: "",
        status: "open",
        dependencies: [],
        patches: [],
      },
      session_id: null,
    });

    // Read until we get the issue_created event
    await readUntil(
      (evts) =>
        evts.slice(eventsBeforeCreate).some((e) => e.event === "issue_created"),
    );

    const issueEvent = events.find((e) => e.event === "issue_created");
    expect(issueEvent).toBeDefined();
    const eventData = JSON.parse(issueEvent!.data);
    expect(eventData.entity_type).toBe("issues");
    expect(eventData.entity_id).toBeTruthy();

    // Clean up the SSE connection
    controller.abort();
    reader.releaseLock();
  });
});

// ---------------------------------------------------------------------------
// Seed data sanity checks
// ---------------------------------------------------------------------------
describe("Seed data", () => {
  beforeAll(async () => {
    await resetServer();
  });

  it("seed issues are loaded", async () => {
    const list = await client.listIssues();
    expect(list.issues.length).toBeGreaterThanOrEqual(10);

    // Verify a known seed issue
    const seed1 = await client.getIssue("i-seed00001");
    expect(seed1.issue.type).toBe("feature");
    expect(seed1.issue.creator).toBe("alice");
    expect(seed1.issue.todo_list!.length).toBeGreaterThan(0);
  });

  it("seed sessions are loaded", async () => {
    const list = await client.listSessions();
    expect(list.sessions.length).toBeGreaterThanOrEqual(4);
  });

  it("seed patches are loaded", async () => {
    const list = await client.listPatches();
    expect(list.patches.length).toBeGreaterThanOrEqual(3);
  });

  it("seed documents are loaded", async () => {
    const list = await client.listDocuments();
    expect(list.documents.length).toBeGreaterThanOrEqual(3);
  });

  it("seed repositories are loaded", async () => {
    const list = await client.listRepositories();
    expect(list.repositories.length).toBeGreaterThanOrEqual(2);
  });

  it("seed agents are loaded", async () => {
    const list = await client.listAgents();
    expect(list.agents.length).toBeGreaterThanOrEqual(3);
  });
});
