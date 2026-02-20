import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Badge, Spinner } from "@metis/ui";
import { apiClient } from "../api/client";
import { patchToBadgeStatus } from "../utils/statusMapping";
import styles from "./PatchesPage.module.css";

export function PatchesPage() {
  const navigate = useNavigate();
  const { data: patches, isLoading, error } = useQuery({
    queryKey: ["patches"],
    queryFn: () => apiClient.listPatches(),
    select: (data) => data.patches,
  });

  return (
    <div className={styles.page}>
      <h2 className={styles.title}>Patches</h2>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}
      {error && (
        <p className={styles.error}>Failed to load patches: {(error as Error).message}</p>
      )}
      {patches && patches.length === 0 && (
        <p className={styles.empty}>No patches found.</p>
      )}
      {patches && patches.length > 0 && (
        <div className={styles.list}>
          {patches.map((record) => (
            <button
              key={record.patch_id}
              className={styles.item}
              onClick={() => navigate(`/patches/${record.patch_id}`)}
            >
              <div className={styles.itemHeader}>
                <span className={styles.itemTitle}>{record.patch.title || record.patch_id}</span>
                <Badge status={patchToBadgeStatus(record.patch.status)} />
              </div>
              <div className={styles.itemMeta}>
                <span className={styles.itemId}>{record.patch_id}</span>
                <span className={styles.itemCreator}>{record.patch.creator}</span>
                <span className={styles.itemTime}>{new Date(record.timestamp).toLocaleDateString()}</span>
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
