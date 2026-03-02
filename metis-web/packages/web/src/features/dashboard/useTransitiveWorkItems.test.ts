import { describe, it, expect } from "vitest";
import type {
  IssueSummaryRecord,
  PatchSummaryRecord,
  DocumentSummaryRecord,
} from "@metis/api";
import {
  extractDocumentPaths,
  findTransitiveChildren,
  findRootIssueIds,
  collectPatchIds,
  collectDocumentPaths,
  buildWorkItems,
} from "./useTransitiveWorkItems";

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

function makeIssueRecord(
  overrides: {
    issue_id?: string;
    status?: string;
    description?: string;
    dependencies?: Array<{ type: "child-of" | "blocked-on"; issue_id: string }>;
    patches?: string[];
    timestamp?: string;
  } = {},
): IssueSummaryRecord {
  return {
    issue_id: overrides.issue_id ?? "issue-1",
    version: BigInt(1),
    timestamp: overrides.timestamp ?? "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    issue: {
      type: "task",
      title: "",
      description: overrides.description ?? "Test issue",
      creator: "testuser",
      status: (overrides.status ?? "open") as IssueSummaryRecord["issue"]["status"],
      progress: "",
      dependencies: overrides.dependencies ?? [],
      patches: overrides.patches ?? [],
    },
  };
}

function makePatchRecord(
  overrides: {
    patch_id?: string;
    status?: string;
    timestamp?: string;
  } = {},
): PatchSummaryRecord {
  return {
    patch_id: overrides.patch_id ?? "patch-1",
    version: BigInt(1),
    timestamp: overrides.timestamp ?? "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    patch: {
      title: "Test patch",
      status: (overrides.status ?? "Open") as PatchSummaryRecord["patch"]["status"],
      is_automatic_backup: false,
      creator: "testuser",
      review_summary: { count: 0, approved: false },
      service_repo_name: "test/repo",
    },
  };
}

function makeDocumentRecord(
  overrides: {
    document_id?: string;
    path?: string | null;
    timestamp?: string;
  } = {},
): DocumentSummaryRecord {
  return {
    document_id: overrides.document_id ?? "doc-1",
    version: BigInt(1),
    timestamp: overrides.timestamp ?? "2026-01-01T00:00:00Z",
    creation_time: "2026-01-01T00:00:00Z",
    document: {
      title: "Test document",
      path: "path" in overrides ? overrides.path : "/docs/test.md",
    },
  };
}

function toMap(issues: IssueSummaryRecord[]): Map<string, IssueSummaryRecord> {
  const map = new Map<string, IssueSummaryRecord>();
  for (const issue of issues) {
    map.set(issue.issue_id, issue);
  }
  return map;
}

// ---------------------------------------------------------------------------
// extractDocumentPaths
// ---------------------------------------------------------------------------

describe("extractDocumentPaths", () => {
  it("extracts a single document path", () => {
    expect(extractDocumentPaths("see /docs/design.md for details")).toEqual([
      "/docs/design.md",
    ]);
  });

  it("extracts multiple document paths", () => {
    const text = "check /docs/a.md and /docs/b.md";
    const paths = extractDocumentPaths(text);
    expect(paths).toContain("/docs/a.md");
    expect(paths).toContain("/docs/b.md");
    expect(paths).toHaveLength(2);
  });

  it("deduplicates repeated paths", () => {
    const text = "/docs/a.md and /docs/a.md again";
    expect(extractDocumentPaths(text)).toEqual(["/docs/a.md"]);
  });

  it("extracts path at start of line", () => {
    expect(extractDocumentPaths("/docs/start.md")).toEqual(["/docs/start.md"]);
  });

  it("returns empty array for text with no paths", () => {
    expect(extractDocumentPaths("no document paths here")).toEqual([]);
  });

  it("does not match paths without .md extension", () => {
    expect(extractDocumentPaths("see /docs/design.txt")).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// findTransitiveChildren
// ---------------------------------------------------------------------------

describe("findTransitiveChildren", () => {
  it("returns just the root when it has no children", () => {
    const issues = [makeIssueRecord({ issue_id: "root" })];
    expect(findTransitiveChildren("root", issues)).toEqual(["root"]);
  });

  it("returns root and direct children", () => {
    const issues = [
      makeIssueRecord({ issue_id: "root" }),
      makeIssueRecord({
        issue_id: "child-1",
        dependencies: [{ type: "child-of", issue_id: "root" }],
      }),
      makeIssueRecord({
        issue_id: "child-2",
        dependencies: [{ type: "child-of", issue_id: "root" }],
      }),
    ];
    const result = findTransitiveChildren("root", issues);
    expect(result).toContain("root");
    expect(result).toContain("child-1");
    expect(result).toContain("child-2");
    expect(result).toHaveLength(3);
  });

  it("returns multi-level transitive closure", () => {
    const issues = [
      makeIssueRecord({ issue_id: "root" }),
      makeIssueRecord({
        issue_id: "child",
        dependencies: [{ type: "child-of", issue_id: "root" }],
      }),
      makeIssueRecord({
        issue_id: "grandchild",
        dependencies: [{ type: "child-of", issue_id: "child" }],
      }),
      makeIssueRecord({
        issue_id: "great-grandchild",
        dependencies: [{ type: "child-of", issue_id: "grandchild" }],
      }),
    ];
    const result = findTransitiveChildren("root", issues);
    expect(result).toContain("root");
    expect(result).toContain("child");
    expect(result).toContain("grandchild");
    expect(result).toContain("great-grandchild");
    expect(result).toHaveLength(4);
  });

  it("ignores blocked-on dependencies", () => {
    const issues = [
      makeIssueRecord({ issue_id: "root" }),
      makeIssueRecord({
        issue_id: "blocked",
        dependencies: [{ type: "blocked-on", issue_id: "root" }],
      }),
    ];
    const result = findTransitiveChildren("root", issues);
    expect(result).toEqual(["root"]);
  });

  it("does not include unrelated issues", () => {
    const issues = [
      makeIssueRecord({ issue_id: "root" }),
      makeIssueRecord({ issue_id: "unrelated" }),
    ];
    const result = findTransitiveChildren("root", issues);
    expect(result).toEqual(["root"]);
  });

  it("handles cycles gracefully", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        dependencies: [{ type: "child-of", issue_id: "b" }],
      }),
      makeIssueRecord({
        issue_id: "b",
        dependencies: [{ type: "child-of", issue_id: "a" }],
      }),
    ];
    const result = findTransitiveChildren("a", issues);
    expect(result).toContain("a");
    expect(result).toContain("b");
    expect(result).toHaveLength(2);
  });

  it("returns only root ID when root is not in the issues list", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "child",
        dependencies: [{ type: "child-of", issue_id: "root" }],
      }),
    ];
    // root is referenced but not in the list itself
    const result = findTransitiveChildren("root", issues);
    expect(result).toContain("root");
    expect(result).toContain("child");
    expect(result).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// findRootIssueIds
