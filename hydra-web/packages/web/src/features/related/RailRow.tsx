import { useNavigate } from "react-router-dom";
import { Avatar, Icons, StatusDot, TypeChip } from "@hydra/ui";
import type { BadgeStatus } from "@hydra/ui";
import type {
  ConversationSummary,
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  ProjectRecord,
  RepositoryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import {
  normalizePatchStatus,
  normalizeSessionStatus,
} from "../../utils/badgeStatus";
import { descriptionSnippet } from "../../utils/text";
import { formatTokenCount } from "../../utils/tokens";
import { conversationTitle } from "../chat/conversationTitle";
import { CONVERSATION_STATUS_TONES } from "../chat/conversationStatusBadge";
import { principalAvatarKind, principalDisplayName } from "../principal/formatPrincipal";
import type { SessionDisplay } from "../sessions/sessionDisplay";
import { AgoTime, RunTime } from "../../components/Runtime/Runtime";
import { useSessionDuration, useSingleSessionDuration } from "../dashboard/useSessionDuration";
import type { ChildStatus } from "../dashboard/computeIssueProgress";
import { PatchRepoLink } from "../patches/PatchRepoLink";
import { ProjectChip } from "../projects/ProjectChip";
import { useProjects } from "../projects/useProjects";
import styles from "./RailRow.module.css";

function resolveProjectKey(
  projects: ProjectRecord[] | undefined,
  projectId: string | null | undefined,
): string | null {
  if (!projects) return null;
  if (projectId) {
    return projects.find((p) => p.project_id === projectId)?.project.key ?? null;
  }
  return projects.find((p) => p.project.key === "default")?.project.key ?? null;
}

interface IssueRailRowProps {
  record: IssueSummaryRecord;
  sessions?: SessionSummaryRecord[];
  /** Child issue statuses for computing the progress bar fraction. Mirrors the
   * desktop IssuesTable wiring; when omitted (e.g. Related-tab contexts that
   * don't have a tree fetch), the progress bar is suppressed. */
  childStatuses?: ChildStatus[];
  /** Optional query string (including leading "?") appended to the link target. */
  linkSearch?: string;
}

function progressFraction(children: ChildStatus[] | undefined): number | null {
  if (!children || children.length === 0) return null;
  const total = children.length;
  const projected = children.filter(
    (c) => c.status === "closed" || c.status === "issue-closed" || c.status === "in-progress",
  ).length;
  return Math.round((projected / total) * 100);
}

function isActive(s: SessionSummaryRecord): boolean {
  return s.session.status === "running" || s.session.status === "pending";
}

export function IssueRailRow({ record, sessions, childStatuses, linkSearch }: IssueRailRowProps) {
  const navigate = useNavigate();
  const issue = record.issue;
  const archived = issue.deleted === true;
  const resolved = issue.status;
  const hasActive = sessions?.some(isActive) ?? false;
  // Resolved status drives the dot color directly. Active sessions
  // override to the "in-progress" tone class even on terminal statuses
  // (a closed issue with a running session reads as still in flight).
  const dotColor = hasActive ? undefined : resolved.color;
  const dotTone: BadgeStatus = hasActive ? "in-progress" : "open";
  const pct = progressFraction(childStatuses);
  const hasActiveChild = !!childStatuses?.some((c) => c.hasActiveTask);
  const { durationText, status: runtimeStatus } = useSessionDuration(sessions);
  const assigneeName = issue.assignee ? principalDisplayName(issue.assignee) : null;
  const { data: projects } = useProjects();
  const projectKey = resolveProjectKey(projects, issue.project_id);
  const to = `/issues/${record.issue_id}${linkSearch ?? ""}`;

  return (
    <div
      className={archived ? `${styles.row} ${styles.archived}` : styles.row}
      onClick={() => navigate(to)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          navigate(to);
        }
      }}
      data-testid={`related-rail-row-issue-${record.issue_id}`}
      data-archived={archived ? "true" : undefined}
    >
      {dotColor ? (
        <span
          className={styles.dotCustomColor}
          style={{ backgroundColor: dotColor }}
          aria-hidden="true"
        />
      ) : (
        <StatusDot status={dotTone} className={styles.dotInRow} />
      )}
      <div className={styles.body}>
        <div className={styles.title}>{issue.title || "(untitled)"}</div>
        <div className={styles.meta}>
          {projectKey && (
            <ProjectChip
              projectKey={projectKey}
              className={styles.projectChip}
              data-testid={`rail-row-project-chip-${record.issue_id}`}
            />
          )}
          {resolved.label && (
            <span className={styles.statusLabel}>{resolved.label}</span>
          )}
          {archived && (
            <span
              className={styles.archivedTag}
              data-testid={`related-rail-row-archived-${record.issue_id}`}
            >
              ARCHIVED
            </span>
          )}
          {issue.type && issue.type !== "unknown" && <TypeChip type={issue.type} />}
          {issue.assignee && assigneeName && (
            <Avatar
              name={assigneeName}
              kind={principalAvatarKind(issue.assignee)}
              size="sm"
              title={`Assignee · ${assigneeName}`}
            />
          )}
          {pct !== null && (
            <span className={styles.progressBar} aria-hidden="true">
              <span
                className={`${styles.progressFill}${hasActiveChild ? ` ${styles.progressFillActive}` : ""}`}
                style={{ width: `${pct}%` }}
              />
            </span>
          )}
          {durationText !== "—" && <RunTime value={durationText} status={runtimeStatus} />}
          <AgoTime iso={record.timestamp} />
        </div>
      </div>
    </div>
  );
}

