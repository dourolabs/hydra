import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useNotifications(isRead?: boolean | null) {
  return useQuery({
    queryKey: ["notifications", { isRead }],
    queryFn: () =>
      apiClient.listNotifications(
        isRead !== undefined && isRead !== null ? { is_read: isRead } : undefined,
      ),
    select: (data) => data.notifications,
  });
}
