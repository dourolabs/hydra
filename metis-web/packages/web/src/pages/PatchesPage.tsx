import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Badge, Panel, Spinner } from "@metis/ui";
import { apiClient } from "../api/client";
import { patchToBadgeStatus } from "../utils/statusMapping";
import { formatTimestamp } from "../utils/time";
import styles from "./PatchesPage.module.css";

function usePatches() {
  return useQuery({
    queryKey: ["patches"],
    queryFn: () => apiClient.listPatches(),
    select: (data) => data.patches,
  });
}

export function PatchesPage() {
  const { data: patches, isLoading, error } = usePatches();

  return (
    <div className={styles.page}>
      <Panel header={<span className={styles.header}>Patches</span>}>
        {isLoading && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}
        {error && (
          <p className={styles.error}>
            Failed to load patches: {(error as Error).message}
          </p>
        )}
        {patches && patches.length === 0 && (
          <p className={styles.empty}>No patches found.</p>
        )}
        {patches && patches.length > 0 && (
          <ul className={styles.list}>
            {patches.map((record) => (
              <li key={record.patch_id} className={styles.item}>
                <Badge status={patchToBadgeStatus(record.patch.status)} />
                <Link to={`/patches/${record.patch_id}`} className={styles.id}>
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
                    GitHub PR
                  </a>
                )}
                <span className={styles.time}>{formatTimestamp(record.timestamp)}</span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}
