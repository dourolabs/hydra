import { Hono } from "hono";
import type { MergeQueue, EnqueueMergePatchRequest } from "@hydra/api";

// Simple in-memory merge queue keyed by "org/repo/branch"
const mergeQueues = new Map<string, string[]>();

export function createMergeQueueRoutes(): Hono {
  const app = new Hono();

  // GET /v1/merge-queues/:org/:repo/:branch/patches
  app.get("/v1/merge-queues/:org/:repo/:branch/patches", (c) => {
    const org = c.req.param("org");
    const repo = c.req.param("repo");
    const branch = c.req.param("branch");
    const key = `${org}/${repo}/${branch}`;
    const patches = mergeQueues.get(key) ?? [];
    const resp: MergeQueue = { patches };
    return c.json(resp);
  });

  // POST /v1/merge-queues/:org/:repo/:branch/patches
  app.post("/v1/merge-queues/:org/:repo/:branch/patches", async (c) => {
    const org = c.req.param("org");
    const repo = c.req.param("repo");
    const branch = c.req.param("branch");
    const key = `${org}/${repo}/${branch}`;
    const body = await c.req.json<EnqueueMergePatchRequest>();
    const queue = mergeQueues.get(key) ?? [];
    queue.push(body.patch_id);
    mergeQueues.set(key, queue);
    const resp: MergeQueue = { patches: queue };
    return c.json(resp);
  });

  return app;
}
