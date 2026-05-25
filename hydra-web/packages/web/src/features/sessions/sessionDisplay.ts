import type {
  Conversation,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { principalDisplayName } from "../principal/formatPrincipal";
import { descriptionSnippet } from "../../utils/text";

export interface SessionDisplay {
  /** Title to render for the session row. Falls back through linked issue
   *  title → conversation title → session prompt. */
  title: string;
  /** Linked issue id, if any. */
  issueId: string | null;
  /** Linked conversation id, if any. */
  conversationId: string | null;
  /** Agent attribution from the linked entity (issue assignee or conversation
   *  agent_name). Null when neither linked entity exposes an agent. */
  agentName: string | null;
}

export function resolveSessionDisplay(
  record: SessionSummaryRecord,
  issueMap: Map<string, IssueSummaryRecord>,
  conversationMap: Map<string, Conversation>,
): SessionDisplay {
  const s = record.session;
  const issueId = s.spawned_from ?? null;
  const conversationId = s.conversation_id ?? null;
  const issue = issueId ? issueMap.get(issueId) ?? null : null;
  const conversation = conversationId
    ? conversationMap.get(conversationId) ?? null
    : null;

  const promptSnippet = descriptionSnippet(s.prompt);
  const title =
    (issue?.issue.title && issue.issue.title.trim()) ||
    (conversation?.title && conversation.title.trim()) ||
    promptSnippet ||
    "(no title)";

  // Phase 4b: `Issue.assignee` is now typed. The agent-attribution label
  // still wants a bare username, so unwrap the principal here.
  const agentName =
    (issue?.issue.assignee && principalDisplayName(issue.issue.assignee)) ??
    conversation?.agent_name ??
    null;

  return { title, issueId, conversationId, agentName };
}
