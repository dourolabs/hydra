import { Hono } from "hono";
import type { Store } from "../store.js";
import type {
  UpsertLabelRequest,
  UpsertLabelResponse,
  LabelRecord,
  ListLabelsResponse,
  LabelSummary,
  Issue,
} from "@metis/api";

const COLLECTION = "labels";

interface LabelData {
  name: string;
  color: string;
  recurse: boolean;
  hidden: boolean;
}

interface LabelAssociation {
  label_id: string;
  object_id: string;
}

// In-memory label associations (not versioned)
const associations: LabelAssociation[] = [];

function generateLabelId(): string {
  const suffix = Math.random().toString(36).slice(2, 11);
  return `l-${suffix}`;
}

function defaultColor(name: string): string {
  const palette = [
    "#e74c3c", "#3498db", "#2ecc71", "#f39c12",
    "#9b59b6", "#1abc9c", "#e67e22", "#34495e",
  ];
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash * 31 + name.charCodeAt(i)) | 0;
  }
  return palette[Math.abs(hash) % palette.length];
}

export function getLabelsForObject(objectId: string): LabelSummary[] {
  const labelIds = associations
    .filter((a) => a.object_id === objectId)
    .map((a) => a.label_id);
  // We need access to the store but this function is called from outside
  // So we store a reference
  if (!_store) return [];
  const summaries: LabelSummary[] = [];
  for (const labelId of labelIds) {
    const entry = _store.get<LabelData>(COLLECTION, labelId);
    if (entry) {
      summaries.push({
        label_id: labelId,
        name: entry.data.name,
        color: entry.data.color,
        recurse: entry.data.recurse,
        hidden: entry.data.hidden,
      });
    }
  }
  return summaries;
}

let _store: Store | null = null;

export function clearAssociations(): void {
  associations.length = 0;
}

export function addAssociation(labelId: string, objectId: string): void {
  if (!associations.some((a) => a.label_id === labelId && a.object_id === objectId)) {
    associations.push({ label_id: labelId, object_id: objectId });
  }
}

/**
 * Resolve label names to IDs, creating any that don't exist, and associate them
 * with the given object. Mirrors the real server's label_names handling.
 */
export function resolveLabelNames(store: Store, names: string[], objectId: string): void {
  _store = store;
  for (const name of names) {
    // Find existing label by name
    const items = store.list<LabelData>(COLLECTION);
    const existing = items.find(({ entry }) => entry.data.name === name);
    let labelId: string;
    if (existing) {
      labelId = existing.id;
    } else {
      // Create the label
      labelId = generateLabelId();
      const labelData: LabelData = {
        name,
        color: defaultColor(name),
        recurse: true,
        hidden: false,
      };
      store.create<LabelData>(COLLECTION, labelId, labelData, null);
    }
    addAssociation(labelId, objectId);
  }
}

export function createLabelRoutes(store: Store): Hono {
  _store = store;
  const app = new Hono();

  // POST /v1/labels
  app.post("/v1/labels", async (c) => {
    const body = await c.req.json<UpsertLabelRequest>();
    const id = generateLabelId();
    const labelData: LabelData = {
      name: body.label.name,
      color: body.label.color ?? defaultColor(body.label.name),
      recurse: body.label.recurse ?? true,
      hidden: body.label.hidden ?? false,
    };
    store.create<LabelData>(COLLECTION, id, labelData, null);
    const resp: UpsertLabelResponse = { label_id: id };
    return c.json(resp, 201);
  });

  // GET /v1/labels
  app.get("/v1/labels", (c) => {
    const items = store.list<LabelData>(COLLECTION);
    const labels: LabelRecord[] = items.map(({ id, entry }) => ({
      label_id: id,
      name: entry.data.name,
      color: entry.data.color,
      recurse: entry.data.recurse,
      hidden: entry.data.hidden,
      created_at: entry.timestamp,
      updated_at: entry.timestamp,
    }));
    const resp: ListLabelsResponse = { labels };
    return c.json(resp);
  });

  // GET /v1/labels/:id
  app.get("/v1/labels/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.get<LabelData>(COLLECTION, id);
    if (!entry) {
      return c.json({ error: `label '${id}' not found` }, 404);
    }
    const record: LabelRecord = {
      label_id: id,
      name: entry.data.name,
      color: entry.data.color,
      recurse: entry.data.recurse,
      hidden: entry.data.hidden,
      created_at: entry.timestamp,
      updated_at: entry.timestamp,
    };
    return c.json(record);
  });

  // PUT /v1/labels/:labelId/objects/:objectId
  app.put("/v1/labels/:labelId/objects/:objectId", (c) => {
    const labelId = c.req.param("labelId");
    const objectId = c.req.param("objectId");
    const cascade = c.req.query("cascade") === "true";

    addAssociation(labelId, objectId);

    // If cascade=true and object is an issue, add to all children
    if (cascade && objectId.startsWith("i-")) {
      const allIssues = store.list<Issue>("issues");
      const childIds = findChildren(objectId, allIssues.map(({ id, entry }) => ({ id, issue: entry.data })));
      for (const childId of childIds) {
        addAssociation(labelId, childId);
      }
    }

    return c.json({ ok: true });
  });

  // DELETE /v1/labels/:labelId/objects/:objectId
  app.delete("/v1/labels/:labelId/objects/:objectId", (c) => {
    const labelId = c.req.param("labelId");
    const objectId = c.req.param("objectId");
    const idx = associations.findIndex(
      (a) => a.label_id === labelId && a.object_id === objectId,
    );
    if (idx >= 0) {
      associations.splice(idx, 1);
    }
    return c.json({ ok: true });
  });

  return app;
}

function findChildren(
  parentId: string,
  issues: { id: string; issue: Issue }[],
): string[] {
  const children: string[] = [];
  for (const { id, issue } of issues) {
    if (issue.dependencies?.some((d: { type: string; issue_id: string }) => d.type === "child-of" && d.issue_id === parentId)) {
      children.push(id);
      children.push(...findChildren(id, issues));
    }
  }
  return children;
}
