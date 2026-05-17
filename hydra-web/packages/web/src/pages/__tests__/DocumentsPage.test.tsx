// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, cleanup } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";
import type { ListDocumentPathsResponse, ListDocumentsResponse, PathChildEntry } from "@hydra/api";

// --- Mocks ---

const mockListDocumentPaths = vi.fn();
const mockListDocuments = vi.fn();

vi.mock("../../api/client", () => ({
  apiClient: {
    listDocumentPaths: (...args: unknown[]) => mockListDocumentPaths(...args),
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
  },
}));

vi.mock("../../layout/useBreadcrumbs", () => ({
  useBreadcrumbs: () => undefined,
}));

vi.mock("../../features/documents/DocumentCreateModal", () => ({
  DocumentCreateModal: () => null,
}));

vi.mock("../DocumentsPage.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

vi.mock("@hydra/ui", () => ({
  Button: ({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) => (
    <button onClick={onClick}>{children}</button>
  ),
  Spinner: () => <span data-testid="spinner" />,
  Icons: new Proxy(
    {},
    {
      get: () => () => <span data-testid="icon" />,
    },
  ),
}));

// --- Helpers ---

function pathEntry(
  full_path: string,
  opts: {
    is_document?: boolean;
    child_count?: number;
    document_id?: string;
    title?: string;
  } = {},
): PathChildEntry {
  const segs = full_path.split("/").filter(Boolean);
  const name = segs[segs.length - 1] ?? "";
  const entry: PathChildEntry = {
    name,
    full_path,
    child_count: BigInt(opts.child_count ?? 1),
    is_document: opts.is_document ?? false,
  };
  if (opts.is_document && opts.document_id) {
    entry.document = { document_id: opts.document_id, title: opts.title ?? "" };
  }
  return entry;
}

function emptyDocsResponse(): ListDocumentsResponse {
  return { documents: [] };
}

function makeQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
}

function renderPage() {
  const client = makeQueryClient();
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <DocumentsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

// --- Import after mocks ---
const { DocumentsPage } = await import("../DocumentsPage");

// --- Tests ---

describe("DocumentsPage batched paths fetch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Seed an explicit empty expansion state so the auto-expand-on-first-visit
    // logic is a no-op, giving us a deterministic initial prefix set of `[/]`.
    localStorage.setItem("hydra:document-tree-expanded", "[]");
    mockListDocuments.mockResolvedValue(emptyDocsResponse());
  });

  afterEach(() => {
    localStorage.clear();
    cleanup();
  });

  it("fires one batched paths request with the root prefix initially and one more when a folder is expanded", async () => {
    let resolveSecond: ((value: ListDocumentPathsResponse) => void) | null = null;

    mockListDocumentPaths.mockImplementation(({ prefixes }: { prefixes: string }) => {
      if (prefixes === "/") {
        return Promise.resolve<ListDocumentPathsResponse>({
          children: [pathEntry("/agents", { child_count: 2 })],
        });
      }
      // Second call includes the newly-expanded `/agents` prefix.
      return new Promise<ListDocumentPathsResponse>((resolve) => {
        resolveSecond = resolve;
      });
    });

    renderPage();

    // Wait for the initial render — the chevron is only rendered after the
    // top-level `/agents` folder lands.
    await waitFor(() => {
      expect(screen.getByLabelText("Expand")).toBeDefined();
    });

    // Exactly one batched fetch fired, scoped to the root prefix.
    expect(mockListDocumentPaths).toHaveBeenCalledTimes(1);
    expect(mockListDocumentPaths).toHaveBeenNthCalledWith(1, { prefixes: "/" });

    // Click the chevron to expand `/agents` without changing activePath.
    screen.getByLabelText("Expand").click();

    // The second fetch fires once, with the union prefix set.
    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenCalledTimes(2);
    });
    expect(mockListDocumentPaths).toHaveBeenNthCalledWith(2, {
      prefixes: "/,/agents",
    });

    // Resolve the pending second request so React Query settles.
    resolveSecond!({
      children: [
        pathEntry("/agents/profile", {
          is_document: true,
          document_id: "d-1",
          title: "Profile",
        }),
      ],
    });
  });

  it("renders an up-one-level entry in the reader pane for a non-root folder and hides it at root", async () => {
    mockListDocumentPaths.mockResolvedValue({
      children: [
        pathEntry("/research", { child_count: 2 }),
        pathEntry("/research/notes", {
          is_document: true,
          document_id: "d-1",
          title: "Notes",
        }),
      ],
    } satisfies ListDocumentPathsResponse);

    renderPage();

    // At root, the up-one-level entry is not rendered.
    await waitFor(() => {
      expect(screen.queryAllByRole("button", { name: /research/ }).length).toBeGreaterThan(0);
    });
    expect(screen.queryByTestId("documents-up-one-level")).toBeNull();

    // Click the `/research` folder row in the reader pane to make it active.
    // Two matching elements render (the left tree's treeitem + the reader
    // pane's docRow button); the docRow button is the second one.
    const researchRows = screen.getAllByRole("button", { name: /research/ });
    const readerRow = researchRows[researchRows.length - 1];
    readerRow.click();

    // The up-one-level entry now appears, labelled with the parent name.
    await waitFor(() => {
      expect(screen.getByTestId("documents-up-one-level")).toBeDefined();
    });
    expect(screen.getByTestId("documents-up-one-level").textContent).toContain("Up to /");
  });

  it("renders leaf documents inline using PathChildDocumentRef without per-folder document lookups", async () => {
    mockListDocumentPaths.mockResolvedValue({
      children: [
        pathEntry("/notes", { child_count: 1 }),
        pathEntry("/notes/welcome", {
          is_document: true,
          document_id: "d-welcome",
          title: "Welcome",
        }),
      ],
    } satisfies ListDocumentPathsResponse);

    // Seed expansion so `/notes` is open from the start.
    localStorage.setItem("hydra:document-tree-expanded", JSON.stringify(["/notes"]));

    renderPage();

    // Leaf document title (from inline `document.title`) should appear in the
    // tree without any per-folder `listDocuments({ path_prefix })` fanout.
    await waitFor(() => {
      const matches = screen.queryAllByText("Welcome");
      expect(matches.length).toBeGreaterThan(0);
    });

    // The only `listDocuments` call permitted is the uncategorized-docs query.
    for (const call of mockListDocuments.mock.calls) {
      const arg = call[0] as { path_prefix?: string; ids?: string };
      expect(arg.path_prefix).toBeUndefined();
      expect(arg.ids).toBeUndefined();
    }
  });
});
