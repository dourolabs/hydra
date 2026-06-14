import { Hono } from "hono";
import type { Store, VersionedEntity } from "../store.js";
import { generateId } from "../id.js";
import type {
  Issue,
  IssueInput,
  UpsertIssueRequest,
  UpsertIssueResponse,
  IssueVersionRecord,
  ListIssuesResponse,
  ListIssueVersionsResponse,
  IssueSummaryRecord,
  IssueSummary,
  Comment,
  AddCommentRequest,
  AddCommentResponse,
  ListCommentsResponse,
  ActorRef,
  Project,
} from "@hydra/api";
import { getLabelsForObject, resolveLabelNames } from "./labels.js";
import { resolveStatusDef } from "../statusResolver.js";

function inputToIssue(store: Store, input: IssueInput): Issue {
  const { status, ...rest } = input;
  return {
    ...rest,
    status: resolveStatusDef(store, input.project_id, status),
    dependencies: input.dependencies ?? [],
    patches: input.patches ?? [],
  };
}

const COLLECTION = "issues";
const SSE_PREFIX = "issue";

// In-memory per-issue comments. Append-only, sequence starts at 1 per issue.
const commentsByIssue: Map<string, Comment[]> = new Map();

export function clearIssueComments(): void {
  commentsByIssue.clear();
}

export function seedIssueComment(
  issueId: string,
  body: string,
  actor: ActorRef,
  createdAt: string,
): void {
  const list = commentsByIssue.get(issueId) ?? [];
  const next = list.length > 0 ? Number(list[list.length - 1].sequence) + 1 : 1;
  list.push({
    issue_id: issueId,
    sequence: BigInt(next),
    body,
    actor,
    created_at: createdAt,
  });
  commentsByIssue.set(issueId, list);
}

/**
 * Append a Comment to the per-issue log and emit a `comment_created` SSE
 * event on the global `/v1/events` stream. Mirrors `appendSessionEvent`
 * for session events and matches the wire shape produced by
 * `hydra-server/src/app/event_bus.rs::add_comment` — `entity_type =
 * "comment"`, `entity_id = issueId`, `entity = comment`, and `version`
 * = the new comment's 1-based per-issue sequence.
 */
export function appendIssueComment(
  store: Store,
  issueId: string,
  body: string,
  actor: ActorRef,
): Comment {
  const list = commentsByIssue.get(issueId) ?? [];
  const nextSeq = list.length > 0 ? Number(list[list.length - 1].sequence) + 1 : 1;
  const comment: Comment = {
    issue_id: issueId,
    sequence: BigInt(nextSeq),
    body,
    actor,
    created_at: new Date().toISOString(),
  };
  list.push(comment);
  commentsByIssue.set(issueId, list);
  store.emitSideEvent(
    "comment_created",
    "comment",
    issueId,
    nextSeq,
    comment.created_at,
    comment,
  );
  return comment;
}

export function getIssueCommentsFor(issueId: string): Comment[] {
  return commentsByIssue.get(issueId) ?? [];
}

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
    project_id: issue.project_id,
    assignee: issue.assignee,
    progress: (issue.progress ?? "").slice(0, 200),
    dependencies: issue.dependencies,
    patches: issue.patches,
    archived: issue.archived,
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

