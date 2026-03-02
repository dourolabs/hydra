import { useMemo, useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type {
  NotificationResponse,
  JobSummaryRecord,
  ListNotificationsResponse,
  UnreadCountResponse,
} from "@metis/api";
import { apiClient } from "../../api/client";
import { useNotifications } from "../notifications/useNotifications";
import type { WorkItem } from "./useTransitiveWorkItems";

export interface ItemNotificationState {
  unread: boolean;
  latestSummary: string;
  notificationIds: string[];
}

/**
 * Determine the item ID that a notification maps to, given the set of work items.
 * - Issues: match on object_kind === "issue" && object_id === issueId
 * - Jobs: match via source_issue_id (preferred) or jobIdToIssueId fallback
 * - Patches: match on object_kind === "patch" && object_id === patchId
 * - Documents: match on object_kind === "document" && object_id === documentId
 */
export function notificationToItemKey(
  n: NotificationResponse,
  itemIdsByKind: Map<string, Set<string>>,
  jobIdToIssueId: Map<string, string>,
): string | null {
  const { object_kind, object_id, source_issue_id } = n.notification;

  if (object_kind === "issue") {
    if (itemIdsByKind.get("issue")?.has(object_id)) {
      return `issue:${object_id}`;
    }
  } else if (object_kind === "job") {
    // Job notifications link to the parent issue via source_issue_id
    if (source_issue_id && itemIdsByKind.get("issue")?.has(source_issue_id)) {
      return `issue:${source_issue_id}`;
    }
    // Fallback: look up the parent issue from the jobsByIssue reverse map
    const fallbackIssueId = jobIdToIssueId.get(object_id);
    if (fallbackIssueId && itemIdsByKind.get("issue")?.has(fallbackIssueId)) {
      return `issue:${fallbackIssueId}`;
    }
  } else if (object_kind === "patch") {
    if (itemIdsByKind.get("patch")?.has(object_id)) {
      return `patch:${object_id}`;
    }
  } else if (object_kind === "document") {
    if (itemIdsByKind.get("document")?.has(object_id)) {
      return `document:${object_id}`;
    }
  }

  return null;
}

export function buildItemKey(item: WorkItem): string {
  return `${item.kind}:${item.id}`;
}

export function buildNotificationMap(
  notifications: NotificationResponse[],
  itemIdsByKind: Map<string, Set<string>>,
  jobIdToIssueId: Map<string, string>,
): Map<string, ItemNotificationState> {
  const map = new Map<string, ItemNotificationState>();

  // Group notifications by item key
  const grouped = new Map<string, NotificationResponse[]>();
  for (const n of notifications) {
    const key = notificationToItemKey(n, itemIdsByKind, jobIdToIssueId);
    if (!key) continue;
    let arr = grouped.get(key);
    if (!arr) {
      arr = [];
      grouped.set(key, arr);
    }
    arr.push(n);
  }

  // Build ItemNotificationState for each group
  for (const [key, notifs] of grouped) {
    // Sort by created_at descending to get the latest first
    notifs.sort(
      (a, b) =>
        new Date(b.notification.created_at).getTime() -
        new Date(a.notification.created_at).getTime(),
    );

    map.set(key, {
      unread: true,
      latestSummary: notifs[0].notification.summary,
      notificationIds: notifs.map((n) => n.notification_id),
    });
  }

  return map;
}

export function useItemNotifications(
  items: WorkItem[],
  jobsByIssue?: Map<string, JobSummaryRecord[]>,
) {
  const { data: notifications } = useNotifications(false);
  const queryClient = useQueryClient();

  // Build a lookup of item IDs grouped by kind for fast matching
  const itemIdsByKind = useMemo(() => {
    const map = new Map<string, Set<string>>();
    for (const item of items) {
      let set = map.get(item.kind);
      if (!set) {
        set = new Set<string>();
        map.set(item.kind, set);
      }
      set.add(item.id);
    }
    return map;
  }, [items]);

  // Build a reverse map from job_id -> issue_id for fallback matching
  const jobIdToIssueId = useMemo(() => {
    const map = new Map<string, string>();
    if (!jobsByIssue) return map;
    for (const [issueId, jobs] of jobsByIssue) {
      for (const job of jobs) {
        map.set(job.job_id, issueId);
      }
    }
    return map;
  }, [jobsByIssue]);

  // Map each notification to its work item and group
  const notificationMap = useMemo(() => {
    if (!notifications) return new Map<string, ItemNotificationState>();
    return buildNotificationMap(notifications, itemIdsByKind, jobIdToIssueId);
  }, [notifications, itemIdsByKind, jobIdToIssueId]);

  // Mark all notifications for an item as read
  const markItemRead = useMutation({
    mutationFn: async (item: WorkItem) => {
      const key = buildItemKey(item);
      const state = notificationMap.get(key);
      if (!state) return;
      await Promise.all(
        state.notificationIds.map((id) => apiClient.markNotificationRead(id)),
      );
    },
    onMutate: async (item: WorkItem) => {
      const key = buildItemKey(item);
      const state = notificationMap.get(key);
      if (!state) return;

      await queryClient.cancelQueries({ queryKey: ["notifications"] });
      await queryClient.cancelQueries({
        queryKey: ["notifications", "unread-count"],
      });

      const prevUnread = queryClient.getQueryData<ListNotificationsResponse>([
        "notifications",
        { isRead: false },
      ]);
      const prevCount = queryClient.getQueryData<UnreadCountResponse>([
        "notifications",
        "unread-count",
      ]);

      // Optimistically remove these notifications from the unread cache
      const idsToRemove = new Set(state.notificationIds);
      queryClient.setQueryData<ListNotificationsResponse>(
        ["notifications", { isRead: false }],
        (old) => {
          if (!old) return old;
          return {
            notifications: old.notifications.filter(
              (n) => !idsToRemove.has(n.notification_id),
            ),
          };
        },
      );

      // Decrement unread count
      if (prevCount) {
        const newCount =
          prevCount.count - BigInt(idsToRemove.size);
        queryClient.setQueryData<UnreadCountResponse>(
          ["notifications", "unread-count"],
          { count: newCount > 0n ? newCount : 0n },
        );
      }

      return { prevUnread, prevCount };
    },
    onError: (_err, _item, context) => {
      if (context?.prevUnread !== undefined) {
        queryClient.setQueryData(
          ["notifications", { isRead: false }],
          context.prevUnread,
        );
      }
      if (context?.prevCount !== undefined) {
        queryClient.setQueryData(
          ["notifications", "unread-count"],
          context.prevCount,
        );
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
      queryClient.invalidateQueries({
        queryKey: ["notifications", "unread-count"],
      });
    },
  });

  const getItemNotification = useCallback(
    (item: WorkItem): ItemNotificationState | undefined => {
      return notificationMap.get(buildItemKey(item));
    },
    [notificationMap],
  );

  return {
    getItemNotification,
    markItemRead: markItemRead.mutateAsync,
    notificationMap,
  };
}
