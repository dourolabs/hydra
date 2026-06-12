import { test, expect } from "../fixtures/auth";
import type { Page } from "@playwright/test";
import type { SessionEvent } from "@hydra/api";

// Verifies the chat reference preview cards: `[[id]]` references in a chat
// message body additionally render as a preview card stacked at the end of
// the message. Inline `[[id]]` rendering in the body is unchanged.
//
// All IDs in this test use lowercase ASCII letters only (4–12 chars after
// the prefix) so they match the `[ipdcs(l)]-[a-z]{4,12}` regex shared by
// `remarkHydraLinks` (inline-link extractor) and `extractHydraReferences`
// (card extractor). Each referenced-object endpoint is mocked below so the
// cards hydrate against synthetic records without depending on the
// mock-server seed file.

const CONVERSATION_ID = "c-refsconv";
const SESSION_ID = "t-refssess";

// IDs referenced from the message bodies. The session uses the `s-` prefix
// expected by the inline-link regex (the mock server happens to mint
// session ids with `t-`, but mocked routes accept any string).
const ISSUE_ID = "i-mainissue";
const PATCH_ID = "p-mainpatch";
const DOCUMENT_ID = "d-maindoc";
const SESSION_REF_ID = "s-mainsess";
const CONVERSATION_REF_ID = "c-mainchat";
const LABEL_ID = "l-mainlabel";
const FENCED_ID = "i-fencedone";
const USER_ISSUE_ID = "i-userissue";

const conversation = {
  conversation_id: CONVERSATION_ID,
  title: "References test conversation",
  agent_name: "swe",
  status: "active",
  creator: "dev-user",
  created_at: "2026-05-15T10:00:00Z",
  updated_at: "2026-05-15T10:30:00Z",
};

const conversationSummary = {
  ...conversation,
  event_count: 2,
  last_event_preview: null,
};

const sessionSummary = {
  session_id: SESSION_ID,
  version: 1,
  timestamp: "2026-05-15T10:00:00Z",
  session: {
    prompt: "",
    creator: "dev-user",
    status: "running",
    creation_time: "2026-05-15T10:00:00Z",
    conversation_id: CONVERSATION_ID,
  },
};

// Assistant message exercises:
//   - the full kind dispatch (issue/patch/document/session/conversation)
//   - dedupe (a second `[[i-mainissue]]` deeper in the body)
//   - the label exclusion (`[[l-mainlabel]]` is rendered inline but never as a card)
//   - the code-fence skip (`[[i-fencedone]]` inside a fenced block is not extracted)
const ASSISTANT_BODY =
  "Status update across the platform-v2 effort:\n" +
  "\n" +
  `- Parent: [[${ISSUE_ID}]]\n` +
  `- Latest patch: [[${PATCH_ID}]]\n` +
  `- Reference doc: [[${DOCUMENT_ID}]]\n` +
  `- Session that ran the migration: [[${SESSION_REF_ID}]]\n` +
  `- Earlier chat: [[${CONVERSATION_REF_ID}]]\n` +
  `- Label (inline only): [[${LABEL_ID}]]\n` +
  "\n" +
  `Reminder for context, see [[${ISSUE_ID}]] again.\n` +
  "\n" +
  "```\n" +
  `Do not extract: [[${FENCED_ID}]]\n` +
  "```\n";

const USER_BODY = `Quick follow-up on [[${USER_ISSUE_ID}]].`;

const sessionEvents: SessionEvent[] = [
  {
    type: "assistant_message",
    content: ASSISTANT_BODY,
    timestamp: "2026-05-15T10:01:00Z",
  },
  {
    type: "user_message",
    content: USER_BODY,
    timestamp: "2026-05-15T10:02:00Z",
  },
];

// Minimal records returned by each kind's GET endpoint. The cards and the
// inline links only depend on a handful of display fields; everything else
// is tolerated by the hooks/components.

function issueRecord(id: string, title: string) {
  return {
    issue_id: id,
    version: 1,
    timestamp: "2026-05-15T09:55:00Z",
    creation_time: "2026-05-15T09:55:00Z",
    issue: {
      type: "feature",
      title,
      description: `${title} description first line.`,
      creator: "dev-user",
      progress: "",
      status: "open",
      assignee: null,
      dependencies: [],
      patches: [],
    },
  };
}

function patchRecord(id: string, title: string) {
  return {
    patch_id: id,
    version: 1,
    timestamp: "2026-05-15T09:55:00Z",
    creation_time: "2026-05-15T09:55:00Z",
    patch: {
      title,
      description: `${title} description first line.`,
      diff: "",
      status: "Open",
      is_automatic_backup: false,
      creator: "dev-user",
      reviews: [],
      service_repo_name: "acme/web-app",
    },
  };
}

