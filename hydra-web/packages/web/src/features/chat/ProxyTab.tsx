import { useCallback, useState } from "react";
import { Button, Spinner } from "@hydra/ui";
import {
  useConversationProxyStatus,
  type ProxyStatus,
  type ProxyTargetWithStatus,
} from "../../hooks/useConversationProxyStatus";
import { buildProxyUrl, mintConversationProxyCookie } from "../../api/proxyAuth";
import { ApiError } from "../../api/client";
import styles from "./ProxyTab.module.css";

interface ProxyTabProps {
  conversationId: string;
}

const STATUS_LABEL: Record<ProxyStatus, string> = {
  starting: "Starting",
  ready: "Ready",
  idle: "Idle",
  unavailable: "Unavailable",
};

export function ProxyTab({ conversationId }: ProxyTabProps) {
  const { targets, sessionId, isLoading } = useConversationProxyStatus(conversationId);
  const [openingPort, setOpeningPort] = useState<number | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [copiedPort, setCopiedPort] = useState<number | null>(null);

  const handleOpen = useCallback(
    async (target: ProxyTargetWithStatus) => {
      setOpenError(null);
      setOpeningPort(target.port);
      try {
        // Mint the cookie BEFORE opening — the new tab must hit the proxy
        // subdomain with the cookie already in place, otherwise the proxy
        // router returns 401 and the user sees a blank failure.
        await mintConversationProxyCookie(conversationId);
        const url = buildProxyUrl({
          port: target.port,
          targetLabel: conversationId,
          readyPath: target.ready_path ?? null,
          mainHost: window.location.host,
          protocol: window.location.protocol,
        });
        window.open(url, "_blank", "noopener");
      } catch (err) {
        if (err instanceof ApiError && err.status === 409) {
          setOpenError(
            "Conversation is idle — send a message to resume before opening.",
          );
        } else {
          setOpenError(
            err instanceof Error ? err.message : "Failed to open proxy.",
          );
        }
      } finally {
        setOpeningPort(null);
      }
    },
    [conversationId],
  );

  const handleCopySessionUrl = useCallback(
    async (target: ProxyTargetWithStatus) => {
      if (!sessionId) return;
      const url = buildProxyUrl({
        port: target.port,
        targetLabel: sessionId,
        readyPath: target.ready_path ?? null,
        mainHost: window.location.host,
        protocol: window.location.protocol,
      });
      try {
        await navigator.clipboard.writeText(url);
        setCopiedPort(target.port);
        window.setTimeout(() => {
          setCopiedPort((current) => (current === target.port ? null : current));
        }, 1500);
      } catch {
        // Clipboard API can be unavailable in insecure contexts; the user
        // can still click "Open in new tab" so this is non-fatal.
      }
    },
    [sessionId],
  );

  if (isLoading) {
    return (
      <div className={styles.proxyTab}>
        <div className={styles.spinnerWrapper}>
          <Spinner size="sm" />
        </div>
      </div>
    );
  }

  if (targets.length === 0) {
    // The tab is hidden by `ChatRightPanel` when there are no advertised
    // targets, so this branch only renders if the session ends between the
    // tab being chosen and the targets being refetched as empty.
    return (
      <div className={styles.proxyTab}>
        <p className={styles.empty}>No proxy targets advertised.</p>
      </div>
    );
  }

  return (
    <div className={styles.proxyTab} data-testid="proxy-tab">
      {openError && (
        <p className={styles.error} role="alert">
          {openError}
        </p>
      )}
      <ul className={styles.list}>
        {targets.map((target) => (
          <li
            key={target.port}
            className={styles.row}
            data-testid={`proxy-row-${target.port}`}
            data-status={target.status}
          >
            <div className={styles.rowHeader}>
              <span className={styles.port}>port {target.port}</span>
              <span
                className={styles.statusBadge}
                data-status={target.status}
                data-testid={`proxy-status-${target.port}`}
              >
                <span className={styles.statusDot} />
                {STATUS_LABEL[target.status]}
              </span>
            </div>
            {target.ready_path && (
              <div className={styles.readyPath}>
                <span className={styles.readyPathLabel}>ready path</span>
                <code className={styles.readyPathValue}>{target.ready_path}</code>
              </div>
            )}
            <div className={styles.actions}>
              {target.status === "idle" ? (
                <span className={styles.idleNote}>
                  idle — send a message to resume
                </span>
              ) : (
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => void handleOpen(target)}
                  disabled={openingPort === target.port}
                  data-testid={`proxy-open-${target.port}`}
                >
                  {openingPort === target.port ? "Opening…" : "Open in new tab"}
                </Button>
              )}
              {sessionId && (
                <button
                  type="button"
                  className={styles.copyButton}
                  onClick={() => void handleCopySessionUrl(target)}
                  title="Copy session-id URL (debug)"
                  data-testid={`proxy-copy-${target.port}`}
                >
                  {copiedPort === target.port ? "copied!" : "copy session URL"}
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
