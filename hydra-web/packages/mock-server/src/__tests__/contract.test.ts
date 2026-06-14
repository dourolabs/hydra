import { describe, it, expect, beforeAll, afterAll, beforeEach } from "vitest";
import { HydraApiClient, ApiError } from "@hydra/api";
import type {
  UpsertIssueRequest,
  CreateSessionRequest,
  UpsertPatchRequest,
  UpsertDocumentRequest,
  UpsertAgentRequest,
  CreateRepositoryRequest,
  UpdateRepositoryRequest,
  UpsertTriggerRequest,
} from "@hydra/api";
import { startMockServer, type MockServerHandle } from "../index.js";

let server: MockServerHandle;
let client: HydraApiClient;
let baseUrl: string;
const originalFetch = globalThis.fetch;

beforeAll(async () => {
  server = await startMockServer({ port: 0 });
  baseUrl = `http://localhost:${server.port}`;
  client = new HydraApiClient({ baseUrl });

  // Inject Authorization header into all requests since HydraApiClient
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
      project_id: "j-defaul",
      assignee: { User: { name: "alice" } },
      dependencies: [],
      patches: [],
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
    expect(fetched.issue.status.key).toBe("open");
    expect(fetched.issue.assignee).toEqual({ User: { name: "alice" } });
    expect(fetched.creation_time).toBeTruthy();

    // List — should contain our issue
    const list = await client.listIssues();
    const found = list.issues.find((i) => i.issue_id === issueId);
    expect(found).toBeDefined();
    expect(found!.issue.status.key).toBe("open");

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
    expect(refetched.issue.status.key).toBe("in-progress");
    expect(refetched.issue.progress).toBe("Working on it");

    // Delete
    const deleted = await client.deleteIssue(issueId);
    expect(deleted.issue_id).toBe(issueId);
    expect(deleted.issue.archived).toBe(true);

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
    expect(deletedFetch.issue.archived).toBe(true);
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
    expect(versions.versions[0].issue.status.key).toBe("open");
    expect(versions.versions[1].issue.status.key).toBe("closed");

    // Get specific version
    const v1 = await client.getIssueVersion(issueId, 1);
    expect(v1.issue.status.key).toBe("open");
  });

  it("list filtering by status", async () => {
    await client.createIssue(issuePayload);
    await client.createIssue({
      issue: { ...issuePayload.issue, status: "closed" },
      session_id: null,
    });

    const openIssues = await client.listIssues({ status: "open" });
    for (const issue of openIssues.issues) {
      expect(issue.issue.status.key).toBe("open");
    }

    const closedIssues = await client.listIssues({ status: "closed" });
    for (const issue of closedIssues.issues) {
      expect(issue.issue.status.key).toBe("closed");
    }
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
    mode: { type: "headless" },
    agent_config: {
      type: "adhoc",
      system_prompt: "Contract test session prompt",
    },
    mount_spec: {
      working_dir: "repo",
      mounts: [
        {
          type: "bundle",
          target: "repo",
          bundle: {
            type: "git_repository",
            url: "https://github.com/test/repo.git",
            rev: "main",
          },
        },
        { type: "documents", target: "documents" },
      ],
    },
  };

  it("round-trip: create → get → list → versions → kill", async () => {
    // Create
    const created = await client.createSession(sessionPayload);
    expect(created.session_id).toBeTruthy();
    const sessionId = created.session_id;

    // Get
    const fetched = await client.getSession(sessionId);
    expect(fetched.session_id).toBe(sessionId);
    expect(fetched.session.mode).toEqual({ type: "headless" });
    expect(fetched.session.agent_config?.system_prompt).toBe(
      "Contract test session prompt",
    );
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
    expect(ctx.mode_kind).toBe("headless");
    // system_prompt no longer flows through WorkerContext — it is
    // delivered via Phase 2 `FirstMessage` over the relay websocket.
    const firstItem = ctx.mounts[0];
    expect(firstItem.type).toBe("bundle");
    if (firstItem.type === "bundle") {
      expect(firstItem.bundle.type).toBe("git_repository");
    }
  });

  it("get session logs", async () => {
    const created = await client.createSession(sessionPayload);
    const resp = await client.getSessionLogs(created.session_id);
    const text = await resp.text();
    expect(text).toContain("[mock]");
  });

  it("GET /v1/sessions/:id/events returns SessionEvent log; unknown session 404s", async () => {
    // Seed pair t-seed00014 + t-seed00015 forms a 2-session resumption chain
    // for conversation c-seed00007.
    const first = await client.getSessionEvents("t-seed00014");
    expect(first.length).toBeGreaterThan(0);
    expect(first[0].type).toBe("user_message");

    const second = await client.getSessionEvents("t-seed00015");
    expect(second[0].type).toBe("resumed");

    // A session that exists but has no SessionEvent log returns an empty
    // array — this is the legacy-fallback signal the frontend depends on.
    const created = await client.createSession(sessionPayload);
    const empty = await client.getSessionEvents(created.session_id);
    expect(empty).toEqual([]);

    await expect(
      client.getSessionEvents("t-does-not-exist"),
    ).rejects.toBeInstanceOf(ApiError);
  });

  it("GET /v1/sessions?conversation_id=X filters to sessions linked to that conversation", async () => {
    const list = await client.listSessions({ conversation_id: "c-seed00007" });
    const ids = list.sessions.map((s) => s.session_id).sort();
    expect(ids).toEqual(["t-seed00014", "t-seed00015"]);
  });
});

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------
describe("Conversations", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("seed conversation chat is reachable via the linked session's events", async () => {
    const conversation = await client.getConversation("c-seed00001");
    expect(conversation.conversation_id).toBe("c-seed00001");
    expect(conversation.title).toBe("Welcome to Hydra");
    expect(conversation.status).toBe("active");

    const linked = await client.listSessions({ conversation_id: "c-seed00001" });
    expect(linked.sessions.length).toBeGreaterThan(0);
    const sid = linked.sessions[0].session_id;
    const sessionEvents = await client.getSessionEvents(sid);
    expect(sessionEvents.length).toBeGreaterThanOrEqual(3);
    expect(sessionEvents[0].type).toBe("user_message");
  });

  it("list returns summaries with chat-text event_count aggregated across linked sessions", async () => {
    const { conversations: list } = await client.listConversations();

    // c-seed00001 (active) is backed by session t-seed00016 which carries 2
    // user_message + 2 assistant_message events.
    const active = list.find((c) => c.conversation_id === "c-seed00001");
    expect(active).toBeDefined();
    expect(active!.event_count).toBe(4);
    expect(active!.last_event_preview).toMatch(/^Assistant: /);

    // c-seed00002 (closed) is backed by session t-seed00017 with 1
    // user_message + 1 assistant_message.
    const closed = list.find((c) => c.conversation_id === "c-seed00002");
    expect(closed).toBeDefined();
    expect(closed!.event_count).toBe(2);
    expect(closed!.last_event_preview).toMatch(/^Assistant: /);
  });

  it("round-trip: create → get → list → send → close", async () => {
    const created = await client.createConversation({ message: "Initial hello" });
    expect(created.conversation_id).toMatch(/^c-/);
    expect(created.status).toBe("active");
    const cid = created.conversation_id;

    const fetched = await client.getConversation(cid);
    expect(fetched.conversation_id).toBe(cid);

    const { conversations: list } = await client.listConversations();
    expect(list.some((c) => c.conversation_id === cid)).toBe(true);

    // sendMessage appends a user_message SessionEvent on the conversation's
    // linked interactive session, mirroring the real backend's
    // resume-on-send behaviour.
    await client.sendMessage(cid, { content: "Follow-up" });

    const linked = await client.listSessions({ conversation_id: cid });
    expect(linked.sessions.length).toBe(1);
    const sid = linked.sessions[0].session_id;

    const sessionEvents = await client.getSessionEvents(sid);
    expect(sessionEvents.length).toBe(2);
    expect(sessionEvents[0]).toMatchObject({
      type: "user_message",
      content: "Initial hello",
    });
    expect(sessionEvents[1]).toMatchObject({
      type: "user_message",
      content: "Follow-up",
    });

    // close sets status to closed; the status transition is observable on
    // the conversation snapshot itself (no separate event log).
    await client.closeConversation(cid);
    const afterClose = await client.getConversation(cid);
    expect(afterClose.status).toBe("closed");
  });

  it("filters list by status and q", async () => {
    const active = (await client.listConversations({ status: "active" }))
      .conversations;
    expect(active.every((c) => c.status === "active")).toBe(true);
    expect(active.some((c) => c.conversation_id === "c-seed00001")).toBe(true);

    const closed = (await client.listConversations({ status: "closed" }))
      .conversations;
    expect(closed.every((c) => c.status === "closed")).toBe(true);

    const matches = (await client.listConversations({ q: "welcome" }))
      .conversations;
    expect(matches.some((c) => c.conversation_id === "c-seed00001")).toBe(true);
  });

  it("GET on unknown id returns 404", async () => {
    await expect(client.getConversation("c-does-not-exist")).rejects.toBeInstanceOf(ApiError);
  });

  it("dev/reset restores seed conversation and clears transient ones", async () => {
    const created = await client.createConversation({ message: "transient" });
    const cid = created.conversation_id;
    expect((await client.getConversation(cid)).conversation_id).toBe(cid);

    await resetServer();

    // Seed is back
    const seed = await client.getConversation("c-seed00001");
    expect(seed.conversation_id).toBe("c-seed00001");
    const closedSeed = await client.getConversation("c-seed00006");
    expect(closedSeed.status).toBe("closed");

    // Transient one is gone
    await expect(client.getConversation(cid)).rejects.toBeInstanceOf(ApiError);
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
    expect(deleted.patch.archived).toBe(true);

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
    expect(deleted.document.archived).toBe(true);

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
    mcp_config_path: null,
    mcp_config: null,
    max_tries: 5,
    max_simultaneous: 3,
    is_default_conversation_agent: false,
    secrets: [],
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
        project_id: "j-defaul",
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
        project_id: "j-defaul",
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

  it("POST /v1/conversations/:id/messages emits session_event_created on /v1/events", async () => {
    // Pre-create the conversation + initial session so the send-message path
    // routes through the existing session rather than spawning a new one
    // (either path emits session_event_created — picking one for clarity).
    const conversation = await client.createConversation({
      agent_name: null,
      session_settings: undefined,
      message: "hello",
    });
    const conversationId = conversation.conversation_id;
    const sessions = await client.listSessions({ conversation_id: conversationId });
    expect(sessions.sessions.length).toBeGreaterThan(0);
    const sessionId = sessions.sessions[0].session_id;

    // Subscribe to /v1/events.
    const controller = new AbortController();
    const resp = await originalFetch(`${baseUrl}/v1/events`, {
      method: "GET",
      headers: { Authorization: "Bearer dev-token-12345" },
      signal: controller.signal,
    });
    expect(resp.status).toBe(200);
    const reader = resp.body!.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    const received: Array<{ event: string; data: string; id?: string }> = [];

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
        if (event || data) parsed.push({ event, data, id });
      }
      return { parsed, remaining };
    }
    async function readUntil(
      predicate: (evts: typeof received) => boolean,
      maxBytes = 65536,
    ) {
      let bytesRead = 0;
      while (!predicate(received) && bytesRead < maxBytes) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        bytesRead += value.length;
        const { parsed, remaining } = parseSSEBuffer(buffer);
        buffer = remaining;
        received.push(...parsed);
      }
    }

    // Wait for the connected event and capture current_seq for monotonicity.
    await readUntil((evts) => evts.some((e) => e.event === "connected"));
    const connected = received.find((e) => e.event === "connected")!;
    const priorSeq = JSON.parse(connected.data).current_seq as number;

    // Post a message; expect session_event_created to fire.
    const beforeSend = received.length;
    await client.sendMessage(conversationId, { content: "follow-up question" });
    await readUntil((evts) =>
      evts.slice(beforeSend).some((e) => e.event === "session_event_created"),
    );

    const sse = received.find((e) => e.event === "session_event_created");
    expect(sse).toBeDefined();
    expect(Number(sse!.id)).toBeGreaterThan(priorSeq);
    const payload = JSON.parse(sse!.data);
    expect(payload.entity_type).toBe("session_event");
    expect(payload.entity_id).toBe(sessionId);
    expect(payload.entity).toMatchObject({
      type: "user_message",
      content: "follow-up question",
    });
    expect(typeof payload.entity.timestamp).toBe("string");
    expect(typeof payload.version).toBe("number");
    expect(typeof payload.timestamp).toBe("string");

    controller.abort();
    reader.releaseLock();
  });
});

