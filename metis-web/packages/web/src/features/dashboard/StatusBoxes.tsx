import type { ChildStatus } from "./computeIssueProgress";
import styles from "./StatusBoxes.module.css";

function getBoxClass(child: ChildStatus): string {
  if (child.hasActiveTask) return styles.statusBoxActive;
  if (child.assignedToUser && child.status === "open") return styles.statusBoxAttention;
  if (child.status === "closed") return styles.statusBoxClosed;
  if (child.status === "in-progress") return styles.statusBoxInProgress;
  if (child.status === "failed") return styles.statusBoxFailed;
  return styles.statusBoxOpen;
}

export function StatusBoxes({ children }: { children: ChildStatus[] }) {
  if (children.length === 0) return null;

  return (
    <span className={styles.statusBoxes}>
      {children.map((child) => (
        <span key={child.id} className={`${styles.statusBox} ${getBoxClass(child)}`} />
      ))}
    </span>
  );
}
