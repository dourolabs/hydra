import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { DocumentPath } from "@hydra/api";
import { ApiError, apiClient } from "../../api/client";

/**
 * Load the markdown body of the prompt document at `loadPath` so an inline
 * editor can be seeded with it. Seeds the local body exactly once per path so a
 * background refetch never clobbers in-progress edits. A 404 (no document yet)
 * seeds an empty body. `loadPath` should be a stable path (the entity's path as
 * opened), not one that changes as the user renames — that would refetch and
 * discard their edits.
 */
export function usePromptDocumentBody(loadPath: string | null) {
  const [body, setBody] = useState("");
  const seededPathRef = useRef<string | null>(null);
  const query = useQuery({
    queryKey: ["documentByPath", loadPath],
    queryFn: async () => {
      if (!loadPath) return null;
      try {
        return await apiClient.getDocumentByPath(loadPath);
      } catch (err) {
        if (err instanceof ApiError && err.status === 404) return null;
        throw err;
      }
    },
    enabled: !!loadPath,
  });

  useEffect(() => {
    if (!query.isSuccess) return;
    if (seededPathRef.current === loadPath) return;
    seededPathRef.current = loadPath;
    setBody(query.data?.document.body_markdown ?? "");
  }, [query.isSuccess, query.data, loadPath]);

  return { body, setBody, loading: query.isLoading };
}

/**
 * Upsert the prompt document at `path` with `body`. 404 → createDocument,
 * otherwise updateDocument. Shared by the status and project prompt editors.
 */
export async function upsertPromptDoc(path: string, body: string) {
  try {
    const existing = await apiClient.getDocumentByPath(path);
    await apiClient.updateDocument(existing.document_id, {
      document: {
        ...existing.document,
        body_markdown: body,
        path: path as DocumentPath,
      },
    });
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) {
      await apiClient.createDocument({
        document: {
          title: path,
          body_markdown: body,
          path: path as DocumentPath,
        },
      });
    } else {
      throw err;
    }
  }
}