interface PatchRailRowProps {
  record: PatchSummaryRecord;
  /** Optional query string (including leading "?") appended to the link target. */
  linkSearch?: string;
}

export function PatchRailRow({ record, linkSearch }: PatchRailRowProps) {
  const navigate = useNavigate();
  const p = record.patch;
  const status: BadgeStatus =
    p.status === "Open" && p.review_summary.approved ? "approved" : normalizePatchStatus(p.status);
  const to = `/patches/${record.patch_id}${linkSearch ?? ""}`;

  return (
    <div
      className={styles.row}
      onClick={() => navigate(to)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          navigate(to);
        }
      }}
      data-testid={`related-rail-row-patch-${record.patch_id}`}
    >
      <StatusDot status={status} className={styles.dotInRow} />
      <div className={styles.body}>
        <div className={styles.title}>{p.title || "(untitled)"}</div>
        <div className={styles.meta}>
          <span className={styles.metaMono}>
            <PatchRepoLink patch={p} />
          </span>
          {p.review_summary.count > 0 && (
            <span
              className={`${styles.metaMono}${p.review_summary.approved ? ` ${styles.metaApproved}` : ""}`}
            >
              {p.review_summary.count} review{p.review_summary.count === 1 ? "" : "s"}
              {p.review_summary.approved ? " ✓" : ""}
            </span>
          )}
          <AgoTime iso={record.timestamp} />
        </div>
      </div>
    </div>
  );
}

interface SessionRailRowProps {
  record: SessionSummaryRecord;
  /** Linked-entity display data resolved by the caller (title + agent
   *  derived from the linked issue or conversation). Optional so callers
   *  that don't resolve linked entities can fall back to the raw prompt. */
  display?: SessionDisplay;
}

