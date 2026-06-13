import { ApiError } from "./errors";
import type { CreateSessionRequest } from "./generated/CreateSessionRequest";
import type { CreateSessionResponse } from "./generated/CreateSessionResponse";
import type { SearchSessionsQuery } from "./generated/SearchSessionsQuery";
import type { ListSessionsResponse } from "./generated/ListSessionsResponse";
import type { SessionVersionRecord } from "./generated/SessionVersionRecord";
import type { KillSessionResponse } from "./generated/KillSessionResponse";
import type { LogsQuery } from "./generated/LogsQuery";
import type { SessionStatusUpdate } from "./generated/SessionStatusUpdate";
import type { SetSessionStatusResponse } from "./generated/SetSessionStatusResponse";
import type { WorkerContext } from "./generated/WorkerContext";
import type { ListSessionVersionsResponse } from "./generated/ListSessionVersionsResponse";
import type { UpsertIssueRequest } from "./generated/UpsertIssueRequest";
import type { UpsertIssueResponse } from "./generated/UpsertIssueResponse";
import type { IssueVersionRecord } from "./generated/IssueVersionRecord";
import type { SearchIssuesQuery } from "./generated/SearchIssuesQuery";
import type { ListIssuesResponse } from "./generated/ListIssuesResponse";
import type { ListIssueVersionsResponse } from "./generated/ListIssueVersionsResponse";
import type { UpsertPatchRequest } from "./generated/UpsertPatchRequest";
import type { UpsertPatchResponse } from "./generated/UpsertPatchResponse";
import type { PatchVersionRecord } from "./generated/PatchVersionRecord";
import type { SearchPatchesQuery } from "./generated/SearchPatchesQuery";
import type { ListPatchesResponse } from "./generated/ListPatchesResponse";
import type { ListPatchVersionsResponse } from "./generated/ListPatchVersionsResponse";
import type { UpsertDocumentRequest } from "./generated/UpsertDocumentRequest";
import type { UpsertDocumentResponse } from "./generated/UpsertDocumentResponse";
import type { DocumentVersionRecord } from "./generated/DocumentVersionRecord";
import type { SearchDocumentsQuery } from "./generated/SearchDocumentsQuery";
import type { ListDocumentsResponse } from "./generated/ListDocumentsResponse";
import type { ListDocumentVersionsResponse } from "./generated/ListDocumentVersionsResponse";
import type { ListDocumentPathsQuery } from "./generated/ListDocumentPathsQuery";
import type { ListDocumentPathsResponse } from "./generated/ListDocumentPathsResponse";
import type { SearchRepositoriesQuery } from "./generated/SearchRepositoriesQuery";
import type { ListRepositoriesResponse } from "./generated/ListRepositoriesResponse";
import type { CreateRepositoryRequest } from "./generated/CreateRepositoryRequest";
import type { UpsertRepositoryResponse } from "./generated/UpsertRepositoryResponse";
import type { UpdateRepositoryRequest } from "./generated/UpdateRepositoryRequest";
import type { RepositoryRecord } from "./generated/RepositoryRecord";
import type { WhoAmIResponse } from "./generated/WhoAmIResponse";
import type { UserSummary } from "./generated/UserSummary";
import type { GithubAppClientIdResponse } from "./generated/GithubAppClientIdResponse";
import type { GithubTokenResponse } from "./generated/GithubTokenResponse";
import type { ListAgentsResponse } from "./generated/ListAgentsResponse";
import type { ListUsersResponse } from "./generated/ListUsersResponse";
import type { AgentResponse } from "./generated/AgentResponse";
import type { UpsertAgentRequest } from "./generated/UpsertAgentRequest";
import type { DeleteAgentResponse } from "./generated/DeleteAgentResponse";
import type { MergeQueue } from "./generated/MergeQueue";
import type { EnqueueMergePatchRequest } from "./generated/EnqueueMergePatchRequest";
import type { DeleteRepositoryResponse } from "./generated/DeleteRepositoryResponse";
import type { IssueVersionRecord as SubmitFormResponse } from "./generated/IssueVersionRecord";
import type { AddCommentRequest } from "./generated/AddCommentRequest";
import type { AddCommentResponse } from "./generated/AddCommentResponse";
import type { ListCommentsResponse } from "./generated/ListCommentsResponse";
import type { UpsertLabelRequest } from "./generated/UpsertLabelRequest";
import type { UpsertLabelResponse } from "./generated/UpsertLabelResponse";
import type { SearchLabelsQuery } from "./generated/SearchLabelsQuery";
import type { ListLabelsResponse } from "./generated/ListLabelsResponse";
import type { LabelRecord } from "./generated/LabelRecord";
import type { ListSecretsResponse } from "./generated/ListSecretsResponse";
import type { SetSecretRequest } from "./generated/SetSecretRequest";
import type { VersionResponse } from "./generated/VersionResponse";
import type { Conversation } from "./generated/Conversation";
import type { ListConversationsResponse } from "./generated/ListConversationsResponse";
import type { CreateConversationRequest } from "./generated/CreateConversationRequest";
import type { ListProxyTargetsResponse } from "./generated/ListProxyTargetsResponse";
import type { SendMessageRequest } from "./generated/SendMessageRequest";
import type { SearchConversationsQuery } from "./generated/SearchConversationsQuery";
import type { SessionEvent } from "./generated/SessionEvent";
import type { ListRelationsRequest } from "./generated/ListRelationsRequest";
import type { ListRelationsResponse } from "./generated/ListRelationsResponse";
import type { UpsertTriggerRequest } from "./generated/UpsertTriggerRequest";
import type { UpsertTriggerResponse } from "./generated/UpsertTriggerResponse";
import type { TriggerVersionRecord } from "./generated/TriggerVersionRecord";
import type { SearchTriggersQuery } from "./generated/SearchTriggersQuery";
import type { ListTriggersResponse } from "./generated/ListTriggersResponse";
import type { ListTriggerVersionsResponse } from "./generated/ListTriggerVersionsResponse";
import type {
  ProjectRef,
  UpsertProjectRequest,
  UpsertProjectResponse,
  UpsertProjectStatusResponse,
  ProjectRecord,
  ListProjectsResponse,
  ProjectStatusesResponse,
} from "./projects";
import type { StatusDefinition } from "./generated/StatusDefinition";
import type { PatchesThroughputQuery } from "./generated/PatchesThroughputQuery";
import type { PatchesOverTimeResponse } from "./generated/PatchesOverTimeResponse";
import type { PatchesTerminalMixResponse } from "./generated/PatchesTerminalMixResponse";
import type { PatchesTimeToMergeResponse } from "./generated/PatchesTimeToMergeResponse";
import type { PatchesInFlightOverTimeResponse } from "./generated/PatchesInFlightOverTimeResponse";
import type { IssuesThroughputQuery } from "./generated/IssuesThroughputQuery";
import type { IssuesCycleTimeResponse } from "./generated/IssuesCycleTimeResponse";
import type { IssuesTimeInStatusBreakdownResponse } from "./generated/IssuesTimeInStatusBreakdownResponse";
import type { IssuesPerStatusDistributionResponse } from "./generated/IssuesPerStatusDistributionResponse";
import type { IssuesOverTimeResponse } from "./generated/IssuesOverTimeResponse";
import type { TokenUsageOverTimeQuery } from "./generated/TokenUsageOverTimeQuery";
import type { TokenUsageOverTimeResponse } from "./generated/TokenUsageOverTimeResponse";
import type { TokenUsageQuery } from "./generated/TokenUsageQuery";
import type { TokenUsageCostPerAgentResponse } from "./generated/TokenUsageCostPerAgentResponse";
import type { TokenUsageTopIssuesByCostResponse } from "./generated/TokenUsageTopIssuesByCostResponse";
import {
  HydraEventSource,
  buildEventsUrl,
  type EventSubscriptionOptions,
  type HydraEventHandler,
  type HydraEventErrorHandler,
} from "./sse";

