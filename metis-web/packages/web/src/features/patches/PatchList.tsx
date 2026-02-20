import { Link } from "react-router-dom";
import { Badge, Spinner } from "@metis/ui";
import { patchToBadgeStatus } from "../../utils/statusMapping";
import { usePatchesByIssue } from "./usePatchesByIssue";
import styles from "./PatchList.module.css";

interface PatchListProps {
  patchIds: string[];
  issueId?: string;
}

export function PatchList({ patchIds, issueId }: PatchListProps) {
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
      {patches.map((record) => {
        const patchUrl = issueId
          ? `/patches/${record.patch_id}?issueId=${issueId}`
          : `/patches/${record.patch_id}`;
        return (
          <li key={record.patch_id} className={styles.item}>
            <Badge status={patchToBadgeStatus(record.patch.status)} />
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
