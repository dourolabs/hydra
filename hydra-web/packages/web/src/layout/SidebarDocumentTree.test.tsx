// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  waitFor,
  fireEvent,
  cleanup,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type {
  ListDocumentPathsResponse,
  PathChildDocumentRef,
  PathChildEntry,
} from "@hydra/api";

// --- Mocks ---

const mockListDocumentPaths = vi.fn();

vi.mock("../api/client", () => ({
  apiClient: {
    listDocumentPaths: (...args: unknown[]) =>
      mockListDocumentPaths(...args),
  },
}));

vi.mock("./Sidebar.module.css", () => ({
  default: new Proxy({}, { get: (_t, prop) => String(prop) }),
}));

// --- Helpers ---

function makeEntry(
  partial: Partial<PathChildEntry> & { name: string; full_path: string },
): PathChildEntry {
  return {
    name: partial.name,
    full_path: partial.full_path,
    child_count: partial.child_count ?? 1n,
    is_document: partial.is_document ?? false,
    document: partial.document,
  };
}

function makeDocRef(documentId: string, title?: string): PathChildDocumentRef {
  return { document_id: documentId, title: title ?? `Doc ${documentId}` };
}

function makePathResponse(
  children: PathChildEntry[],
): ListDocumentPathsResponse {
  return { children };
}

/**
 * Build a batched-response mock from a per-prefix child map. Mirrors the
 * server: returns the union of children across the requested prefixes,
 * deduped by `full_path` (first occurrence wins).
 */
function batchedResponder(
  perPrefix: Record<string, PathChildEntry[]>,
): (query: { prefixes?: string; prefix?: string | null }) => Promise<ListDocumentPathsResponse> {
  return (query) => {
    const raw =
      typeof query.prefixes === "string"
        ? query.prefixes.split(",").filter(Boolean)
        : query.prefix != null
          ? [query.prefix]
          : ["/"];
    const seen = new Set<string>();
    const out: PathChildEntry[] = [];
    for (const p of raw) {
      const normalized = p.endsWith("/") || p === "/" ? p : `${p}/`;
      // Look up by either normalized form (with trailing slash) or the
      // non-trailing form for convenience.
      const key = perPrefix[normalized] != null ? normalized : p;
      const entries = perPrefix[key] ?? [];
      for (const entry of entries) {
        if (seen.has(entry.full_path)) continue;
        seen.add(entry.full_path);
        out.push(entry);
      }
    }
    return Promise.resolve(makePathResponse(out));
  };
}

function renderTree() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <SidebarDocumentTree />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

// --- Import after mocks ---
const { SidebarDocumentTree } = await import("./SidebarDocumentTree");

