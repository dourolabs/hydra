import { useQuery } from "@tanstack/react-query";
import type { SearchConversationsQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useConversations(query?: Partial<SearchConversationsQuery>) {
  return useQuery({
    queryKey: ["conversations", query],
    queryFn: () => apiClient.listConversations(query),
  });
}

export function useConversation(conversationId: string) {
  return useQuery({
    queryKey: ["conversation", conversationId],
    queryFn: () => apiClient.getConversation(conversationId),
    enabled: !!conversationId,
  });
}

export function useConversationEvents(conversationId: string) {
  return useQuery({
    queryKey: ["conversationEvents", conversationId],
    queryFn: () => apiClient.getConversationEvents(conversationId),
    enabled: !!conversationId,
  });
}
