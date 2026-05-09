import { useMemo } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Badge, Button } from "@hydra/ui";
import type { ConversationSummary } from "@hydra/api";
import { useConversations } from "../features/chat/useConversations";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { normalizeConversationStatus } from "../utils/statusMapping";
import { formatRelativeTime } from "../utils/time";
import styles from "./ChatListPage.module.css";

function conversationTitle(c: ConversationSummary): string {
  return c.title || c.last_event_preview || "Untitled conversation";
}

export function ChatListPage() {
  const navigate = useNavigate();
  const { data, isLoading, error, refetch } = useConversations();

  const sorted = useMemo(() => {
    if (!data) return [];
    return [...data].sort(
      (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    );
  }, [data]);

  return (
    <div className={styles.page}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>Chat</h2>
        <Button variant="primary" size="sm" onClick={() => navigate("/chat/new")}>
          New Chat
        </Button>
      </div>

      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load conversations: ${error.message}`}
          onRetry={() => refetch()}
        />
      )}

      {!isLoading && !error && sorted.length === 0 && (
        <EmptyState message="No conversations yet." />
      )}

      {sorted.length > 0 && (
        <ul className={styles.list}>
          {sorted.map((c) => (
            <li key={c.conversation_id}>
              <Link to={`/chat/${c.conversation_id}`} className={styles.row}>
                <div className={styles.rowMain}>
                  <span className={styles.rowTitle}>{conversationTitle(c)}</span>
                  <Badge status={normalizeConversationStatus(c.status)} />
                </div>
                <div className={styles.rowMeta}>
                  <span className={styles.metaItem}>{c.event_count} events</span>
                  <span className={styles.metaItem}>{c.creator}</span>
                  <span className={styles.metaItem}>{formatRelativeTime(c.updated_at)}</span>
                </div>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
