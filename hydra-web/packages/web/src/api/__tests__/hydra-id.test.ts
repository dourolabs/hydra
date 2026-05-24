import { describe, it, expect } from "vitest";
import {
  HYDRA_ID_PREFIXES,
  hydraIdKind,
  isConversationId,
  isDocumentId,
  isIssueId,
  isLabelId,
  isPatchId,
  isSessionId,
} from "@hydra/api";
import type { HydraIdKind } from "@hydra/api";

describe("hydraIdKind", () => {
  const cases: Array<[string, HydraIdKind]> = [
    ["i-abcdef", "issue"],
    ["p-abcdef", "patch"],
    ["d-abcdef", "document"],
    ["s-abcdef", "session"],
    ["l-abcdef", "label"],
    ["c-abcdef", "conversation"],
  ];

  for (const [id, expected] of cases) {
    it(`classifies "${id}" as ${expected}`, () => {
      expect(hydraIdKind(id)).toBe(expected);
    });
  }

  it("returns null for empty string", () => {
    expect(hydraIdKind("")).toBeNull();
  });

  it("returns null for strings with no recognized prefix", () => {
    expect(hydraIdKind("xyz-foo")).toBeNull();
    expect(hydraIdKind("foobar")).toBeNull();
    expect(hydraIdKind("z-abcdef")).toBeNull();
  });

  it("does not validate the suffix character set", () => {
    // The TS helper trusts the server; suffix validation lives on the Rust
    // side. Anything starting with `i-` is classified as an issue.
    expect(hydraIdKind("i-anything!")).toBe("issue");
    expect(hydraIdKind("i-")).toBe("issue");
    expect(hydraIdKind("i-123")).toBe("issue");
  });
});

describe("HYDRA_ID_PREFIXES", () => {
  it("mirrors the Rust enum kinds", () => {
    expect(Object.keys(HYDRA_ID_PREFIXES).sort()).toEqual(
      ["conversation", "document", "issue", "label", "patch", "session"].sort(),
    );
  });

  it("matches the Rust prefix strings", () => {
    expect(HYDRA_ID_PREFIXES).toEqual({
      issue: "i-",
      patch: "p-",
      document: "d-",
      session: "s-",
      label: "l-",
      conversation: "c-",
    });
  });
});

describe("type guards", () => {
  it("isIssueId matches only issue ids", () => {
    expect(isIssueId("i-abc")).toBe(true);
    expect(isIssueId("p-abc")).toBe(false);
    expect(isIssueId("")).toBe(false);
    expect(isIssueId("xyz")).toBe(false);
  });

  it("isPatchId matches only patch ids", () => {
    expect(isPatchId("p-abc")).toBe(true);
    expect(isPatchId("i-abc")).toBe(false);
  });

  it("isDocumentId matches only document ids", () => {
    expect(isDocumentId("d-abc")).toBe(true);
    expect(isDocumentId("c-abc")).toBe(false);
  });

  it("isSessionId matches only session ids", () => {
    expect(isSessionId("s-abc")).toBe(true);
    expect(isSessionId("i-abc")).toBe(false);
  });

  it("isLabelId matches only label ids", () => {
    expect(isLabelId("l-abc")).toBe(true);
    expect(isLabelId("i-abc")).toBe(false);
  });

  it("isConversationId matches only conversation ids", () => {
    expect(isConversationId("c-abc")).toBe(true);
    expect(isConversationId("i-abc")).toBe(false);
  });

  it("all guards return false for unknown prefixes", () => {
    const unknown = "xyz-foo";
    expect(isIssueId(unknown)).toBe(false);
    expect(isPatchId(unknown)).toBe(false);
    expect(isDocumentId(unknown)).toBe(false);
    expect(isSessionId(unknown)).toBe(false);
    expect(isLabelId(unknown)).toBe(false);
    expect(isConversationId(unknown)).toBe(false);
  });

  it("all guards return false for empty string", () => {
    expect(isIssueId("")).toBe(false);
    expect(isPatchId("")).toBe(false);
    expect(isDocumentId("")).toBe(false);
    expect(isSessionId("")).toBe(false);
    expect(isLabelId("")).toBe(false);
    expect(isConversationId("")).toBe(false);
  });
});
