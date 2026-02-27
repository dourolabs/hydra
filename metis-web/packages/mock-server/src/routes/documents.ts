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
  DocumentSummaryRecord,
  DocumentSummary,
} from "@metis/api";

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
    created_by: document.created_by,
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
    const pathPrefix = c.req.query("path_prefix");
    const pathIsExact = c.req.query("path_is_exact") === "true";
    const items = store.list<Document>(COLLECTION, includeDeleted);

    let filtered = items;
    if (pathPrefix) {
      filtered = items.filter(({ entry }) => {
        const docPath = entry.data.path;
        if (!docPath) return false;
        if (pathIsExact) return docPath === pathPrefix;
        return docPath.startsWith(pathPrefix);
      });
    }

    const documents: DocumentSummaryRecord[] = filtered.map(({ id, entry }) => {
      const creationTime = store.getCreationTime(COLLECTION, id)!;
      return toSummaryRecord(id, entry.version, entry.timestamp, entry.data, creationTime);
    });
    const resp: ListDocumentsResponse = { documents };
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
