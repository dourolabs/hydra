import { Link, useParams, useSearchParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { usePatch } from "../features/patches/usePatch";
import { PatchDetail } from "../features/patches/PatchDetail";
import { ApiError } from "../api/client";
import styles from "./PatchDetailPage.module.css";

export function PatchDetailPage() {
  const { patchId } = useParams<{ patchId: string }>();
  const [searchParams] = useSearchParams();
  const issueId = searchParams.get("issueId");
  const { data: record, isLoading, error } = usePatch(patchId ?? "");

  const backTo = issueId ? `/issues/${issueId}` : "/";
  const backLabel = issueId
    ? `\u2190 Back to ${issueId}`
    : "\u2190 Back to dashboard";

  return (
    <div className={styles.page}>
      <Link to={backTo} className={styles.back}>
        {backLabel}
      </Link>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <div className={styles.errorContainer}>
          {error instanceof ApiError && error.status === 404 ? (
            <p className={styles.error}>
              Patch <strong>{patchId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load patch: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && (
        <PatchDetail record={record} referringIssueId={issueId ?? undefined} />
      )}
    </div>
  );
}