export function SessionRailRow({ record, display }: SessionRailRowProps) {
  const navigate = useNavigate();
  const s = record.session;
  const status: BadgeStatus = normalizeSessionStatus(s.status);
  const title = display?.title || descriptionSnippet(s.prompt) || "(no prompt)";
  const agentName = display?.agentName ?? null;
  const { durationText, status: runtimeStatus } = useSingleSessionDuration(record);

  return (
    <div
      className={styles.row}
      onClick={() => navigate(`/sessions/${record.session_id}`)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          navigate(`/sessions/${record.session_id}`);
        }
      }}
      data-testid={`related-rail-row-session-${record.session_id}`}
    >
      <StatusDot status={status} className={styles.dotInRow} />
      <div className={styles.body}>
        <div className={styles.title}>{title}</div>
        <div className={styles.meta}>
          {agentName && <span className={styles.agent}>{agentName}</span>}
          {s.usage && (
            <span
              className={styles.tokens}
              title={`${s.usage.input_tokens} input · ${s.usage.cache_read_input_tokens} cache read · ${s.usage.cache_creation_input_tokens} cache creation · ${s.usage.output_tokens} output`}
            >
              <span className={styles.tokensInput}>
                <span aria-hidden="true">↓</span>
                {formatTokenCount(
                  s.usage.input_tokens +
                    s.usage.cache_read_input_tokens +
                    s.usage.cache_creation_input_tokens,
                )}
              </span>
              <span className={styles.tokensOutput}>
                <span aria-hidden="true">↑</span>
                {formatTokenCount(s.usage.output_tokens)}
              </span>
            </span>
          )}
          {durationText !== "—" && <RunTime value={durationText} status={runtimeStatus} />}
          <AgoTime iso={record.timestamp} />
        </div>
      </div>
    </div>
  );
}

interface DocumentRailRowProps {
  record: DocumentSummaryRecord;
}

function documentTitle(doc: DocumentSummaryRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}

export function DocumentRailRow({ record }: DocumentRailRowProps) {
  const navigate = useNavigate();
  return (
    <div
      className={styles.row}
      onClick={() => navigate(`/documents/${record.document_id}`)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          navigate(`/documents/${record.document_id}`);
        }
      }}
      data-testid={`related-rail-row-document-${record.document_id}`}
    >
      <Icons.IconDoc size={14} className={styles.docIcon} aria-hidden="true" />
      <div className={styles.body}>
        <div className={styles.title}>{documentTitle(record)}</div>
        <div className={styles.meta}>
          {record.document.path && <span className={styles.metaMono}>{record.document.path}</span>}
          <AgoTime iso={record.timestamp} />
        </div>
      </div>
    </div>
  );
}

interface ChatRailRowProps {
  conversation: ConversationSummary;
}

export function ChatRailRow({ conversation }: ChatRailRowProps) {
  const navigate = useNavigate();
  const status: BadgeStatus = CONVERSATION_STATUS_TONES[conversation.status];
  const messageLabel =
    conversation.event_count === 1 ? "1 msg" : `${conversation.event_count} msgs`;
  const to = `/chat/${conversation.conversation_id}`;

  return (
    <div
      className={styles.row}
      onClick={() => navigate(to)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          navigate(to);
        }
      }}
      data-testid={`related-rail-row-chat-${conversation.conversation_id}`}
    >
      <StatusDot status={status} className={styles.dotInRow} />
      <div className={styles.body}>
        <div className={styles.title}>{conversationTitle(conversation)}</div>
        <div className={styles.meta}>
          <span className={styles.metaMono}>{conversation.creator}</span>
          <span className={styles.metaMono}>{messageLabel}</span>
          <AgoTime iso={conversation.updated_at} />
        </div>
      </div>
    </div>
  );
}

interface RepositoryRailRowProps {
  record: RepositoryRecord;
}

export function RepositoryRailRow({ record }: RepositoryRailRowProps) {
  return (
    <div className={styles.row} data-testid={`related-rail-row-repository-${record.name}`}>
      <Icons.IconRepo size={14} className={styles.docIcon} aria-hidden="true" />
      <div className={styles.body}>
        <div className={styles.title}>{record.name}</div>
        <div className={styles.meta}>
          {record.repository.default_branch && (
            <span className={styles.metaMono}>{record.repository.default_branch}</span>
          )}
        </div>
      </div>
    </div>
  );
}
