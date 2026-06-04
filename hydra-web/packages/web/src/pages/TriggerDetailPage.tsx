import { useParams } from "react-router-dom";
import { Spinner } from "@hydra/ui";
import { useTrigger } from "../features/triggers/useTriggers";
import { TriggerDetail } from "../features/triggers/TriggerDetail";
import { ApiError } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./TriggerDetailPage.module.css";

export function TriggerDetailPage() {
  const { triggerId } = useParams<{ triggerId: string }>();
  const { data: record, isLoading, error } = useTrigger(triggerId ?? "");

  useBreadcrumbs(
    [
      { label: "Workspace", to: "/" },
      { label: "Triggers", to: "/triggers" },
    ],
    triggerId ?? "",
    "code",
  );

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
              Trigger <strong>{triggerId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load trigger: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && <TriggerDetail record={record} />}
    </div>
  );
}
