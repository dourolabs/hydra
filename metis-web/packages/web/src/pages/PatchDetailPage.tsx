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
  const filterParam = searchParams.get("filter");
  const { data: record, isLoading, error } = usePatch(patchId ?? "");

  const dashboardReturnUrl = filterParam ? `/?selected=${filterParam}` : "/";

  let breadcrumbItems: BreadcrumbItem[];
  if (fromDashboard && issueId) {
    const issueParams = new URLSearchParams({ from: "dashboard" });
    if (filterParam) issueParams.set("filter", filterParam);
    breadcrumbItems = [
      { label: "Dashboard", to: dashboardReturnUrl },
      { label: `Issue ${issueId}`, to: `/issues/${issueId}?${issueParams.toString()}` },
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