describe("SidebarDocumentTree", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders top-level folders fetched from a single batched listDocumentPaths call", async () => {
    mockListDocumentPaths.mockImplementation(
      batchedResponder({
        "/": [
          makeEntry({
            name: "docs",
            full_path: "/docs",
            child_count: 2n,
            is_document: false,
          }),
          makeEntry({
            name: "research",
            full_path: "/research",
            child_count: 3n,
            is_document: false,
          }),
        ],
      }),
    );

    renderTree();

    await waitFor(() => {
      expect(
        screen.getByTestId("sidebar-doc-tree-folder-/docs"),
      ).toBeTruthy();
    });
    expect(
      screen.getByTestId("sidebar-doc-tree-folder-/research"),
    ).toBeTruthy();
    // The initial call asks only for the root listing.
    expect(mockListDocumentPaths).toHaveBeenCalledWith({ prefixes: "/" });
    expect(mockListDocumentPaths).toHaveBeenCalledTimes(1);
  });

  it("renders top-level documents as leaf links using inline document ref", async () => {
    mockListDocumentPaths.mockImplementation(
      batchedResponder({
        "/": [
          makeEntry({
            name: "readme",
            full_path: "/readme",
            child_count: 1n,
            is_document: true,
            document: makeDocRef("d-readme"),
          }),
        ],
      }),
    );

    renderTree();

    await waitFor(() => {
      const link = screen.getByTestId("sidebar-doc-tree-leaf-d-readme");
      expect(link.getAttribute("href")).toBe("/documents/d-readme");
    });
  });

  it("limits top-level entries to 10", async () => {
    const many: PathChildEntry[] = Array.from({ length: 15 }, (_, i) =>
      makeEntry({
        name: `folder-${i}`,
        full_path: `/folder-${i}`,
        child_count: 1n,
        is_document: false,
      }),
    );
    mockListDocumentPaths.mockImplementation(
      batchedResponder({ "/": many }),
    );

    renderTree();

    await waitFor(() => {
      expect(
        screen.getByTestId("sidebar-doc-tree-folder-/folder-0"),
      ).toBeTruthy();
    });
    expect(
      screen.getByTestId("sidebar-doc-tree-folder-/folder-9"),
    ).toBeTruthy();
    expect(
      screen.queryByTestId("sidebar-doc-tree-folder-/folder-10"),
    ).toBeNull();
    expect(
      screen.queryByTestId("sidebar-doc-tree-folder-/folder-14"),
    ).toBeNull();
  });

  it("expanding a folder refires listDocumentPaths with the union of prefixes and renders children", async () => {
    mockListDocumentPaths.mockImplementation(
      batchedResponder({
        "/": [
          makeEntry({
            name: "research",
            full_path: "/research",
            child_count: 2n,
            is_document: false,
          }),
        ],
        "/research/": [
          makeEntry({
            name: "adr-001",
            full_path: "/research/adr-001",
            child_count: 1n,
            is_document: true,
            document: makeDocRef("d-adr001"),
          }),
          makeEntry({
            name: "adr-002",
            full_path: "/research/adr-002",
            child_count: 1n,
            is_document: true,
            document: makeDocRef("d-adr002"),
          }),
        ],
      }),
    );

    renderTree();

    const folder = await screen.findByTestId(
      "sidebar-doc-tree-folder-/research",
    );

    // Folder is initially collapsed; only the root listing has been fetched.
    expect(folder.getAttribute("aria-expanded")).toBe("false");
    expect(mockListDocumentPaths).toHaveBeenCalledTimes(1);
    expect(mockListDocumentPaths).toHaveBeenLastCalledWith({ prefixes: "/" });

    fireEvent.click(folder);

    await waitFor(() => {
      expect(folder.getAttribute("aria-expanded")).toBe("true");
    });

    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenLastCalledWith({
        prefixes: "/,/research/",
      });
    });

    await waitFor(() => {
      expect(
        screen.getByTestId("sidebar-doc-tree-leaf-d-adr001"),
      ).toBeTruthy();
    });
    expect(screen.getByTestId("sidebar-doc-tree-leaf-d-adr002")).toBeTruthy();

    // No fanout to /v1/documents — exactly two batched listDocumentPaths
    // calls so far (root, then root + /research).
    expect(mockListDocumentPaths).toHaveBeenCalledTimes(2);

    // Collapsing refires the call back to just the root prefix and hides
    // the children.
    fireEvent.click(folder);
    expect(folder.getAttribute("aria-expanded")).toBe("false");
    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenLastCalledWith({ prefixes: "/" });
    });
    expect(screen.queryByTestId("sidebar-doc-tree-leaf-d-adr001")).toBeNull();
  });

  it("renders nothing when listDocumentPaths returns no children", async () => {
    mockListDocumentPaths.mockResolvedValue(makePathResponse([]));

    renderTree();

    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenCalledWith({ prefixes: "/" });
    });
    expect(screen.queryByTestId("sidebar-doc-tree")).toBeNull();
  });

  // --- Hybrid row tests (is_document=true && child_count > 1) ---
  // The chevron toggle uses testid `sidebar-doc-tree-hybrid-<full_path>`;
  // the NavLink uses testid `sidebar-doc-tree-leaf-<document_id>`.
  describe("hybrid rows", () => {
    it("renders a chevron toggle AND a NavLink using the inline document ref", async () => {
      mockListDocumentPaths.mockImplementation(
        batchedResponder({
          "/": [
            makeEntry({
              name: "guide",
              full_path: "/guide",
              child_count: 3n,
              is_document: true,
              document: makeDocRef("d-guide"),
            }),
          ],
        }),
      );

      renderTree();

      await waitFor(() => {
        expect(
          screen.getByTestId("sidebar-doc-tree-hybrid-/guide"),
        ).toBeTruthy();
      });
      const link = await screen.findByTestId("sidebar-doc-tree-leaf-d-guide");
      expect(link.getAttribute("href")).toBe("/documents/d-guide");
    });

    it("clicking the chevron toggles aria-expanded and renders children", async () => {
      mockListDocumentPaths.mockImplementation(
        batchedResponder({
          "/": [
            makeEntry({
              name: "guide",
              full_path: "/guide",
              child_count: 3n,
              is_document: true,
              document: makeDocRef("d-guide"),
            }),
          ],
          "/guide/": [
            makeEntry({
              name: "chapter-1",
              full_path: "/guide/chapter-1",
              child_count: 1n,
              is_document: true,
              document: makeDocRef("d-ch1"),
            }),
            makeEntry({
              name: "chapter-2",
              full_path: "/guide/chapter-2",
              child_count: 1n,
              is_document: true,
              document: makeDocRef("d-ch2"),
            }),
          ],
        }),
      );

      renderTree();

      const chevron = await screen.findByTestId(
        "sidebar-doc-tree-hybrid-/guide",
      );

      expect(chevron.getAttribute("aria-expanded")).toBe("false");
      expect(mockListDocumentPaths).toHaveBeenCalledTimes(1);

      fireEvent.click(chevron);

      await waitFor(() => {
        expect(chevron.getAttribute("aria-expanded")).toBe("true");
      });
      await waitFor(() => {
        expect(mockListDocumentPaths).toHaveBeenLastCalledWith({
          prefixes: "/,/guide/",
        });
      });
      await waitFor(() => {
        expect(
          screen.getByTestId("sidebar-doc-tree-leaf-d-ch1"),
        ).toBeTruthy();
      });
      expect(screen.getByTestId("sidebar-doc-tree-leaf-d-ch2")).toBeTruthy();
    });

    it("clicking the chevron does NOT navigate (does not click the link)", async () => {
      mockListDocumentPaths.mockImplementation(
        batchedResponder({
          "/": [
            makeEntry({
              name: "guide",
              full_path: "/guide",
              child_count: 3n,
              is_document: true,
              document: makeDocRef("d-guide"),
            }),
          ],
        }),
      );

      renderTree();

      const link = await screen.findByTestId("sidebar-doc-tree-leaf-d-guide");
      const linkClick = vi.fn();
      link.addEventListener("click", linkClick);

      const chevron = screen.getByTestId("sidebar-doc-tree-hybrid-/guide");
      fireEvent.click(chevron);

      await waitFor(() => {
        expect(chevron.getAttribute("aria-expanded")).toBe("true");
      });
      expect(linkClick).not.toHaveBeenCalled();
    });

    it("clicking the link does NOT toggle expansion", async () => {
      mockListDocumentPaths.mockImplementation(
        batchedResponder({
          "/": [
            makeEntry({
              name: "guide",
              full_path: "/guide",
              child_count: 3n,
              is_document: true,
              document: makeDocRef("d-guide"),
            }),
          ],
          "/guide/": [
            makeEntry({
              name: "chapter-1",
              full_path: "/guide/chapter-1",
              child_count: 1n,
              is_document: true,
              document: makeDocRef("d-ch1"),
            }),
          ],
        }),
      );

      renderTree();

      const link = await screen.findByTestId("sidebar-doc-tree-leaf-d-guide");
      const chevron = screen.getByTestId("sidebar-doc-tree-hybrid-/guide");

      expect(chevron.getAttribute("aria-expanded")).toBe("false");

      // Click on the link itself (the name) — should not toggle.
      fireEvent.click(link);

      // aria-expanded stays false, no extra batched fetch has been triggered.
      expect(chevron.getAttribute("aria-expanded")).toBe("false");
      expect(mockListDocumentPaths).toHaveBeenCalledTimes(1);
      expect(
        screen.queryByTestId("sidebar-doc-tree-leaf-loading-/guide/chapter-1"),
      ).toBeNull();
    });
  });
});
