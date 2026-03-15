import { Hono } from "hono";
import type { Store } from "../store.js";
import type { Issue } from "@metis/api";

interface RelationResponse {
  source_id: string;
  target_id: string;
  rel_type: string;
}

interface ListRelationsResponse {
  relations: RelationResponse[];
}

/**
 * Build relations from issue dependencies stored in the issue entities.
 * For a dependency { type: "child-of", issue_id: parentId } on issue childId,
 * we emit a relation { source_id: childId, target_id: parentId, rel_type: "child-of" }.
 */
function buildRelationsFromIssues(store: Store): RelationResponse[] {
  const items = store.list<Issue>("issues", false);
  const relations: RelationResponse[] = [];
  for (const { id, entry } of items) {
    for (const dep of entry.data.dependencies ?? []) {
      relations.push({
        source_id: id,
        target_id: dep.issue_id,
        rel_type: dep.type,
      });
    }
    // Patch relations: issue owns patches
    for (const patchId of entry.data.patches ?? []) {
      relations.push({
        source_id: patchId,
        target_id: id,
        rel_type: "patch-for",
      });
    }
  }
  return relations;
}

function findTransitive(
  startIds: Set<string>,
  relations: RelationResponse[],
  relType: string,
  direction: "source" | "target",
): RelationResponse[] {
  // Walk transitively: if direction=source, we look for relations where target_id is in our set
  // and expand source_id into the set.
  const visited = new Set<string>(startIds);
  const result: RelationResponse[] = [];
  const queue = [...startIds];

  while (queue.length > 0) {
    const current = queue.shift()!;
    for (const rel of relations) {
      if (rel.rel_type !== relType) continue;
      if (direction === "source") {
        // We queried by target_ids, expand via source_id
        if (rel.target_id === current && !visited.has(rel.source_id)) {
          visited.add(rel.source_id);
          queue.push(rel.source_id);
          result.push(rel);
        }
      } else {
        // We queried by source_ids, expand via target_id
        if (rel.source_id === current && !visited.has(rel.target_id)) {
          visited.add(rel.target_id);
          queue.push(rel.target_id);
          result.push(rel);
        }
      }
    }
  }
  // Also include direct matches
  for (const rel of relations) {
    if (rel.rel_type !== relType) continue;
    if (direction === "source" && startIds.has(rel.target_id)) {
      if (!result.includes(rel)) result.push(rel);
    } else if (direction === "target" && startIds.has(rel.source_id)) {
      if (!result.includes(rel)) result.push(rel);
    }
  }
  return result;
}

export function createRelationRoutes(store: Store): Hono {
  const app = new Hono();

  app.get("/v1/relations", (c) => {
    const sourceId = c.req.query("source_id");
    const sourceIds = c.req.query("source_ids");
    const targetId = c.req.query("target_id");
    const targetIds = c.req.query("target_ids");
    const objectId = c.req.query("object_id");
    const relType = c.req.query("rel_type");
    const transitive = c.req.query("transitive") === "true";

    const allRelations = buildRelationsFromIssues(store);

    let filtered: RelationResponse[];

    if (transitive && relType && (sourceId || targetId || sourceIds || targetIds)) {
      const ids = new Set<string>();
      if (sourceId) ids.add(sourceId);
      if (sourceIds) sourceIds.split(",").forEach((s) => ids.add(s.trim()));
      if (targetId) ids.add(targetId);
      if (targetIds) targetIds.split(",").forEach((s) => ids.add(s.trim()));

      const direction = sourceId || sourceIds ? "target" : "source";
      filtered = findTransitive(ids, allRelations, relType, direction);
    } else {
      filtered = allRelations;

      if (relType) {
        filtered = filtered.filter((r) => r.rel_type === relType);
      }
      if (sourceId) {
        filtered = filtered.filter((r) => r.source_id === sourceId);
      }
      if (sourceIds) {
        const ids = new Set(sourceIds.split(",").map((s) => s.trim()));
        filtered = filtered.filter((r) => ids.has(r.source_id));
      }
      if (targetId) {
        filtered = filtered.filter((r) => r.target_id === targetId);
      }
      if (targetIds) {
        const ids = new Set(targetIds.split(",").map((s) => s.trim()));
        filtered = filtered.filter((r) => ids.has(r.target_id));
      }
      if (objectId) {
        filtered = filtered.filter(
          (r) => r.source_id === objectId || r.target_id === objectId,
        );
      }
    }

    const resp: ListRelationsResponse = { relations: filtered };
    return c.json(resp);
  });

  return app;
}
