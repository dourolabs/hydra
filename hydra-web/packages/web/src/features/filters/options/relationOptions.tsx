import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icons } from "@hydra/ui";
import { apiClient } from "../../../api/client";
import type { FilterOption } from "../types";
import styles from "./relationOptions.module.css";

export type RelationKind = "issue" | "patch" | "session" | "chat" | "document";

/**
 * Edge query for `useIssueRelationsIndex`. Each entry describes one relation
 * traversal — `rel_type` is the wire relation name, `direction` says whether
 * to follow the edge outbound (issue → x) or inbound (x → issue). The result
 * is a Map<issueId, Set<relatedId>> aggregating every relation that matched
 * any entry across the loaded issues, deduped.
 */
export interface RelationEdge {
  rel_type: string;
  direction: "outbound" | "inbound";
}

/**
 * Fetches all `has-patch`-style relations whose source (or target) is any of
 * `issueIds`, builds Map<issueId, Set<relatedId>>. One bulk request per edge;
 * keyed on the sorted issueId list so navigation invalidates correctly.
 */
export function useIssueRelationsIndex(
  issueIds: string[],
  edges: RelationEdge[],
): { index: Map<string, Set<string>>; isLoading: boolean } {
  const idsParam = useMemo(() => [...issueIds].sort().join(","), [issueIds]);
  const edgeKey = useMemo(
    () => edges.map((e) => `${e.direction}:${e.rel_type}`).join("|"),
    [edges],
  );

  const query = useQuery({
    queryKey: ["filter-relations-index", edgeKey, idsParam],
    queryFn: async () => {
      if (!idsParam) return [] as { issueId: string; relatedId: string }[];
      const out: { issueId: string; relatedId: string }[] = [];
      for (const edge of edges) {
        const params =
          edge.direction === "outbound"
            ? { source_ids: idsParam, rel_type: edge.rel_type }
            : { target_ids: idsParam, rel_type: edge.rel_type };
        const resp = await apiClient.listRelations(params);
        for (const rel of resp.relations) {
          if (edge.direction === "outbound") {
            out.push({ issueId: rel.source_id, relatedId: rel.target_id });
          } else {
            out.push({ issueId: rel.target_id, relatedId: rel.source_id });
          }
        }
      }
      return out;
    },
    enabled: !!idsParam,
    staleTime: 30_000,
  });

  const index = useMemo(() => {
    const map = new Map<string, Set<string>>();
    for (const { issueId, relatedId } of query.data ?? []) {
      const set = map.get(issueId) ?? new Set<string>();
      set.add(relatedId);
      map.set(issueId, set);
    }
    return map;
  }, [query.data]);

  return { index, isLoading: query.isLoading };
}

/**
 * Builds the option list for a relation filter from an entity list. Each
 * option's `chip` / `render` show a kind-specific icon + monospaced id +
 * title; `sub` carries a searchable secondary string (e.g. agent, branch).
 */
export interface RelationEntity {
  id: string;
  title: string;
  sub?: string;
}

function iconFor(kind: RelationKind) {
  if (kind === "issue") return Icons.IconIssue;
  if (kind === "patch") return Icons.IconPatch;
  if (kind === "session") return Icons.IconAgent;
  if (kind === "chat") return Icons.IconChat;
  return Icons.IconDoc;
}

export function relationOptionsFromEntities(
  kind: RelationKind,
  entities: RelationEntity[],
): FilterOption[] {
  const Icon = iconFor(kind);
  return entities.map((entity) => ({
    value: entity.id,
    label: entity.title,
    sub: entity.sub,
    chip: (
      <span className={styles.chip}>
        <Icon size={10} />
        <span className={styles.id}>{entity.id}</span>
      </span>
    ),
    render: (
      <span className={styles.row}>
        <Icon size={12} />
        <span className={styles.id}>{entity.id}</span>
        <span className={styles.title}>{entity.title}</span>
      </span>
    ),
  }));
}
