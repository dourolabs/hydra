import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icons, type BadgeStatus } from "@hydra/ui";
import type { SessionSummaryRecord, Status as SessionStatus } from "@hydra/api";
import { apiClient } from "../../api/client";
import type { FilterDefinitions, FilterOption } from "../filters";
import { useUserOptions } from "../filters/options/userOptions";
import { statusOptions } from "../filters/options/statusOptions";
import {
  relationOptionsFromEntities,
  type RelationEntity,
} from "../filters/options/relationOptions";

// Display order for the Status picker. Mirrors the chip-button row removed
// from `SessionsView` (running → pending → created → complete → failed). The
// brief explicitly excludes `unknown` from the picker since the server never
// uses it as a meaningful classification.
const SESSION_STATUS_TONES: Record<
  Exclude<SessionStatus, "unknown">,
  BadgeStatus
> = {
  running: "running",
  pending: "pending",
  created: "created",
  complete: "complete",
  failed: "failed",
};

function buildStatusOptions(): FilterOption[] {
  return statusOptions(SESSION_STATUS_TONES);
}

/**
 * Builds the `SESSION_FILTERS` definition map for the Sessions page. Loaded as
 * a hook because the `creator` user list and the relation pickers' option
 * lists come from live React Query data.
 *
 * Every entry maps to a server-side query param on `listSessions`:
 *   - `status`        → `?status=` (CSV, multi).
 *   - `creator`       → `?creator=` (single-select, replaces Mine/All toggle).
 *   - `relatedIssue`  → `?spawned_from_ids=` (CSV, multi, direct).
 *   - `relatedChat`   → `?conversation_id=` (single-select; the server param
 *                       is single-valued).
 *   - `relatedPatch`  → `?spawned_from_ids=` via /v1/relations 2-hop in
 *                       `useRelationFilteredSessionIds`.
 *
 * `creator` and `relatedChat` are `singleSelect: true` because their backing
 * server params accept a single value. `not_in` is unsupported for every
 * entry, so each definition leaves `notInSupported` unset.
 *
 * The `agent` and "source kind" filters from the brief are deferred — the
 * server-side `SearchSessionsQuery` exposes neither. See the PR-3 issue body
 * for the deferral rationale.
 */
export function useSessionFilters(): FilterDefinitions<SessionSummaryRecord> {
  const userOpts = useUserOptions();

  const issueListQuery = useQuery({
    queryKey: ["filter-options-issues"],
    queryFn: () => apiClient.listIssues({ limit: 100 }),
    staleTime: 60_000,
  });
  const patchListQuery = useQuery({
    queryKey: ["filter-options-patches"],
    queryFn: () => apiClient.listPatches({ limit: 100 }),
    staleTime: 60_000,
  });
  const conversationListQuery = useQuery({
    queryKey: ["filter-options-conversations"],
    queryFn: () => apiClient.listConversations({ limit: 100 }),
    staleTime: 60_000,
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

  const patchEntities: RelationEntity[] = useMemo(
    () =>
      (patchListQuery.data?.patches ?? []).map((p) => ({
        id: p.patch_id,
        title: p.patch.title,
        sub: p.patch.branch_name ?? p.patch.creator,
      })),
    [patchListQuery.data],
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

  return useMemo<FilterDefinitions<SessionSummaryRecord>>(() => {
    return {
      status: {
        label: "Status",
        icon: Icons.IconDot,
        group: "properties",
        kind: "enum",
        options: buildStatusOptions(),
        // Server filters; `apply` matches the wire shape for completeness.
        apply: (rec, filter) => filter.values.includes(rec.session.status),
      },
      creator: {
        label: "Creator",
        icon: Icons.IconAgent,
        group: "people",
        kind: "user",
        singleSelect: true,
        options: userOpts,
        apply: (rec, filter) => {
          const creator = rec.session.creator;
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
      relatedIssue: {
        label: "Related issue",
        icon: Icons.IconIssue,
        group: "relations",
        kind: "relation",
        entityLabel: "issue",
        options: relationOptionsFromEntities("issue", issueEntities),
        apply: (rec, filter) => {
          const spawned = rec.session.spawned_from;
          return !!spawned && filter.values.includes(spawned);
        },
      },
      relatedChat: {
        label: "Related chat",
        icon: Icons.IconChat,
        group: "relations",
        kind: "relation",
        singleSelect: true,
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
    };
  }, [userOpts, issueEntities, patchEntities, conversationEntities]);
}
