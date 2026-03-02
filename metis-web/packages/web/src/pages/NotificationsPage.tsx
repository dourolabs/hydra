import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner } from "@metis/ui";
import type { NotificationResponse, ListNotificationsResponse } from "@metis/api";
import { apiClient } from "../api/client";
import { useNotifications } from "../features/notifications/useNotifications";
import { formatRelativeTime } from "../utils/time";
import styles from "./NotificationsPage.module.css";

type Filter = "unread" | "all";

function objectRoute(objectKind: string, objectId: string, sourceIssueId?: string | null): string {
  switch (objectKind) {
    case "issue":
      return `/issues/${objectId}`;
    case "patch":
      return `/patches/${objectId}`;
    case "job":
      return sourceIssueId
        ? `/issues/${sourceIssueId}/jobs/${objectId}/logs`
        : `/`;
    case "document":
      return `/documents/${objectId}`;
    default:
      return `/`;
  }
}

export function NotificationsPage() {
  const [filter, setFilter] = useState<Filter>("unread");
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const isReadParam = filter === "unread" ? false : null;
  const { data: notifications, isLoading, error } = useNotifications(isReadParam);

  const markRead = useMutation({
    mutationFn: (id: string) => apiClient.markNotificationRead(id),
    onMutate: async (id: string) => {
      await queryClient.cancelQueries({ queryKey: ["notifications"] });

      const prevUnread = queryClient.getQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: false }],
      );
      const prevAll = queryClient.getQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: null }],
      );

      // Unread cache: remove the notification entirely
      queryClient.setQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: false }],
        (old) => {
          if (!old) return old;
          return {
            notifications: old.notifications.filter(
              (n) => n.notification_id !== id,
            ),
          };
        },
      );

      // All cache: still toggle is_read to true (keep the item in the list)
      queryClient.setQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: null }],
        (old) => {
          if (!old) return old;
          return {
            notifications: old.notifications.map((n) =>
              n.notification_id === id
                ? { ...n, notification: { ...n.notification, is_read: true } }
                : n,
            ),
          };
        },
      );

      return { prevUnread, prevAll };
    },
    onError: (_err, _id, context) => {
      if (context?.prevUnread !== undefined) {
        queryClient.setQueryData(["notifications", { isRead: false }], context.prevUnread);
      }
      if (context?.prevAll !== undefined) {
        queryClient.setQueryData(["notifications", { isRead: null }], context.prevAll);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  const markAllRead = useMutation({
    mutationFn: () => apiClient.markAllNotificationsRead(),
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ["notifications"] });

      const prevUnread = queryClient.getQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: false }],
      );
      const prevAll = queryClient.getQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: null }],
      );

      // Unread cache: empty the list
      queryClient.setQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: false }],
        (old) => (old ? { notifications: [] } : old),
      );

      // All cache: still mark every item as read
      queryClient.setQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: null }],
        (old) => {
          if (!old) return old;
          return {
            notifications: old.notifications.map((n) => ({
              ...n,
              notification: { ...n.notification, is_read: true },
            })),
          };
        },
      );

      return { prevUnread, prevAll };
    },
    onError: (_err, _vars, context) => {
      if (context?.prevUnread !== undefined) {
        queryClient.setQueryData(["notifications", { isRead: false }], context.prevUnread);
      }
      if (context?.prevAll !== undefined) {
        queryClient.setQueryData(["notifications", { isRead: null }], context.prevAll);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  const handleNotificationClick = (notification: NotificationResponse) => {
    if (!notification.notification.is_read) {
      markRead.mutate(notification.notification_id);
    }
    const route = objectRoute(
      notification.notification.object_kind,
      notification.notification.object_id,
      notification.notification.source_issue_id,
    );
    navigate(route);
  };

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <h1 className={styles.title}>Notifications</h1>
        <button
          className={styles.markAllButton}
          onClick={() => markAllRead.mutate()}
          disabled={markAllRead.isPending}
        >
          Mark all as read
        </button>
      </div>

      <div className={styles.filterTabs}>
        <button
          className={filter === "unread" ? styles.filterTabActive : styles.filterTab}
          onClick={() => setFilter("unread")}
        >
          Unread
        </button>
        <button
          className={filter === "all" ? styles.filterTabActive : styles.filterTab}
          onClick={() => setFilter("all")}
        >
          All
        </button>
      </div>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>
          Failed to load notifications: {(error as Error).message}
        </p>
      )}

      {notifications && notifications.length === 0 && (
        <p className={styles.empty}>
          {filter === "unread" ? "All caught up!" : "No notifications"}
        </p>
      )}

      {notifications && notifications.length > 0 && (
        <Panel header={<span>Notifications</span>}>
          <ul className={styles.notificationList}>
            {notifications.map((n) => (
              <li key={n.notification_id}>
                <div
                  className={styles.notificationRow}
                  role="button"
                  tabIndex={0}
                  onClick={() => handleNotificationClick(n)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      handleNotificationClick(n);
                    }
                  }}
                >
                  <span
                    className={n.notification.is_read ? styles.readDot : styles.unreadDot}
                  />
                  <div className={styles.notificationContent}>
                    <span
                      className={
                        n.notification.is_read ? styles.summary : styles.summaryUnread
                      }
                    >
                      {n.notification.summary}
                    </span>
                    <div className={styles.notificationMeta}>
                      <span className={styles.objectLink}>
                        {n.notification.object_kind}/{n.notification.object_id}
                      </span>
                      <span className={styles.time}>
                        {formatRelativeTime(n.notification.created_at)}
                      </span>
                    </div>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </Panel>
      )}
    </div>
  );
}
