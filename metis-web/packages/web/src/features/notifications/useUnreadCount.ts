import { useNotifications } from "./useNotifications";

export function useUnreadCount() {
  const { data: notifications, ...rest } = useNotifications(false);
  return {
    ...rest,
    data: notifications != null ? notifications.length : undefined,
  };
}
