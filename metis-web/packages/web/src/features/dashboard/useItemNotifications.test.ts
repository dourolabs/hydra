import { describe, expect, it } from "vitest";
import type { NotificationResponse } from "@metis/api";
import type { WorkItem } from "./useTransitiveWorkItems";
import {
  notificationToItemKey,
  buildItemKey,
  buildNotificationMap,
} from "./useItemNotifications";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

let notifCounter = 0;

function makeNotification(
  overrides: Partial<{
    notification_id: string;
    object_kind: string;
    object_id: string;
    source_issue_id: string | null;
    summary: string;
    created_at: string;
  }> = {},
): NotificationResponse {
  notifCounter++;
  return {
    notification_id: overrides.notification_id ?? `n-${notifCounter}`,
    notification: {
      recipient: { Username: "actor-1" },
      source_actor: null,
      object_kind: overrides.object_kind ?? "issue",
      object_id: overrides.object_id ?? `obj-${notifCounter}`,
      object_version: 1n,
      event_type: "updated",
      summary: overrides.summary ?? `Notification ${notifCounter}`,
      source_issue_id:
        overrides.source_issue_id !== undefined
          ? overrides.source_issue_id
          : null,
      policy: "default",
      is_read: false,
      created_at: overrides.created_at ?? "2026-01-01T00:00:00Z",
    },
  };
}

function makeItemIdsByKind(
  entries: Record<string, string[]>,
): Map<string, Set<string>> {
  const map = new Map<string, Set<string>>();
  for (const [kind, ids] of Object.entries(entries)) {
    map.set(kind, new Set(ids));
  }
  return map;
}

// ---------------------------------------------------------------------------
// notificationToItemKey
// ---------------------------------------------------------------------------

