/**
 * Throughput analytics wire types and query shapes.
 *
 * TODO: these are inline placeholder shapes mirroring the response specs in
 * Analytics PR 1 (i-hvaitdun) and PR 2 (i-lzxnkbfx). When the backend lands
 * and `hydra-common/src/api/v1/analytics.rs` exports ts-rs bindings, swap
 * each shape here for the generated type and delete the inline form.
 *
 * The path prefix is `/v1/analytics/throughput/...`; see `HydraApiClient`
 * for the typed methods that call them.
 */

import type { ProjectRef } from "./generated/ProjectRef";

/** ISO-8601 timestamp string. */
export type IsoTimestamp = string;

export type ThroughputBucket = "day" | "week";

/**
 * Common query params shared across all throughput endpoints. The page
 * derives `from` / `to` from the time-range URL param + the current time;
 * each typed query interface picks the params it actually accepts.
 */
export interface BaseThroughputQuery {
  from: IsoTimestamp;
  to: IsoTimestamp;
  repo_name?: string;
  creator?: string;
}

export interface PatchesOverTimeQuery extends BaseThroughputQuery {
  bucket?: ThroughputBucket;
  status?: string;
}

export interface PatchesOverTimeBucket {
  bucket_start: IsoTimestamp;
  created: number;
  merged: number;
}

export interface PatchesOverTimeResponse {
  buckets: PatchesOverTimeBucket[];
}

export interface PatchesTerminalMixQuery extends BaseThroughputQuery {
  status?: string;
}

export interface PatchesTerminalMixResponse {
  merged: number;
  closed: number;
}

export interface HistogramBin {
  bin_start_seconds: number;
  /** `null` for the open-ended last bin (e.g. "30d+"). */
  bin_end_seconds: number | null;
  count: number;
}

export interface PatchesTimeToMergeQuery extends BaseThroughputQuery {
  status?: string;
}

export interface PatchesTimeToMergeResponse {
  /** `null` when `count` is zero. */
  median_seconds: number | null;
  /** `null` when `count` is zero. */
  p95_seconds: number | null;
  count: number;
  histogram: HistogramBin[];
}

export interface PatchesInFlightOverTimeQuery extends BaseThroughputQuery {
  bucket?: ThroughputBucket;
  status?: string;
}

export interface PatchesInFlightBucket {
  bucket_start: IsoTimestamp;
  in_flight: number;
}

export interface PatchesInFlightOverTimeResponse {
  buckets: PatchesInFlightBucket[];
}

export type IssueTypeKey =
  | "feature"
  | "bug"
  | "chore"
  | "task"
  | "merge-request"
  | "review-request";

export interface BaseIssuesThroughputQuery extends BaseThroughputQuery {
  project_id?: ProjectRef;
  /** Comma-joined list of {@link IssueTypeKey} values; multi-select. */
  issue_types?: string;
  assignee?: string;
  /** Comma-joined list of status_key values; multi-select. */
  status_keys?: string;
}

export interface IssuesCycleTimeQuery extends BaseIssuesThroughputQuery {
  bucket?: ThroughputBucket;
}

export interface IssuesCycleTimeResponse {
  median_seconds: number | null;
  p95_seconds: number | null;
  count: number;
  histogram: HistogramBin[];
}

export interface IssuesTimeInStatusBreakdownQuery extends BaseIssuesThroughputQuery {
  project_id: ProjectRef;
}

export interface StatusTimeSegment {
  status_key: string;
  label: string;
  color: string;
  mean_seconds: number;
}

export interface IssuesTimeInStatusBreakdownResponse {
  project_id: ProjectRef;
  status_segments: StatusTimeSegment[];
  issue_count: number;
}

export interface IssuesPerStatusDistributionQuery extends BaseIssuesThroughputQuery {
  project_id: ProjectRef;
}

export interface StatusDistribution {
  status_key: string;
  label: string;
  color: string;
  /** `null` when `sample_count` is zero. */
  median_seconds: number | null;
  /** `null` when `sample_count` is zero. */
  p95_seconds: number | null;
  sample_count: number;
}

export interface IssuesPerStatusDistributionResponse {
  project_id: ProjectRef;
  statuses: StatusDistribution[];
}

export interface IssuesOverTimeQuery extends BaseIssuesThroughputQuery {
  bucket?: ThroughputBucket;
}

export interface IssuesOverTimeBucket {
  bucket_start: IsoTimestamp;
  created: number;
  reached_terminal: number;
}

export interface IssuesOverTimeResponse {
  buckets: IssuesOverTimeBucket[];
}