export function createIssueRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/issues
  app.post("/v1/issues", async (c) => {
    const body = await c.req.json<UpsertIssueRequest>();
    const id = generateId("issue");
    const issue: Issue = inputToIssue(store, body.issue);
    const entry = store.create<Issue>(COLLECTION, id, issue, SSE_PREFIX);

    // Resolve label_names: create missing labels and associate them with the issue
    if (body.label_names && body.label_names.length > 0) {
      resolveLabelNames(store, body.label_names, id);
    }

    const resp: UpsertIssueResponse = {
      issue_id: id,
      version: BigInt(entry.version),
      issue,
    };
    return c.json(resp, 201);
  });

  // PUT /v1/issues/:id
  app.put("/v1/issues/:id", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<UpsertIssueRequest>();
    const issue: Issue = inputToIssue(store, body.issue);
    const entry = store.update<Issue>(COLLECTION, id, issue, SSE_PREFIX);
    const resp: UpsertIssueResponse = {
      issue_id: id,
      version: BigInt(entry.version),
      issue,
    };
    return c.json(resp);
  });

  // GET /v1/issues/:id
  app.get("/v1/issues/:id", (c) => {
    const id = c.req.param("id");
    const includeDeleted = c.req.query("include_archived") === "true";
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
    const includeDeleted = c.req.query("include_archived") === "true";
    const ids = c.req.query("ids");
    const issueType = c.req.query("issue_type");
    const status = c.req.query("status");
    const projectId = c.req.query("project_id");
    const assignee = c.req.query("assignee");
    const creator = c.req.query("creator");
    const q = c.req.query("q");
    const labelsParam = c.req.query("labels");
    const limitParam = c.req.query("limit");
    const cursorParam = c.req.query("cursor");
    const countParam = c.req.query("count");
    const sortParam = c.req.query("sort");
    const bucketByParam = c.req.query("bucket_by");
    const bucketLimitParam = c.req.query("bucket_limit");

    // Validate bucket_by + bucket_limit + cursor combinations, matching the
    // real backend (see PR-3 [[i-aiunhiwa]]'s route-handler validation).
    if (bucketByParam !== undefined) {
      if (bucketByParam !== "project_status") {
        return c.json(
          { error: `unsupported bucket_by value: '${bucketByParam}'` },
          400,
        );
      }
      if (cursorParam) {
        return c.json(
          { error: "bucket_by is incompatible with cursor" },
          400,
        );
      }
      const bucketLimitNum = Number(bucketLimitParam);
      if (
        bucketLimitParam === undefined ||
        Number.isNaN(bucketLimitNum) ||
        bucketLimitNum <= 0
      ) {
        return c.json(
          { error: "bucket_by requires bucket_limit > 0" },
          400,
        );
      }
    }

    const items = store.list<Issue>(COLLECTION, includeDeleted);

    let filtered = items;
    if (ids) {
      // Parity with prod: `SearchIssuesQuery::ids` is `Vec<IssueId>`, and
      // `IssueId::try_from` (`hydra-common/src/ids.rs`) requires the `i-`
      // prefix. A single bad element rejects the whole query as 400. The
      // mock used to accept any string here, which masked client bugs where
      // mixed-prefix relation target_ids leaked into the CSV.
      const parsed = ids.split(",").map((s) => s.trim());
      const invalid = parsed.filter((s) => !s.startsWith("i-"));
      if (invalid.length > 0) {
        return c.json(
          {
            error: `invalid issue id(s) in 'ids' parameter: ${invalid.join(", ")}`,
          },
          400,
        );
      }
      const idSet = new Set(parsed);
      filtered = filtered.filter(({ id }) => idSet.has(id));
    }
    if (issueType) {
      filtered = filtered.filter(({ entry }) => entry.data.type === issueType);
    }
    if (status) {
      // Parity with the backend `?status=` filter (see [[p-urywauam]]):
      // CSV of `StatusKey` strings, OR-matched. Any string is a valid
      // StatusKey (per-project keys like `inbox`/`triage`, not just the
      // five legacy enum values). Trim entries and drop empties; if all
      // entries are empty after trimming, treat as no status filter.
      const statuses = status
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      if (statuses.length > 0) {
        filtered = filtered.filter(({ entry }) => statuses.includes(entry.data.status.key));
      }
    }
    if (projectId) {
      filtered = filtered.filter(({ entry }) => entry.data.project_id === projectId);
    }
    if (assignee) {
      // Phase 4b: query param `assignee` is the canonical path form
      // (`users/<x>` / `agents/<x>` / `external/<sys>/<x>`). The mock
      // store still keeps a typed `Principal` on each issue, so
      // compare against the path-form encoding here.
      filtered = filtered.filter(({ entry }) => {
        const p = entry.data.assignee;
        if (!p) return false;
        const path =
          "User" in p
            ? `users/${p.User.name}`
            : "Agent" in p
              ? `agents/${p.Agent.name}`
              : `external/${p.External.system}/${p.External.username}`;
        return path === assignee;
      });
    }
    if (creator) {
      filtered = filtered.filter(({ entry }) => entry.data.creator === creator);
    }
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) =>
        entry.data.title.toLowerCase().includes(lower) ||
        entry.data.description.toLowerCase().includes(lower),
      );
    }
    if (labelsParam) {
      const labelIds = labelsParam.split(",").map((s) => s.trim());
      filtered = filtered.filter(({ id }) => {
        const issueLabels = getLabelsForObject(id);
        if (!issueLabels) return false;
        return labelIds.some((labelId) =>
          issueLabels.some((l: { label_id: string }) => l.label_id === labelId),
        );
      });
    }

    // `sort=project_status_time_desc` mirrors PR-1's backend ordering:
    // (project.priority ASC, status.position ASC, created_at DESC, id DESC).
    // Default (`sort` omitted) preserves the historical updated_at DESC mock
    // shape so callers that don't set sort are unaffected.
    if (sortParam === "project_status_time_desc") {
      const priorityCache = new Map<string, number>();
      const projectPriority = (projectId: string): number => {
        const cached = priorityCache.get(projectId);
        if (cached !== undefined) return cached;
        const proj = store.get<Project>("projects", projectId);
        const value = proj?.data.priority ?? 0;
        priorityCache.set(projectId, value);
        return value;
      };
      const sortableEntries: Array<{
        id: string;
        entry: VersionedEntity<Issue>;
        priority: number;
        position: number;
        createdAt: string;
      }> = filtered.map(({ id, entry }) => ({
        id,
        entry,
        priority: projectPriority(entry.data.project_id),
        position: entry.data.status.position ?? 0,
        createdAt: store.getCreationTime(COLLECTION, id) ?? "",
      }));
      sortableEntries.sort((a, b) => {
        if (a.priority !== b.priority) return a.priority - b.priority;
        if (a.position !== b.position) return a.position - b.position;
        if (a.createdAt !== b.createdAt) {
          return b.createdAt.localeCompare(a.createdAt);
        }
        return b.id.localeCompare(a.id);
      });
      filtered = sortableEntries.map(({ id, entry }) => ({ id, entry }));
    } else {
      // Sort by last-update time descending (most recently updated first)
      // for stable pagination
      filtered.sort((a, b) => {
        return b.entry.timestamp.localeCompare(a.entry.timestamp);
      });
    }

    const totalCount = filtered.length;

    // `bucket_by=project_status`: after filters + sort, group by
    // `(project_id, status.key)`, truncate each group to `bucket_limit`,
    // then re-concatenate preserving the global sort order across cells.
    // Matches PR-3 [[i-aiunhiwa]]'s memory/SQLite/Postgres v2 semantics.
    // Bucketed queries never paginate by cursor: `next_cursor` is always
    // null; per-cell "load more" is a follow-up unbucketed call.
    let nextCursor: string | null = null;
    if (bucketByParam === "project_status") {
      const bucketLimit = Number(bucketLimitParam);
      const seenByCell = new Map<string, number>();
      const truncated: typeof filtered = [];
      for (const item of filtered) {
        const ck = `${item.entry.data.project_id}::${item.entry.data.status.key}`;
        const seen = seenByCell.get(ck) ?? 0;
        if (seen < bucketLimit) {
          truncated.push(item);
          seenByCell.set(ck, seen + 1);
        }
      }
      filtered = truncated;
      if (limitParam !== undefined && limitParam !== null) {
        const limit = Number(limitParam);
        if (limit >= 0 && filtered.length > limit) {
          filtered = filtered.slice(0, limit);
        }
      }
    } else {
      // Apply cursor-based pagination
      if (cursorParam) {
        const cursorIndex = filtered.findIndex(({ id }) => id === cursorParam);
        if (cursorIndex !== -1) {
          filtered = filtered.slice(cursorIndex + 1);
        }
      }

      if (limitParam !== undefined && limitParam !== null) {
        const limit = Number(limitParam);
        if (limit >= 0 && filtered.length > limit) {
          nextCursor = filtered[limit - 1]?.id ?? null;
          filtered = filtered.slice(0, limit);
        }
      }
    }

    const issues: IssueSummaryRecord[] = filtered.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListIssuesResponse = {
      issues,
      next_cursor: nextCursor,
      total_count: countParam === "true" ? BigInt(totalCount) : undefined,
    };
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

  // GET /v1/issues/:id/comments — list comments most-recent-first
  app.get("/v1/issues/:id/comments", (c) => {
    const id = c.req.param("id");
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const limitParam = c.req.query("limit");
    const beforeParam = c.req.query("before_sequence");
    const limit = Math.min(Math.max(Number(limitParam ?? 50), 1), 200);
    const before =
      beforeParam !== undefined && beforeParam !== null
        ? BigInt(beforeParam)
        : null;

    const all = commentsByIssue.get(id) ?? [];
    const desc = [...all].sort((a, b) => {
      // bigint compare
      if (a.sequence === b.sequence) return 0;
      return a.sequence < b.sequence ? 1 : -1;
    });
    const filtered = before === null ? desc : desc.filter((c) => c.sequence < before);
    const page = filtered.slice(0, limit);
    const nextBefore: bigint | null =
      page.length === limit ? page[page.length - 1].sequence : null;

    const resp: ListCommentsResponse = {
      comments: page,
      next_before_sequence: nextBefore,
    };
    return c.json(resp);
  });

  // POST /v1/issues/:id/comments — add a new comment
  app.post("/v1/issues/:id/comments", async (c) => {
    const id = c.req.param("id");
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const body = await c.req.json<AddCommentRequest>();
    const text = (body?.body ?? "").trim();
    if (!text) {
      return c.json({ error: "comment body must not be empty or whitespace-only" }, 400);
    }
    // Mock actor: the auth middleware injects an actor on the request context,
    // but the mock-server's store doesn't surface it; use a generic User actor
    // matching the seeded fixture conventions so the UI renders something.
    const actor: ActorRef = {
      Authenticated: {
        actor_id: { User: { name: "alice" } },
      },
    };
    const comment = appendIssueComment(store, id, text, actor);
    const resp: AddCommentResponse = { comment };
    return c.json(resp, 201);
  });

  // POST /v1/issues/:id/feedback — submit feedback
  app.post("/v1/issues/:id/feedback", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<{ feedback: string }>();
    const existing = store.get<Issue>(COLLECTION, id);
    if (!existing) {
      return c.json({ error: `issue '${id}' not found` }, 404);
    }
    const updated: Issue = { ...existing.data, feedback: body.feedback };
    const entry = store.update<Issue>(COLLECTION, id, updated, SSE_PREFIX);
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime));
  });

  return app;
}
