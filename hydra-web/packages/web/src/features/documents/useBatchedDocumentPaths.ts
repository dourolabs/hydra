import { useCallback, useMemo } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import type { ListDocumentPathsResponse, PathChildEntry } from "@hydra/api";
import { apiClient } from "../../api/client";

const ROOT_PREFIX = "/";

function normalize(prefix: string | null): string {
  if (prefix == null || prefix === "" || prefix === ROOT_PREFIX) {
    return ROOT_PREFIX;
  }
  return prefix.endsWith("/") ? prefix : `${prefix}/`;
}

function isImmediateChild(fullPath: string, normPrefix: string): boolean {
  if (!fullPath.startsWith(normPrefix)) return false;
  const rest = fullPath.slice(normPrefix.length);
  return rest.length > 0 && !rest.includes("/");
}

export interface BatchedDocumentPaths {
  data: ListDocumentPathsResponse | undefined;
  isLoading: boolean;
  error: Error | null;
  /**
   * Immediate children of the given prefix, derived from the single flat
   * response. `null` represents the root listing (equivalent to "/").
   */
  getChildren: (prefix: string | null) => PathChildEntry[];
}

/**
 * Fire a single batched `GET /v1/documents/paths?prefixes=…` for the union of
 * `prefixes`. The server returns one flat list (deduped by `full_path`); the
 * hook splits it back into per-prefix immediate children for callers.
 *
 * Pass `null` (or `"/"`) to request the root listing.
 */
export function useBatchedDocumentPaths(
  prefixes: Array<string | null>,
): BatchedDocumentPaths {
  const normalized = useMemo(() => {
    const set = new Set<string>();
    for (const p of prefixes) set.add(normalize(p));
    return Array.from(set).sort();
  }, [prefixes]);

  const queryParam = useMemo(() => normalized.join(","), [normalized]);

  const query = useQuery<ListDocumentPathsResponse, Error>({
    queryKey: ["documentPathsBatch", queryParam],
    queryFn: () => apiClient.listDocumentPaths({ prefixes: queryParam }),
    enabled: normalized.length > 0,
    // Keep showing previously fetched children while a new union refetches,
    // so expanding/collapsing a folder doesn't unmount the tree.
    placeholderData: keepPreviousData,
  });

  const children = query.data?.children;
  const getChildren = useCallback(
    (prefix: string | null): PathChildEntry[] => {
      const norm = normalize(prefix);
      return (children ?? []).filter((entry) =>
        isImmediateChild(entry.full_path, norm),
      );
    },
    [children],
  );

  return {
    data: query.data,
    isLoading: query.isLoading,
    error: query.error,
    getChildren,
  };
}
