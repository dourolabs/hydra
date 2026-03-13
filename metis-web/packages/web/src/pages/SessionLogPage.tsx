import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Badge, Button, Spinner, Tabs } from "@metis/ui";
import { normalizeSessionStatus } from "../utils/statusMapping";
import { getRuntime } from "../utils/time";
import { useSession } from "../features/sessions/useSession";
import { SessionLogViewer } from "../features/sessions/SessionLogViewer";
import { SessionSettings } from "../features/sessions/SessionSettings";
import { KillSessionModal } from "../features/sessions/KillSessionModal";
import { ApiError } from "../api/client";
import { Breadcrumbs } from "../layout/Breadcrumbs";
import styles from "./SessionLogPage.module.css";

const TABS = [
  { id: "logs", label: "Logs" },
  { id: "settings", label: "Settings" },
];

export function SessionLogPage() {
  const { issueId, sessionId } = useParams<{
    issueId: string;
    sessionId: string;
  }>();
  const { data: record, isLoading, error } = useSession(sessionId ?? "");
  const [activeTab, setActiveTab] = useState("logs");
  const [killModalOpen, setKillModalOpen] = useState(false);
  const [killRequested, setKillRequested] = useState(false);

  return (
    <div className={styles.page}>
      <Breadcrumbs
        items={[
          { label: "Dashboard", to: "/" },
          { label: `Issue ${issueId}`, to: `/issues/${issueId}` },
        ]}
        current={`Session ${sessionId}`}
      />

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
          {/* Session metadata header */}
          <div className={styles.header}>
            <div className={styles.headerTop}>
              <span className={styles.sessionId}>{record.session_id}</span>
              <Badge status={normalizeSessionStatus(record.session.status)} />
              {record.session.status === "running" && (
                killRequested ? (
                  <span className={styles.terminating}>
                    <Spinner size="sm" />
                    Terminating...
                  </span>
                ) : (
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => setKillModalOpen(true)}
                  >
                    Kill Session
                  </Button>
                )
              )}
            </div>
            <div className={styles.meta}>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Issue</span>
                <Link to={`/issues/${issueId}`} className={styles.metaLink}>
                  {issueId}
                </Link>
              </div>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Runtime</span>
                <span className={styles.metaValue}>
                  {getRuntime(record.session.start_time, record.session.end_time)}
                </span>
              </div>
              {record.session.creation_time && (
                <div className={styles.metaItem}>
                  <span className={styles.metaLabel}>Created</span>
                  <span className={styles.metaValue}>
                    {new Date(record.session.creation_time).toLocaleString()}
                  </span>
                </div>
              )}
            </div>
          </div>

          {/* Tab bar */}
          <Tabs tabs={TABS} activeTab={activeTab} onTabChange={setActiveTab} />

          {/* Tab content */}
          {activeTab === "logs" && (
            <SessionLogViewer sessionId={record.session_id} status={record.session.status} />
          )}
          {activeTab === "settings" && <SessionSettings task={record.task} />}

          <KillSessionModal
            open={killModalOpen}
            onClose={() => setKillModalOpen(false)}
            onKillSuccess={() => setKillRequested(true)}
            sessionId={record.session_id}
          />
        </>
      )}
    </div>
  );
}
