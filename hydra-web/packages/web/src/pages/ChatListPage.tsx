import { useMemo } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Icons } from "@hydra/ui";
import type { Conversation, ConversationStatus } from "@hydra/api";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { formatRelativeTime } from "../utils/time";
import { apiClient } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./ChatListPage.module.css";

function statusTone(status: ConversationStatus): string {
  return status;
}

export function ChatListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Chats");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useConversations();

  const createMutation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const sorted = useMemo(() => {
    if (!data) return [];
    return [...data].sort(
      (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    );
  }, [data]);

  const totalLabel = sorted.length === 1 ? "1 CHAT" : `${sorted.length} CHATS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORK · {totalLabel}</span>
          <h1 className={styles.pageTitle}>Chats</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button
          variant="primary"
          size="sm"
          onClick={() => createMutation.mutate()}
          disabled={createMutation.isPending}
        >
          <Icons.IconPlus />
          {createMutation.isPending ? "Creating…" : "New chat"}
        </Button>
      </div>

      {createMutation.error && (
        <div className={styles.errorBanner}>
          Failed to create conversation: {createMutation.error.message}
        </div>
      )}

      {error && (
        <div className={styles.errorBanner}>
          Failed to load conversations: {error.message}
        </div>
      )}

      <div className={styles.body}>
        {isLoading && sorted.length === 0 && (
          <div className={styles.empty}>Loading chats…</div>
        )}

        {!isLoading && !error && sorted.length === 0 && (
          <div className={styles.empty}>No conversations yet.</div>
        )}

        {sorted.length > 0 && (
          <ul className={styles.list} data-testid="chats-list">
            {sorted.map((c) => (
              <li key={c.conversation_id}>
                <Link
                  to={`/chat/${c.conversation_id}`}
                  className={styles.row}
                  data-testid={`chats-list-row-${c.conversation_id}`}
                >
                  <span className={styles.statusDot} data-tone={statusTone(c.status)} />
                  <span className={styles.title}>{conversationTitle(c)}</span>
                  <span className={styles.eventCount}>{c.event_count} msgs</span>
                  <span className={styles.creator}>{c.creator}</span>
                  <span className={styles.updated}>{formatRelativeTime(c.updated_at)}</span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