// ---------------------------------------------------------------------------
// Session events fixture coverage
// ---------------------------------------------------------------------------
describe("Session events seed coverage", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("every t-seedNNNNN session has a non-empty event log covering all variants", async () => {
    const { sessions } = await client.listSessions({ limit: 1000 });
    const seedIds = sessions
      .map((s) => s.session_id)
      .filter((id) => /^t-seed\d{5}$/.test(id));
    expect(seedIds.length).toBeGreaterThanOrEqual(18);

    const variantsSeen = new Set<string>();
    for (const id of seedIds) {
      const events = await client.getSessionEvents(id);
      expect(events.length, `${id} should have at least one event`).toBeGreaterThan(0);
      for (const event of events) variantsSeen.add(event.type);
    }

    for (const variant of [
      "user_message",
      "assistant_message",
      "tool_use",
      "suspending",
      "resumed",
      "closed",
    ]) {
      expect(variantsSeen.has(variant), `missing variant: ${variant}`).toBe(true);
    }
  });
});

// ---------------------------------------------------------------------------
// BFF proxy rewrite: /api/v1/* -> /v1/*
// ---------------------------------------------------------------------------
describe("BFF proxy rewrite", () => {
  beforeEach(async () => {
    await resetServer();
  });

  it("GET /api/v1/issues returns issues when cookie auth is provided", async () => {
    const resp = await originalFetch(`${baseUrl}/api/v1/issues`, {
      method: "GET",
      headers: {
        Cookie: "hydra_token=dev-token-12345",
      },
    });
    expect(resp.status).toBe(200);
    const body = await resp.json();
    expect(body.issues).toBeDefined();
    expect(body.issues.length).toBeGreaterThan(0);
  });

  it("GET /api/v1/issues returns 401 without cookie", async () => {
    const resp = await originalFetch(`${baseUrl}/api/v1/issues`, {
      method: "GET",
    });
    expect(resp.status).toBe(401);
    const body = await resp.json();
    expect(body.error).toContain("not authenticated");
  });

  it("POST /api/v1/issues creates an issue via cookie auth", async () => {
    const resp = await originalFetch(`${baseUrl}/api/v1/issues`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Cookie: "hydra_token=dev-token-12345",
      },
      body: JSON.stringify({
        issue: {
          type: "task",
          title: "",
          description: "Created via BFF proxy",
          creator: "dev-user",
          progress: "",
          status: "open",
          project_id: "j-defaul",
          dependencies: [],
          patches: [],
        },
        session_id: null,
      }),
    });
    expect(resp.status).toBe(201);
    const body = await resp.json();
    expect(body.issue_id).toBeTruthy();
  });

  it("GET /api/v1/issues/:id retrieves a specific issue", async () => {
    // Create an issue first via direct API
    const createResp = await originalFetch(`${baseUrl}/v1/issues`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: "Bearer dev-token-12345",
      },
      body: JSON.stringify({
        issue: {
          type: "task",
          title: "",
          description: "BFF proxy get test",
          creator: "dev-user",
          progress: "",
          status: "open",
          project_id: "j-defaul",
          dependencies: [],
          patches: [],
        },
        session_id: null,
      }),
    });
    const created = await createResp.json();

    // Fetch via BFF proxy
    const resp = await originalFetch(
      `${baseUrl}/api/v1/issues/${created.issue_id}`,
      {
        method: "GET",
        headers: {
          Cookie: "hydra_token=dev-token-12345",
        },
      },
    );
    expect(resp.status).toBe(200);
    const body = await resp.json();
    expect(body.issue_id).toBe(created.issue_id);
    expect(body.issue.description).toBe("BFF proxy get test");
  });

  it("preserves query parameters in rewritten URL", async () => {
    const resp = await originalFetch(
      `${baseUrl}/api/v1/issues?status=open`,
      {
        method: "GET",
        headers: {
          Cookie: "hydra_token=dev-token-12345",
        },
      },
    );
    expect(resp.status).toBe(200);
    const body = await resp.json();
    for (const issue of body.issues) {
      expect(issue.issue.status.key).toBe("open");
    }
  });

  it("SSE /api/v1/events works through BFF proxy", async () => {
    const controller = new AbortController();
    const resp = await originalFetch(`${baseUrl}/api/v1/events`, {
      method: "GET",
      headers: {
        Cookie: "hydra_token=dev-token-12345",
      },
      signal: controller.signal,
    });
    expect(resp.status).toBe(200);

    const reader = resp.body!.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    // Read until we get the connected event
    let bytesRead = 0;
    const events: Array<{ event: string; data: string }> = [];
    while (bytesRead < 65536) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      bytesRead += value.length;

      const blocks = buffer.split("\n\n");
      buffer = blocks.pop() ?? "";
      for (const block of blocks) {
        if (!block.trim()) continue;
        let event = "";
        let data = "";
        for (const line of block.split("\n")) {
          if (line.startsWith("event:")) event = line.slice(6).trim();
          else if (line.startsWith("data:")) data = line.slice(5).trim();
        }
        if (event || data) events.push({ event, data });
      }

      if (events.some((e) => e.event === "connected")) break;
    }

    expect(events.some((e) => e.event === "connected")).toBe(true);
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

  it("seed conversations are loaded", async () => {
    const { conversations: list } = await client.listConversations();
    expect(list.length).toBeGreaterThanOrEqual(1);
    expect(list.some((c) => c.conversation_id === "c-seed00001")).toBe(true);
  });

  it("seed triggers are loaded with both Cron and Once schedules", async () => {
    const list = await client.listTriggers();
    expect(list.triggers.length).toBeGreaterThanOrEqual(2);
    const schedules = list.triggers.map((t) => t.trigger.schedule);
    expect(schedules.some((s) => "Cron" in s)).toBe(true);
    expect(schedules.some((s) => "Once" in s)).toBe(true);
  });

  it("seed triggers expose `created` relations with created_at for firing history", async () => {
    const list = await client.listTriggers();
    const cronTrigger = list.triggers.find((t) => "Cron" in t.trigger.schedule);
    expect(cronTrigger).toBeDefined();
    const fires = await client.listRelations({
      source_id: cronTrigger!.trigger_id,
      rel_type: "created",
    });
    expect(fires.relations.length).toBeGreaterThanOrEqual(2);
    for (const rel of fires.relations) {
      expect(rel.rel_type).toBe("created");
      expect(rel.created_at).toBeTruthy();
      expect(typeof rel.created_at).toBe("string");
    }
  });

  it("seed projects include bespoke engineering-v2 with six ordered statuses", async () => {
    const list = await client.listProjects();
    expect(list.projects.length).toBeGreaterThanOrEqual(1);

    const engv2 = list.projects.find((p) => p.project.key === "engineering-v2");
    expect(engv2).toBeDefined();
    expect(engv2!.project_id).toBe("j-engv2");
    expect(engv2!.project.name).toBe("Engineering v2");
    expect(engv2!.project.prompt_path).toBe("/projects/engineering-v2/prompt.md");

    const statusKeys = engv2!.project.statuses.map((s) => s.key);
    expect(statusKeys).toEqual([
      "inbox",
      "backlog",
      "pending",
      "in-development",
      "in-review",
      "pending-release",
    ]);

    // Status definitions for statuses with on_enter carry per-status prompts.
    const backlog = engv2!.project.statuses.find((s) => s.key === "backlog")!;
    expect(backlog.on_enter).toEqual({
      assign_to: { Agent: { name: "pm" } },
      attach_form: null,
    });
    expect(backlog.prompt_path).toBe("/projects/engineering-v2/statuses/backlog.md");

    const inReview = engv2!.project.statuses.find((s) => s.key === "in-review")!;
    expect(inReview.on_enter).toEqual({
      assign_to: { Agent: { name: "reviewer" } },
      attach_form: "/forms/review.yaml",
    });

    // Terminal-for-dependencies status flips both unblocks_* flags.
    const release = engv2!.project.statuses.find((s) => s.key === "pending-release")!;
    expect(release.unblocks_parents).toBe(true);
    expect(release.unblocks_dependents).toBe(true);
    expect(release.cascades_to_children).toBe(false);
  });

  it("GET /v1/projects/:id/statuses returns engineering-v2 status list", async () => {
    const resp = await client.getProjectStatuses("j-engv2");
    expect(resp.statuses.map((s) => s.key)).toEqual([
      "inbox",
      "backlog",
      "pending",
      "in-development",
      "in-review",
      "pending-release",
    ]);
  });

  it("seed issues with project_id round-trip resolved_status through GET /v1/issues/:id", async () => {
    // i-seed00018 was seeded with project_id=j-engv2 and resolved_status for
    // `in-review` (color=#8b5cf6). Both fields must survive the fixture →
    // store → wire round trip so the frontend can render the project-specific
    // status badge without a second round trip.
    const inReview = await client.getIssue("i-seed00018");
    expect(inReview.issue.project_id).toBe("j-engv2");
    expect(inReview.issue.status).toMatchObject({
      key: "in-review",
      label: "In review",
      color: "#8b5cf6",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
    });

    const inDevelopment = await client.getIssue("i-seed00012");
    expect(inDevelopment.issue.project_id).toBe("j-engv2");
    expect(inDevelopment.issue.status.key).toBe("in-development");

    // Spread coverage: at least four distinct status keys across the
    // project_id-tagged seed issues, so the issues list renders varied chips.
    const list = await client.listIssues({ limit: 100 });
    const taggedKeys = new Set<string>();
    for (const item of list.issues) {
      const summary = await client.getIssue(item.issue_id);
      if (summary.issue.project_id === "j-engv2") {
        taggedKeys.add(summary.issue.status.key);
      }
    }
    expect(taggedKeys.size).toBeGreaterThanOrEqual(4);
  });

  it("issues lacking an explicit project_id default to the seeded default project", async () => {
    // i-seed00001's fixture has no `project_id`; the seed-loader
    // backfills it to `j-defaul` (the seeded default project), mirroring
    // the real-server `seed_default_project` migration. The pre-PR
    // "null project_id falls back to DefaultProject at render time"
    // shape is gone — every issue now carries a real project id.
    const seed1 = await client.getIssue("i-seed00001");
    expect(seed1.issue.project_id).toBe("j-defaul");
  });

  it("seed documents include all four prompt levels", async () => {
    // System prompt.
    const sys = await client.getDocumentByPath("/agents/system_prompt.md");
    expect(sys.document.title).toBe("System prompt");
    expect(sys.document.body_markdown).toContain("hydra issues");

    // DefaultProject prompt + statuses.
    const defaultProj = await client.listDocuments({
      path_prefix: "/projects/default",
    });
    const defaultPaths = defaultProj.documents.map((d) => d.document.path);
    expect(defaultPaths).toEqual(
      expect.arrayContaining([
        "/projects/default/prompt.md",
        "/projects/default/statuses/open.md",
        "/projects/default/statuses/in-progress.md",
      ]),
    );

    // Bespoke project prompt + per-status prompts.
    const engv2Prompts = await client.listDocuments({
      path_prefix: "/projects/engineering-v2",
    });
    const engv2Paths = engv2Prompts.documents.map((d) => d.document.path);
    expect(engv2Paths).toEqual(
      expect.arrayContaining([
        "/projects/engineering-v2/prompt.md",
        "/projects/engineering-v2/statuses/backlog.md",
        "/projects/engineering-v2/statuses/in-development.md",
        "/projects/engineering-v2/statuses/in-review.md",
      ]),
    );
  });

  it("auto_archive_after_seconds round-trips through per-status CRUD", async () => {
    await resetServer();
    const window = BigInt(1209600);
    // Add a status with the field set; the response must echo it.
    const created = await client.createProjectStatus("j-engv2", {
      key: "archive-trial",
      label: "Archive trial",
      color: "#abcdef",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 0,
      auto_archive_after_seconds: window,
    });
    expect(Number(created.status.auto_archive_after_seconds)).toBe(1209600);

    // GET /v1/projects/:id/statuses must include the field unchanged.
    const after = await client.getProjectStatuses("j-engv2");
    const fetched = after.statuses.find((s) => s.key === "archive-trial");
    expect(Number(fetched?.auto_archive_after_seconds)).toBe(1209600);

    // Clearing the field via PUT must round-trip back to null/undefined.
    const cleared = await client.updateProjectStatus("j-engv2", "archive-trial", {
      key: "archive-trial",
      label: "Archive trial",
      color: "#abcdef",
      unblocks_parents: false,
      unblocks_dependents: false,
      cascades_to_children: false,
      position: 0,
    });
    expect(cleared.status.auto_archive_after_seconds ?? null).toBeNull();
    const afterClear = await client.getProjectStatuses("j-engv2");
    const refetched = afterClear.statuses.find((s) => s.key === "archive-trial");
    expect(refetched?.auto_archive_after_seconds ?? null).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------
describe("Triggers", () => {
  beforeEach(async () => {
    await resetServer();
  });

  const triggerPayload: UpsertTriggerRequest = {
    enabled: true,
    schedule: { Cron: { expression: "0 0 * * *", timezone: "UTC" } },
    actions: [
      {
        CreateIssue: {
          type: "task",
          title: "Nightly health check",
          description: "Run the nightly health check sweep.",
          project_id: "j-defaul",
          status: "open",
        },
      },
    ],
    creator: "dev-user",
  };

  it("round-trip: create → get → list → update → delete → 404", async () => {
    const created = await client.createTrigger(triggerPayload);
    expect(created.trigger_id).toBeTruthy();
    const triggerId = created.trigger_id;

    const fetched = await client.getTrigger(triggerId);
    expect(fetched.trigger_id).toBe(triggerId);
    expect(fetched.trigger.enabled).toBe(true);
    expect("Cron" in fetched.trigger.schedule).toBe(true);
    expect(fetched.creation_time).toBeTruthy();

    const list = await client.listTriggers();
    expect(list.triggers.some((t) => t.trigger_id === triggerId)).toBe(true);

    await client.updateTrigger(triggerId, { ...triggerPayload, enabled: false });
    const refetched = await client.getTrigger(triggerId);
    expect(refetched.trigger.enabled).toBe(false);

    const deleted = await client.deleteTrigger(triggerId);
    expect(deleted.trigger.archived).toBe(true);

    await expect(client.getTrigger(triggerId)).rejects.toThrow(ApiError);
  });

  it("versions: create → update → list versions", async () => {
    const created = await client.createTrigger(triggerPayload);
    await client.updateTrigger(created.trigger_id, {
      ...triggerPayload,
      enabled: false,
    });
    const versions = await client.listTriggerVersions(created.trigger_id);
    expect(versions.versions.length).toBeGreaterThanOrEqual(2);
  });
});
