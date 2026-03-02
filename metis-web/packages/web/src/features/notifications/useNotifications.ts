import { useQuery } from "@tanstack/react-query";
import type { ListNotificationsQuery } from "@metis/api";
import { apiClient } from "../../api/client";

const DEFAULT_NOTIFICATION_LIMIT = 200;

export function useNotifications(isRead?: boolean | null) {
  return useQuery({
    queryKey: ["notifications", { isRead }],
    queryFn: () => {
      const query: Partial<ListNotificationsQuery> = {
        limit: DEFAULT_NOTIFICATION_LIMIT,
      };
      if (isRead !== undefined && isRead !== null) {
        query.is_read = isRead;
      }
      return apiClient.listNotifications(query);
    },
    select: (data) => data.notifications,
  });
}
