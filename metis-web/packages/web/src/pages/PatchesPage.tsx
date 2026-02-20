import { Link } from "react-router-dom";
import { Panel, Spinner, Badge } from "@metis/ui";
import { usePatches } from "../features/patches/usePatches";
import { patchToBadgeStatus } from "../utils/statusMapping";
import { descriptionSnippet } from "../utils/text";
import styles from "./PatchesPage.module.css";

export function PatchesPage() {
  const { data: patches, isLoading, error } = usePatches();

  return (
    <div className={styles.page}>
      <h1 className={styles.title}>Patches</h1>
      <Panel>
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
            {patches.map((p) => (
              <Link
                key={p.patch_id}
                to={`/patches/${p.patch_id}`}
                className={styles.item}
              >
                <div className={styles.itemHeader}>
                  <Badge status={patchToBadgeStatus(p.patch.status)} />
                  <span className={styles.patchId}>{p.patch_id}</span>
                </div>
                <span className={styles.patchTitle}>{p.patch.title}</span>
                {p.patch.description && (
                  <span className={styles.patchDesc}>
                    {descriptionSnippet(p.patch.description)}
                  </span>
                )}
              </Link>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}