function documentRecord(id: string, title: string) {
  return {
    document_id: id,
    version: 1,
    timestamp: "2026-05-15T09:55:00Z",
    creation_time: "2026-05-15T09:55:00Z",
    document: {
      title,
      body_markdown: `${title} body first line.`,
      path: `/research/${title.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`,
    },
  };
}

function sessionRecord(id: string) {
  return {
    session_id: id,
    version: 1,
    timestamp: "2026-05-15T09:55:00Z",
    session: {
      creator: "dev-user",
      status: "complete",
      creation_time: "2026-05-15T09:30:00Z",
      end_time: "2026-05-15T09:55:00Z",
      agent_config: { agent_name: "swe" },
      mount_spec: { working_dir: "repo", mounts: [] },
      mode: { type: "task" },
    },
  };
}

function conversationRecord(id: string, title: string) {
  return {
    conversation_id: id,
    title,
    agent_name: "scribe",
    status: "active",
    creator: "dev-user",
    created_at: "2026-05-14T09:00:00Z",
    updated_at: "2026-05-14T09:30:00Z",
  };
}

async function mockChatRoutes(page: Page) {
  await page.route(/\/api\/v1\/conversations(\?|$)/, (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ conversations: [conversationSummary] }),
    });
  });
  await page.route(new RegExp(`/api/v1/conversations/${CONVERSATION_ID}$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(conversation),
    });
  });
  await page.route(new RegExp(`/api/v1/conversations/${CONVERSATION_REF_ID}$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(conversationRecord(CONVERSATION_REF_ID, "Earlier chat")),
    });
  });
  await page.route(new RegExp(`/api/v1/sessions/${SESSION_ID}/events($|\\?)`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(sessionEvents),
    });
  });
  // Referenced-session GET (used by the SessionPreviewCard).
  await page.route(new RegExp(`/api/v1/sessions/${SESSION_REF_ID}$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(sessionRecord(SESSION_REF_ID)),
    });
  });
  await page.route(/\/api\/v1\/sessions(\?|$)/, (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ sessions: [sessionSummary] }),
    });
  });
  // Issue / patch / document GETs. Match any id under each collection so the
  // mocks also cover fall-through assertions (e.g. unused fenced/label ids).
  // `useIssue` appends `?include_deleted=true` to the URL, so the issue
  // matcher tolerates either bare `/<id>` or a trailing query string.
  await page.route(/\/api\/v1\/issues\/[^/?]+(?:\?|$)/, (route) => {
    const id = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(issueRecord(id, `Issue ${id} title`)),
    });
  });
  await page.route(/\/api\/v1\/patches\/([^/?]+)$/, (route) => {
    const id = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(patchRecord(id, `Patch ${id} title`)),
    });
  });
  await page.route(/\/api\/v1\/documents\/([^/?]+)$/, (route) => {
    const id = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(documentRecord(id, `Document ${id} title`)),
    });
  });
  // Labels — used by the inline LabelLink for `[[l-mainlabel]]`. The card
  // path explicitly skips labels regardless of this response.
  await page.route(new RegExp(`/api/v1/labels/${LABEL_ID}$`), (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        label_id: LABEL_ID,
        name: "main-label",
        color: "#3498db",
        recurse: true,
        hidden: false,
        created_at: "2026-05-15T09:00:00Z",
        updated_at: "2026-05-15T09:00:00Z",
      }),
    });
  });
}

test.describe("Chat reference preview cards @chat:reference-preview-cards", () => {
  test("assistant + user messages stack preview cards in source order with dedupe and code-fence skip @chat:reference-preview-cards", async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    const list = page.getByTestId("chat-message-list");
    await expect(list).toBeVisible();
    await expect(list).toContainText("Status update across the platform-v2 effort");

    // Two messages, two preview blocks (one per message body).
    const previewBlocks = page.getByTestId("message-references-preview");
    await expect(previewBlocks).toHaveCount(2);

    // ── Assistant block ────────────────────────────────────────────────
    const assistantBlock = previewBlocks.first();
    await expect(
      assistantBlock.getByRole("button", { name: new RegExp(`^Issue ${ISSUE_ID}`) }),
    ).toBeVisible();
    await expect(
      assistantBlock.getByRole("button", { name: new RegExp(`^Patch ${PATCH_ID}`) }),
    ).toBeVisible();
    await expect(
      assistantBlock.getByRole("button", { name: new RegExp(`^Document ${DOCUMENT_ID}`) }),
    ).toBeVisible();
    await expect(
      assistantBlock.getByRole("button", { name: new RegExp(`^Session ${SESSION_REF_ID}`) }),
    ).toBeVisible();
    await expect(
      assistantBlock.getByRole("button", {
        name: new RegExp(`^Conversation ${CONVERSATION_REF_ID}`),
      }),
    ).toBeVisible();

    // Exactly five cards: dedupe collapsed the second [[i-mainissue]] into
    // the first; the label and the code-fenced id produced no cards.
    const assistantCards = assistantBlock.getByRole("button");
    await expect(assistantCards).toHaveCount(5);

    // Cards are emitted in source order.
    const assistantLabels = await assistantCards.evaluateAll((els) =>
      els.map((el) => el.getAttribute("aria-label") ?? ""),
    );
    expect(assistantLabels.map((l) => l.split(" ")[0])).toEqual([
      "Issue",
      "Patch",
      "Document",
      "Session",
      "Conversation",
    ]);

    // No card for the label reference.
    await expect(assistantBlock.getByRole("button", { name: new RegExp(LABEL_ID) })).toHaveCount(0);

    // No card for the id inside the fenced code block.
    await expect(assistantBlock.getByRole("button", { name: new RegExp(FENCED_ID) })).toHaveCount(0);

    // Inline `[[id]]` rendering for assistant messages is unchanged: the
    // assistant body still emits a titled anchor for each id. The deduped
    // second occurrence of [[i-mainissue]] still renders as its own link
    // inline — dedupe is for cards, not for inline links.
    const inlineIssueLinks = page.locator(`a[href="/issues/${ISSUE_ID}"]`);
    await expect(inlineIssueLinks.first()).toBeVisible();
    expect(await inlineIssueLinks.count()).toBeGreaterThanOrEqual(2);
    await expect(page.locator(`a[href="/patches/${PATCH_ID}"]`).first()).toBeVisible();
    await expect(page.locator(`a[href="/documents/${DOCUMENT_ID}"]`).first()).toBeVisible();

    // ── User block ─────────────────────────────────────────────────────
    const userBlock = previewBlocks.nth(1);
    await expect(
      userBlock.getByRole("button", { name: new RegExp(`^Issue ${USER_ISSUE_ID}`) }),
    ).toBeVisible();
    await expect(userBlock.getByRole("button")).toHaveCount(1);

    // The inline `[[i-userissue]]` in the user bubble stays as plain text —
    // no anchor is rendered there. The card alone provides navigation.
    await expect(page.locator(`a[href="/issues/${USER_ISSUE_ID}"]`)).toHaveCount(0);
    await expect(list).toContainText(`[[${USER_ISSUE_ID}]]`);

    // ── Whole-surface click navigates to the target route ─────────────
    const issueCard = assistantBlock.getByRole("button", {
      name: new RegExp(`^Issue ${ISSUE_ID}`),
    });
    await issueCard.click();
    await expect(page).toHaveURL(new RegExp(`/issues/${ISSUE_ID}$`));
  });

  test("preview card is keyboard-focusable; Enter activates onClick @chat:reference-preview-cards", async ({
    authenticatedPage: page,
  }) => {
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    const assistantBlock = page.getByTestId("message-references-preview").first();
    const issueCard = assistantBlock.getByRole("button", {
      name: new RegExp(`^Issue ${ISSUE_ID}`),
    });
    await expect(issueCard).toBeVisible();

    await issueCard.focus();
    await expect(issueCard).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/issues/${ISSUE_ID}$`));
  });

  test("no horizontal overflow at 375px mobile viewport @chat:reference-preview-cards", async ({
    authenticatedPage: page,
  }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.addInitScript(() => {
      window.localStorage.setItem("hydra-sidebar-hidden", "1");
    });
    await mockChatRoutes(page);
    await page.goto(`/chat/${CONVERSATION_ID}`);

    // Wait until at least one card has hydrated so we measure the actual
    // laid-out width, not the pre-hydration skeleton.
    await expect(
      page.getByTestId("message-references-preview").first().getByRole("button").first(),
    ).toBeVisible();
    await page.waitForLoadState("networkidle");

    const overflow = await page.evaluate(() => {
      const root = document.documentElement;
      return { scrollWidth: root.scrollWidth, clientWidth: root.clientWidth };
    });
    expect(
      overflow.scrollWidth,
      `scrollWidth=${overflow.scrollWidth} clientWidth=${overflow.clientWidth}`,
    ).toBeLessThanOrEqual(overflow.clientWidth + 1);
  });
});
