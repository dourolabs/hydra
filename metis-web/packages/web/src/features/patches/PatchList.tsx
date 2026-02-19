import { Badge, Spinner, type BadgeStatus } from "@metis/ui";
import { usePatchesByIssue } from "./usePatchesByIssue";
import styles from "./PatchList.module.css";

interface PatchListProps {
  patchIds: string[];
}

/** Map patch statuses to BadgeStatus values. */
function toBadgeStatus(status: string): BadgeStatus {
  const mapped: Record<string, BadgeStatus> = {
    Open: "open",
    Merged: "closed",
    Closed: "failed",
    ChangesRequested: "rejected",
  };
  const s = mapped[status];
  if (s) return s;
  return "open";
}

export function PatchList({ patchIds }: PatchListProps) {
  const { data: patches, isLoading, error } = usePatchesByIssue(patchIds);

  if (patchIds.length === 0) {
    return <p className={styles.empty}>No patches.</p>;
  }

  if (isLoading) {
    return <Spinner size="sm" />;
  }

  if (error) {
    return (
      <p className={styles.error}>
        Failed to load patches: {(error as Error).message}
      </p>
    );
  }

  if (patches.length === 0) {
    return <p className={styles.empty}>No patches.</p>;
  }

  return (
    <ul className={styles.list}>
      {patches.map((patch) => (
        <li key={patch.patch_id} className={styles.item}>
          <Badge status={toBadgeStatus(patch.status)} />
          <span className={styles.id}>{patch.patch_id}</span>
          <span className={styles.title}>{patch.title}</span>
          {patch.github_url && (
            <a
              href={patch.github_url}
              target="_blank"
              rel="noopener noreferrer"
              className={styles.prLink}
            >
              GitHub PR ↗
            </a>
          )}
        </li>
      ))}
    </ul>
  );
}
