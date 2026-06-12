import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { Comment, ListCommentsResponse } from "@hydra/api";

const mockListIssueComments = vi.fn();
const mockAddIssueComment = vi.fn();

vi.mock("../../../api/client", () => ({
  apiClient: {
    listIssueComments: (...args: unknown[]) => mockListIssueComments(...args),
    addIssueComment: (...args: unknown[]) => mockAddIssueComment(...args),
  },
}));

vi.mock("../../../components/Markdown", () => ({
  Markdown: ({ content }: { content: string }) => (
    <div data-testid="comment-body-md">{content}</div>
  ),
}));

vi.mock("@hydra/ui", () => ({
  Avatar: ({ name }: { name: string }) => (
    <span data-testid="comment-avatar">{name}</span>
  ),
  Button: ({
    children,
    onClick,
    disabled,
    ...rest
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    disabled?: boolean;
  } & React.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button onClick={onClick} disabled={disabled} {...rest}>
      {children}
    </button>
  ),
}));

vi.mock("../CommentsPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("../../auth/useAuth", () => ({
  useAuth: () => ({
    user: { actor: { type: "user", username: "alice" } },
    loading: false,
    error: null,
    loginWithDevice: () => Promise.resolve(),
    cancelDeviceFlow: () => {},
    logout: () => Promise.resolve(),
    githubAuthAvailable: true,
    deviceFlowInfo: null,
  }),
}));

const { CommentsPanel } = await import("../CommentsPanel");

function makeComment(seq: number, body: string): Comment {
  return {
    issue_id: "i-1",
    sequence: BigInt(seq),
    body,
    actor: {
      Authenticated: { actor_id: { User: { name: "alice" } } },
    },
    created_at: "2026-01-01T00:00:00Z",
  } as Comment;
}

function makeWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: React.ReactNode }) =>
    React.createElement(QueryClientProvider, { client: queryClient }, children);
}

async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

describe("CommentsPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders empty-state when there are no comments", async () => {
    mockListIssueComments.mockResolvedValue({
      comments: [],
      next_before_sequence: null,
    } satisfies ListCommentsResponse);

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    await screen.findByTestId("comments-empty");
    expect(screen.queryByTestId("comments-load-more")).toBeNull();
  });

  it("renders fetched comments in DESC order", async () => {
    mockListIssueComments.mockResolvedValue({
      comments: [makeComment(2, "second"), makeComment(1, "first")],
      next_before_sequence: null,
    } satisfies ListCommentsResponse);

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    await screen.findAllByTestId("comment-row");
    const bodies = screen.getAllByTestId("comment-body-md");
    expect(bodies[0].textContent).toBe("second");
    expect(bodies[1].textContent).toBe("first");
  });

  it("shows Load more when next_before_sequence is set and paginates on click", async () => {
    mockListIssueComments
      .mockResolvedValueOnce({
        comments: [makeComment(5, "page1-c5"), makeComment(4, "page1-c4")],
        next_before_sequence: 4n,
      } satisfies ListCommentsResponse)
      .mockResolvedValueOnce({
        comments: [makeComment(3, "page2-c3"), makeComment(2, "page2-c2")],
        next_before_sequence: null,
      } satisfies ListCommentsResponse);

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    const loadMore = await screen.findByTestId("comments-load-more");
    fireEvent.click(loadMore);

    await flush();
    await screen.findByText("page2-c3");

    expect(mockListIssueComments).toHaveBeenCalledTimes(2);
    expect(mockListIssueComments).toHaveBeenNthCalledWith(2, "i-1", {
      limit: 50,
      beforeSequence: 4n,
    });
  });

  it("disables the submit button when the textarea is empty or whitespace", async () => {
    mockListIssueComments.mockResolvedValue({
      comments: [],
      next_before_sequence: null,
    } satisfies ListCommentsResponse);

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    const submit = (await screen.findByTestId(
      "comments-composer-submit",
    )) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    const textarea = screen.getByTestId(
      "comments-composer-textarea",
    ) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "   " } });
    expect(submit.disabled).toBe(true);

    fireEvent.change(textarea, { target: { value: "hi" } });
    expect(submit.disabled).toBe(false);
  });

  it("posts a comment on submit, clears the textarea, and re-fetches the list", async () => {
    mockListIssueComments
      .mockResolvedValueOnce({
        comments: [],
        next_before_sequence: null,
      } satisfies ListCommentsResponse)
      .mockResolvedValueOnce({
        comments: [makeComment(1, "hello world")],
        next_before_sequence: null,
      } satisfies ListCommentsResponse);
    mockAddIssueComment.mockResolvedValue({
      comment: makeComment(1, "hello world"),
    });

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    const textarea = (await screen.findByTestId(
      "comments-composer-textarea",
    )) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "hello world" } });

    const submit = screen.getByTestId("comments-composer-submit");
    fireEvent.click(submit);

    await flush();
    await screen.findByText("hello world");

    expect(mockAddIssueComment).toHaveBeenCalledWith("i-1", { body: "hello world" });
    expect(textarea.value).toBe("");
    expect(mockListIssueComments).toHaveBeenCalledTimes(2);
  });

  it("optimistically prepends a comment before the server responds", async () => {
    mockListIssueComments.mockResolvedValueOnce({
      comments: [makeComment(3, "existing")],
      next_before_sequence: null,
    } satisfies ListCommentsResponse);
    // The POST never resolves during this test — the optimistic row must
    // appear before we ever hear back from the server.
    const pending: { resolve: () => void } = { resolve: () => {} };
    mockAddIssueComment.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          pending.resolve = () => resolve();
        }),
    );

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    await screen.findByText("existing");

    const textarea = screen.getByTestId(
      "comments-composer-textarea",
    ) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "draft body" } });
    fireEvent.click(screen.getByTestId("comments-composer-submit"));

    await flush();

    // The optimistic comment shows up immediately at the top of the list,
    // without waiting for the in-flight POST.
    const bodies = screen.getAllByTestId("comment-body-md");
    expect(bodies[0].textContent).toBe("draft body");
    expect(bodies[1].textContent).toBe("existing");
    expect(mockListIssueComments).toHaveBeenCalledTimes(1);

    // Cleanly resolve the in-flight POST so React Query doesn't leak.
    pending.resolve();
  });

  it("rolls back the optimistic comment when the mutation fails", async () => {
    mockListIssueComments.mockResolvedValue({
      comments: [makeComment(3, "existing")],
      next_before_sequence: null,
    } satisfies ListCommentsResponse);
    mockAddIssueComment.mockRejectedValue(new Error("server rejected"));

    render(<CommentsPanel issueId="i-1" />, { wrapper: makeWrapper() });

    await screen.findByText("existing");

    const textarea = screen.getByTestId(
      "comments-composer-textarea",
    ) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "doomed" } });
    fireEvent.click(screen.getByTestId("comments-composer-submit"));

    await flush();
    await screen.findByTestId("comments-composer-error");

    // After rollback the optimistic comment is gone; only the pre-existing
    // row remains. The composer keeps the draft so the user can retry.
    const bodies = screen.getAllByTestId("comment-body-md");
    expect(bodies).toHaveLength(1);
    expect(bodies[0].textContent).toBe("existing");
    expect(textarea.value).toBe("doomed");
  });
});
