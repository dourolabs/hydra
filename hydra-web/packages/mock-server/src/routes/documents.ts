import { Hono } from "hono";
import type { Store } from "../store.js";
import { generateId } from "../id.js";
import type {
  Document,
  UpsertDocumentRequest,
  UpsertDocumentResponse,
  DocumentVersionRecord,
  ListDocumentsResponse,
  ListDocumentVersionsResponse,
  ListDocumentPathsResponse,
  PathChildEntry,
  DocumentSummaryRecord,
  DocumentSummary,
} from "@hydra/api";

const COLLECTION = "documents";
const SSE_PREFIX = "document";

function toVersionRecord(
  documentId: string,
  version: number,
  timestamp: string,
  document: Document,
  creationTime: string,
): DocumentVersionRecord {
  return {
    document_id: documentId,
    version: BigInt(version),
    timestamp,
    document,
    creation_time: creationTime,
  };
}

function toSummaryRecord(
  documentId: string,
  version: number,
  timestamp: string,
  document: Document,
  creationTime: string,
): DocumentSummaryRecord {
  const summary: DocumentSummary = {
    title: document.title,
    path: document.path,
    deleted: document.deleted,
  };
  return {
    document_id: documentId,
    version: BigInt(version),
    timestamp,
    document: summary,
    creation_time: creationTime,
  };
}

