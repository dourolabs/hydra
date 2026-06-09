import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icons, TypeChip } from "@hydra/ui";
import type {
  IssueSummaryRecord,
  IssueType,
  StatusDefinition,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import type { Filter, FilterDefinitions, FilterOption } from "../filters";
import { useUserOptions } from "../filters/options/userOptions";
import {
  useProjects,
  useProjectStatuses,
} from "../projects/useProjects";
import { StatusChip } from "../projects/StatusChip";
import {
  relationOptionsFromEntities,
  type RelationEntity,
} from "../filters/options/relationOptions";

const ISSUE_TYPE_OPTIONS: { value: IssueType; label: string }[] = [
  { value: "task", label: "Task" },
  { value: "bug", label: "Bug" },
  { value: "feature", label: "Feature" },
  { value: "chore", label: "Chore" },
  { value: "merge-request", label: "Merge request" },
  { value: "review-request", label: "Review request" },
];

function buildStatusOptions(statuses: StatusDefinition[]): FilterOption[] {
  return statuses.map((s) => ({
    value: s.key,
    label: s.label,
    // The StatusChip badge tone is driven by the project's declared
    // `color` / `icon`, so the chip and row both render the project's
    // declared display props rather than the hardcoded `BadgeStatus`
    // palette used by the legacy 5-enum dropdown.
    chip: <StatusChip status={s} />,
    render: <StatusChip status={s} />,
  }));
}

function buildTypeOptions(): FilterOption[] {
  return ISSUE_TYPE_OPTIONS.map((opt) => ({
    value: opt.value,
    label: opt.label,
    chip: <TypeChip type={opt.value} />,
    render: <TypeChip type={opt.value} />,
  }));
}

// Server-side filtering means none of the `apply` functions below are invoked
// from the Issues page (the page maps Filter[] → IssueFilters via
// `filtersToIssuesQuery` and lets the server narrow). `apply` stays defined
// for the foundation contract (and for any future client-side consumer such
// as a synchronous unit test) but matches what the server would do.
function valueIncludes(haystack: string | null, values: string[]): boolean {
  if (haystack === null) return false;
  return values.includes(haystack);
}

export interface UseIssueFiltersOptions {
  /**
   * Current filter chips. Used to decide which relation-picker option lists
   * to fetch eagerly: if the user already has (or rehydrates from URL) a
   * `relatedPatch` chip, we kick off the patch list immediately so the
   * picker is populated when opened. See `addMenuOpen` for the other gate.
   */
  filters?: Filter[];
  /**
   * Whether the FilterBar's add-filter menu is currently open. When `true`,
   * all four relation-picker option lists become eligible to fetch so the
   * picker isn't empty when the user clicks through. Defaults to `false`.
   */
  addMenuOpen?: boolean;
}

/**
 * Builds the `ISSUE_FILTERS` definition map for the Issues page. Loaded as a
 * hook because the option lists for `assignee` / `creator` and the relation
 * picker option lists all come from live React Query data.
 *
 * Every entry maps to a server-side query param:
 *   - `status` / `type`        → `?status=` / `?issue_type=` on listIssues.
 *   - `project`                → `?project_id=` on listIssues. Also
 *                                re-scopes the `status` option list to that
 *                                project's declared StatusKeys via
 *                                `useProjectStatuses(project_id)`.
 *   - `creator` / `assignee`   → `?creator=` / `?assignee=` on listIssues.
 *   - relation filters          → `/v1/relations` lookup → `ids=` on
 *                                listIssues. See `useRelationFilteredIssueIds`.
 *
 * `status` / `project` / `type` / `creator` / `assignee` are `singleSelect: true` because
 * the backing server param accepts a single value. Relations stay
 * multi-select; their resolver unions related issue ids across the selected
 * entities. `not_in` is unsupported server-side for every entry, so each
 * definition leaves `notInSupported` unset (the ValuePicker hides the
 * is/is-not toggle for these).
 *
 * The `repository` filter was confirmed out of scope by the reviewer (the
 * server-side surface doesn't carry repo_name on IssueSummary today).
 *
 * Relation-picker option lists (`listPatches` / `listSessions` /
 * `listConversations` / `listIssues`, each `limit=100`) are lazy: they only
 * fire when the matching relation chip is already on the bar (e.g.
 * URL-rehydrated `?relatedPatch=p-aa`) or when the add-filter menu opens, so
 * a cold-cache Issues page paint without any relation filter makes zero
 * extra option-list requests.
 */
export function useIssueFilters(
  options: UseIssueFiltersOptions = {},
): FilterDefinitions<IssueSummaryRecord> {
  const { filters = [], addMenuOpen = false } = options;
  const userOpts = useUserOptions();
  const { data: projects } = useProjects();

  // The current project filter (single-select) re-scopes the Status filter
  // dropdown to that project's declared statuses. Falls back to the seeded
  // default project's statuses (legacy 5-enum list) when no project is set.
  const projectFilter = filters.find((f) => f.id === "project");
  const currentProjectFilterId = projectFilter?.values[0] ?? null;
  const { data: projectStatusesResp } = useProjectStatuses(
    currentProjectFilterId,
  );

  const needPatch =
    addMenuOpen || filters.some((f) => f.id === "relatedPatch");
  const needSession =
    addMenuOpen || filters.some((f) => f.id === "relatedSession");
  const needConversation =
    addMenuOpen || filters.some((f) => f.id === "relatedChat");
  const needIssue =
    addMenuOpen || filters.some((f) => f.id === "parentOrChild");

  // Bounded option lists for the relation pickers. Filtering happens
  // server-side; the picker just needs entity ids + display data.
  const patchListQuery = useQuery({
    queryKey: ["filter-options-patches"],
    queryFn: () => apiClient.listPatches({ limit: 100 }),
    staleTime: 60_000,
    enabled: needPatch,
  });
  const sessionListQuery = useQuery({
    queryKey: ["filter-options-sessions"],
    queryFn: () => apiClient.listSessions({ limit: 100 }),
    staleTime: 60_000,
    enabled: needSession,
  });
  const conversationListQuery = useQuery({
    queryKey: ["filter-options-conversations"],
    queryFn: () => apiClient.listConversations({ limit: 100 }),
    staleTime: 60_000,
    enabled: needConversation,
  });
  const issueListQuery = useQuery({
    queryKey: ["filter-options-issues"],
    queryFn: () => apiClient.listIssues({ limit: 100 }),
    staleTime: 60_000,
    enabled: needIssue,
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

  const projectOptions = useMemo<FilterOption[]>(
    () =>
      (projects ?? []).map((p) => ({
        value: p.project_id,
        label: p.project.key,
        sub: p.project.name,
        chip: <span>{p.project.key}</span>,
        render: <span>{p.project.key}</span>,
      })),
    [projects],
  );

  const statusOpts = useMemo<FilterOption[]>(
    () => buildStatusOptions(projectStatusesResp?.statuses ?? []),
    [projectStatusesResp],
  );

  return useMemo<FilterDefinitions<IssueSummaryRecord>>(() => {
    return {
      status: {
        label: "Status",
        icon: Icons.IconDot,
        group: "properties",
        kind: "enum",
        singleSelect: true,
        options: statusOpts,
        apply: (rec, filter) =>
          valueIncludes(rec.issue.status.key, filter.values),
      },
      project: {
        label: "Project",
        icon: Icons.IconFilter,
        group: "properties",
        kind: "enum",
        singleSelect: true,
        options: projectOptions,
        // Server-side filtering — `apply` is unused but matches the relation
        // filter convention of `() => false` so a stray client-side
        // `applyFilters(...)` call doesn't silently treat every row as a
        // match.
        apply: () => false,
      },
      type: {
        label: "Type",
        icon: Icons.IconFilter,
        group: "properties",
        kind: "enum",
        singleSelect: true,
        options: buildTypeOptions(),
        apply: (rec, filter) => valueIncludes(rec.issue.type, filter.values),
      },
      assignee: {
        label: "Assignee",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        singleSelect: true,
        options: userOpts,
        // The wire form of `assignee` is a Principal path (`users/<name>` /
        // `agents/<name>`), which is what the user-options list keys on.
        apply: (rec, filter) => {
          const p = rec.issue.assignee;
          if (!p) return false;
          let path: string | null = null;
          if ("User" in p) path = `users/${p.User.name}`;
          else if ("Agent" in p) path = `agents/${p.Agent.name}`;
          return path !== null && filter.values.includes(path);
        },
      },
      creator: {
        label: "Creator",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        singleSelect: true,
        options: userOpts,
        // Creator on the wire is a bare username; strip the Principal-path
        // prefix to compare.
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
      // Relation filters are resolved server-side via
      // `useRelationFilteredIssueIds`; the page never invokes `apply` on these.
      // `() => false` is a guardrail so stray client-side `applyFilters(...)`
      // calls don't silently fall through and treat every row as a match.
      relatedChat: {
        label: "Related chat",
        icon: Icons.IconChat,
        group: "relations",
        kind: "relation",
        entityLabel: "chat",
        options: relationOptionsFromEntities("chat", conversationEntities),
        apply: () => false,
      },
      relatedPatch: {
        label: "Related patch",
        icon: Icons.IconPatch,
        group: "relations",
        kind: "relation",
        entityLabel: "patch",
        options: relationOptionsFromEntities("patch", patchEntities),
        apply: () => false,
      },
      relatedSession: {
        label: "Related session",
        icon: Icons.IconAgent,
        group: "relations",
        kind: "relation",
        entityLabel: "session",
        options: relationOptionsFromEntities("session", sessionEntities),
        apply: () => false,
      },
      parentOrChild: {
        label: "Parent or child issue",
        icon: Icons.IconIssue,
        group: "relations",
        kind: "relation",
        entityLabel: "parent or child issue",
        options: relationOptionsFromEntities("issue", issueEntities),
        apply: () => false,
      },
    };
  }, [
    userOpts,
    statusOpts,
    projectOptions,
    conversationEntities,
    patchEntities,
    sessionEntities,
    issueEntities,
  ]);
}
