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
  ListDocumentsResponse,
  PathChildEntry,
} from "@hydra/api";

// --- Mocks ---

const mockListDocumentPaths = vi.fn();
const mockListDocuments = vi.fn();

vi.mock("../api/client", () => ({
  apiClient: {
    listDocumentPaths: (...args: unknown[]) =>
      mockListDocumentPaths(...args),
    listDocuments: (...args: unknown[]) => mockListDocuments(...args),
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
  };
}

function makePathResponse(
  children: PathChildEntry[],
): ListDocumentPathsResponse {
  return { children };
}

function makeDocumentRecord(documentId: string, path: string) {
  return {
    document_id: documentId,
    version: 1n,
    timestamp: "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title: `Doc ${documentId}`,
      path,
      deleted: false,
      labels: [],
    },
  };
}

function makeDocumentsResponse(
  documentId: string,
  path: string,
): ListDocumentsResponse {
  return {
    documents: [makeDocumentRecord(documentId, path)],
  } as ListDocumentsResponse;
}

function makeDocumentsResponseMulti(
  docs: Array<{ documentId: string; path: string }>,
): ListDocumentsResponse {
  return {
    documents: docs.map((d) => makeDocumentRecord(d.documentId, d.path)),
  } as ListDocumentsResponse;
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

  it("renders top-level folders fetched from listDocumentPaths", async () => {
    mockListDocumentPaths.mockImplementation(
      ({ prefix }: { prefix: string | null }) => {
        if (prefix == null) {
          return Promise.resolve(
            makePathResponse([
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
            ]),
          );
        }
        return Promise.resolve(makePathResponse([]));
      },
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
    expect(mockListDocumentPaths).toHaveBeenCalledWith({ prefix: null });
  });

  it("renders top-level documents as leaf links to /documents/<id>", async () => {
    mockListDocumentPaths.mockResolvedValue(
      makePathResponse([
        makeEntry({
          name: "readme",
          full_path: "/readme",
          child_count: 1n,
          is_document: true,
        }),
      ]),
    );
    mockListDocuments.mockResolvedValue(
      makeDocumentsResponse("d-readme", "/readme"),
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
    mockListDocumentPaths.mockResolvedValue(makePathResponse(many));

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

  it("expanding a folder fires listDocumentPaths with the folder prefix and renders children", async () => {
    mockListDocumentPaths.mockImplementation(
      ({ prefix }: { prefix: string | null }) => {
        if (prefix == null) {
          return Promise.resolve(
            makePathResponse([
              makeEntry({
                name: "research",
                full_path: "/research",
                child_count: 2n,
                is_document: false,
              }),
            ]),
          );
        }
        if (prefix === "/research") {
          return Promise.resolve(
            makePathResponse([
              makeEntry({
                name: "adr-001",
                full_path: "/research/adr-001",
                child_count: 1n,
                is_document: true,
              }),
              makeEntry({
                name: "adr-002",
                full_path: "/research/adr-002",
                child_count: 1n,
                is_document: true,
              }),
            ]),
          );
        }
        return Promise.resolve(makePathResponse([]));
      },
    );
    mockListDocuments.mockImplementation(
      (query: { path_prefix: string; path_is_exact?: boolean }) => {
        if (query.path_prefix === "/research") {
          return Promise.resolve(
            makeDocumentsResponseMulti([
              { documentId: "d-adr001", path: "/research/adr-001" },
              { documentId: "d-adr002", path: "/research/adr-002" },
            ]),
          );
        }
        return Promise.resolve({ documents: [] } as ListDocumentsResponse);
      },
    );

    renderTree();

    const folder = await screen.findByTestId(
      "sidebar-doc-tree-folder-/research",
    );

    // Folder is initially collapsed and child fetch has not been issued.
    expect(folder.getAttribute("aria-expanded")).toBe("false");
    expect(mockListDocumentPaths).not.toHaveBeenCalledWith({
      prefix: "/research",
    });
    expect(mockListDocuments).not.toHaveBeenCalled();

    fireEvent.click(folder);

    await waitFor(() => {
      expect(folder.getAttribute("aria-expanded")).toBe("true");
    });

    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenCalledWith({
        prefix: "/research",
      });
    });

    await waitFor(() => {
      expect(screen.getByTestId("sidebar-doc-tree-leaf-d-adr001")).toBeTruthy();
    });
    expect(screen.getByTestId("sidebar-doc-tree-leaf-d-adr002")).toBeTruthy();

    // Exactly one batched listDocuments call for the folder prefix — no N+1 per leaf.
    expect(mockListDocuments).toHaveBeenCalledTimes(1);
    const docsCallArg = mockListDocuments.mock.calls[0]?.[0] as {
      path_prefix: string;
      path_is_exact?: boolean;
    };
    expect(docsCallArg.path_prefix).toBe("/research");
    expect(docsCallArg.path_is_exact).toBeFalsy();

    // Collapsing hides the children.
    fireEvent.click(folder);
    expect(folder.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByTestId("sidebar-doc-tree-leaf-d-adr001")).toBeNull();
  });

  it("renders nothing when listDocumentPaths returns no children", async () => {
    mockListDocumentPaths.mockResolvedValue(makePathResponse([]));

    renderTree();

    await waitFor(() => {
      expect(mockListDocumentPaths).toHaveBeenCalledWith({ prefix: null });
    });
    expect(screen.queryByTestId("sidebar-doc-tree")).toBeNull();
  });

  // --- Hybrid row tests (is_document=true && child_count > 1) ---
  // The chevron toggle uses testid `sidebar-doc-tree-hybrid-<full_path>`;
  // the NavLink uses testid `sidebar-doc-tree-leaf-<document_id>`.
  describe("hybrid rows", () => {
    it("renders a chevron toggle AND a NavLink to /documents/<id>", async () => {
      mockListDocumentPaths.mockImplementation(
        ({ prefix }: { prefix: string | null }) => {
          if (prefix == null) {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "guide",
                  full_path: "/guide",
                  child_count: 3n,
                  is_document: true,
                }),
              ]),
            );
          }
          return Promise.resolve(makePathResponse([]));
        },
      );
      mockListDocuments.mockResolvedValue(
        makeDocumentsResponse("d-guide", "/guide"),
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
        ({ prefix }: { prefix: string | null }) => {
          if (prefix == null) {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "guide",
                  full_path: "/guide",
                  child_count: 3n,
                  is_document: true,
                }),
              ]),
            );
          }
          if (prefix === "/guide") {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "chapter-1",
                  full_path: "/guide/chapter-1",
                  child_count: 1n,
                  is_document: true,
                }),
                makeEntry({
                  name: "chapter-2",
                  full_path: "/guide/chapter-2",
                  child_count: 1n,
                  is_document: true,
                }),
              ]),
            );
          }
          return Promise.resolve(makePathResponse([]));
        },
      );
      // Both the hybrid's own per-row exact-path lookup and the batched
      // `path_is_exact`-falsy lookup hit `path_prefix: "/guide"`; return the
      // full set of docs under `/guide` so the children leaves resolve from
      // the batched map.
      mockListDocuments.mockImplementation(
        ({ path_prefix }: { path_prefix: string }) => {
          if (path_prefix === "/guide") {
            return Promise.resolve(
              makeDocumentsResponseMulti([
                { documentId: "d-guide", path: "/guide" },
                { documentId: "d-ch1", path: "/guide/chapter-1" },
                { documentId: "d-ch2", path: "/guide/chapter-2" },
              ]),
            );
          }
          return Promise.resolve({ documents: [] } as ListDocumentsResponse);
        },
      );

      renderTree();

      const chevron = await screen.findByTestId(
        "sidebar-doc-tree-hybrid-/guide",
      );

      expect(chevron.getAttribute("aria-expanded")).toBe("false");
      expect(mockListDocumentPaths).not.toHaveBeenCalledWith({
        prefix: "/guide",
      });

      fireEvent.click(chevron);

      await waitFor(() => {
        expect(chevron.getAttribute("aria-expanded")).toBe("true");
      });
      await waitFor(() => {
        expect(mockListDocumentPaths).toHaveBeenCalledWith({
          prefix: "/guide",
        });
      });
      await waitFor(() => {
        expect(screen.getByTestId("sidebar-doc-tree-leaf-d-ch1")).toBeTruthy();
      });
      expect(screen.getByTestId("sidebar-doc-tree-leaf-d-ch2")).toBeTruthy();
    });

    it("clicking the chevron does NOT navigate (does not click the link)", async () => {
      mockListDocumentPaths.mockImplementation(
        ({ prefix }: { prefix: string | null }) => {
          if (prefix == null) {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "guide",
                  full_path: "/guide",
                  child_count: 3n,
                  is_document: true,
                }),
              ]),
            );
          }
          return Promise.resolve(makePathResponse([]));
        },
      );
      mockListDocuments.mockResolvedValue(
        makeDocumentsResponse("d-guide", "/guide"),
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
        ({ prefix }: { prefix: string | null }) => {
          if (prefix == null) {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "guide",
                  full_path: "/guide",
                  child_count: 3n,
                  is_document: true,
                }),
              ]),
            );
          }
          if (prefix === "/guide") {
            return Promise.resolve(
              makePathResponse([
                makeEntry({
                  name: "chapter-1",
                  full_path: "/guide/chapter-1",
                  child_count: 1n,
                  is_document: true,
                }),
              ]),
            );
          }
          return Promise.resolve(makePathResponse([]));
        },
      );
      mockListDocuments.mockResolvedValue(
        makeDocumentsResponse("d-guide", "/guide"),
      );

      renderTree();

      const link = await screen.findByTestId("sidebar-doc-tree-leaf-d-guide");
      const chevron = screen.getByTestId("sidebar-doc-tree-hybrid-/guide");

      expect(chevron.getAttribute("aria-expanded")).toBe("false");

      // Click on the link itself (the name) — should not toggle.
      fireEvent.click(link);

      // aria-expanded stays false, and children fetch is not triggered.
      expect(chevron.getAttribute("aria-expanded")).toBe("false");
      expect(mockListDocumentPaths).not.toHaveBeenCalledWith({
        prefix: "/guide",
      });
      expect(
        screen.queryByTestId("sidebar-doc-tree-leaf-loading-/guide/chapter-1"),
      ).toBeNull();
    });
  });
});