export function createDocumentRoutes(store: Store): Hono {
  const app = new Hono();

  // POST /v1/documents
  app.post("/v1/documents", async (c) => {
    const body = await c.req.json<UpsertDocumentRequest>();
    const id = generateId("document");
    const entry = store.create<Document>(COLLECTION, id, body.document, SSE_PREFIX);
    const resp: UpsertDocumentResponse = {
      document_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp, 201);
  });

  // PUT /v1/documents/:id
  app.put("/v1/documents/:id", async (c) => {
    const id = c.req.param("id");
    const body = await c.req.json<UpsertDocumentRequest>();
    const entry = store.update<Document>(COLLECTION, id, body.document, SSE_PREFIX);
    const resp: UpsertDocumentResponse = {
      document_id: id,
      version: BigInt(entry.version),
    };
    return c.json(resp);
  });

  // GET /v1/documents/paths — must be registered BEFORE /v1/documents/:id
  app.get("/v1/documents/paths", (c) => {
    // `prefix` and `prefixes` are mutually exclusive. `prefixes` is a
    // comma-separated list (matching the Rust serde-helper encoding).
    const prefixParam = c.req.query("prefix");
    const prefixesParam = c.req.query("prefixes");
    if (prefixParam && prefixesParam) {
      return c.json(
        { error: "specify either `prefix` or `prefixes`, not both" },
        400,
      );
    }
    const rawPrefixes: string[] = prefixesParam
      ? prefixesParam.split(",").map((p) => p.trim()).filter((p) => p.length > 0)
      : prefixParam
        ? [prefixParam]
        : ["/"];
    const items = store.list<Document>(COLLECTION, false);

    // Index path -> document for is_document entries (used to populate the
    // inline `document` ref).
    const docByPath = new Map<string, { id: string; title: string }>();
    for (const { id, entry } of items) {
      if (entry.data.path) {
        docByPath.set(entry.data.path, { id, title: entry.data.title });
      }
    }

    const children: PathChildEntry[] = [];
    const seenPaths = new Set<string>();
    for (const prefix of rawPrefixes) {
      const normalizedPrefix = prefix.endsWith("/") ? prefix : `${prefix}/`;
      const segmentCounts = new Map<string, number>();
      const segmentIsDoc = new Map<string, boolean>();
      for (const { entry } of items) {
        const docPath = entry.data.path;
        if (!docPath || !docPath.startsWith(normalizedPrefix)) continue;
        const rest = docPath.slice(normalizedPrefix.length);
        if (!rest) continue;
        const slashIdx = rest.indexOf("/");
        const segment = slashIdx >= 0 ? rest.slice(0, slashIdx) : rest;
        segmentCounts.set(segment, (segmentCounts.get(segment) || 0) + 1);
        if (docPath === `${normalizedPrefix}${segment}`) {
          segmentIsDoc.set(segment, true);
        }
      }

      const entries = Array.from(segmentCounts.entries())
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([name, child_count]) => {
          const full_path = `${normalizedPrefix}${name}`;
          const is_document = segmentIsDoc.get(name) || false;
          const docRef = is_document ? docByPath.get(full_path) : undefined;
          const childEntry: PathChildEntry = {
            name,
            full_path,
            child_count: BigInt(child_count),
            is_document,
          };
          if (docRef) {
            childEntry.document = {
              document_id: docRef.id,
              title: docRef.title,
            };
          }
          return childEntry;
        });

      for (const entry of entries) {
        if (seenPaths.has(entry.full_path)) continue;
        seenPaths.add(entry.full_path);
        children.push(entry);
      }
    }

    const resp: ListDocumentPathsResponse = { children };
    return c.json(resp);
  });

  // GET /v1/documents/:id
  app.get("/v1/documents/:id", (c) => {
    const id = c.req.param("id");
    const includeDeleted = c.req.query("include_deleted") === "true";
    const entry = store.get<Document>(COLLECTION, id, includeDeleted);
    if (!entry) {
      return c.json({ error: `document '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  // GET /v1/documents/:id/versions/:version
  app.get("/v1/documents/:id/versions/:version", (c) => {
    const id = c.req.param("id");
    const version = Number(c.req.param("version"));
    const entry = store.getVersion<Document>(COLLECTION, id, version);
    if (!entry) {
      return c.json({ error: `document '${id}' version ${version} not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  // GET /v1/documents
  app.get("/v1/documents", (c) => {
    const includeDeleted = c.req.query("include_deleted") === "true";
    const ids = c.req.query("ids");
    const pathPrefix = c.req.query("path_prefix");
    const pathIsExact = c.req.query("path_is_exact") === "true";
    const q = c.req.query("q");
    const limitParam = c.req.query("limit");
    const cursorParam = c.req.query("cursor");
    const countParam = c.req.query("count");
    const items = store.list<Document>(COLLECTION, includeDeleted);

    let filtered = items;
    if (ids) {
      const idSet = new Set(ids.split(",").map((s) => s.trim()));
      filtered = filtered.filter(({ id }) => idSet.has(id));
    }
    if (pathPrefix) {
      filtered = filtered.filter(({ entry }) => {
        const docPath = entry.data.path;
        if (!docPath) return false;
        if (pathIsExact) return docPath === pathPrefix;
        return docPath.startsWith(pathPrefix);
      });
    }
    if (q) {
      const lower = q.toLowerCase();
      filtered = filtered.filter(({ entry }) => {
        const titleMatch = entry.data.title.toLowerCase().includes(lower);
        const pathMatch = entry.data.path?.toLowerCase().includes(lower) ?? false;
        return titleMatch || pathMatch;
      });
    }

    // Sort by last-update time descending (most recently updated first) for stable pagination
    filtered.sort((a, b) => {
      return b.entry.timestamp.localeCompare(a.entry.timestamp);
    });

    const totalCount = filtered.length;

    // Apply cursor-based pagination
    if (cursorParam) {
      const cursorIndex = filtered.findIndex(({ id }) => id === cursorParam);
      if (cursorIndex !== -1) {
        filtered = filtered.slice(cursorIndex + 1);
      }
    }

    let nextCursor: string | null = null;
    if (limitParam !== undefined && limitParam !== null) {
      const limit = Number(limitParam);
      if (Number.isFinite(limit) && limit >= 0 && filtered.length > limit) {
        nextCursor = filtered[limit - 1]?.id ?? null;
        filtered = filtered.slice(0, limit);
      }
    }

    const documents: DocumentSummaryRecord[] = filtered.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListDocumentsResponse = {
      documents,
      next_cursor: nextCursor,
      total_count: countParam === "true" ? BigInt(totalCount) : undefined,
    };
    return c.json(resp);
  });

  // GET /v1/documents/:id/versions
  app.get("/v1/documents/:id/versions", (c) => {
    const id = c.req.param("id");
    const allVersions = store.listVersions<Document>(COLLECTION, id);
    if (allVersions.length === 0) {
      return c.json({ error: `document '${id}' not found` }, 404);
    }
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    const versions = allVersions.map((v) =>
      toVersionRecord(id, v.version, v.timestamp, v.data, creationTime),
    );
    const resp: ListDocumentVersionsResponse = { versions };
    return c.json(resp);
  });

  // DELETE /v1/documents/:id
  app.delete("/v1/documents/:id", (c) => {
    const id = c.req.param("id");
    const entry = store.delete<Document>(COLLECTION, id, SSE_PREFIX);
    const creationTime = store.getCreationTime(COLLECTION, id)!;
    return c.json(
      toVersionRecord(id, entry.version, entry.timestamp, entry.data, creationTime),
    );
  });

  return app;
}
