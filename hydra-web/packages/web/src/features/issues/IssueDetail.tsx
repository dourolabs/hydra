import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Badge, Button, Icons, MarkdownViewer, TypeChip } from "@hydra/ui";
import type { IssueVersionRecord } from "@hydra/api";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useIssue } from "./useIssue";
import { IssueRelatedIssues } from "./IssueRelatedIssues";
import { IssueActivity } from "./IssueActivity";
import { IssueUpdateModal } from "./IssueUpdateModal";
import { FeedbackModal } from "./FeedbackModal";
import { SessionList } from "../sessions/SessionList";
import { PatchList } from "../patches/PatchList";
import { IssueLabelEditor } from "./IssueLabelEditor";
import { useSessionsByIssue } from "../sessions/useSessionsByIssue";
import { useSessionDuration } from "../dashboard/useSessionDuration";
import styles from "./IssueDetail.module.css";

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "";
  const sec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day}d ago`;
  const mo = Math.floor(day / 30);
  return `${mo}mo ago`;
}

function BlockedItemLink({ issueId }: { issueId: string }) {
  const { data: record } = useIssue(issueId);
  const title = record?.issue.title || issueId;
  return (
    <span className={styles.blockedItem}>
      {record && <Badge status={normalizeIssueStatus(record.issue.status)} />}
      <Link to={`/issues/${issueId}`} className={styles.blockedLink}>
        {title}
      </Link>
    </span>
  );
}

function DepRow({ issueId }: { issueId: string }) {
  const { data: record } = useIssue(issueId);
  const title = record?.issue.title || issueId;
  return (
    <Link to={`/issues/${issueId}`} className={styles.depRow} title={title}>
      {record && <Badge status={normalizeIssueStatus(record.issue.status)} />}
      <span className={styles.depRowTitle}>{title}</span>
    </Link>
  );
}

interface IssueDetailProps {
  record: IssueVersionRecord;
}

type TabKey = "sessions" | "patches" | "activity" | "sub-issues";

const TABS: { key: TabKey; label: string }[] = [
  { key: "sessions", label: "Sessions" },
  { key: "patches", label: "Patches" },
  { key: "activity", label: "Activity" },
  { key: "sub-issues", label: "Sub-issues" },
];

export function IssueDetail({ record }: IssueDetailProps) {
  const { issue } = record;
  const issueId = record.issue_id;

  const [activeTab, setActiveTab] = useState<TabKey>("sessions");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [feedbackModalOpen, setFeedbackModalOpen] = useState(false);

  const { data: sessions } = useSessionsByIssue(issueId);
  const { durationText, isRunning } = useSessionDuration(sessions);

  const blockedOnIds = useMemo(
    () =>
      issue.dependencies
        .filter((d) => d.type === "blocked-on")
        .map((d) => d.issue_id),
    [issue.dependencies],
  );

  const parentIds = useMemo(
    () =>
      issue.dependencies
        .filter((d) => d.type === "child-of")
        .map((d) => d.issue_id),
    [issue.dependencies],
  );

  const status = normalizeIssueStatus(issue.status);
  const settings = issue.session_settings;

  return (
    <div className={styles.detail}>
      {/* ── Left column ── */}
      <div className={styles.main}>
        <div className={styles.mainInner}>
          <div className={styles.titleRow}>
            <span className={styles.titleId}>{issueId}</span>
            <Badge status={status} />
            {issue.type && issue.type !== "unknown" && <TypeChip type={issue.type} />}
            <div className={styles.headActions}>
              {isRunning && <span className={styles.sessionTimer}>{durationText}</span>}
              <Button variant="secondary" size="sm" onClick={() => setFeedbackModalOpen(true)}>
                Give feedback
              </Button>
            </div>
          </div>

          <h1 className={styles.title}>{issue.title || issueId}</h1>

          <div className={styles.metaRow}>
            {issue.creator && (
              <>
                <span>opened by {issue.creator}</span>
                <span className={styles.metaSep}>·</span>
              </>
            )}
            <span>{relativeTime(record.creation_time)}</span>
            {settings?.repo_name && (
              <>
                <span className={styles.metaSep}>·</span>
                <span>{settings.repo_name}</span>
              </>
            )}
            {settings?.branch && (
              <>
                <span className={styles.metaSep}>/</span>
                <span>{settings.branch}</span>
              </>
            )}
          </div>

          {blockedOnIds.length > 0 && (
            <div className={styles.blockedBanner}>
              <span className={styles.blockedLabel}>Blocked on</span>
              {blockedOnIds.map((id) => (
                <BlockedItemLink key={id} issueId={id} />
              ))}
            </div>
          )}

          <div className={styles.description}>
            {issue.description ? (
              <MarkdownViewer content={issue.description} />
            ) : (
              <p className={styles.descriptionEmpty}>No description.</p>
            )}
          </div>

          {issue.progress && (
            <div className={styles.section}>
              <span className={styles.sectionLabel}>Progress</span>
              <div className={styles.sectionBody}>
                <MarkdownViewer content={issue.progress} />
              </div>
            </div>
          )}

          {issue.feedback && (
            <div className={styles.section}>
              <span className={styles.sectionLabel}>Feedback</span>
              <div className={styles.sectionBody}>
                <MarkdownViewer content={issue.feedback} />
              </div>
            </div>
          )}

          <div className={styles.tabs} role="tablist">
            {TABS.map((t) => (
              <button
                key={t.key}
                type="button"
                role="tab"
                className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
                aria-selected={activeTab === t.key}
                onClick={() => setActiveTab(t.key)}
                data-testid={`issue-tab-${t.key}`}
              >
                {t.label}
              </button>
            ))}
          </div>

          <div className={styles.tabContent}>
            {activeTab === "sessions" && <SessionList issueId={issueId} />}
            {activeTab === "patches" && (
              <PatchList patchIds={issue.patches ?? []} issueId={issueId} />
            )}
            {activeTab === "activity" && <IssueActivity issueId={issueId} />}
            {activeTab === "sub-issues" && <IssueRelatedIssues issueId={issueId} />}
          </div>
        </div>
      </div>

      {/* ── Right rail ── */}
      <aside className={styles.side}>
        <div className={styles.block}>
          <span className={styles.blockLabel}>Status</span>
          <button
            type="button"
            className={styles.statusButton}
            onClick={() => setUpdateModalOpen(true)}
            data-testid="status-chip"
          >
            <Badge status={status} />
            <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
              <path
                fillRule="evenodd"
                d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
                clipRule="evenodd"
              />
            </svg>
          </button>
        </div>

        <div className={styles.block}>
          <span className={styles.blockLabel}>Assignee</span>
          {issue.assignee ? (
            <span className={styles.blockValue}>
              <Icons.IconAgent size={12} />
              {issue.assignee}
            </span>
          ) : (
            <span className={`${styles.blockValue} ${styles.blockEmpty}`}>Unassigned</span>
          )}
        </div>

        <div className={styles.block}>
          <span className={styles.blockLabel}>Type</span>
          {issue.type && issue.type !== "unknown" ? (
            <TypeChip type={issue.type} />
          ) : (
            <span className={`${styles.blockValue} ${styles.blockEmpty}`}>—</span>
          )}
        </div>

        {settings?.repo_name && (
          <div className={styles.block}>
            <span className={styles.blockLabel}>Repository</span>
            <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
              {settings.repo_name}
            </span>
          </div>
        )}

        {settings?.branch && (
          <div className={styles.block}>
            <span className={styles.blockLabel}>Branch</span>
            <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
              {settings.branch}
            </span>
          </div>
        )}

        <div className={styles.block}>
          <span className={styles.blockLabel}>Created</span>
          <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
            {formatTimestamp(record.creation_time)}
          </span>
        </div>

        <div className={styles.block}>
          <span className={styles.blockLabel}>Updated</span>
          <span className={`${styles.blockValue} ${styles.blockValueMono}`}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>

        <div className={styles.block}>
          <span className={styles.blockLabel}>Labels</span>
          <IssueLabelEditor issueId={issueId} labels={record.labels ?? []} />
        </div>

        {blockedOnIds.length > 0 && (
          <div className={styles.block}>
            <span className={styles.blockLabel}>Blocked on</span>
            <div className={styles.depList}>
              {blockedOnIds.map((id) => (
                <DepRow key={id} issueId={id} />
              ))}
            </div>
          </div>
        )}

        {parentIds.length > 0 && (
          <div className={styles.block}>
            <span className={styles.blockLabel}>Parent</span>
            <div className={styles.depList}>
              {parentIds.map((id) => (
                <DepRow key={id} issueId={id} />
              ))}
            </div>
          </div>
        )}
      </aside>

      <IssueUpdateModal
        open={updateModalOpen}
        onClose={() => setUpdateModalOpen(false)}
        issueId={issueId}
        issue={issue}
      />

      <FeedbackModal
        open={feedbackModalOpen}
        onClose={() => setFeedbackModalOpen(false)}
        issueId={issueId}
      />
    </div>
  );
}
