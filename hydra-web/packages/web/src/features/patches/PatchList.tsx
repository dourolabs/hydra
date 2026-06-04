import { Link } from "react-router-dom";
import { Badge, Spinner } from "@hydra/ui";
import { normalizePatchStatus } from "../../utils/badgeStatus";
import { useIssuePatches } from "./useIssuePatches";
import styles from "./PatchList.module.css";

interface PatchListProps {
  issueId: string;
}

export function PatchList({ issueId }: PatchListProps) {
  const { data: patches, isLoading, error } = useIssuePatches(issueId);

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
      {patches.map((record) => {
        const patchUrl = `/patches/${record.patch_id}?issueId=${issueId}`;
        return (
          <li key={record.patch_id} className={styles.item}>
            <Badge status={normalizePatchStatus(record.patch.status)} />
            <Link to={patchUrl} className={styles.id}>
              {record.patch_id}
            </Link>
            <span className={styles.title}>{record.patch.title}</span>
            {record.patch.github?.url && (
              <a
                href={record.patch.github.url}
                target="_blank"
                rel="noopener noreferrer"
                className={styles.prLink}
              >
                GitHub PR ↗
              </a>
            )}
          </li>
        );
      })}
    </ul>
  );
}
