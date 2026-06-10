import { Hono } from "hono";
import type {
  PatchesOverTimeResponse,
  PatchesTerminalMixResponse,
  PatchesTimeToMergeResponse,
  PatchesInFlightOverTimeResponse,
  IssuesCycleTimeResponse,
  IssuesTimeInStatusBreakdownResponse,
  IssuesPerStatusDistributionResponse,
  IssuesOverTimeResponse,
} from "@hydra/api";

/**
 * Stub handlers for `/v1/analytics/throughput/...` endpoints planned in
 * Analytics PRs 1+2. These return small synthetic fixtures sufficient to
 * exercise the slicer panel + time-range picker UX during dev / Playwright
 * tests. The chart implementations land in PRs 4+5; this PR only needs the
 * fixture to be a stable, parseable shape.
 *
 * `from` / `to` are validated as ISO timestamps; everything else is
 * accepted permissively (we mirror filters at the wire-shape level, not
 * the semantic level).
 */

interface CommonParams {
  from: string;
  to: string;
}

function badRequest(message: string) {
  return { error: message } as const;
}

function readCommon(params: URLSearchParams): CommonParams | { error: string } {
  const from = params.get("from");
  const to = params.get("to");
  if (!from) return badRequest("missing required query param 'from'");
  if (!to) return badRequest("missing required query param 'to'");
  if (Number.isNaN(Date.parse(from))) return badRequest("invalid 'from' timestamp");
  if (Number.isNaN(Date.parse(to))) return badRequest("invalid 'to' timestamp");
  return { from, to };
}

function syntheticBuckets(from: string, to: string, count: number = 3) {
  const start = new Date(from).getTime();
  const end = new Date(to).getTime();
  const step = Math.max(1, Math.floor((end - start) / Math.max(count, 1)));
  return Array.from({ length: count }, (_, i) => ({
    bucket_start: new Date(start + step * i).toISOString(),
  }));
}

export function createAnalyticsRoutes(): Hono {
  const app = new Hono();

  // ── Patches ──

  app.get("/v1/analytics/throughput/patches/over_time", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const buckets = syntheticBuckets(common.from, common.to);
    const resp: PatchesOverTimeResponse = {
      buckets: buckets.map((b, i) => ({
        bucket_start: b.bucket_start,
        created: 2 + i,
        merged: 1 + i,
      })),
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/patches/terminal_mix", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const resp: PatchesTerminalMixResponse = { merged: 27, closed: 4 };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/patches/time_to_merge", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const resp: PatchesTimeToMergeResponse = {
      median_seconds: 18000,
      p95_seconds: 86400,
      count: 7,
      histogram: [
        { bin_start_seconds: 0, bin_end_seconds: 3600, count: 1 },
        { bin_start_seconds: 3600, bin_end_seconds: 14400, count: 3 },
        { bin_start_seconds: 14400, bin_end_seconds: 86400, count: 2 },
        { bin_start_seconds: 86400, bin_end_seconds: 604800, count: 1 },
      ],
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/patches/in_flight_over_time", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const buckets = syntheticBuckets(common.from, common.to);
    const resp: PatchesInFlightOverTimeResponse = {
      buckets: buckets.map((b, i) => ({
        bucket_start: b.bucket_start,
        in_flight: 5 + i,
      })),
    };
    return c.json(resp);
  });

  // ── Issues ──

  app.get("/v1/analytics/throughput/issues/cycle_time", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const resp: IssuesCycleTimeResponse = {
      median_seconds: 86400,
      p95_seconds: 604800,
      count: 9,
      histogram: [
        { bin_start_seconds: 0, bin_end_seconds: 3600, count: 1 },
        { bin_start_seconds: 3600, bin_end_seconds: 86400, count: 4 },
        { bin_start_seconds: 86400, bin_end_seconds: 604800, count: 3 },
        { bin_start_seconds: 604800, bin_end_seconds: 2592000, count: 1 },
      ],
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/issues/time_in_status_breakdown", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const projectId = params.get("project_id");
    if (!projectId) {
      return c.json(badRequest("missing required query param 'project_id'"), 400);
    }
    const resp: IssuesTimeInStatusBreakdownResponse = {
      project_id: projectId,
      status_segments: [
        { status_key: "open", label: "Open", color: "#3498db", mean_seconds: 1200 },
        { status_key: "in-progress", label: "In progress", color: "#f1c40f", mean_seconds: 21600 },
        { status_key: "closed", label: "Closed", color: "#2ecc71", mean_seconds: 0 },
      ],
      issue_count: 12,
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/issues/per_status_distribution", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const projectId = params.get("project_id");
    if (!projectId) {
      return c.json(badRequest("missing required query param 'project_id'"), 400);
    }
    const resp: IssuesPerStatusDistributionResponse = {
      project_id: projectId,
      statuses: [
        {
          status_key: "open",
          label: "Open",
          color: "#3498db",
          median_seconds: 1200,
          p95_seconds: 7200,
          sample_count: 10,
        },
        {
          status_key: "in-progress",
          label: "In progress",
          color: "#f1c40f",
          median_seconds: 18000,
          p95_seconds: 86400,
          sample_count: 8,
        },
      ],
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/issues/over_time", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const buckets = syntheticBuckets(common.from, common.to);
    const resp: IssuesOverTimeResponse = {
      buckets: buckets.map((b, i) => ({
        bucket_start: b.bucket_start,
        created: 4 + i,
        reached_terminal: 2 + i,
      })),
    };
    return c.json(resp);
  });

  return app;
}