// ---------------------------------------------------------------------------

describe("findRootIssueIds", () => {
  it("returns all issues when none have child-of dependencies", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a" }),
      makeIssueRecord({ issue_id: "b" }),
    ];
    expect(findRootIssueIds(issues)).toEqual(["a", "b"]);
  });

  it("excludes issues with child-of dependencies", () => {
    const issues = [
      makeIssueRecord({ issue_id: "root" }),
      makeIssueRecord({
        issue_id: "child",
        dependencies: [{ type: "child-of", issue_id: "root" }],
      }),
    ];
    expect(findRootIssueIds(issues)).toEqual(["root"]);
  });

  it("returns empty array for empty input", () => {
    expect(findRootIssueIds([])).toEqual([]);
  });

  it("includes issues with only blocked-on dependencies", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        dependencies: [{ type: "blocked-on", issue_id: "b" }],
      }),
      makeIssueRecord({ issue_id: "b" }),
    ];
    const result = findRootIssueIds(issues);
    expect(result).toContain("a");
    expect(result).toContain("b");
  });
});

// ---------------------------------------------------------------------------
// collectPatchIds
// ---------------------------------------------------------------------------

describe("collectPatchIds", () => {
  it("returns empty array when no issues have patches", () => {
    const issues = [makeIssueRecord({ issue_id: "a" })];
    expect(collectPatchIds(["a"], toMap(issues))).toEqual([]);
  });

  it("collects patch IDs from a single issue", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a", patches: ["p-1", "p-2"] }),
    ];
    expect(collectPatchIds(["a"], toMap(issues))).toEqual(["p-1", "p-2"]);
  });

  it("collects patch IDs from multiple issues", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a", patches: ["p-1"] }),
      makeIssueRecord({ issue_id: "b", patches: ["p-2"] }),
    ];
    const result = collectPatchIds(["a", "b"], toMap(issues));
    expect(result).toContain("p-1");
    expect(result).toContain("p-2");
  });

  it("deduplicates patch IDs shared across issues", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a", patches: ["p-1"] }),
      makeIssueRecord({ issue_id: "b", patches: ["p-1"] }),
    ];
    expect(collectPatchIds(["a", "b"], toMap(issues))).toEqual(["p-1"]);
  });

  it("skips issue IDs not in the map", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a", patches: ["p-1"] }),
    ];
    expect(collectPatchIds(["a", "missing"], toMap(issues))).toEqual(["p-1"]);
  });
});

// ---------------------------------------------------------------------------
// collectDocumentPaths
// ---------------------------------------------------------------------------

