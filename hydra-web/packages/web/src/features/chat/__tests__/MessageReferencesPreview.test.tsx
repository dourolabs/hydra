import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { render } from "@testing-library/react";

// --- CSS module stubs ---
vi.mock("../MessageReferencesPreview.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));
vi.mock("../previewCards/previewCards.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Hook mocks (configurable per-test via mutable state). ---
type HookState = { data?: unknown; isLoading?: boolean; isError?: boolean };

const issueState: HookState = {};
const patchState: HookState = {};
const documentState: HookState = {};
const sessionState: HookState = {};
const conversationState: HookState = {};

vi.mock("../../issues/useIssue", () => ({
  useIssue: () => issueState,
}));
vi.mock("../../patches/usePatch", () => ({
  usePatch: () => patchState,
}));
vi.mock("../../documents/useDocument", () => ({
  useDocument: () => documentState,
}));
vi.mock("../../sessions/useSession", () => ({
  useSession: () => sessionState,
}));
vi.mock("../useConversations", () => ({
  useConversation: () => conversationState,
}));
vi.mock("../../projects/useProjects", () => ({
  useProjects: () => ({ data: [] }),
}));

// --- Stub @hydra/ui with what the card path needs. ---
//
// PreviewCard renders as a button so the aria-label is queryable.
vi.mock("@hydra/ui", () => {
  interface PreviewCardLikeProps {
    tone: string;
    topRow?: React.ReactNode;
    title?: React.ReactNode;
    bodyExcerpt?: React.ReactNode;
    footer?: React.ReactNode;
    onClick?: () => void;
    ariaLabel: string;
  }
  const PreviewCard = ({
    tone,
    topRow,
    title,
    bodyExcerpt,
    footer,
    onClick,
    ariaLabel,
  }: PreviewCardLikeProps) => (
    <button type="button" data-tone={tone} aria-label={ariaLabel} onClick={onClick}>
      <span>{topRow}</span>
      <span>{title}</span>
      {bodyExcerpt && <span>{bodyExcerpt}</span>}
      {footer && <span>{footer}</span>}
    </button>
  );
  const Badge = ({ status }: { status: string }) => (
    <span data-testid="badge" data-status={status} />
  );
  const Avatar = ({ name }: { name: string }) => (
    <span data-testid="avatar">{name}</span>
  );
  const TypeChip = ({ type }: { type: string }) => (
    <span data-testid="type-chip" data-type={type} />
  );
  const Icons = {
    IconArchive: () => <span />,
    IconDoc: () => <span />,
    IconHalfCircle: () => <span />,
    IconPlay: () => <span />,
    IconEye: () => <span />,
    IconSearch: () => <span />,
    IconRefresh: () => <span />,
    IconAlert: () => <span />,
    IconCheck: () => <span />,
    IconX: () => <span />,
  };
  return { PreviewCard, Badge, Avatar, TypeChip, Icons };
});

// --- Stub the AgoTime component to a plain span. ---
vi.mock("../../../components/Runtime/Runtime", () => ({
  AgoTime: ({ iso }: { iso?: string | null }) => <span data-testid="ago">{iso}</span>,
}));

// --- Stub react-router-dom's useNavigate. ---
const navigateMock = vi.fn();
vi.mock("react-router-dom", () => ({
  useNavigate: () => navigateMock,
}));

const { MessageReferencesPreview } = await import("../MessageReferencesPreview");

function clearStates() {
  for (const s of [issueState, patchState, documentState, sessionState, conversationState]) {
    delete s.data;
    delete s.isLoading;
    delete s.isError;
  }
}

beforeEach(() => {
  clearStates();
  navigateMock.mockReset();
});

describe("MessageReferencesPreview — empty/skip behavior", () => {
  it("renders nothing when content is empty", () => {
    const { container } = render(<MessageReferencesPreview content="" />);
    expect(container.firstChild).toBeNull();
  });

  it("renders nothing when content has no Hydra references", () => {
    const { container } = render(
      <MessageReferencesPreview content="Just some prose with no ids." />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("skips unsupported kinds (`l-...` labels)", () => {
    issueState.isError = true; // any rendered card is the fallback
    const { container } = render(
      <MessageReferencesPreview content="[[l-aaaa]] [[i-aaaa]]" />,
    );
    const buttons = container.querySelectorAll("button");
    expect(buttons.length).toBe(1);
    expect(buttons[0]!.getAttribute("aria-label")).toBe("Issue i-aaaa");
  });

  it("renders nothing when content has only unsupported references", () => {
    const { container } = render(
      <MessageReferencesPreview content="[[l-aaaa]] [[l-bbbb]]" />,
    );
    expect(container.firstChild).toBeNull();
  });
});

describe("MessageReferencesPreview — dispatch and order", () => {
  it("routes each kind to its preview card by aria-label", () => {
    // All hooks report error so each card renders its fallback (which includes
    // the kind label) — that's an easy discriminator across all five kinds.
    issueState.isError = true;
    patchState.isError = true;
    documentState.isError = true;
    sessionState.isError = true;
    conversationState.isError = true;

    const { container } = render(
      <MessageReferencesPreview content="[[i-aaaa]] [[p-bbbb]] [[d-cccc]] [[s-dddd]] [[c-eeee]]" />,
    );
    const labels = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labels).toEqual([
      "Issue i-aaaa",
      "Patch p-bbbb",
      "Document d-cccc",
      "Session s-dddd",
      "Conversation c-eeee",
    ]);
  });

  it("dedupes repeated ids", () => {
    issueState.isError = true;
    const { container } = render(
      <MessageReferencesPreview content="[[i-aaaa]] then [[i-aaaa]] again" />,
    );
    expect(container.querySelectorAll("button").length).toBe(1);
  });

  it("preserves first-seen order across mixed kinds", () => {
    patchState.isError = true;
    issueState.isError = true;
    documentState.isError = true;

    const { container } = render(
      <MessageReferencesPreview content="[[p-aaaa]] / [[i-bbbb]] / [[d-cccc]]" />,
    );
    const labels = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labels).toEqual([
      "Patch p-aaaa",
      "Issue i-bbbb",
      "Document d-cccc",
    ]);
  });

  it("skips references inside fenced code blocks", () => {
    issueState.isError = true;
    patchState.isError = true;
    const text = "[[i-real]]\n```\n[[i-fenced]]\n```\n[[p-real]]";
    const { container } = render(<MessageReferencesPreview content={text} />);
    const labels = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labels).toEqual(["Issue i-real", "Patch p-real"]);
  });

  it("skips references inside inline code spans", () => {
    patchState.isError = true;
    const { container } = render(
      <MessageReferencesPreview content="`[[i-fenced]]` and [[p-real]]" />,
    );
    const labels = Array.from(container.querySelectorAll("button")).map((b) =>
      b.getAttribute("aria-label"),
    );
    expect(labels).toEqual(["Patch p-real"]);
  });
});

describe("MessageReferencesPreview — loading and fallback states", () => {
  it("renders a fallback card on hook isError, with the id and kind", () => {
    issueState.isError = true;
    const { container } = render(<MessageReferencesPreview content="[[i-broken]]" />);
    const button = container.querySelector("button");
    expect(button).not.toBeNull();
    expect(button!.getAttribute("aria-label")).toBe("Issue i-broken");
    // Fallback uses neutral tone (no status known).
    expect(button!.getAttribute("data-tone")).toBe("neutral");
  });

  it("renders a skeleton in the same chrome while the hook is loading", () => {
    issueState.isLoading = true;
    const { container } = render(<MessageReferencesPreview content="[[i-loading]]" />);
    const button = container.querySelector("button");
    expect(button).not.toBeNull();
    expect(button!.getAttribute("aria-label")).toBe("Loading Issue i-loading");
    expect(button!.getAttribute("data-tone")).toBe("neutral");
  });

  it("renders a fallback when data is missing (404-shaped success)", () => {
    issueState.data = null;
    const { container } = render(<MessageReferencesPreview content="[[i-empty]]" />);
    const button = container.querySelector("button");
    expect(button!.getAttribute("aria-label")).toBe("Issue i-empty");
  });

  it("renders the issue title from real data when the hook resolves", () => {
    issueState.data = {
      issue_id: "i-aaaa",
      version: 1n,
      timestamp: "2026-05-01T00:00:00Z",
      creation_time: "2026-05-01T00:00:00Z",
      issue: {
        type: "task",
        title: "Wire up preview cards",
        description: "First line of body.\nSecond line.",
        creator: "alice",
        status: "open",
        progress: "",
        dependencies: [],
        patches: [],
      },
    };
    const { container } = render(<MessageReferencesPreview content="[[i-aaaa]]" />);
    const button = container.querySelector("button");
    expect(button!.getAttribute("aria-label")).toBe(
      "Issue i-aaaa: Wire up preview cards",
    );
    expect(button!.textContent).toContain("Wire up preview cards");
    expect(button!.textContent).toContain("First line of body.");
  });
});