describe("notificationToItemKey", () => {
  const emptyJobMap = new Map<string, string>();

  it("returns issue:<id> for an issue notification matching a known issue", () => {
    const n = makeNotification({ object_kind: "issue", object_id: "i-1" });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBe("issue:i-1");
  });

  it("returns null for an issue notification with an unknown issue", () => {
    const n = makeNotification({ object_kind: "issue", object_id: "i-unknown" });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });

  it("returns issue:<source_issue_id> for a job notification with matching source_issue_id", () => {
    const n = makeNotification({
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: "i-1",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBe("issue:i-1");
  });

  it("returns null for a job notification with source_issue_id not matching any known issue", () => {
    const n = makeNotification({
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: "i-unknown",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });

  it("uses jobIdToIssueId fallback when source_issue_id is absent", () => {
    const n = makeNotification({
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: null,
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const jobMap = new Map([["j-1", "i-1"]]);
    expect(notificationToItemKey(n, ids, jobMap)).toBe("issue:i-1");
  });

  it("returns null when source_issue_id is absent and job_id not in jobIdToIssueId", () => {
    const n = makeNotification({
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: null,
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });

  it("falls back to jobIdToIssueId when source_issue_id does not match but fallback does", () => {
    const n = makeNotification({
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: "i-unknown",
    });
    const ids = makeItemIdsByKind({ issue: ["i-2"] });
    const jobMap = new Map([["j-1", "i-2"]]);
    expect(notificationToItemKey(n, ids, jobMap)).toBe("issue:i-2");
  });

  it("returns patch:<id> for a patch notification matching a known patch", () => {
    const n = makeNotification({ object_kind: "patch", object_id: "p-1" });
    const ids = makeItemIdsByKind({ patch: ["p-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBe("patch:p-1");
  });

  it("returns null for a patch notification with an unknown patch", () => {
    const n = makeNotification({ object_kind: "patch", object_id: "p-unknown" });
    const ids = makeItemIdsByKind({ patch: ["p-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });

  it("returns document:<id> for a document notification matching a known document", () => {
    const n = makeNotification({ object_kind: "document", object_id: "d-1" });
    const ids = makeItemIdsByKind({ document: ["d-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBe("document:d-1");
  });

  it("returns null for a document notification with an unknown document", () => {
    const n = makeNotification({
      object_kind: "document",
      object_id: "d-unknown",
    });
    const ids = makeItemIdsByKind({ document: ["d-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });

  it("returns null for an unknown object_kind", () => {
    const n = makeNotification({ object_kind: "widget", object_id: "w-1" });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    expect(notificationToItemKey(n, ids, emptyJobMap)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// buildItemKey
// ---------------------------------------------------------------------------

describe("buildItemKey", () => {
  it("returns issue:<id> for issue work items", () => {
    const item: WorkItem = {
      kind: "issue",
      id: "i-42",
      data: {} as WorkItem & { kind: "issue" } extends { data: infer D }
        ? D
        : never,
      lastUpdated: "2026-01-01T00:00:00Z",
      isTerminal: false,
    };
    expect(buildItemKey(item)).toBe("issue:i-42");
  });

  it("returns patch:<id> for patch work items", () => {
    const item: WorkItem = {
      kind: "patch",
      id: "p-7",
      data: {} as WorkItem & { kind: "patch" } extends { data: infer D }
        ? D
        : never,
      lastUpdated: "2026-01-01T00:00:00Z",
      isTerminal: false,
    };
    expect(buildItemKey(item)).toBe("patch:p-7");
  });

  it("returns document:<id> for document work items", () => {
    const item: WorkItem = {
      kind: "document",
      id: "d-3",
      data: {} as WorkItem & { kind: "document" } extends { data: infer D }
        ? D
        : never,
      lastUpdated: "2026-01-01T00:00:00Z",
      isTerminal: false,
    };
    expect(buildItemKey(item)).toBe("document:d-3");
  });
});

// ---------------------------------------------------------------------------
// buildNotificationMap
// ---------------------------------------------------------------------------

describe("buildNotificationMap", () => {
  const emptyJobMap = new Map<string, string>();

  it("returns an empty map for empty notifications", () => {
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const result = buildNotificationMap([], ids, emptyJobMap);
    expect(result.size).toBe(0);
  });

  it("creates one entry for a single matching issue notification", () => {
    const n = makeNotification({
      notification_id: "n-single",
      object_kind: "issue",
      object_id: "i-1",
      summary: "Something happened",
      created_at: "2026-01-15T10:00:00Z",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const result = buildNotificationMap([n], ids, emptyJobMap);

    expect(result.size).toBe(1);
    const state = result.get("issue:i-1");
    expect(state).toEqual({
      unread: true,
      latestSummary: "Something happened",
      notificationIds: ["n-single"],
    });
  });

  it("groups multiple notifications for the same item, sorted by created_at descending", () => {
    const older = makeNotification({
      notification_id: "n-old",
      object_kind: "issue",
      object_id: "i-1",
      summary: "Older update",
      created_at: "2026-01-01T00:00:00Z",
    });
    const newer = makeNotification({
      notification_id: "n-new",
      object_kind: "issue",
      object_id: "i-1",
      summary: "Newer update",
      created_at: "2026-01-02T00:00:00Z",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    // Pass in chronological order — function should sort descending
    const result = buildNotificationMap([older, newer], ids, emptyJobMap);

    const state = result.get("issue:i-1");
    expect(state).toBeDefined();
    expect(state!.latestSummary).toBe("Newer update");
    expect(state!.notificationIds).toEqual(["n-new", "n-old"]);
  });

  it("creates separate entries for notifications targeting different items", () => {
    const n1 = makeNotification({
      notification_id: "n-1",
      object_kind: "issue",
      object_id: "i-1",
      summary: "Issue update",
    });
    const n2 = makeNotification({
      notification_id: "n-2",
      object_kind: "patch",
      object_id: "p-1",
      summary: "Patch update",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"], patch: ["p-1"] });
    const result = buildNotificationMap([n1, n2], ids, emptyJobMap);

    expect(result.size).toBe(2);
    expect(result.has("issue:i-1")).toBe(true);
    expect(result.has("patch:p-1")).toBe(true);
  });

  it("excludes notifications that do not match any known item", () => {
    const matching = makeNotification({
      notification_id: "n-match",
      object_kind: "issue",
      object_id: "i-1",
    });
    const orphan = makeNotification({
      notification_id: "n-orphan",
      object_kind: "issue",
      object_id: "i-unknown",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const result = buildNotificationMap([matching, orphan], ids, emptyJobMap);

    expect(result.size).toBe(1);
    expect(result.get("issue:i-1")!.notificationIds).toEqual(["n-match"]);
  });

  it("groups job notifications under parent issue via fallback mapping", () => {
    const jobNotif = makeNotification({
      notification_id: "n-job",
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: null,
      summary: "Job completed",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const jobMap = new Map([["j-1", "i-1"]]);
    const result = buildNotificationMap([jobNotif], ids, jobMap);

    expect(result.size).toBe(1);
    const state = result.get("issue:i-1");
    expect(state).toBeDefined();
    expect(state!.latestSummary).toBe("Job completed");
    expect(state!.notificationIds).toEqual(["n-job"]);
  });

  it("groups mixed issue and job notifications for the same parent issue", () => {
    const issueNotif = makeNotification({
      notification_id: "n-issue",
      object_kind: "issue",
      object_id: "i-1",
      summary: "Issue updated",
      created_at: "2026-01-01T00:00:00Z",
    });
    const jobNotif = makeNotification({
      notification_id: "n-job",
      object_kind: "job",
      object_id: "j-1",
      source_issue_id: "i-1",
      summary: "Job finished",
      created_at: "2026-01-02T00:00:00Z",
    });
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const result = buildNotificationMap(
      [issueNotif, jobNotif],
      ids,
      emptyJobMap,
    );

    expect(result.size).toBe(1);
    const state = result.get("issue:i-1");
    expect(state).toBeDefined();
    expect(state!.latestSummary).toBe("Job finished");
    expect(state!.notificationIds).toEqual(["n-job", "n-issue"]);
  });

  it("captures all notification IDs in the notificationIds array", () => {
    const notifs = [
      makeNotification({
        notification_id: "n-a",
        object_kind: "issue",
        object_id: "i-1",
        created_at: "2026-01-03T00:00:00Z",
      }),
      makeNotification({
        notification_id: "n-b",
        object_kind: "issue",
        object_id: "i-1",
        created_at: "2026-01-02T00:00:00Z",
      }),
      makeNotification({
        notification_id: "n-c",
        object_kind: "issue",
        object_id: "i-1",
        created_at: "2026-01-01T00:00:00Z",
      }),
    ];
    const ids = makeItemIdsByKind({ issue: ["i-1"] });
    const result = buildNotificationMap(notifs, ids, emptyJobMap);

    const state = result.get("issue:i-1");
    expect(state!.notificationIds).toHaveLength(3);
    expect(state!.notificationIds).toContain("n-a");
    expect(state!.notificationIds).toContain("n-b");
    expect(state!.notificationIds).toContain("n-c");
  });
});