describe("collectDocumentPaths", () => {
  it("returns empty array when no issues have document paths", () => {
    const issues = [makeIssueRecord({ issue_id: "a", description: "no docs" })];
    expect(collectDocumentPaths(["a"], toMap(issues))).toEqual([]);
  });

  it("extracts document paths from issue descriptions", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        description: "Design at /designs/dashboard.md",
      }),
    ];
    expect(collectDocumentPaths(["a"], toMap(issues))).toEqual([
      "/designs/dashboard.md",
    ]);
  });

  it("deduplicates paths across multiple issues", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        description: "See /docs/shared.md",
      }),
      makeIssueRecord({
        issue_id: "b",
        description: "Also /docs/shared.md",
      }),
    ];
    expect(collectDocumentPaths(["a", "b"], toMap(issues))).toEqual([
      "/docs/shared.md",
    ]);
  });

  it("collects paths from multiple issues", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        description: "See /docs/a.md",
      }),
      makeIssueRecord({
        issue_id: "b",
        description: "See /docs/b.md",
      }),
    ];
    const result = collectDocumentPaths(["a", "b"], toMap(issues));
    expect(result).toContain("/docs/a.md");
    expect(result).toContain("/docs/b.md");
  });
});

// ---------------------------------------------------------------------------
// buildWorkItems
// ---------------------------------------------------------------------------

describe("buildWorkItems", () => {
  it("returns empty array for empty inputs", () => {
    expect(buildWorkItems([], new Map(), [], [], [])).toEqual([]);
  });

  it("creates issue work items", () => {
    const issues = [makeIssueRecord({ issue_id: "a", status: "open" })];
    const items = buildWorkItems(["a"], toMap(issues), [], [], []);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("issue");
    expect(items[0].id).toBe("a");
    expect(items[0].isTerminal).toBe(false);
  });

  it("marks terminal issue statuses correctly", () => {
    const issues = [
      makeIssueRecord({ issue_id: "a", status: "closed" }),
      makeIssueRecord({ issue_id: "b", status: "failed" }),
      makeIssueRecord({ issue_id: "c", status: "open" }),
    ];
    const items = buildWorkItems(["a", "b", "c"], toMap(issues), [], [], []);
    const terminal = items.filter((i) => i.isTerminal);
    const active = items.filter((i) => !i.isTerminal);
    expect(terminal).toHaveLength(2);
    expect(active).toHaveLength(1);
  });

  it("creates patch work items", () => {
    const patch = makePatchRecord({ patch_id: "p-1", status: "Open" });
    const items = buildWorkItems([], new Map(), [patch], [], []);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("patch");
    expect(items[0].id).toBe("p-1");
    expect(items[0].isTerminal).toBe(false);
  });

  it("marks Merged and Closed patches as terminal", () => {
    const merged = makePatchRecord({ patch_id: "p-1", status: "Merged" });
    const closed = makePatchRecord({ patch_id: "p-2", status: "Closed" });
    const open = makePatchRecord({ patch_id: "p-3", status: "Open" });
    const items = buildWorkItems([], new Map(), [merged, closed, open], [], []);
    expect(items.find((i) => i.id === "p-1")!.isTerminal).toBe(true);
    expect(items.find((i) => i.id === "p-2")!.isTerminal).toBe(true);
    expect(items.find((i) => i.id === "p-3")!.isTerminal).toBe(false);
  });

  it("creates document work items matched by path", () => {
    const doc = makeDocumentRecord({
      document_id: "doc-1",
      path: "/docs/design.md",
    });
    const items = buildWorkItems(
      [],
      new Map(),
      [],
      [doc],
      ["/docs/design.md"],
    );
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("document");
    expect(items[0].id).toBe("doc-1");
    expect(items[0].isTerminal).toBe(false);
  });

  it("does not include documents whose path is not in the requested paths", () => {
    const doc = makeDocumentRecord({
      document_id: "doc-1",
      path: "/docs/other.md",
    });
    const items = buildWorkItems(
      [],
      new Map(),
      [],
      [doc],
      ["/docs/design.md"],
    );
    expect(items).toHaveLength(0);
  });

  it("does not include documents with null path", () => {
    const doc = makeDocumentRecord({
      document_id: "doc-1",
      path: null,
    });
    const items = buildWorkItems([], new Map(), [], [doc], ["/docs/test.md"]);
    expect(items).toHaveLength(0);
  });

  it("combines issues, patches, and documents in one list", () => {
    const issues = [makeIssueRecord({ issue_id: "a" })];
    const patches = [makePatchRecord({ patch_id: "p-1" })];
    const docs = [
      makeDocumentRecord({ document_id: "doc-1", path: "/docs/a.md" }),
    ];
    const items = buildWorkItems(
      ["a"],
      toMap(issues),
      patches,
      docs,
      ["/docs/a.md"],
    );
    expect(items).toHaveLength(3);
    const kinds = items.map((i) => i.kind);
    expect(kinds).toContain("issue");
    expect(kinds).toContain("patch");
    expect(kinds).toContain("document");
  });

  it("skips issue IDs not in the map", () => {
    const items = buildWorkItems(["missing"], new Map(), [], [], []);
    expect(items).toHaveLength(0);
  });

  it("sets lastUpdated from the record timestamp", () => {
    const issues = [
      makeIssueRecord({
        issue_id: "a",
        timestamp: "2026-02-15T12:00:00Z",
      }),
    ];
    const items = buildWorkItems(["a"], toMap(issues), [], [], []);
    expect(items[0].lastUpdated).toBe("2026-02-15T12:00:00Z");
  });
});
