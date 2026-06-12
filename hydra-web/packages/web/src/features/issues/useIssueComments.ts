import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import type {
  AddCommentRequest,
  AddCommentResponse,
  ListCommentsResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";

const PAGE_SIZE = 50;

/**
 * Paginated comments for an issue. Pages are most-recent-first; the next
 * cursor is the wire's `next_before_sequence` (absent / nullish means the
 * stream is exhausted).
 */
export function useIssueComments(issueId: string) {
  return useInfiniteQuery<ListCommentsResponse, Error>({
    queryKey: ["issues", issueId, "comments"],
    queryFn: ({ pageParam }) =>
      apiClient.listIssueComments(issueId, {
        limit: PAGE_SIZE,
        beforeSequence: pageParam as bigint | undefined,
      }),
    initialPageParam: undefined as bigint | undefined,
    getNextPageParam: (last) => last.next_before_sequence ?? undefined,
    enabled: !!issueId,
  });
}

export function useAddIssueComment(issueId: string) {
  const queryClient = useQueryClient();
  return useMutation<AddCommentResponse, Error, AddCommentRequest>({
    mutationFn: (body) => apiClient.addIssueComment(issueId, body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issues", issueId, "comments"] });
    },
  });
}
