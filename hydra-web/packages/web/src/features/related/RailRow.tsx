import { useNavigate } from "react-router-dom";
import { Icons, TypeChip } from "@hydra/ui";
import type { BadgeStatus } from "@hydra/ui";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import {
  normalizeIssueStatus,
  normalizePatchStatus,
  normalizeSessionStatus,
} from "../../utils/statusMapping";
import { descriptionSnippet } from "../../utils/text";
import { formatTokenCount } from "../../utils/tokens";
import { AgoTime, RunTime } from "../../components/Runtime/Runtime";
import {
  useSessionDuration,
  useSingleSessionDuration,
} from "../dashboard/useSessionDuration";
import type { ChildStatus } from "../dashboard/computeIssueProgress";
import styles from "./RailRow.module.css";

const STATUS_DOT_CLASS: Partial<Record<BadgeStatus, string>> = {
  open: styles.toneOpen,
  "in-progress": styles.toneInProgress,
  closed: styles.toneClosed,
  "issue-closed": styles.toneClosed,
  approved: styles.toneClosed,
  failed: styles.toneFailed,
  dropped: styles.toneDropped,
  blocked: styles.toneBlocked,
  pending: styles.toneInProgress,
  running: styles.toneInProgress,
  complete: styles.toneClosed,
  "changes-requested": styles.toneRejected,
  rejected: styles.toneRejected,
  merged: styles.toneClosed,
};

function StatusDot({ status }: { status: BadgeStatus }) {
  const cls = STATUS_DOT_CLASS[status] ?? styles.toneNeutral;
  return <span className={`${styles.dot} ${cls}`} aria-hidden="true" />;
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
  const normalized = normalizeIssueStatus(issue.status);
  const hasActive = sessions?.some(isActive) ?? false;
  const status: BadgeStatus = hasActive ? "in-progress" : normalized;
  const pct = progressFraction(childStatuses);
  const hasActiveChild = !!childStatuses?.some((c) => c.hasActiveTask);
  const { durationText, status: runtimeStatus } = useSessionDuration(sessions);
  const to = `/issues/${record.issue_id}${linkSearch ?? ""}`;

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
      data-testid={`related-rail-row-issue-${record.issue_id}`}
    >
      <StatusDot status={status} />
      <div className={styles.body}>
        <div className={styles.title}>{issue.title || "(untitled)"}</div>
        <div className={styles.meta}>
          {issue.type && issue.type !== "unknown" && <TypeChip type={issue.type} />}
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
    p.status === "Open" && p.review_summary.approved
      ? "approved"
      : normalizePatchStatus(p.status);
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
      <StatusDot status={status} />
      <div className={styles.body}>
        <div className={styles.title}>{p.title || "(untitled)"}</div>
        <div className={styles.meta}>
          <span className={styles.metaMono}>{p.service_repo_name}</span>
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
}

export function SessionRailRow({ record }: SessionRailRowProps) {
  const navigate = useNavigate();
  const s = record.session;
  const status: BadgeStatus = normalizeSessionStatus(s.status);
  const promptText = descriptionSnippet(s.prompt) || "(no prompt)";
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
      <StatusDot status={status} />
      <div className={styles.body}>
        <div className={styles.title}>{promptText}</div>
        <div className={styles.meta}>
          <span className={styles.agent}>{s.creator}</span>
          {s.usage && (
            <span
              className={styles.tokens}
              title={`${s.usage.input_tokens} input · ${s.usage.output_tokens} output`}
            >
              <span className={styles.tokensInput}>
                <span aria-hidden="true">↓</span>
                {formatTokenCount(s.usage.input_tokens)}
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
          {record.document.path && (
            <span className={styles.metaMono}>{record.document.path}</span>
          )}
          <AgoTime iso={record.timestamp} />
        </div>
      </div>
    </div>
  );
}
