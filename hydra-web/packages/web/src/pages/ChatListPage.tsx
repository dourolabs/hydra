import { useMemo } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Badge, Button, Icons } from "@hydra/ui";
import type { Conversation, SearchConversationsQuery } from "@hydra/api";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { useConversations } from "../features/chat/useConversations";
import { conversationTitle } from "../features/chat/conversationTitle";
import { compareConversationsByBucketThenUpdated } from "../utils/conversationOrder";
import { formatRelativeTime } from "../utils/time";
import { apiClient } from "../api/client";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./ChatListPage.module.css";

type Scope = "mine" | "all";

function parseScope(raw: string | null): Scope {
  return raw === "all" ? "all" : "mine";
}

export function ChatListPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Chats");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const displayName = user ? actorDisplayName(user.actor) : null;
  const [searchParams, setSearchParams] = useSearchParams();
  const scope = parseScope(searchParams.get("scope"));

  const query = useMemo<Partial<SearchConversationsQuery> | undefined>(() => {
    if (scope === "mine" && displayName) return { creator: displayName };
    return undefined;
  }, [scope, displayName]);

  const { data, isLoading, error } = useConversations(query);

  const createMutation = useMutation({
    mutationFn: () => apiClient.createConversation({}),
    onSuccess: (conversation: Conversation) => {
      queryClient.invalidateQueries({ queryKey: ["conversations"] });
      navigate(`/chat/${conversation.conversation_id}`);
    },
  });

  const sorted = useMemo(() => {
    if (!data) return [];
    return [...data].sort(compareConversationsByBucketThenUpdated);
  }, [data]);

  const totalLabel = sorted.length === 1 ? "1 CHAT" : `${sorted.length} CHATS`;

  const setScope = (next: Scope) => {
    setSearchParams((prev) => {
      const params = new URLSearchParams(prev);
      if (next === "all") {
        params.set("scope", "all");
      } else {
        params.delete("scope");
      }
      return params;
    });
  };

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>WORK · {totalLabel}</span>
          <h1 className={styles.pageTitle}>Chats</h1>
        </div>
        <span className={styles.headSpacer} />
        <div
          className={styles.scopeToggle}
          role="tablist"
          aria-label="Chat scope"
          data-testid="chats-scope-toggle"
        >
          <button
            type="button"
            role="tab"
            aria-selected={scope === "mine"}
            className={`${styles.scopeOption}${
              scope === "mine" ? ` ${styles.scopeOptionActive}` : ""
            }`}
            onClick={() => setScope("mine")}
            data-testid="chats-scope-mine"
          >
            Mine
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={scope === "all"}
            className={`${styles.scopeOption}${
              scope === "all" ? ` ${styles.scopeOptionActive}` : ""
            }`}
            onClick={() => setScope("all")}
            data-testid="chats-scope-all"
          >
            All
          </button>
        </div>
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
          <div className={styles.tableWrap}>
            <table className={styles.table} data-testid="chats-list">
              <thead>
                <tr>
                  <th className={styles.colTitle}>Title</th>
                  <th className={styles.colStatus}>Status</th>
                  <th className={styles.colCreator}>Creator</th>
                  <th className={styles.colMessages}>Messages</th>
                  <th className={styles.colUpdated}>Updated</th>
                </tr>
              </thead>
              <tbody>
                {sorted.map((c) => (
                  <tr
                    key={c.conversation_id}
                    onClick={() => navigate(`/chat/${c.conversation_id}`)}
                    data-testid={`chats-list-row-${c.conversation_id}`}
                  >
                    <td className={styles.colTitle}>
                      <div className={styles.titleCell}>
                        <span className={styles.titleText}>{conversationTitle(c)}</span>
                      </div>
                    </td>
                    <td className={styles.colStatus}>
                      <Badge status={`conv-${c.status}`} />
                    </td>
                    <td className={styles.colCreator}>{c.creator}</td>
                    <td className={styles.colMessages}>{c.event_count}</td>
                    <td className={styles.colUpdated}>{formatRelativeTime(c.updated_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
