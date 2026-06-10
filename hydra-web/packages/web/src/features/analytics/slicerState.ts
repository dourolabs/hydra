import type { IssueType } from "@hydra/api";

/**
 * URL-backed slicer state for the Throughput analytics page. The page reads
 * and writes this via `useSearchParams`; every consumer (data hooks, panel,
 * tests) goes through these helpers so the URL contract stays canonical.
 */

export type TimeRange = "7d" | "30d" | "90d" | "all-time";

export const TIME_RANGE_OPTIONS: readonly TimeRange[] = [
  "7d",
  "30d",
  "90d",
  "all-time",
] as const;

export const DEFAULT_TIME_RANGE: TimeRange = "30d";

export const ISSUE_TYPE_OPTIONS: readonly IssueType[] = [
  "feature",
  "bug",
  "chore",
  "task",
  "merge-request",
  "review-request",
] as const;

export interface SlicerState {
  range: TimeRange;
  projectId: string | null;
  statusKeys: string[];
  repoName: string | null;
  issueTypes: IssueType[];
  assignee: string | null;
  creator: string | null;
}

export const URL_PARAMS = {
  range: "range",
  projectId: "project_id",
  statusKeys: "status_keys",
  repoName: "repo_name",
  issueTypes: "issue_types",
  /**
   * Singular form. Written when exactly one issue type is selected so the
   * URL stays backwards-compatible with `?issue_type=feature` bookmarks; the
   * plural [`URL_PARAMS.issueTypes`] is used for two or more.
   */
  issueTypeLegacy: "issue_type",
  assignee: "assignee",
  creator: "creator",
} as const;

export function isTimeRange(value: string): value is TimeRange {
  return (TIME_RANGE_OPTIONS as readonly string[]).includes(value);
}

function isIssueType(value: string): value is IssueType {
  return (ISSUE_TYPE_OPTIONS as readonly string[]).includes(value);
}

/** Pull the slicer state out of URL search params. Unknown values are dropped. */
export function readSlicerState(params: URLSearchParams): SlicerState {
  const rawRange = params.get(URL_PARAMS.range);
  const range: TimeRange = rawRange && isTimeRange(rawRange) ? rawRange : DEFAULT_TIME_RANGE;

  const statusKeys = (params.get(URL_PARAMS.statusKeys) ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  const rawIssueTypes = params.get(URL_PARAMS.issueTypes);
  let issueTypes: IssueType[];
  if (rawIssueTypes !== null) {
    issueTypes = rawIssueTypes
      .split(",")
      .map((s) => s.trim())
      .filter(isIssueType);
  } else {
    // Legacy single-form fallback so old bookmarks (`?issue_type=feature`)
    // round-trip into the new shape.
    const legacy = params.get(URL_PARAMS.issueTypeLegacy);
    issueTypes = legacy && isIssueType(legacy) ? [legacy] : [];
  }

  return {
    range,
    projectId: params.get(URL_PARAMS.projectId),
    statusKeys,
    repoName: params.get(URL_PARAMS.repoName),
    issueTypes,
    assignee: params.get(URL_PARAMS.assignee),
    creator: params.get(URL_PARAMS.creator),
  };
}

/**
 * Apply a partial slicer update to existing search params. Returns the
 * mutated `URLSearchParams` so callers can pass it straight to
 * `setSearchParams`. The `range` param is always persisted, even at its
 * default — keeps the link explicit.
 */
export function writeSlicerState(
  params: URLSearchParams,
  patch: Partial<SlicerState>,
): URLSearchParams {
  const next = new URLSearchParams(params);

  if (patch.range !== undefined) next.set(URL_PARAMS.range, patch.range);

  if (patch.projectId !== undefined) {
    if (patch.projectId) next.set(URL_PARAMS.projectId, patch.projectId);
    else next.delete(URL_PARAMS.projectId);
  }

  if (patch.statusKeys !== undefined) {
    if (patch.statusKeys.length > 0) next.set(URL_PARAMS.statusKeys, patch.statusKeys.join(","));
    else next.delete(URL_PARAMS.statusKeys);
  }

  if (patch.repoName !== undefined) {
    if (patch.repoName) next.set(URL_PARAMS.repoName, patch.repoName);
    else next.delete(URL_PARAMS.repoName);
  }

  if (patch.issueTypes !== undefined) {
    // Singular `issue_type=` for one, plural `issue_types=a,b` for two or
    // more — the unused key is deleted so we never leave both set.
    if (patch.issueTypes.length === 1) {
      next.set(URL_PARAMS.issueTypeLegacy, patch.issueTypes[0]);
      next.delete(URL_PARAMS.issueTypes);
    } else if (patch.issueTypes.length > 1) {
      next.set(URL_PARAMS.issueTypes, patch.issueTypes.join(","));
      next.delete(URL_PARAMS.issueTypeLegacy);
    } else {
      next.delete(URL_PARAMS.issueTypes);
      next.delete(URL_PARAMS.issueTypeLegacy);
    }
  }

  if (patch.assignee !== undefined) {
    if (patch.assignee) next.set(URL_PARAMS.assignee, patch.assignee);
    else next.delete(URL_PARAMS.assignee);
  }

  if (patch.creator !== undefined) {
    if (patch.creator) next.set(URL_PARAMS.creator, patch.creator);
    else next.delete(URL_PARAMS.creator);
  }

  return next;
}

/**
 * Convert a `TimeRange` into the concrete `(from, to)` ISO timestamps the
 * backend endpoints expect. `now` is injected so tests can pin time.
 *
 * `all-time` returns a fixed origin (`2020-01-01Z`) so the link stays
 * deterministic — the backend treats anything older than its data as
 * unbounded-low anyway.
 */
export function timeWindow(
  range: TimeRange,
  now: Date = new Date(),
): { from: string; to: string } {
  const to = now.toISOString();
  if (range === "all-time") {
    return { from: "2020-01-01T00:00:00.000Z", to };
  }
  const daysByRange: Record<Exclude<TimeRange, "all-time">, number> = {
    "7d": 7,
    "30d": 30,
    "90d": 90,
  };
  const days = daysByRange[range];
  const from = new Date(now.getTime() - days * 24 * 60 * 60 * 1000).toISOString();
  return { from, to };
}
