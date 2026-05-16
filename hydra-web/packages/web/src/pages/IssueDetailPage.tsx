import { useParams, useSearchParams } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useIssue } from "../features/issues/useIssue";
import { IssueDetail } from "../features/issues/IssueDetail";
import { ApiError } from "../api/client";
import type { BreadcrumbItem } from "../layout/Breadcrumbs";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./IssueDetailPage.module.css";

export function IssueDetailPage() {
  const { issueId } = useParams<{ issueId: string }>();
  const [searchParams] = useSearchParams();
  const fromDashboard = searchParams.get("from") === "dashboard";
  const filterParam = searchParams.get("filter");
  const tab = searchParams.get("tab");
  const { data: record, isLoading, error } = useIssue(issueId ?? "");

  let issuesReturnUrl = "/";
  if (fromDashboard) {
    const returnParams = new URLSearchParams();
    if (filterParam) returnParams.set("selected", filterParam);
    if (tab) returnParams.set("tab", tab);
    const qs = returnParams.toString();
    issuesReturnUrl = qs ? `/?${qs}` : "/";
  }

  const breadcrumbItems: BreadcrumbItem[] = [
    { label: "Workspace", to: "/" },
    { label: "Issues", to: issuesReturnUrl },
  ];

  useBreadcrumbs(breadcrumbItems, issueId ?? "", "code");

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
              Issue <strong>{issueId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load issue: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && <IssueDetail record={record} />}
    </div>
  );
}
