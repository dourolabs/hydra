import { useMemo } from "react";
import { Link } from "react-router-dom";
import { Badge, Panel, Spinner } from "@metis/ui";
import type { PatchSummaryRecord } from "@metis/api";
import { usePatches } from "../features/patches/usePatches";
import { patchToBadgeStatus } from "../utils/statusMapping";
import { formatRelativeTime } from "../utils/time";
import styles from "./PatchesPage.module.css";

export function PatchesPage() {
  const { data: patches, isLoading, error } = usePatches();

  const sorted = useMemo(() => {
    if (!patches) return [];
    return [...patches]
      .filter((p) => !p.patch.deleted)
      .sort((a, b) => b.timestamp.localeCompare(a.timestamp));
  }, [patches]);

  return (
    <div className={styles.page}>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && <p className={styles.error}>Failed to load patches: {(error as Error).message}</p>}

      {patches && sorted.length === 0 && <p className={styles.empty}>No patches found.</p>}

      {sorted.length > 0 && (
        <Panel header={<span className={styles.sectionTitle}>Patches</span>}>
          <ul className={styles.patchList}>
            {sorted.map((record) => (
              <PatchRow key={record.patch_id} record={record} />
            ))}
          </ul>
        </Panel>
      )}
    </div>
  );
}

interface PatchRowProps {
  record: PatchSummaryRecord;
}

function PatchRow({ record }: PatchRowProps) {
  const { patch } = record;

  return (
    <li>
      <Link to={`/patches/${record.patch_id}`} className={styles.patchRow}>
        <span className={styles.patchTitle}>{patch.title}</span>
        <div className={styles.patchStatus}>
          <Badge status={patchToBadgeStatus(patch.status)} />
          <span className={styles.patchId}>{record.patch_id}</span>
        </div>
        <div className={styles.patchMeta}>
          {patch.branch_name && <span className={styles.branch}>{patch.branch_name}</span>}
          {patch.service_repo_name && (
            <span className={styles.repo}>{patch.service_repo_name}</span>
          )}
          {patch.github?.url && (
            <a
              href={patch.github.url}
              target="_blank"
              rel="noopener noreferrer"
              className={styles.prLink}
              onClick={(e) => e.stopPropagation()}
            >
              PR #{String(patch.github.number)}
            </a>
          )}
          <span className={styles.time}>{formatRelativeTime(record.timestamp)}</span>
        </div>
      </Link>
    </li>
  );
}
