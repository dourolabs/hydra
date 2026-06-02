import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icons, TypeChip, type BadgeStatus } from "@hydra/ui";
import type {
  IssueStatus,
  IssueSummaryRecord,
  IssueType,
  Principal,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import type { Filter, FilterDefinitions, FilterOption } from "../filters";
import { useUserOptions } from "../filters/options/userOptions";
import { statusOptions } from "../filters/options/statusOptions";
import {
  relationOptionsFromEntities,
  useIssueRelationsIndex,
  type RelationEdge,
  type RelationEntity,
} from "../filters/options/relationOptions";

const ISSUE_STATUS_TONES: Record<IssueStatus, BadgeStatus> = {
  open: "open",
  "in-progress": "in-progress",
  closed: "issue-closed",
  failed: "failed",
  dropped: "dropped",
  unknown: "unknown",
};

const ISSUE_STATUS_DISPLAY_ORDER: IssueStatus[] = [
  "open",
  "in-progress",
  "failed",
  "closed",
  "dropped",
];

const ISSUE_TYPE_OPTIONS: { value: IssueType; label: string }[] = [
  { value: "task", label: "Task" },
  { value: "bug", label: "Bug" },
  { value: "feature", label: "Feature" },
  { value: "chore", label: "Chore" },
  { value: "merge-request", label: "Merge request" },
  { value: "review-request", label: "Review request" },
];

function buildStatusOptions(): FilterOption[] {
  const tones: Record<string, BadgeStatus> = {};
  for (const status of ISSUE_STATUS_DISPLAY_ORDER) {
    tones[status] = ISSUE_STATUS_TONES[status];
  }
  return statusOptions(tones);
}

function buildTypeOptions(): FilterOption[] {
  return ISSUE_TYPE_OPTIONS.map((opt) => ({
    value: opt.value,
    label: opt.label,
    chip: <TypeChip type={opt.value} />,
    render: <TypeChip type={opt.value} />,
  }));
}

function principalToPath(p: Principal | null | undefined): string | null {
  if (!p) return null;
  if ("User" in p) return `users/${p.User.name}`;
  if ("Agent" in p) return `agents/${p.Agent.name}`;
  if ("External" in p) return `external/${p.External.system}/${p.External.username}`;
  return null;
}

function applyValuesMatch(item: string | null, filter: Filter): boolean {
  if (item === null) return false;
  return filter.values.includes(item);
}

interface UseIssueFiltersArgs {
  loadedIssues: IssueSummaryRecord[];
}

const ISSUE_RELATION_EDGES = {
  patch: [{ rel_type: "has-patch", direction: "outbound" } as RelationEdge],
  chat: [{ rel_type: "refers-to", direction: "inbound" } as RelationEdge],
  session: [
    // sessions linked to an issue use `spawned_from` rather than a relation
    // row; handled separately by listing sessions and grouping client-side.
  ] as RelationEdge[],
  parentOrChild: [
    { rel_type: "child-of", direction: "outbound" } as RelationEdge,
    { rel_type: "child-of", direction: "inbound" } as RelationEdge,
  ],
};

/**
 * Builds the `ISSUE_FILTERS` definition map for the Issues page. Loaded as a
 * hook because the option lists for `assignee` / `creator` / `repository`
 * and the relation indices for relation filters all come from live React
 * Query data.
 *
 * The relation filters operate over a per-page index: for the currently
 * loaded `issueIds` we fetch `has-patch`, `refers-to`, `child-of` relations
 * and an associated list of all sessions whose `spawned_from` is one of the
 * loaded issues. The picker option lists are bounded fetches of all
 * patches / sessions / conversations / issues so the user can choose any
 * target — but the actual matching is limited to entries appearing in the
 * relation index, keeping work O(loaded issues × edges) regardless of how
 * many entities exist in the system.
 */
export function useIssueFilters({
  loadedIssues,
}: UseIssueFiltersArgs): FilterDefinitions<IssueSummaryRecord> {
  const userOpts = useUserOptions();

  const issueIds = useMemo(() => loadedIssues.map((rec) => rec.issue_id), [
    loadedIssues,
  ]);

  const { index: patchIndex } = useIssueRelationsIndex(
    issueIds,
    ISSUE_RELATION_EDGES.patch,
  );
  const { index: chatIndex } = useIssueRelationsIndex(
    issueIds,
    ISSUE_RELATION_EDGES.chat,
  );
  const { index: relativeIndex } = useIssueRelationsIndex(
    issueIds,
    ISSUE_RELATION_EDGES.parentOrChild,
  );

  // Sessions linked to issues go through `spawned_from`, not a relation row.
  // Fetch sessions for the loaded issue ids and bucket them by source.
  const sessionsByIssueQuery = useQuery({
    queryKey: ["filter-sessions-by-issue", [...issueIds].sort().join(",")],
    queryFn: async () => {
      if (issueIds.length === 0) return [];
      const resp = await apiClient.listSessions({
        spawned_from_ids: issueIds.join(","),
        limit: Math.max(100, issueIds.length * 4),
      });
      return resp.sessions;
    },
    enabled: issueIds.length > 0,
    staleTime: 30_000,
  });

  const sessionIndex = useMemo(() => {
    const map = new Map<string, Set<string>>();
    for (const session of sessionsByIssueQuery.data ?? []) {
      const issueId = session.session.spawned_from;
      if (!issueId) continue;
      const set = map.get(issueId) ?? new Set<string>();
      set.add(session.session_id);
      map.set(issueId, set);
    }
    return map;
  }, [sessionsByIssueQuery.data]);

  // Option lists for relation pickers — bounded fetches of recent entities.
  // Picking an id the filter index can't see is harmless: `apply` falls
  // through to `false` so the filter narrows to nothing.
  const patchListQuery = useQuery({
    queryKey: ["filter-options-patches"],
    queryFn: () => apiClient.listPatches({ limit: 100 }),
    staleTime: 60_000,
  });
  const sessionListQuery = useQuery({
    queryKey: ["filter-options-sessions"],
    queryFn: () => apiClient.listSessions({ limit: 100 }),
    staleTime: 60_000,
  });
  const conversationListQuery = useQuery({
    queryKey: ["filter-options-conversations"],
    queryFn: () => apiClient.listConversations({ limit: 100 }),
    staleTime: 60_000,
  });
  const issueListQuery = useQuery({
    queryKey: ["filter-options-issues"],
    queryFn: () => apiClient.listIssues({ limit: 100 }),
    staleTime: 60_000,
  });

  const patchEntities: RelationEntity[] = useMemo(
    () =>
      (patchListQuery.data?.patches ?? []).map((p) => ({
        id: p.patch_id,
        title: p.patch.title,
        sub: p.patch.branch_name ?? p.patch.creator,
      })),
    [patchListQuery.data],
  );

  const sessionEntities: RelationEntity[] = useMemo(
    () =>
      (sessionListQuery.data?.sessions ?? []).map((s) => ({
        id: s.session_id,
        title: s.session.prompt || s.session_id,
        sub: s.session.creator,
      })),
    [sessionListQuery.data],
  );

  const conversationEntities: RelationEntity[] = useMemo(
    () =>
      (conversationListQuery.data ?? []).map((c) => ({
        id: c.conversation_id,
        title: c.title ?? c.conversation_id,
        sub: c.agent_name ?? c.creator,
      })),
    [conversationListQuery.data],
  );

  const issueEntities: RelationEntity[] = useMemo(
    () =>
      (issueListQuery.data?.issues ?? []).map((i) => ({
        id: i.issue_id,
        title: i.issue.title,
        sub: i.issue.creator,
      })),
    [issueListQuery.data],
  );

  return useMemo<FilterDefinitions<IssueSummaryRecord>>(() => {
    return {
      status: {
        label: "Status",
        icon: Icons.IconDot,
        group: "properties",
        kind: "enum",
        options: buildStatusOptions(),
        apply: (rec, filter) =>
          applyValuesMatch(rec.issue.status, filter),
      },
      type: {
        label: "Type",
        icon: Icons.IconFilter,
        group: "properties",
        kind: "enum",
        options: buildTypeOptions(),
        apply: (rec, filter) => applyValuesMatch(rec.issue.type, filter),
      },
      assignee: {
        label: "Assignee",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        options: userOpts,
        apply: (rec, filter) =>
          applyValuesMatch(principalToPath(rec.issue.assignee), filter),
      },
      creator: {
        label: "Creator",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        // The Creator field on the wire is a bare username, not a Principal
        // path. We surface the same user-option list (with `users/<name>` /
        // `agents/<name>` values) for consistency and strip the prefix when
        // matching.
        options: userOpts,
        apply: (rec, filter) => {
          const creator = rec.issue.creator;
          return filter.values.some((v) => {
            const bare = v.startsWith("users/")
              ? v.slice("users/".length)
              : v.startsWith("agents/")
                ? v.slice("agents/".length)
                : v;
            return bare === creator;
          });
        },
      },
      relatedChat: {
        label: "Related chat",
        icon: Icons.IconChat,
        group: "relations",
        kind: "relation",
        entityLabel: "chat",
        options: relationOptionsFromEntities("chat", conversationEntities),
        apply: (rec, filter) => {
          const set = chatIndex.get(rec.issue_id);
          if (!set) return false;
          return filter.values.some((v) => set.has(v));
        },
      },
      relatedPatch: {
        label: "Related patch",
        icon: Icons.IconPatch,
        group: "relations",
        kind: "relation",
        entityLabel: "patch",
        options: relationOptionsFromEntities("patch", patchEntities),
        apply: (rec, filter) => {
          const set = patchIndex.get(rec.issue_id);
          if (!set) return false;
          return filter.values.some((v) => set.has(v));
        },
      },
      relatedSession: {
        label: "Related session",
        icon: Icons.IconAgent,
        group: "relations",
        kind: "relation",
        entityLabel: "session",
        options: relationOptionsFromEntities("session", sessionEntities),
        apply: (rec, filter) => {
          const set = sessionIndex.get(rec.issue_id);
          if (!set) return false;
          return filter.values.some((v) => set.has(v));
        },
      },
      parentOrChild: {
        label: "Parent or child issue",
        icon: Icons.IconIssue,
        group: "relations",
        kind: "relation",
        entityLabel: "parent or child issue",
        options: relationOptionsFromEntities("issue", issueEntities),
        apply: (rec, filter) => {
          const set = relativeIndex.get(rec.issue_id);
          if (!set) return false;
          return filter.values.some((v) => set.has(v));
        },
      },
    };
  }, [
    userOpts,
    conversationEntities,
    patchEntities,
    sessionEntities,
    issueEntities,
    chatIndex,
    patchIndex,
    sessionIndex,
    relativeIndex,
  ]);
}
