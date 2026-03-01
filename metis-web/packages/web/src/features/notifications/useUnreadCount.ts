import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useUnreadCount() {
  return useQuery({
    queryKey: ["notifications", "unread-count"],
    queryFn: () => apiClient.getUnreadCount(),
    select: (data) => data.count,
  });
}
