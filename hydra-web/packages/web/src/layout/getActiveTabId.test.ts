// @vitest-environment jsdom
import { describe, it, expect } from "vitest";

import { getActiveTabId } from "./getActiveTabId";

describe("getActiveTabId", () => {
  it("returns 'issues' for the root path", () => {
    expect(getActiveTabId("/")).toBe("issues");
  });

  it("returns 'issues' for issue list with query params", () => {
    // The pathname passed in by react-router does not include the search
    // string, but make sure the prefix check is robust to in-section routes.
    expect(getActiveTabId("/issues")).toBe("issues");
    expect(getActiveTabId("/issues/i-abc123")).toBe("issues");
    expect(getActiveTabId("/issues/i-abc123/sessions/s-xyz/logs")).toBe("issues");
  });

  it("returns 'patches' for the patches section", () => {
    expect(getActiveTabId("/patches")).toBe("patches");
    expect(getActiveTabId("/patches/")).toBe("patches");
    expect(getActiveTabId("/patches/p-abcdef")).toBe("patches");
  });

  it("returns 'sessions' for the sessions section", () => {
    expect(getActiveTabId("/sessions")).toBe("sessions");
    expect(getActiveTabId("/sessions/t-abc123")).toBe("sessions");
  });

  it("returns 'chat' for the chat section", () => {
    expect(getActiveTabId("/chat")).toBe("chat");
    expect(getActiveTabId("/chat/c-abc123")).toBe("chat");
  });

  it("falls back to 'more' for ancillary destinations", () => {
    expect(getActiveTabId("/agents")).toBe("more");
    expect(getActiveTabId("/secrets")).toBe("more");
    expect(getActiveTabId("/repositories")).toBe("more");
    expect(getActiveTabId("/projects")).toBe("more");
    expect(getActiveTabId("/documents")).toBe("more");
    expect(getActiveTabId("/triggers")).toBe("more");
    expect(getActiveTabId("/analytics/throughput")).toBe("more");
    expect(getActiveTabId("/analytics/token-usage")).toBe("more");
    expect(getActiveTabId("/settings")).toBe("more");
  });

  it("does not treat unrelated paths whose names start with a primary as that tab", () => {
    // Guards against the naive `startsWith('/patches')` matching `/patches-archive`.
    expect(getActiveTabId("/patches-archive")).toBe("more");
    expect(getActiveTabId("/sessionsfoo")).toBe("more");
    expect(getActiveTabId("/chatroom")).toBe("more");
    expect(getActiveTabId("/issuesfoo")).toBe("more");
  });
});
