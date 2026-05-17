import { useMemo } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import type { ListDocumentPathsResponse, PathChildEntry } from "@hydra/api";
import { apiClient } from "../../api/client";

const ROOT = "/";

function normalize(p: string | null): string {
  return p == null ? ROOT : p;
}

function joinPrefix(prefix: string): string {
  return prefix === ROOT ? "/" : `${prefix}/`;
}

/**
 * Group the flat union returned by `/v1/documents/paths?prefixes=…` by
 * immediate-parent prefix. An entry belongs to prefix `P` iff its `full_path`
 * sits exactly one segment past `P`.
 */
function groupByPrefix(
  prefixes: string[],
  entries: PathChildEntry[],
): Map<string, PathChildEntry[]> {
  const map = new Map<string, PathChildEntry[]>();
  for (const p of prefixes) map.set(p, []);
  for (const entry of entries) {
    for (const p of prefixes) {
      const boundary = joinPrefix(p);
      if (!entry.full_path.startsWith(boundary)) continue;
      const rest = entry.full_path.slice(boundary.length);
      if (rest.length === 0 || rest.includes("/")) continue;
      map.get(p)!.push(entry);
      break;
    }
  }
  return map;
}

export interface BatchedDocumentPaths {
  childrenMap: Map<string, PathChildEntry[]>;
  getChildren: (prefix: string | null) => PathChildEntry[];
  isLoading: boolean;
  isFetching: boolean;
  error: Error | null;
}

/**
 * Fetch path-tree children for many prefixes in a single request. `null`
 * denotes the root; the returned map keys it as `"/"`.
 */
export function useBatchedDocumentPaths(
  prefixes: (string | null)[],
): BatchedDocumentPaths {
  const stablePrefixes = useMemo(() => {
    const set = new Set<string>();
    for (const p of prefixes) set.add(normalize(p));
    return [...set].sort();
  }, [prefixes]);

  const prefixesCsv = stablePrefixes.join(",");

  const query = useQuery<ListDocumentPathsResponse, Error>({
    queryKey: ["documentPathsBatch", prefixesCsv],
    queryFn: () => apiClient.listDocumentPaths({ prefixes: prefixesCsv }),
    enabled: stablePrefixes.length > 0,
    placeholderData: keepPreviousData,
  });

  const childrenMap = useMemo(
    () => groupByPrefix(stablePrefixes, query.data?.children ?? []),
    [stablePrefixes, query.data],
  );

  const getChildren = useMemo(
    () =>
      (prefix: string | null): PathChildEntry[] =>
        childrenMap.get(normalize(prefix)) ?? [],
    [childrenMap],
  );

  return {
    childrenMap,
    getChildren,
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    error: query.error ?? null,
  };
}
