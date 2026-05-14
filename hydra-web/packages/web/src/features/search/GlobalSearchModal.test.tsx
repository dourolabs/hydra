// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// --- API client mocks ---

const listIssues = vi.fn();
const listPatches = vi.fn();
const listDocuments = vi.fn();
const listConversations = vi.fn();
const listSessions = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listIssues: (...args: unknown[]) => listIssues(...args),
    listPatches: (...args: unknown[]) => listPatches(...args),
    listDocuments: (...args: unknown[]) => listDocuments(...args),
    listConversations: (...args: unknown[]) => listConversations(...args),
    listSessions: (...args: unknown[]) => listSessions(...args),
  },
}));

vi.mock("./GlobalSearchModal.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Import after mocks ---
const { GlobalSearchModal } = await import("./GlobalSearchModal");

// --- Fixtures ---

function issueRow(id: string, title: string) {
  return {
    issue_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title,
      description: "",
      creator: "alice",
      status: "open",
      progress: "",
      dependencies: [],
      patches: [],
    },
  };
}

function patchRow(id: string, title: string) {
  return {
    patch_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title,
      status: "Open",
      is_automatic_backup: false,
      creator: "alice",
      review_summary: { approved: 0, changes_requested: 0, commented: 0 },
      service_repo_name: "org/repo",
    },
  };
}

function documentRow(id: string, title: string, path?: string) {
  return {
    document_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: { title, path: path ?? null },
  };
}

