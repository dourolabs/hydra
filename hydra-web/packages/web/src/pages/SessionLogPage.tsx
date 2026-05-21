import { useState, useCallback } from "react";
import { Link, useParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Badge, Button, Spinner } from "@hydra/ui";
import type { SessionVersionRecord } from "@hydra/api";
import { normalizeSessionStatus } from "../utils/statusMapping";
import { getRuntime } from "../utils/time";
import { useSession } from "../features/sessions/useSession";
import { SessionLogViewer } from "../features/sessions/SessionLogViewer";
import { SessionSettings } from "../features/sessions/SessionSettings";
import { DeleteConfirmModal } from "../components/DeleteConfirmModal/DeleteConfirmModal";
import { apiClient, ApiError } from "../api/client";
import { useToast } from "../features/toast/useToast";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./SessionLogPage.module.css";

type TabKey = "logs" | "settings";

const TABS: { key: TabKey; label: string }[] = [
  { key: "logs", label: "Logs" },
  { key: "settings", label: "Settings" },
];

export function SessionLogPage() {
  const { issueId, sessionId } = useParams<{
    issueId?: string;
    sessionId: string;
  }>();
  const { data: record, isLoading, error } = useSession(sessionId ?? "");
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = useState<TabKey>("logs");
  const [killModalOpen, setKillModalOpen] = useState(false);
  const [killRequested, setKillRequested] = useState(false);

  const killMutation = useMutation({
    mutationFn: () => apiClient.killSession(record!.session_id),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["session", record!.session_id] });
      const previous = queryClient.getQueryData<SessionVersionRecord>([
        "session",
        record!.session_id,
      ]);
      if (previous) {
        queryClient.setQueryData<SessionVersionRecord>(["session", record!.session_id], {
          ...previous,
          session: { ...previous.session, status: "failed" },
        });
      }
      return { previous };
    },
    onSuccess: () => {
      addToast("Session killed successfully", "success");
      setKillRequested(true);
      setKillModalOpen(false);
    },
    onError: (err, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(["session", record!.session_id], context.previous);
      }
      addToast(
        err instanceof Error ? err.message : "Failed to kill session",
        "error",
      );
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["session", record!.session_id] });
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });

  const handleKillConfirm = useCallback(() => {
    killMutation.mutate();
  }, [killMutation]);

  useBreadcrumbs(
    issueId
      ? [
          { label: "Workspace", to: "/" },
          { label: "Issues", to: "/" },
          { label: issueId, to: `/issues/${issueId}`, kind: "code" },
        ]
      : [
          { label: "Workspace", to: "/" },
          { label: "Sessions", to: "/sessions" },
        ],
    sessionId ?? "",
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
              Session <strong>{sessionId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load session: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && (
        <>
          <div className={styles.header}>
            <div className={styles.headerInner}>
              <div className={styles.headerTop}>
                <span className={styles.agentName}>
                  <Avatar name={record.session.creator} kind="human" size="md" />
                  <span className={styles.agentLabel}>Creator</span>
                  {record.session.creator}
                </span>
                <Badge status={normalizeSessionStatus(record.session.status)} />
                <span className={styles.headerSpacer} />
                {record.session.status === "running" &&
                  (killRequested ? (
                    <span className={styles.terminating}>
                      <Spinner size="sm" />
                      Terminating…
                    </span>
                  ) : (
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={() => setKillModalOpen(true)}
                    >
                      Kill Session
                    </Button>
                  ))}
              </div>
              <div className={styles.meta}>
                {issueId && (
                  <span className={styles.metaItem}>
                    <span className={styles.metaLabel}>Issue</span>
                    <Link to={`/issues/${issueId}`} className={styles.metaLink}>
                      {issueId}
                    </Link>
                  </span>
                )}
                <span className={styles.metaItem}>
                  <span className={styles.metaLabel}>Runtime</span>
                  <span className={styles.metaValue}>
                    {getRuntime(record.session.start_time, record.session.end_time)}
                  </span>
                </span>
                {record.session.start_time && (
                  <span className={styles.metaItem}>
                    <span className={styles.metaLabel}>Started</span>
                    <span className={styles.metaValue}>
                      {new Date(record.session.start_time).toLocaleString()}
                    </span>
                  </span>
                )}
                {record.session.creation_time && (
                  <span className={styles.metaItem}>
                    <span className={styles.metaLabel}>Created</span>
                    <span className={styles.metaValue}>
                      {new Date(record.session.creation_time).toLocaleString()}
                    </span>
                  </span>
                )}
              </div>
            </div>
          </div>

          <div className={styles.tabs}>
            <div className={styles.tabsInner} role="tablist">
              {TABS.map((t) => (
                <button
                  key={t.key}
                  type="button"
                  role="tab"
                  className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
                  aria-selected={activeTab === t.key}
                  onClick={() => setActiveTab(t.key)}
                  data-testid={`session-tab-${t.key}`}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </div>

          <div className={styles.body}>
            {activeTab === "logs" && (
              <div className={styles.bodyFull}>
                <SessionLogViewer
                  sessionId={record.session_id}
                  status={record.session.status}
                />
              </div>
            )}
            {activeTab === "settings" && (
              <div className={styles.bodyInner}>
                <SessionSettings task={record.session} />
              </div>
            )}
          </div>

          <DeleteConfirmModal
            open={killModalOpen}
            onClose={() => setKillModalOpen(false)}
            entityName={record.session_id}
            entityLabel="Session"
            onConfirm={handleKillConfirm}
            isPending={killMutation.isPending}
            actionLabel="Kill"
            buttonLabel="Kill Session"
            pendingLabel="Killing..."
            description="This will terminate the running session and cannot be undone."
          />
        </>
      )}
    </div>
  );
}
