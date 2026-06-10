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
 * Stub handlers for `/v1/analytics/throughput/...` endpoints. These return
 * small synthetic fixtures sufficient to exercise the slicer panel +
 * time-range picker UX during dev / Playwright tests.
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
        created: BigInt(2 + i),
        merged: BigInt(1 + i),
      })),
    };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/patches/terminal_mix", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    const resp: PatchesTerminalMixResponse = { merged: BigInt(27), closed: BigInt(4) };
    return c.json(resp);
  });

  app.get("/v1/analytics/throughput/patches/time_to_merge", (c) => {
    const params = new URL(c.req.url).searchParams;
    const common = readCommon(params);
    if ("error" in common) return c.json(common, 400);
    // Mirrors the prod bin scheme from
    // hydra-server/src/analytics/buckets.rs: [0,1h), [1h,4h), [4h,1d), [1d,3d),
    // [3d,7d), [7d,14d), [14d,30d), [30d, +inf). Last bin has bin_end_seconds=null.
    const resp: PatchesTimeToMergeResponse = {
      median_seconds: BigInt(18000),
      p95_seconds: BigInt(86400 * 3),
      count: BigInt(8),
      histogram: [
        { bin_start_seconds: BigInt(0), bin_end_seconds: BigInt(3600), count: BigInt(1) },
        { bin_start_seconds: BigInt(3600), bin_end_seconds: BigInt(14400), count: BigInt(2) },
        { bin_start_seconds: BigInt(14400), bin_end_seconds: BigInt(86400), count: BigInt(2) },
        { bin_start_seconds: BigInt(86400), bin_end_seconds: BigInt(86400 * 3), count: BigInt(1) },
        { bin_start_seconds: BigInt(86400 * 3), bin_end_seconds: BigInt(86400 * 7), count: BigInt(1) },
        { bin_start_seconds: BigInt(86400 * 7), bin_end_seconds: BigInt(86400 * 14), count: BigInt(0) },
        { bin_start_seconds: BigInt(86400 * 14), bin_end_seconds: BigInt(86400 * 30), count: BigInt(0) },
        { bin_start_seconds: BigInt(86400 * 30), bin_end_seconds: null, count: BigInt(1) },
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
        in_flight: BigInt(5 + i),
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
      median_seconds: BigInt(86400),
      p95_seconds: BigInt(604800),
      count: BigInt(9),
      histogram: [
        { bin_start_seconds: BigInt(0), bin_end_seconds: BigInt(3600), count: BigInt(1) },
        { bin_start_seconds: BigInt(3600), bin_end_seconds: BigInt(86400), count: BigInt(4) },
        { bin_start_seconds: BigInt(86400), bin_end_seconds: BigInt(604800), count: BigInt(3) },
        { bin_start_seconds: BigInt(604800), bin_end_seconds: BigInt(2592000), count: BigInt(1) },
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
        { status_key: "open", label: "Open", color: "#3498db", mean_seconds: BigInt(1200) },
        { status_key: "in-progress", label: "In progress", color: "#f1c40f", mean_seconds: BigInt(21600) },
        { status_key: "closed", label: "Closed", color: "#2ecc71", mean_seconds: BigInt(0) },
      ],
      issue_count: BigInt(12),
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
          median_seconds: BigInt(1200),
          p95_seconds: BigInt(7200),
          sample_count: BigInt(10),
        },
        {
          status_key: "in-progress",
          label: "In progress",
          color: "#f1c40f",
          median_seconds: BigInt(18000),
          p95_seconds: BigInt(86400),
          sample_count: BigInt(8),
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
        created: BigInt(4 + i),
        reached_terminal: BigInt(2 + i),
      })),
    };
    return c.json(resp);
  });

  return app;
}
