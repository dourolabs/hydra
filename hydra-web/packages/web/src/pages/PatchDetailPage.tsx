import { useParams, useSearchParams } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { usePatch } from "../features/patches/usePatch";
import { PatchDetail } from "../features/patches/PatchDetail";
import { ApiError } from "../api/client";
import type { BreadcrumbItem } from "../layout/Breadcrumbs";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./PatchDetailPage.module.css";

export function PatchDetailPage() {
  const { patchId } = useParams<{ patchId: string }>();
  const [searchParams] = useSearchParams();
  const issueId = searchParams.get("issueId");
  const fromDashboard = searchParams.get("from") === "dashboard";
  const filterParam = searchParams.get("filter");
  const { data: record, isLoading, error } = usePatch(patchId ?? "");

  let breadcrumbItems: BreadcrumbItem[];
  if (issueId) {
    const issueParams = new URLSearchParams();
    if (fromDashboard) issueParams.set("from", "dashboard");
    if (filterParam) issueParams.set("filter", filterParam);
    const qs = issueParams.toString();
    breadcrumbItems = [
      { label: "Workspace", to: "/" },
      { label: "Issues", to: "/" },
      { label: issueId, to: `/issues/${issueId}${qs ? `?${qs}` : ""}`, kind: "code" },
    ];
  } else {
    breadcrumbItems = [
      { label: "Workspace", to: "/" },
      { label: "Patches", to: "/patches" },
    ];
  }

  useBreadcrumbs(breadcrumbItems, record?.patch.title || patchId || "");

  return (
    <div className={styles.page}>
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