export interface HydraApiClientOptions {
  /** Base URL prefix for API requests. Defaults to "/api". */
  baseUrl?: string;
}

/**
 * Serialize an object's non-null/undefined values as URLSearchParams.
 */
function toSearchParams(obj: Record<string, unknown>): URLSearchParams {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(obj)) {
    if (value === undefined || value === null) continue;
    if (Array.isArray(value)) {
      for (const item of value) {
        params.append(key, String(item));
      }
    } else {
      params.set(key, String(value));
    }
  }
  return params;
}

/**
 * Typed API client for the hydra-server, mirroring the Rust HydraClientInterface.
 *
 * All requests go through the BFF proxy (default prefix `/api`) which injects
 * the authentication token from the HttpOnly cookie.
 */
export class HydraApiClient {
  private readonly baseUrl: string;

  constructor(options?: HydraApiClientOptions) {
    this.baseUrl = options?.baseUrl ?? "/api";
  }

  // ---------------------------------------------------------------------------
  // Internal helpers
  // ---------------------------------------------------------------------------

  private async request<T>(
    method: string,
    path: string,
    options?: {
      body?: unknown;
      query?: Record<string, unknown>;
    },
  ): Promise<T> {
    let url = `${this.baseUrl}${path}`;
    if (options?.query) {
      const params = toSearchParams(options.query);
      const qs = params.toString();
      if (qs) url += `?${qs}`;
    }

    const init: RequestInit = { method, credentials: "include" };
    if (options?.body !== undefined) {
      init.headers = { "Content-Type": "application/json" };
      init.body = JSON.stringify(options.body);
    }

    const response = await fetch(url, init);
    if (!response.ok) {
      throw await ApiError.fromResponse(response);
    }
    return (await response.json()) as T;
  }

  private get<T>(path: string, query?: Record<string, unknown>): Promise<T> {
    return this.request<T>("GET", path, { query });
  }

