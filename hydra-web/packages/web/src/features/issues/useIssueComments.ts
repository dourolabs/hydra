import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import type {
  ActorIdentity,
  ActorRef,
  AddCommentRequest,
  AddCommentResponse,
  Comment,
  ListCommentsResponse,
} from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAuth } from "../auth/useAuth";

const PAGE_SIZE = 50;

type CommentsCache = InfiniteData<ListCommentsResponse, bigint | undefined>;

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

interface AddCommentContext {
  previous?: CommentsCache;
}

function identityToActorRef(identity: ActorIdentity): ActorRef {
  if (identity.type === "user") {
    return { Authenticated: { actor_id: { User: { name: identity.username } } } };
  }
  if (identity.type === "agent") {
    return { Authenticated: { actor_id: { Agent: { name: identity.name } } } };
  }
  return {
    Authenticated: { actor_id: { Adhoc: { session_id: identity.session_id } } },
  };
}

export function useAddIssueComment(issueId: string) {
  const queryClient = useQueryClient();
  const { user } = useAuth();

  return useMutation<AddCommentResponse, Error, AddCommentRequest, AddCommentContext>({
    mutationFn: (body) => apiClient.addIssueComment(issueId, body),
    onMutate: async ({ body }) => {
      await queryClient.cancelQueries({ queryKey: ["issues", issueId, "comments"] });
      const previous = queryClient.getQueryData<CommentsCache>([
        "issues",
        issueId,
        "comments",
      ]);

      let maxSeq = 0n;
      if (previous) {
        for (const page of previous.pages) {
          for (const c of page.comments) {
            if (c.sequence > maxSeq) maxSeq = c.sequence;
          }
        }
      }

      const placeholder: Comment = {
        issue_id: issueId,
        sequence: maxSeq + 1n,
        body,
        actor: user
          ? identityToActorRef(user.actor)
          : { Authenticated: { actor_id: { User: { name: "you" } } } },
        created_at: new Date().toISOString(),
      };

      const firstPage: ListCommentsResponse = previous?.pages[0] ?? {
        comments: [],
        next_before_sequence: null,
      };
      const updatedFirstPage: ListCommentsResponse = {
        ...firstPage,
        comments: [placeholder, ...firstPage.comments],
      };
      const next: CommentsCache = previous
        ? { ...previous, pages: [updatedFirstPage, ...previous.pages.slice(1)] }
        : { pages: [updatedFirstPage], pageParams: [undefined] };

      queryClient.setQueryData<CommentsCache>(
        ["issues", issueId, "comments"],
        next,
      );

      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(
          ["issues", issueId, "comments"],
          context.previous,
        );
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["issues", issueId, "comments"] });
    },
  });
}
