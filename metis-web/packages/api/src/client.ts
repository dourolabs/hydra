import { ApiError } from "./errors";
import type { HydraId } from "./generated/HydraId";
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
import type { AddTodoItemRequest } from "./generated/AddTodoItemRequest";
import type { ReplaceTodoListRequest } from "./generated/ReplaceTodoListRequest";
import type { SetTodoItemStatusRequest } from "./generated/SetTodoItemStatusRequest";
import type { TodoListResponse } from "./generated/TodoListResponse";
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
import type { SearchRepositoriesQuery } from "./generated/SearchRepositoriesQuery";
import type { ListRepositoriesResponse } from "./generated/ListRepositoriesResponse";
import type { CreateRepositoryRequest } from "./generated/CreateRepositoryRequest";
import type { UpsertRepositoryResponse } from "./generated/UpsertRepositoryResponse";
import type { UpdateRepositoryRequest } from "./generated/UpdateRepositoryRequest";
import type { RepositoryRecord } from "./generated/RepositoryRecord";
import type { WhoAmIResponse } from "./generated/WhoAmIResponse";
import type { UserSummary } from "./generated/UserSummary";
import type { GithubTokenResponse } from "./generated/GithubTokenResponse";
import type { ListAgentsResponse } from "./generated/ListAgentsResponse";
import type { AgentResponse } from "./generated/AgentResponse";
import type { UpsertAgentRequest } from "./generated/UpsertAgentRequest";
import type { DeleteAgentResponse } from "./generated/DeleteAgentResponse";
import type { MergeQueue } from "./generated/MergeQueue";
import type { EnqueueMergePatchRequest } from "./generated/EnqueueMergePatchRequest";
import type { DeleteRepositoryResponse } from "./generated/DeleteRepositoryResponse";
import type { UpsertLabelRequest } from "./generated/UpsertLabelRequest";
import type { UpsertLabelResponse } from "./generated/UpsertLabelResponse";
import type { SearchLabelsQuery } from "./generated/SearchLabelsQuery";
import type { ListLabelsResponse } from "./generated/ListLabelsResponse";
import type { LabelRecord } from "./generated/LabelRecord";
import type { ListSecretsResponse } from "./generated/ListSecretsResponse";
import type { SetSecretRequest } from "./generated/SetSecretRequest";
import type { VersionResponse } from "./generated/VersionResponse";
import {
  HydraEventSource,
  buildEventsUrl,
  type EventSubscriptionOptions,
  type HydraEventHandler,
  type HydraEventErrorHandler,
} from "./sse";

// ---------------------------------------------------------------------------
// Relations types (not yet in generated/ — defined inline)
// ---------------------------------------------------------------------------

export interface RelationResponse {
  source_id: HydraId;
  target_id: HydraId;
  rel_type: string;
}

export interface ListRelationsRequest {
  source_id?: HydraId;
  source_ids?: string;
  target_id?: HydraId;
  target_ids?: string;
  object_id?: HydraId;
  rel_type?: string;
  transitive?: boolean;
}

export interface ListRelationsResponse {
  relations: RelationResponse[];
}

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
  setSessionStatus(sessionId: string, status: SessionStatusUpdate): Promise<SetSessionStatusResponse> {
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

  /** DELETE /v1/issues/:issueId */
  deleteIssue(issueId: string): Promise<IssueVersionRecord> {
    return this.del(`/v1/issues/${encodeURIComponent(issueId)}`);
  }

  /** POST /v1/issues/:issueId/todo-items */
  addTodoItem(issueId: string, request: AddTodoItemRequest): Promise<TodoListResponse> {
    return this.post(`/v1/issues/${encodeURIComponent(issueId)}/todo-items`, request);
  }

  /** PUT /v1/issues/:issueId/todo-items */
  replaceTodoList(issueId: string, request: ReplaceTodoListRequest): Promise<TodoListResponse> {
    return this.put(`/v1/issues/${encodeURIComponent(issueId)}/todo-items`, request);
  }

  /** POST /v1/issues/:issueId/todo-items/:index */
  setTodoItemStatus(
    issueId: string,
    index: number,
    request: SetTodoItemStatusRequest,
  ): Promise<TodoListResponse> {
    return this.post(
      `/v1/issues/${encodeURIComponent(issueId)}/todo-items/${encodeURIComponent(String(index))}`,
      request,
    );
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
  async getDocumentByPath(
    path: string,
    includeDeleted?: boolean,
  ): Promise<DocumentVersionRecord> {
    const query: Partial<SearchDocumentsQuery> = {
      q: null,
      path_prefix: path,
      path_is_exact: true,
      created_by: null,
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

  /** GET /v1/users/:username */
  getUserInfo(username: string): Promise<UserSummary> {
    return this.get(`/v1/users/${encodeURIComponent(username)}`);
  }

  /** GET /v1/github/token */
  async getGithubToken(): Promise<string> {
    const resp = await this.get<GithubTokenResponse>("/v1/github/token");
    return resp.github_token;
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
    return this.get(
      `/v1/merge-queues/${repoName}/${encodeURIComponent(branch)}/patches`,
    );
  }

  /**
   * POST /v1/merge-queues/:organization/:repo/:branch/patches
   * @param repoName — Full repo name in "org/repo" format.
   */
  enqueueMergePatch(repoName: string, branch: string, patchId: string): Promise<MergeQueue> {
    const body: EnqueueMergePatchRequest = { patch_id: patchId };
    return this.post(
      `/v1/merge-queues/${repoName}/${encodeURIComponent(branch)}/patches`,
      body,
    );
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
    return this.request("PUT", `/v1/labels/${encodeURIComponent(labelId)}/objects/${encodeURIComponent(objectId)}`, { query });
  }

  /** DELETE /v1/labels/:labelId/objects/:objectId */
  removeLabelFromObject(labelId: string, objectId: string): Promise<void> {
    return this.del(`/v1/labels/${encodeURIComponent(labelId)}/objects/${encodeURIComponent(objectId)}`);
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
    return this.put(`/v1/users/${encodeURIComponent(username)}/secrets/${encodeURIComponent(name)}`, body);
  }

  /** DELETE /v1/users/:username/secrets/:name */
  deleteSecret(username: string, name: string): Promise<void> {
    return this.del(`/v1/users/${encodeURIComponent(username)}/secrets/${encodeURIComponent(name)}`);
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
