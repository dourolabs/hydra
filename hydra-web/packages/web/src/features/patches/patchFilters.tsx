import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icons, type BadgeStatus } from "@hydra/ui";
import type { PatchStatus, PatchSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import type { Filter, FilterDefinitions, FilterOption } from "../filters";
import { useUserOptions } from "../filters/options/userOptions";
import { useRepoOptions } from "../filters/options/repoOptions";
import { statusOptions } from "../filters/options/statusOptions";
import {
  relationOptionsFromEntities,
  type RelationEntity,
} from "../filters/options/relationOptions";

const PATCH_STATUS_TONES: Record<PatchStatus, BadgeStatus> = {
  Open: "open",
  ChangesRequested: "changes-requested",
  Merged: "merged",
  Closed: "closed",
  Unknown: "unknown",
};

const PATCH_STATUS_DISPLAY_ORDER: PatchStatus[] = [
  "Open",
  "ChangesRequested",
  "Merged",
  "Closed",
];

const PATCH_STATUS_LABELS: Partial<Record<PatchStatus, string>> = {
  Open: "Open",
  ChangesRequested: "Changes requested",
  Merged: "Merged",
  Closed: "Closed",
};

function buildStatusOptions(): FilterOption[] {
  const tones: Record<string, BadgeStatus> = {};
  for (const status of PATCH_STATUS_DISPLAY_ORDER) {
    tones[status] = PATCH_STATUS_TONES[status];
  }
  return statusOptions(tones, PATCH_STATUS_LABELS);
}

function valueIncludes(haystack: string | null, values: string[]): boolean {
  if (haystack === null) return false;
  return values.includes(haystack);
}

export interface UsePatchFiltersOptions {
  /**
   * Current filter chips. Used to decide which relation-picker option lists
   * to fetch eagerly: if the user already has (or rehydrates from URL) a
   * `relatedIssue` chip, we kick off the issues list immediately so the
   * picker is populated when opened. See `addMenuOpen` for the other gate.
   */
  filters?: Filter[];
  /**
   * Whether the FilterBar's add-filter menu is currently open. When `true`,
   * both relation-picker option lists become eligible to fetch so the
   * picker isn't empty when the user clicks through. Defaults to `false`.
   */
  addMenuOpen?: boolean;
}

/**
 * Builds the `PATCH_FILTERS` definition map for the Patches page. Loaded as a
 * hook because the option lists for `author` / `repository` and the relation
 * pickers all come from live React Query data.
 *
 * Every entry maps to a server-side query param:
 *   - `status`      → `?status=` (Vec<PatchStatus>) on listPatches.
 *   - `repository`  → `?repo_name=` on listPatches.
 *   - `author`      → `?creator=` on listPatches (Principal-path prefix stripped).
 *   - relation filters → `/v1/relations` lookup → `ids=` on listPatches.
 *     See `useRelationFilteredPatchIds`.
 *
 * The `status` filter is multi-select because the server `?status[]=` param
 * accepts an array. `repository` / `author` are single-select. Relations stay
 * multi-select; their resolver unions related patch ids across the selected
 * entities.
 *
 * Relation-picker option lists (`listIssues` / `listSessions`, each
 * `limit=100`) are lazy: they only fire when the matching relation chip is
 * already on the bar (e.g. URL-rehydrated `?relatedIssue=i-aa`) or when the
 * add-filter menu opens, so a cold-cache Patches page paint without any
 * relation filter makes zero extra option-list requests.
 */
export function usePatchFilters(
  options: UsePatchFiltersOptions = {},
): FilterDefinitions<PatchSummaryRecord> {
  const { filters = [], addMenuOpen = false } = options;
  const userOpts = useUserOptions();
  const repoOpts = useRepoOptions();

  const needIssue =
    addMenuOpen || filters.some((f) => f.id === "relatedIssue");
  const needSession =
    addMenuOpen || filters.some((f) => f.id === "relatedSession");

  // Bounded option lists for the relation pickers. Filtering happens
  // server-side; the picker just needs entity ids + display data.
  const issueListQuery = useQuery({
    queryKey: ["filter-options-issues"],
    queryFn: () => apiClient.listIssues({ limit: 100 }),
    staleTime: 60_000,
    enabled: needIssue,
  });
  const sessionListQuery = useQuery({
    queryKey: ["filter-options-sessions"],
    queryFn: () => apiClient.listSessions({ limit: 100 }),
    staleTime: 60_000,
    enabled: needSession,
  });

  const issueEntities: RelationEntity[] = useMemo(
    () =>
      (issueListQuery.data?.issues ?? []).map((i) => ({
        id: i.issue_id,
        title: i.issue.title,
        sub: i.issue.creator,
      })),
    [issueListQuery.data],
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

  return useMemo<FilterDefinitions<PatchSummaryRecord>>(() => {
    return {
      status: {
        label: "Status",
        icon: Icons.IconDot,
        group: "properties",
        kind: "enum",
        options: buildStatusOptions(),
        apply: (rec, filter) =>
          valueIncludes(rec.patch.status, filter.values),
      },
      repository: {
        label: "Repository",
        icon: Icons.IconRepo,
        group: "properties",
        kind: "enum",
        singleSelect: true,
        options: repoOpts,
        apply: (rec, filter) =>
          valueIncludes(rec.patch.service_repo_name ?? null, filter.values),
      },
      author: {
        label: "Author",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        singleSelect: true,
        options: userOpts,
        // Creator on the wire is a bare username; strip the Principal-path
        // prefix to compare.
        apply: (rec, filter) => {
          const creator = rec.patch.creator;
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
      // `useRelationFilteredPatchIds`; the page never invokes `apply` on these.
      // `() => false` is a guardrail so stray client-side `applyFilters(...)`
      // calls don't silently fall through and treat every row as a match.
      relatedIssue: {
        label: "Related issue",
        icon: Icons.IconIssue,
        group: "relations",
        kind: "relation",
        entityLabel: "issue",
        options: relationOptionsFromEntities("issue", issueEntities),
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
    };
  }, [userOpts, repoOpts, issueEntities, sessionEntities]);
}
