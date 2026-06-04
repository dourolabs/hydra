import { describe, it, expect } from "vitest";
import type { TriggerAction, TriggerSchedule } from "@hydra/api";
import {
  buildUpsertRequest,
  emptyAction,
  emptyTriggerDraft,
  initialDraftFromExisting,
  type TriggerDraft,
} from "../triggerDraft";

function draftWith(overrides: Partial<TriggerDraft> = {}): TriggerDraft {
  return {
    ...emptyTriggerDraft(),
    cronExpression: "0 9 * * 1-5",
    actions: [
      {
        ...emptyAction(),
        title: "Daily standup",
        description: "Post the standup thread",
      },
    ],
    ...overrides,
  };
}

describe("buildUpsertRequest (covers buildSchedule)", () => {
  describe("cron schedule branch", () => {
    it("builds a Cron schedule from a trimmed expression", () => {
      const req = buildUpsertRequest(
        draftWith({ cronExpression: "  0 9 * * 1-5  " }),
        "alice",
      );
      expect(req).not.toBeNull();
      expect(req!.schedule).toEqual({ Cron: { expression: "0 9 * * 1-5" } });
      expect(req!.creator).toBe("alice");
      expect(req!.enabled).toBe(true);
    });

    it("includes timezone when provided (trimmed)", () => {
      const req = buildUpsertRequest(
        draftWith({ cronExpression: "* * * * *", cronTimezone: "  UTC  " }),
        "alice",
      );
      expect(req!.schedule).toEqual({
        Cron: { expression: "* * * * *", timezone: "UTC" },
      });
    });

    it("omits timezone when blank or whitespace-only", () => {
      const req = buildUpsertRequest(
        draftWith({ cronExpression: "* * * * *", cronTimezone: "   " }),
        "alice",
      );
      expect(req!.schedule).toEqual({ Cron: { expression: "* * * * *" } });
      expect("timezone" in (req!.schedule as { Cron: object }).Cron).toBe(false);
    });

    it("returns null when cron expression is empty or whitespace", () => {
      expect(
        buildUpsertRequest(draftWith({ cronExpression: "" }), "alice"),
      ).toBeNull();
      expect(
        buildUpsertRequest(draftWith({ cronExpression: "   " }), "alice"),
      ).toBeNull();
    });
  });

  describe("once schedule branch", () => {
    it("builds a Once schedule from a trimmed timestamp", () => {
      const req = buildUpsertRequest(
        draftWith({
          scheduleKind: "once",
          cronExpression: "",
          onceAt: "  2026-06-10T09:00:00Z  ",
        }),
        "bob",
      );
      expect(req!.schedule).toEqual({ Once: { at: "2026-06-10T09:00:00Z" } });
    });

    it("returns null when onceAt is empty or whitespace", () => {
      expect(
        buildUpsertRequest(
          draftWith({ scheduleKind: "once", cronExpression: "", onceAt: "" }),
          "bob",
        ),
      ).toBeNull();
      expect(
        buildUpsertRequest(
          draftWith({
            scheduleKind: "once",
            cronExpression: "",
            onceAt: "   ",
          }),
          "bob",
        ),
      ).toBeNull();
    });
  });

  describe("action wire shape", () => {
    it("emits a minimal CreateIssue action when only required fields are set", () => {
      const req = buildUpsertRequest(
        draftWith({
          actions: [
            {
              ...emptyAction(),
              title: "T",
              description: "D",
            },
          ],
        }),
        "alice",
      );
      expect(req!.actions).toEqual([
        { CreateIssue: { type: "task", title: "T", description: "D" } },
      ]);
    });

    it("includes assignee (trimmed), status, and session_settings.repo_name (trimmed) when present", () => {
      const req = buildUpsertRequest(
        draftWith({
          actions: [
            {
              type: "task",
              title: "T",
              description: "D",
              assignee: "  alice  ",
              status: "open",
              repoName: "  acme/widgets  ",
            },
          ],
        }),
        "alice",
      );
      expect(req!.actions).toEqual([
        {
          CreateIssue: {
            type: "task",
            title: "T",
            description: "D",
            assignee: "alice",
            status: "open",
            session_settings: { repo_name: "acme/widgets" },
          },
        },
      ]);
    });

    it("omits assignee, status, and session_settings when blank/empty", () => {
      const req = buildUpsertRequest(
        draftWith({
          actions: [
            {
              type: "task",
              title: "T",
              description: "D",
              assignee: "   ",
              status: "",
              repoName: "   ",
            },
          ],
        }),
        "alice",
      );
      const ci = req!.actions[0].CreateIssue;
      expect(ci).toEqual({ type: "task", title: "T", description: "D" });
      expect("assignee" in ci).toBe(false);
      expect("status" in ci).toBe(false);
      expect("session_settings" in ci).toBe(false);
    });

    it("round-trips multiple actions in order", () => {
      const req = buildUpsertRequest(
        draftWith({
          actions: [
            {
              ...emptyAction(),
              title: "First",
              description: "first desc",
              assignee: "alice",
            },
            {
              ...emptyAction(),
              title: "Second",
              description: "second desc",
              repoName: "acme/widgets",
            },
          ],
        }),
        "alice",
      );
      expect(req!.actions).toHaveLength(2);
      expect(req!.actions[0].CreateIssue.title).toBe("First");
      expect(req!.actions[0].CreateIssue.assignee).toBe("alice");
      expect(req!.actions[1].CreateIssue.title).toBe("Second");
      expect(req!.actions[1].CreateIssue.session_settings).toEqual({
        repo_name: "acme/widgets",
      });
    });

    it("returns null when any action has an empty title or description", () => {
      expect(
        buildUpsertRequest(
          draftWith({
            actions: [{ ...emptyAction(), title: "", description: "D" }],
          }),
          "alice",
        ),
      ).toBeNull();
      expect(
        buildUpsertRequest(
          draftWith({
            actions: [{ ...emptyAction(), title: "T", description: "   " }],
          }),
          "alice",
        ),
      ).toBeNull();
    });

    it("propagates enabled=false through to the request", () => {
      const req = buildUpsertRequest(
        draftWith({ enabled: false }),
        "alice",
      );
      expect(req!.enabled).toBe(false);
    });
  });
});

