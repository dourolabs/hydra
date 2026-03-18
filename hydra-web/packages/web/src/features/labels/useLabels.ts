import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useLabels() {
  return useQuery({
    queryKey: ["labels"],
    queryFn: () => apiClient.listLabels(),
    select: (data) => data.labels.filter((l) => !l.hidden),
  });
}

export function useInboxLabel() {
  return useQuery({
    queryKey: ["labels"],
    queryFn: () => apiClient.listLabels(),
    select: (data) => data.labels.find((l) => l.name === "inbox" && l.hidden) ?? null,
  });
}
