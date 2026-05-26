import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../api/client";

export function useUsers() {
  return useQuery({
    queryKey: ["users"],
    queryFn: () => apiClient.listUsers(),
    select: (data) => data.users,
  });
}