describe("initialDraftFromExisting (covers draftFromTriggerActions)", () => {
  const cronSched: TriggerSchedule = {
    Cron: { expression: "0 9 * * 1-5", timezone: "UTC" },
  };

  it("hydrates a cron schedule and its timezone", () => {
    const draft = initialDraftFromExisting(cronSched, true, [
      {
        CreateIssue: { type: "task", title: "T", description: "D" },
      },
    ]);
    expect(draft.scheduleKind).toBe("cron");
    expect(draft.cronExpression).toBe("0 9 * * 1-5");
    expect(draft.cronTimezone).toBe("UTC");
    expect(draft.onceAt).toBe("");
    expect(draft.enabled).toBe(true);
  });

  it("hydrates a cron schedule with no timezone as empty string", () => {
    const draft = initialDraftFromExisting(
      { Cron: { expression: "* * * * *" } },
      false,
      [{ CreateIssue: { type: "task", title: "T", description: "D" } }],
    );
    expect(draft.cronTimezone).toBe("");
    expect(draft.enabled).toBe(false);
  });

  it("hydrates a Once schedule", () => {
    const draft = initialDraftFromExisting(
      { Once: { at: "2026-06-10T09:00:00Z" } },
      true,
      [{ CreateIssue: { type: "task", title: "T", description: "D" } }],
    );
    expect(draft.scheduleKind).toBe("once");
    expect(draft.onceAt).toBe("2026-06-10T09:00:00Z");
    expect(draft.cronExpression).toBe("");
    expect(draft.cronTimezone).toBe("");
  });

  it("seeds a single empty action when the actions list is empty", () => {
    const draft = initialDraftFromExisting(cronSched, true, []);
    expect(draft.actions).toEqual([emptyAction()]);
  });

  it("round-trips multiple actions, defaulting optional fields to empty strings", () => {
    const actions: TriggerAction[] = [
      {
        CreateIssue: {
          type: "task",
          title: "First",
          description: "first desc",
          assignee: "alice",
          status: "open",
          session_settings: { repo_name: "acme/widgets" },
        },
      },
      {
        CreateIssue: {
          type: "task",
          title: "Second",
          description: "second desc",
        },
      },
    ];
    const draft = initialDraftFromExisting(cronSched, true, actions);
    expect(draft.actions).toHaveLength(2);
    expect(draft.actions[0]).toEqual({
      type: "task",
      title: "First",
      description: "first desc",
      assignee: "alice",
      status: "open",
      repoName: "acme/widgets",
    });
    expect(draft.actions[1]).toEqual({
      type: "task",
      title: "Second",
      description: "second desc",
      assignee: "",
      status: "",
      repoName: "",
    });
  });

  it("normalizes null assignee/status from the wire as empty strings", () => {
    const draft = initialDraftFromExisting(cronSched, true, [
      {
        CreateIssue: {
          type: "task",
          title: "T",
          description: "D",
          assignee: null,
          status: null,
        },
      },
    ]);
    expect(draft.actions[0].assignee).toBe("");
    expect(draft.actions[0].status).toBe("");
  });

  it("survives a hydrate -> buildUpsertRequest round-trip", () => {
    const actions: TriggerAction[] = [
      {
        CreateIssue: {
          type: "task",
          title: "T",
          description: "D",
          assignee: "alice",
          status: "open",
          session_settings: { repo_name: "acme/widgets" },
        },
      },
    ];
    const draft = initialDraftFromExisting(cronSched, true, actions);
    const req = buildUpsertRequest(draft, "alice");
    expect(req).not.toBeNull();
    expect(req!.schedule).toEqual(cronSched);
    expect(req!.actions).toEqual(actions);
    expect(req!.enabled).toBe(true);
  });
});
