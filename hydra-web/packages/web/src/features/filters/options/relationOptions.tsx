import { Icons } from "@hydra/ui";
import type { FilterOption } from "../types";
import styles from "./relationOptions.module.css";

export type RelationKind = "issue" | "patch" | "session" | "chat" | "document";

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