  private post<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>("POST", path, { body });
  }

  private put<T>(path: string, body?: unknown): Promise<T> {
    return this.request<T>("PUT", path, { body });
  }

  private del<T>(path: string): Promise<T> {
    return this.request<T>("DELETE", path);
  }

  // ---------------------------------------------------------------------------
  // Sessions
  // ---------------------------------------------------------------------------

  /** POST /v1/sessions */
  createSession(request: CreateSessionRequest): Promise<CreateSessionResponse> {
    return this.post("/v1/sessions", request);
  }

  /** GET /v1/sessions */
  listSessions(query?: Partial<SearchSessionsQuery>): Promise<ListSessionsResponse> {
    return this.get("/v1/sessions", query as Record<string, unknown>);
  }

  /** GET /v1/sessions/:sessionId */
  getSession(sessionId: string): Promise<SessionVersionRecord> {
    return this.get(`/v1/sessions/${encodeURIComponent(sessionId)}`);
  }

  /** GET /v1/sessions/:sessionId/versions/:version */
  getSessionVersion(sessionId: string, version: number): Promise<SessionVersionRecord> {
    return this.get(
      `/v1/sessions/${encodeURIComponent(sessionId)}/versions/${encodeURIComponent(String(version))}`,
    );
  }

  /** DELETE /v1/sessions/:sessionId */
  killSession(sessionId: string): Promise<KillSessionResponse> {
    return this.del(`/v1/sessions/${encodeURIComponent(sessionId)}`);
  }

  /**
   * GET /v1/sessions/:sessionId/logs
   *
   * Returns the raw Response so callers can read it as text or stream SSE.
   * For streaming logs set `query.watch = true`.
   */
  async getSessionLogs(sessionId: string, query?: Partial<LogsQuery>): Promise<Response> {
    let url = `${this.baseUrl}/v1/sessions/${encodeURIComponent(sessionId)}/logs`;
    if (query) {
      const params = toSearchParams(query as Record<string, unknown>);
      const qs = params.toString();
      if (qs) url += `?${qs}`;
    }
    const response = await fetch(url);
    if (!response.ok) {
      throw await ApiError.fromResponse(response);
    }
    return response;
  }

  /** POST /v1/sessions/:sessionId/status */
  setSessionStatus(
    sessionId: string,
    status: SessionStatusUpdate,
  ): Promise<SetSessionStatusResponse> {
    return this.post(`/v1/sessions/${encodeURIComponent(sessionId)}/status`, status);
  }

  /** GET /v1/sessions/:sessionId/context */
  getSessionContext(sessionId: string): Promise<WorkerContext> {
    return this.get(`/v1/sessions/${encodeURIComponent(sessionId)}/context`);
  }

  /** GET /v1/sessions/:sessionId/versions */
  listSessionVersions(sessionId: string): Promise<ListSessionVersionsResponse> {
    return this.get(`/v1/sessions/${encodeURIComponent(sessionId)}/versions`);
  }

  /**
   * GET /v1/sessions/:sessionId/events
   *
   * Returns the append-only `SessionEvent` log for a single session, in
   * insertion order. Used by the chat read path (designs/sessions-orthogonality-redesign.md §3.4.1)
   * which fan-out fetches per-session event logs and merges them in
   * chronological order across a conversation's resumption chain.
   */
  getSessionEvents(sessionId: string): Promise<SessionEvent[]> {
    return this.get(`/v1/sessions/${encodeURIComponent(sessionId)}/events`);
  }

  /** GET /v1/sessions/:sessionId/proxy-targets */
  listProxyTargets(sessionId: string): Promise<ListProxyTargetsResponse> {
    return this.get(`/v1/sessions/${encodeURIComponent(sessionId)}/proxy-targets`);
  }

  /**
   * POST /v1/sessions/:sessionId/proxy-auth
   *
   * Mints the proxy cookie bound to this session. The cookie is set on the
   * response (`Set-Cookie`); it is HttpOnly so the browser will attach it to
   * subsequent `<port>-<sessionId>.proxy.<host>` requests but JS cannot read
   * it. The server returns 204 with no body.
   */
  async mintSessionProxyAuth(sessionId: string): Promise<void> {
    await this.postNoContent(`/v1/sessions/${encodeURIComponent(sessionId)}/proxy-auth`);
  }

  /**
   * POST /v1/conversations/:conversationId/proxy-auth
   *
   * Same as `mintSessionProxyAuth` but scoped to the conversation's
   * currently-active session. Returns 409 if the conversation has no active
   * session — the UI surfaces "send a message to resume" in that case.
   */
  async mintConversationProxyAuth(conversationId: string): Promise<void> {
    await this.postNoContent(`/v1/conversations/${encodeURIComponent(conversationId)}/proxy-auth`);
  }

  /**
   * POST that expects a 204 (No Content) response — used for endpoints whose
   * effect lives in headers (e.g. `Set-Cookie` on the proxy-mint endpoints).
   * Avoids the `request<T>` helper's `response.json()` call which fails on a
   * truly-empty body.
   */
  private async postNoContent(path: string): Promise<void> {
    const url = `${this.baseUrl}${path}`;
    const response = await fetch(url, {
      method: "POST",
      credentials: "include",
    });
    if (!response.ok) {
      throw await ApiError.fromResponse(response);
    }
  }

  // ---------------------------------------------------------------------------
  // Issues
  // ---------------------------------------------------------------------------

  /** POST /v1/issues */
  createIssue(request: UpsertIssueRequest): Promise<UpsertIssueResponse> {
    return this.post("/v1/issues", request);
  }

  /** PUT /v1/issues/:issueId */
  updateIssue(issueId: string, request: UpsertIssueRequest): Promise<UpsertIssueResponse> {
    return this.put(`/v1/issues/${encodeURIComponent(issueId)}`, request);
  }

  /** GET /v1/issues/:issueId */
  getIssue(issueId: string, includeDeleted?: boolean): Promise<IssueVersionRecord> {
    const query = includeDeleted ? { include_deleted: "true" } : undefined;
    return this.get(`/v1/issues/${encodeURIComponent(issueId)}`, query);
  }

  /** GET /v1/issues/:issueId/versions/:version */
  getIssueVersion(issueId: string, version: number): Promise<IssueVersionRecord> {
    return this.get(
      `/v1/issues/${encodeURIComponent(issueId)}/versions/${encodeURIComponent(String(version))}`,
    );
  }

  /** GET /v1/issues */
  listIssues(query?: Partial<SearchIssuesQuery>): Promise<ListIssuesResponse> {
    return this.get("/v1/issues", query as Record<string, unknown>);
  }

  /** GET /v1/issues/:issueId/versions */
  listIssueVersions(issueId: string): Promise<ListIssueVersionsResponse> {
    return this.get(`/v1/issues/${encodeURIComponent(issueId)}/versions`);
  }

  /** POST /v1/issues/:issueId/actions */
  submitForm(
    issueId: string,
    actionId: string,
    values: Record<string, unknown>,
  ): Promise<SubmitFormResponse> {
    return this.post(`/v1/issues/${encodeURIComponent(issueId)}/actions`, {
      action_id: actionId,
      values,
    });
  }

  /** DELETE /v1/issues/:issueId */
  deleteIssue(issueId: string): Promise<IssueVersionRecord> {
    return this.del(`/v1/issues/${encodeURIComponent(issueId)}`);
  }

  /** POST /v1/issues/:issueId/feedback */
  submitFeedback(issueId: string, feedback: string): Promise<IssueVersionRecord> {
    return this.post(`/v1/issues/${encodeURIComponent(issueId)}/feedback`, { feedback });
  }

  /** GET /v1/issues/:issueId/comments — list comments most-recent-first. */
  listIssueComments(
    issueId: string,
    opts?: { limit?: number; beforeSequence?: bigint | number },
  ): Promise<ListCommentsResponse> {
    const query: Record<string, unknown> = {};
    if (opts?.limit !== undefined) query.limit = opts.limit;
    if (opts?.beforeSequence !== undefined) {
      query.before_sequence = String(opts.beforeSequence);
    }
    return this.get(`/v1/issues/${encodeURIComponent(issueId)}/comments`, query);
  }

  /** POST /v1/issues/:issueId/comments — add a new comment. */
  addIssueComment(
    issueId: string,
    body: AddCommentRequest,
  ): Promise<AddCommentResponse> {
    return this.post(`/v1/issues/${encodeURIComponent(issueId)}/comments`, body);
  }

  // ---------------------------------------------------------------------------
  // Patches
  // ---------------------------------------------------------------------------

  /** POST /v1/patches */
  createPatch(request: UpsertPatchRequest): Promise<UpsertPatchResponse> {
    return this.post("/v1/patches", request);
  }

  /** PUT /v1/patches/:patchId */
  updatePatch(patchId: string, request: UpsertPatchRequest): Promise<UpsertPatchResponse> {
    return this.put(`/v1/patches/${encodeURIComponent(patchId)}`, request);
  }

  /** GET /v1/patches/:patchId */
  getPatch(patchId: string): Promise<PatchVersionRecord> {
    return this.get(`/v1/patches/${encodeURIComponent(patchId)}`);
  }

  /** GET /v1/patches/:patchId/versions/:version */
  getPatchVersion(patchId: string, version: number): Promise<PatchVersionRecord> {
    return this.get(
      `/v1/patches/${encodeURIComponent(patchId)}/versions/${encodeURIComponent(String(version))}`,
    );
  }

  /** GET /v1/patches */
  listPatches(query?: Partial<SearchPatchesQuery>): Promise<ListPatchesResponse> {
    return this.get("/v1/patches", query as Record<string, unknown>);
  }

  /** GET /v1/patches/:patchId/versions */
  listPatchVersions(patchId: string): Promise<ListPatchVersionsResponse> {
    return this.get(`/v1/patches/${encodeURIComponent(patchId)}/versions`);
  }

  /** DELETE /v1/patches/:patchId */
  deletePatch(patchId: string): Promise<PatchVersionRecord> {
    return this.del(`/v1/patches/${encodeURIComponent(patchId)}`);
  }

  // ---------------------------------------------------------------------------
  // Documents
  // ---------------------------------------------------------------------------

  /** POST /v1/documents */
  createDocument(request: UpsertDocumentRequest): Promise<UpsertDocumentResponse> {
    return this.post("/v1/documents", request);
  }

  /** PUT /v1/documents/:documentId */
  updateDocument(
    documentId: string,
    request: UpsertDocumentRequest,
  ): Promise<UpsertDocumentResponse> {
    return this.put(`/v1/documents/${encodeURIComponent(documentId)}`, request);
  }

  /** GET /v1/documents/:documentId */
  getDocument(documentId: string, includeDeleted?: boolean): Promise<DocumentVersionRecord> {
    const query = includeDeleted ? { include_deleted: "true" } : undefined;
    return this.get(`/v1/documents/${encodeURIComponent(documentId)}`, query);
  }

  /**
   * Fetch a document by its exact path using the list endpoint with path_is_exact=true,
   * then fetches the full record via the detail endpoint.
   */
  async getDocumentByPath(path: string, includeDeleted?: boolean): Promise<DocumentVersionRecord> {
    const query: Partial<SearchDocumentsQuery> = {
      q: null,
      path_prefix: path,
      path_is_exact: true,
      include_deleted: includeDeleted ?? null,
    };
    const response = await this.listDocuments(query);
    const summary = response.documents[0];
    if (!summary) {
      throw new ApiError(404, `document with path '${path}' not found`);
    }
    return this.getDocument(summary.document_id, includeDeleted);
  }

  /** GET /v1/documents */
  listDocuments(query?: Partial<SearchDocumentsQuery>): Promise<ListDocumentsResponse> {
    return this.get("/v1/documents", query as Record<string, unknown>);
  }

  /** GET /v1/documents/paths */
  listDocumentPaths(query?: Partial<ListDocumentPathsQuery>): Promise<ListDocumentPathsResponse> {
    return this.get("/v1/documents/paths", query as Record<string, unknown>);
  }

  /** GET /v1/documents/:documentId/versions */
  listDocumentVersions(documentId: string): Promise<ListDocumentVersionsResponse> {
    return this.get(`/v1/documents/${encodeURIComponent(documentId)}/versions`);
  }

  /** GET /v1/documents/:documentId/versions/:version */
  getDocumentVersion(documentId: string, version: number): Promise<DocumentVersionRecord> {
    return this.get(
      `/v1/documents/${encodeURIComponent(documentId)}/versions/${encodeURIComponent(String(version))}`,
    );
  }

  /** DELETE /v1/documents/:documentId */
  deleteDocument(documentId: string): Promise<DocumentVersionRecord> {
    return this.del(`/v1/documents/${encodeURIComponent(documentId)}`);
  }

  // ---------------------------------------------------------------------------
  // Repositories
  // ---------------------------------------------------------------------------

  /** GET /v1/repositories */
  listRepositories(query?: Partial<SearchRepositoriesQuery>): Promise<ListRepositoriesResponse> {
    return this.get("/v1/repositories", query as Record<string, unknown>);
  }

  /** POST /v1/repositories */
  createRepository(request: CreateRepositoryRequest): Promise<UpsertRepositoryResponse> {
    return this.post("/v1/repositories", request);
  }

  /**
   * PUT /v1/repositories/:organization/:repo
   * @param repoName — Full repo name in "org/repo" format.
   */
  updateRepository(
    repoName: string,
    request: UpdateRepositoryRequest,
  ): Promise<UpsertRepositoryResponse> {
    return this.put(`/v1/repositories/${repoName}`, request);
  }

  /**
   * DELETE /v1/repositories/:organization/:repo
   * @param repoName — Full repo name in "org/repo" format.
   */
  async deleteRepository(repoName: string): Promise<RepositoryRecord> {
    const resp = await this.del<DeleteRepositoryResponse>(`/v1/repositories/${repoName}`);
    return resp.repository;
  }

  // ---------------------------------------------------------------------------
  // Auth / Users
  // ---------------------------------------------------------------------------

  /** GET /v1/whoami */
  whoami(): Promise<WhoAmIResponse> {
    return this.get("/v1/whoami");
  }

  /** GET /v1/users */
  listUsers(): Promise<ListUsersResponse> {
    return this.get("/v1/users");
  }

  /** GET /v1/users/:username */
  getUserInfo(username: string): Promise<UserSummary> {
    return this.get(`/v1/users/${encodeURIComponent(username)}`);
  }

  /** GET /v1/github/token */
  async getGithubToken(): Promise<string> {
    const resp = await this.get<GithubTokenResponse>("/v1/github/token");
    return resp.github_token;
  }

  /** GET /v1/github/app/client-id */
  getGithubAppClientId(): Promise<GithubAppClientIdResponse> {
    return this.get("/v1/github/app/client-id");
  }

  // ---------------------------------------------------------------------------
  // Agents
  // ---------------------------------------------------------------------------

  /** GET /v1/agents */
  listAgents(): Promise<ListAgentsResponse> {
    return this.get("/v1/agents");
  }

  /** GET /v1/agents/:name */
  getAgent(name: string): Promise<AgentResponse> {
    return this.get(`/v1/agents/${encodeURIComponent(name)}`);
  }

  /** POST /v1/agents */
  createAgent(request: UpsertAgentRequest): Promise<AgentResponse> {
    return this.post("/v1/agents", request);
  }

  /** PUT /v1/agents/:name */
  updateAgent(name: string, request: UpsertAgentRequest): Promise<AgentResponse> {
    return this.put(`/v1/agents/${encodeURIComponent(name)}`, request);
  }

  /** DELETE /v1/agents/:name */
  deleteAgent(name: string): Promise<DeleteAgentResponse> {
    return this.del(`/v1/agents/${encodeURIComponent(name)}`);
  }

  // ---------------------------------------------------------------------------
  // Merge Queues
  // ---------------------------------------------------------------------------

  /**
   * GET /v1/merge-queues/:organization/:repo/:branch/patches
   * @param repoName — Full repo name in "org/repo" format.
   */
  getMergeQueue(repoName: string, branch: string): Promise<MergeQueue> {
    return this.get(`/v1/merge-queues/${repoName}/${encodeURIComponent(branch)}/patches`);
  }

  /**
   * POST /v1/merge-queues/:organization/:repo/:branch/patches
   * @param repoName — Full repo name in "org/repo" format.
   */
  enqueueMergePatch(repoName: string, branch: string, patchId: string): Promise<MergeQueue> {
    const body: EnqueueMergePatchRequest = { patch_id: patchId };
    return this.post(`/v1/merge-queues/${repoName}/${encodeURIComponent(branch)}/patches`, body);
  }

  // ---------------------------------------------------------------------------
  // Labels
  // ---------------------------------------------------------------------------

  /** POST /v1/labels */
  createLabel(request: UpsertLabelRequest): Promise<UpsertLabelResponse> {
    return this.post("/v1/labels", request);
  }

  /** GET /v1/labels */
  listLabels(query?: Partial<SearchLabelsQuery>): Promise<ListLabelsResponse> {
    return this.get("/v1/labels", query as Record<string, unknown>);
  }

  /** GET /v1/labels/:labelId */
  getLabel(labelId: string): Promise<LabelRecord> {
    return this.get(`/v1/labels/${encodeURIComponent(labelId)}`);
  }

  /** PUT /v1/labels/:labelId/objects/:objectId */
  addLabelToObject(labelId: string, objectId: string, cascade?: boolean): Promise<void> {
    const query = cascade ? { cascade: "true" } : undefined;
    return this.request(
      "PUT",
      `/v1/labels/${encodeURIComponent(labelId)}/objects/${encodeURIComponent(objectId)}`,
      { query },
    );
  }

  /** DELETE /v1/labels/:labelId/objects/:objectId */
  removeLabelFromObject(labelId: string, objectId: string): Promise<void> {
    return this.del(
      `/v1/labels/${encodeURIComponent(labelId)}/objects/${encodeURIComponent(objectId)}`,
    );
  }

  // ---------------------------------------------------------------------------
  // Secrets
  // ---------------------------------------------------------------------------

  /** GET /v1/users/:username/secrets */
  listSecrets(username: string): Promise<ListSecretsResponse> {
    return this.get(`/v1/users/${encodeURIComponent(username)}/secrets`);
  }

  /** PUT /v1/users/:username/secrets/:name */
  setSecret(username: string, name: string, value: string): Promise<void> {
    const body: SetSecretRequest = { value };
    return this.put(
      `/v1/users/${encodeURIComponent(username)}/secrets/${encodeURIComponent(name)}`,
      body,
    );
  }

  /** DELETE /v1/users/:username/secrets/:name */
  deleteSecret(username: string, name: string): Promise<void> {
    return this.del(
      `/v1/users/${encodeURIComponent(username)}/secrets/${encodeURIComponent(name)}`,
    );
  }

  // ---------------------------------------------------------------------------
  // Version
  // ---------------------------------------------------------------------------

  /** GET /v1/version */
  getVersion(): Promise<VersionResponse> {
    return this.get("/v1/version");
  }

  // ---------------------------------------------------------------------------
  // Relations
  // ---------------------------------------------------------------------------

  /** GET /v1/relations */
  listRelations(query: ListRelationsRequest): Promise<ListRelationsResponse> {
    return this.get("/v1/relations", query as Record<string, unknown>);
  }

  // ---------------------------------------------------------------------------
  // Triggers
  // ---------------------------------------------------------------------------

  /** POST /v1/triggers */
  createTrigger(request: UpsertTriggerRequest): Promise<UpsertTriggerResponse> {
    return this.post("/v1/triggers", request);
  }

  /** PUT /v1/triggers/:triggerId */
  updateTrigger(triggerId: string, request: UpsertTriggerRequest): Promise<UpsertTriggerResponse> {
    return this.put(`/v1/triggers/${encodeURIComponent(triggerId)}`, request);
  }

  /** GET /v1/triggers/:triggerId */
  getTrigger(triggerId: string): Promise<TriggerVersionRecord> {
    return this.get(`/v1/triggers/${encodeURIComponent(triggerId)}`);
  }

  /** GET /v1/triggers */
  listTriggers(query?: Partial<SearchTriggersQuery>): Promise<ListTriggersResponse> {
    return this.get("/v1/triggers", query as Record<string, unknown>);
  }

  /** GET /v1/triggers/:triggerId/versions */
  listTriggerVersions(triggerId: string): Promise<ListTriggerVersionsResponse> {
    return this.get(`/v1/triggers/${encodeURIComponent(triggerId)}/versions`);
  }

  /** DELETE /v1/triggers/:triggerId */
  deleteTrigger(triggerId: string): Promise<TriggerVersionRecord> {
    return this.del(`/v1/triggers/${encodeURIComponent(triggerId)}`);
  }

  // ---------------------------------------------------------------------------
  // Conversations
  // ---------------------------------------------------------------------------

  /** GET /v1/conversations */
  listConversations(
    query?: Partial<SearchConversationsQuery>,
  ): Promise<ListConversationsResponse> {
    return this.get("/v1/conversations", query as Record<string, unknown>);
  }

  /** GET /v1/conversations/:conversationId */
  getConversation(conversationId: string): Promise<Conversation> {
    return this.get(`/v1/conversations/${encodeURIComponent(conversationId)}`);
  }

  /** POST /v1/conversations */
  createConversation(request: CreateConversationRequest): Promise<Conversation> {
    return this.post("/v1/conversations", request);
  }

  /** POST /v1/conversations/:conversationId/messages */
  sendMessage(conversationId: string, request: SendMessageRequest): Promise<void> {
    return this.post(`/v1/conversations/${encodeURIComponent(conversationId)}/messages`, request);
  }

  /** POST /v1/conversations/:conversationId/close */
  closeConversation(conversationId: string): Promise<void> {
    return this.post(`/v1/conversations/${encodeURIComponent(conversationId)}/close`);
  }

  /** POST /v1/conversations/:conversationId/resume */
  resumeConversation(conversationId: string): Promise<Conversation> {
    return this.post(`/v1/conversations/${encodeURIComponent(conversationId)}/resume`);
  }

  // ---------------------------------------------------------------------------
  // Projects
  // ---------------------------------------------------------------------------

  /** POST /v1/projects */
  createProject(request: UpsertProjectRequest): Promise<UpsertProjectResponse> {
    return this.post("/v1/projects", request);
  }

  /** GET /v1/projects */
  listProjects(): Promise<ListProjectsResponse> {
    return this.get("/v1/projects");
  }

  /** GET /v1/projects/:projectRef — accepts either an id (`j-…`) or key. */
  getProject(projectRef: ProjectRef): Promise<ProjectRecord> {
    return this.get(`/v1/projects/${encodeURIComponent(projectRef)}`);
  }

  /** PUT /v1/projects/:projectRef — accepts either an id (`j-…`) or key. */
  updateProject(
    projectRef: ProjectRef,
    request: UpsertProjectRequest,
  ): Promise<UpsertProjectResponse> {
    return this.put(`/v1/projects/${encodeURIComponent(projectRef)}`, request);
  }

  /**
   * POST /v1/projects/:projectRef/archive — archive a project, cascading
   * to every non-archived issue it owns. Idempotent.
   */
  archiveProject(projectRef: ProjectRef): Promise<UpsertProjectResponse> {
    return this.post(`/v1/projects/${encodeURIComponent(projectRef)}/archive`);
  }

  /**
   * POST /v1/projects/:projectRef/unarchive — unarchive a project.
   * No reverse cascade.
   */
  unarchiveProject(projectRef: ProjectRef): Promise<UpsertProjectResponse> {
    return this.post(`/v1/projects/${encodeURIComponent(projectRef)}/unarchive`);
  }

  /** GET /v1/projects/:projectRef/statuses — accepts either an id (`j-…`) or key. */
  getProjectStatuses(projectRef: ProjectRef): Promise<ProjectStatusesResponse> {
    return this.get(`/v1/projects/${encodeURIComponent(projectRef)}/statuses`);
  }

  /** POST /v1/projects/:projectRef/statuses — add a new status. */
  createProjectStatus(
    projectRef: ProjectRef,
    status: StatusDefinition,
  ): Promise<UpsertProjectStatusResponse> {
    return this.post(`/v1/projects/${encodeURIComponent(projectRef)}/statuses`, status);
  }

  /**
   * PUT /v1/projects/:projectRef/statuses/:statusKey — update or
   * rename an existing status. A body whose `key` differs from
   * `statusKey` is a rename in place.
   */
  updateProjectStatus(
    projectRef: ProjectRef,
    statusKey: string,
    status: StatusDefinition,
  ): Promise<UpsertProjectStatusResponse> {
    return this.put(
      `/v1/projects/${encodeURIComponent(projectRef)}/statuses/${encodeURIComponent(statusKey)}`,
      status,
    );
  }

  /**
   * POST /v1/projects/:projectRef/statuses/:statusKey/archive —
   * archive a status (flip `archived = true` in place) and
   * cascade-archive every non-archived issue at that status.
   * Idempotent.
   */
  archiveProjectStatus(
    projectRef: ProjectRef,
    statusKey: string,
  ): Promise<UpsertProjectResponse> {
    return this.post(
      `/v1/projects/${encodeURIComponent(projectRef)}/statuses/${encodeURIComponent(statusKey)}/archive`,
    );
  }

  /**
   * POST /v1/projects/:projectRef/statuses/:statusKey/unarchive —
   * unarchive a status. No reverse cascade.
   */
  unarchiveProjectStatus(
    projectRef: ProjectRef,
    statusKey: string,
  ): Promise<UpsertProjectResponse> {
    return this.post(
      `/v1/projects/${encodeURIComponent(projectRef)}/statuses/${encodeURIComponent(statusKey)}/unarchive`,
    );
  }

  // ---------------------------------------------------------------------------
  // Analytics — Throughput
  // ---------------------------------------------------------------------------

  /** GET /v1/analytics/throughput/patches/over_time */
  getPatchesThroughputOverTime(query: PatchesThroughputQuery): Promise<PatchesOverTimeResponse> {
    return this.get(
      "/v1/analytics/throughput/patches/over_time",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/patches/terminal_mix */
  getPatchesThroughputTerminalMix(
    query: PatchesThroughputQuery,
  ): Promise<PatchesTerminalMixResponse> {
    return this.get(
      "/v1/analytics/throughput/patches/terminal_mix",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/patches/time_to_merge */
  getPatchesThroughputTimeToMerge(
    query: PatchesThroughputQuery,
  ): Promise<PatchesTimeToMergeResponse> {
    return this.get(
      "/v1/analytics/throughput/patches/time_to_merge",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/patches/in_flight_over_time */
  getPatchesThroughputInFlightOverTime(
    query: PatchesThroughputQuery,
  ): Promise<PatchesInFlightOverTimeResponse> {
    return this.get(
      "/v1/analytics/throughput/patches/in_flight_over_time",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/issues/cycle_time */
  getIssuesThroughputCycleTime(query: IssuesThroughputQuery): Promise<IssuesCycleTimeResponse> {
    return this.get(
      "/v1/analytics/throughput/issues/cycle_time",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/issues/time_in_status_breakdown */
  getIssuesThroughputTimeInStatusBreakdown(
    query: IssuesThroughputQuery,
  ): Promise<IssuesTimeInStatusBreakdownResponse> {
    return this.get(
      "/v1/analytics/throughput/issues/time_in_status_breakdown",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/issues/per_status_distribution */
  getIssuesThroughputPerStatusDistribution(
    query: IssuesThroughputQuery,
  ): Promise<IssuesPerStatusDistributionResponse> {
    return this.get(
      "/v1/analytics/throughput/issues/per_status_distribution",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/throughput/issues/over_time */
  getIssuesThroughputOverTime(query: IssuesThroughputQuery): Promise<IssuesOverTimeResponse> {
    return this.get(
      "/v1/analytics/throughput/issues/over_time",
      query as unknown as Record<string, unknown>,
    );
  }

  // ---------------------------------------------------------------------------
  // Analytics — Token usage
  // ---------------------------------------------------------------------------

  /** GET /v1/analytics/token_usage/over_time */
  getTokenUsageOverTime(query: TokenUsageOverTimeQuery): Promise<TokenUsageOverTimeResponse> {
    return this.get(
      "/v1/analytics/token_usage/over_time",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/token_usage/cost_per_agent */
  getTokenUsageCostPerAgent(
    query: TokenUsageQuery,
  ): Promise<TokenUsageCostPerAgentResponse> {
    return this.get(
      "/v1/analytics/token_usage/cost_per_agent",
      query as unknown as Record<string, unknown>,
    );
  }

  /** GET /v1/analytics/token_usage/top_issues_by_cost */
  getTokenUsageTopIssuesByCost(
    query: TokenUsageQuery,
  ): Promise<TokenUsageTopIssuesByCostResponse> {
    return this.get(
      "/v1/analytics/token_usage/top_issues_by_cost",
      query as unknown as Record<string, unknown>,
    );
  }

  // ---------------------------------------------------------------------------
  // Events (SSE)
  // ---------------------------------------------------------------------------

  /**
   * Open an SSE connection to GET /v1/events.
   * Returns a HydraEventSource that can be closed with `.close()`.
   */
  subscribeEvents(
    onEvent: HydraEventHandler,
    options?: EventSubscriptionOptions,
    onError?: HydraEventErrorHandler,
  ): HydraEventSource {
    const url = buildEventsUrl(this.baseUrl, options);
    return new HydraEventSource(url, onEvent, onError);
  }
}
