import type { ChildStatus } from "./computeIssueProgress";
import styles from "./StatusBoxes.module.css";

function getBoxClass(child: ChildStatus): string {
  if (child.hasActiveTask) return styles.statusBoxActive;
  if (child.assignedToUser && (child.status === "open" || child.status === "in-progress")) return styles.statusBoxAttention;
  if (child.status === "closed") return styles.statusBoxClosed;
  if (child.status === "in-progress") return styles.statusBoxInProgress;
  if (child.status === "failed") return styles.statusBoxFailed;
  return styles.statusBoxOpen;
}

function sortKey(child: ChildStatus): number {
  if (child.status === "closed") return 0;
  if (child.status === "failed" || child.status === "dropped" || child.status === "rejected") return 1;
  if (child.assignedToUser && (child.status === "open" || child.status === "in-progress")) return 2;
  if (child.hasActiveTask) return 3;
  if (child.status === "in-progress") return 4;
  return 5; // open
}

export function StatusBoxes({ children }: { children: ChildStatus[] }) {
  if (children.length === 0) return null;

  const sorted = [...children].sort((a, b) => sortKey(a) - sortKey(b));

  return (
    <span className={styles.statusBoxes}>
      {sorted.map((child) => (
        <span key={child.id} className={`${styles.statusBox} ${getBoxClass(child)}`} />
      ))}
    </span>
  );
}