function conversationRow(id: string, title: string | null) {
  return {
    conversation_id: id,
    title,
    agent_name: null,
    status: "idle",
    event_count: 0,
    last_event_preview: title ? null : "preview",
    creator: "alice",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function sessionRow(id: string, prompt: string, spawnedFrom: string | null) {
  return {
    session_id: id,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    session: {
      prompt,
      spawned_from: spawnedFrom,
      creator: "alice",
      status: "running",
    },
  };
}

function defaultMocks() {
  listIssues.mockResolvedValue({ issues: [issueRow("i-1", "Issue one")] });
  listPatches.mockResolvedValue({ patches: [patchRow("p-1", "Patch one")] });
  listDocuments.mockResolvedValue({
    documents: [documentRow("d-1", "Doc one", "/docs/one")],
  });
  listConversations.mockResolvedValue([conversationRow("c-1", "Chat one")]);
  listSessions.mockResolvedValue({
    sessions: [
      sessionRow("s-1", "session one prompt", "i-spawned"),
      sessionRow("s-orphan", "orphan", null),
    ],
  });
}

function LocationProbe() {
  const location = useLocation();
  return (
    <div data-testid="location">{`${location.pathname}${location.search}`}</div>
  );
}

function renderModal(
  overrides: { open?: boolean; onClose?: () => void } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/start"]}>
        <Routes>
          <Route
            path="*"
            element={
              <>
                <LocationProbe />
                <GlobalSearchModal
                  open={overrides.open ?? true}
                  onClose={overrides.onClose ?? (() => {})}
                />
              </>
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function typeQuery(value: string) {
  const input = screen.getByTestId(
    "global-search-input",
  ) as HTMLInputElement;
  fireEvent.change(input, { target: { value } });
  // The hook debounces the query by 200ms before firing requests, and react-query
  // resolves its promises on a microtask. Wait until at least one group is rendered.
  await waitFor(
    () =>
      expect(
        screen.queryByTestId("global-search-empty-hint"),
      ).toBeNull(),
    { timeout: 1500 },
  );
  await waitFor(
    () =>
      expect(
        screen.queryAllByTestId(/^global-search-group-/).length,
      ).toBeGreaterThan(0),
    { timeout: 1500 },
  );
  return input;
}

beforeEach(() => {
  defaultMocks();
});

afterEach(() => {
  cleanup();
  listIssues.mockReset();
  listPatches.mockReset();
  listDocuments.mockReset();
  listConversations.mockReset();
  listSessions.mockReset();
});

describe("GlobalSearchModal rendering", () => {
  it("renders the empty-state hint when there is no query", () => {
    renderModal();
    expect(screen.getByTestId("global-search-empty-hint")).toBeTruthy();
    expect(listIssues).not.toHaveBeenCalled();
  });

  it("does not render when closed", () => {
    renderModal({ open: false });
    expect(screen.queryByTestId("global-search-modal")).toBeNull();
  });

  it("renders five grouped sections when results arrive for all types", async () => {
    renderModal();
    await typeQuery("foo");

    await waitFor(() => {
      expect(screen.getByTestId("global-search-group-issue")).toBeTruthy();
      expect(screen.getByTestId("global-search-group-patch")).toBeTruthy();
      expect(screen.getByTestId("global-search-group-document")).toBeTruthy();
      expect(
        screen.getByTestId("global-search-group-conversation"),
      ).toBeTruthy();
      expect(screen.getByTestId("global-search-group-session")).toBeTruthy();
    });

    // Per-type API call was made with the right query and limit.
    expect(listIssues).toHaveBeenCalledWith({ q: "foo", limit: 5 });
    expect(listPatches).toHaveBeenCalledWith({ q: "foo", limit: 5 });
    expect(listDocuments).toHaveBeenCalledWith({ q: "foo", limit: 5 });
    expect(listConversations).toHaveBeenCalledWith({ q: "foo", limit: 5 });
    expect(listSessions).toHaveBeenCalledWith({ q: "foo", limit: 5 });
  });

  it("hides empty groups and renders 'No results' when nothing matches", async () => {
    listIssues.mockResolvedValue({ issues: [] });
    listPatches.mockResolvedValue({ patches: [] });
    listDocuments.mockResolvedValue({ documents: [] });
    listConversations.mockResolvedValue([]);
    listSessions.mockResolvedValue({ sessions: [] });

    renderModal();
    const input = screen.getByTestId(
      "global-search-input",
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "xyz" } });

    await waitFor(
      () => expect(screen.getByTestId("global-search-no-results")).toBeTruthy(),
      { timeout: 1500 },
    );
    expect(screen.queryByTestId("global-search-group-issue")).toBeNull();
    expect(screen.queryByTestId("global-search-group-patch")).toBeNull();
  });

  it("disables the row for an orphan session (no spawned_from)", async () => {
    listIssues.mockResolvedValue({ issues: [] });
    listPatches.mockResolvedValue({ patches: [] });
    listDocuments.mockResolvedValue({ documents: [] });
    listConversations.mockResolvedValue([]);
    listSessions.mockResolvedValue({
      sessions: [sessionRow("s-orphan", "lonely", null)],
    });

    renderModal();
    await typeQuery("foo");

    const orphan = screen.getByTestId(
      "global-search-row-session-s-orphan",
    ) as HTMLButtonElement;
    expect(orphan.disabled).toBe(true);
  });
});

describe("GlobalSearchModal keyboard navigation", () => {
  it("Arrow keys move selection across flattened rows and Enter navigates", async () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    const input = await typeQuery("foo");

    // Initial selection is the first item (Issues group -> i-1).
    await waitFor(() =>
      expect(
        screen
          .getByTestId("global-search-row-issue-i-1")
          .getAttribute("aria-selected"),
      ).toBe("true"),
    );

    // Down: move to next row (Patches -> p-1).
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(
      screen
        .getByTestId("global-search-row-patch-p-1")
        .getAttribute("aria-selected"),
    ).toBe("true");

    // Down again: Documents -> d-1.
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(
      screen
        .getByTestId("global-search-row-document-d-1")
        .getAttribute("aria-selected"),
    ).toBe("true");

    // Up: back to Patches.
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(
      screen
        .getByTestId("global-search-row-patch-p-1")
        .getAttribute("aria-selected"),
    ).toBe("true");

    // Enter -> navigate to /patches/p-1 and close.
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onClose).toHaveBeenCalledTimes(1);

    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe("/patches/p-1"),
    );
  });

  it("Escape closes the modal", async () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    const input = await typeQuery("foo");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Clicking the backdrop closes the modal", () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    fireEvent.click(screen.getByTestId("global-search-backdrop"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Clicking a row navigates to its href and closes", async () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    await typeQuery("foo");
    fireEvent.click(screen.getByTestId("global-search-row-document-d-1"));
    expect(onClose).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe("/documents/d-1"),
    );
  });

  it("Session row routes to the issue-scoped logs path", async () => {
    const onClose = vi.fn();
    renderModal({ onClose });
    await typeQuery("foo");
    fireEvent.click(screen.getByTestId("global-search-row-session-s-1"));
    expect(onClose).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe(
        "/issues/i-spawned/sessions/s-1/logs",
      ),
    );
  });
});
