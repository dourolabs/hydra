import { useParams, useSearchParams } from "react-router-dom";
import { Spinner } from "@metis/ui";
import { usePatch } from "../features/patches/usePatch";
import { PatchDetail } from "../features/patches/PatchDetail";
import { ApiError } from "../api/client";
import { Breadcrumbs, type BreadcrumbItem } from "../layout/Breadcrumbs";
import styles from "./PatchDetailPage.module.css";

export function PatchDetailPage() {
  const { patchId } = useParams<{ patchId: string }>();
  const [searchParams] = useSearchParams();
  const issueId = searchParams.get("issueId");
  const fromDashboard = searchParams.get("from") === "dashboard";
  const { data: record, isLoading, error } = usePatch(patchId ?? "");

  let breadcrumbItems: BreadcrumbItem[];
  if (fromDashboard && issueId) {
    breadcrumbItems = [
      { label: "Dashboard", to: `/?selected=${issueId}` },
      { label: `Issue ${issueId}`, to: `/issues/${issueId}?from=dashboard` },
    ];
  } else if (issueId) {
    breadcrumbItems = [
      { label: "Issues", to: "/issues" },
      { label: `Issue ${issueId}`, to: `/issues/${issueId}` },
    ];
  } else {
    breadcrumbItems = [{ label: "Patches", to: "/patches" }];
  }

  return (
    <div className={styles.page}>
      <Breadcrumbs items={breadcrumbItems} current={`Patch ${patchId}`} />

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
