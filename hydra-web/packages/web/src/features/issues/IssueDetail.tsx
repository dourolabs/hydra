import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Badge, Button, TypeChip } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import type { IssueVersionRecord } from "@hydra/api";
import { StatusChip } from "../projects/StatusChip";
import { useConversations } from "../chat/useConversations";
import { useIssue } from "./useIssue";
import { IssueAssigneePicker } from "./IssueAssigneePicker";
import { IssueProjectPicker } from "./IssueProjectPicker";
import { IssueStatusPicker } from "./IssueStatusPicker";
import { IssueRightPanel, type IssueRightPanelTabKey } from "./IssueRightPanel";
import { IssueUpdateModal } from "./IssueUpdateModal";
import { FeedbackModal } from "./FeedbackModal";
import { ArchiveIssueButton } from "./ArchiveIssueButton";
import { useArchiveIssue } from "./useArchiveIssue";
import { FormPanel } from "./FormPanel";
import { CommentsPanel } from "./CommentsPanel";
import { SessionList } from "../sessions/SessionList";
import { useSessionsByIssue } from "../sessions/useSessionsByIssue";
import { useSessionDuration } from "../dashboard/useSessionDuration";
import { MobileTabBar, type MobileTabBarItem } from "../../components/MobileTabBar";
import {
  OverflowMenu,
  type OverflowMenuItem,
} from "../../components/OverflowMenu";
import styles from "./IssueDetail.module.css";

type MobileTabKey = "overview" | IssueRightPanelTabKey;

const MOBILE_TABS: MobileTabBarItem[] = [
  { key: "overview", label: "Overview" },
  { key: "related", label: "Related" },
  { key: "activity", label: "Activity" },
  { key: "details", label: "Details" },
];

function BlockedItemLink({ issueId }: { issueId: string }) {
  const { data: record } = useIssue(issueId);
  const title = record?.issue.title || issueId;
  return (
    <span className={styles.blockedItem}>
      {record && <StatusChip status={record.issue.status} />}
      <Link to={`/issues/${issueId}`} className={styles.blockedLink}>
        {title}
      </Link>
    </span>
  );
}

interface IssueDetailProps {
  record: IssueVersionRecord;
}

export function IssueDetail({ record }: IssueDetailProps) {
  const { issue } = record;
  const issueId = record.issue_id;
  const navigate = useNavigate();

  const [mobileTab, setMobileTab] = useState<MobileTabKey>("overview");
  const [rightPanelTab, setRightPanelTab] = useState<IssueRightPanelTabKey>("related");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [feedbackModalOpen, setFeedbackModalOpen] = useState(false);
  // Local-only coordination state for the project/status pickers. When
  // the user picks a different project from the project picker we DO NOT
  // auto-commit — instead we stash the pending id here and force the
  // status picker into "Select a status…" mode. The combined change is
  // committed atomically when the user picks a status. Not persisted
  // across reloads or navigation — same trade-off as the right-rail
  // IssueUpdateModal.
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);

  const { data: sessions } = useSessionsByIssue(issueId);
  const { durationText, isRunning } = useSessionDuration(sessions);
  const { archive: archiveIssue, isPending: archivePending } = useArchiveIssue(issueId);

  // Live (non-closed) spawned conversation for this issue, if any. The
  // server-side filter narrows to this issue; we then pick the first
  // Active/Idle row to drive the header affordance.
  const { data: spawnedConversations } = useConversations(
    { spawned_from: issueId, include_deleted: false },
    { enabled: !!issueId },
  );
  const liveConversation = useMemo(
    () => spawnedConversations?.find((c) => c.status !== "closed") ?? null,
    [spawnedConversations],
  );

  const blockedOnIds = useMemo(
    () => issue.dependencies.filter((d) => d.type === "blocked-on").map((d) => d.issue_id),
    [issue.dependencies],
  );

  const handleMobileTabChange = useCallback((key: string) => {
    switch (key) {
      case "overview":
        setMobileTab("overview");
        return;
      case "related":
      case "activity":
      case "details":
        setMobileTab(key);
        setRightPanelTab(key);
        return;
    }
  }, []);

  const handleRightPanelChange = useCallback((key: IssueRightPanelTabKey) => {
    setRightPanelTab(key);
  }, []);

  const overviewActive = mobileTab === "overview";

  const overflowItems = useMemo<OverflowMenuItem[]>(() => {
    const items: OverflowMenuItem[] = [];
    if (liveConversation) {
      const label =
        liveConversation.status === "idle"
          ? "Resume Conversation"
          : "Open Conversation";
      items.push({
        key: "conversation",
        label,
        onSelect: () => navigate(`/chat/${liveConversation.conversation_id}`),
        testId: "issue-overflow-conversation",
      });
    }
    items.push({
      key: "feedback",
      label: "Give feedback",
      onSelect: () => setFeedbackModalOpen(true),
      testId: "issue-overflow-feedback",
    });
    if (issue.deleted !== true) {
      items.push({
        key: "archive",
        label: archivePending ? "Archiving…" : "Archive",
        onSelect: archiveIssue,
        disabled: archivePending,
        testId: "issue-overflow-archive",
      });
    }
    return items;
  }, [liveConversation, navigate, issue.deleted, archivePending, archiveIssue]);

  return (
    <div className={styles.detail}>
      <MobileTabBar
        className={styles.mobileTabBar}
        tabs={MOBILE_TABS}
        activeKey={mobileTab}
        onChange={handleMobileTabChange}
        testIdPrefix="issue-mobile-tab-"
      />
      {/* ── Left column ── */}
      <div
        className={styles.main}
        data-mobile-active={overviewActive ? "true" : "false"}
        data-testid="issue-detail-main"
      >
        <div className={styles.mainInner}>
          <div className={styles.titleRow}>
            <span className={styles.titleId}>{issueId}</span>
            {issue.type && issue.type !== "unknown" && <TypeChip type={issue.type} />}
            {issue.deleted === true && (
              <Badge status="archived" data-testid="issue-archived-badge" />
            )}
            <div className={styles.headRight}>
              {isRunning && (
                <span className={styles.sessionTimer}>{durationText}</span>
              )}
              <div className={styles.headActions}>
                {liveConversation && (
                  <Link
                    to={`/chat/${liveConversation.conversation_id}`}
                    data-testid="issue-open-conversation"
                    data-conversation-status={liveConversation.status}
                    className={styles.openConversation}
                  >
                    {liveConversation.status === "idle"
                      ? "Resume Conversation"
                      : "Open Conversation"}
                  </Link>
                )}
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => setFeedbackModalOpen(true)}
                >
                  Give feedback
                </Button>
                {issue.deleted !== true && (
                  <ArchiveIssueButton
                    issueId={issueId}
                    variant="secondary"
                    data-testid="issue-detail-archive"
                  />
                )}
              </div>
              <div className={styles.headOverflow}>
                <OverflowMenu
                  items={overflowItems}
                  triggerLabel="More actions"
                  triggerTestId="issue-overflow-trigger"
                  menuTestId="issue-overflow-menu"
                />
              </div>
            </div>
          </div>

          <h1 className={styles.title}>{issue.title || issueId}</h1>

          <div className={styles.metaRow}>
            <IssueProjectPicker
              issueId={issueId}
              issue={issue}
              hideLabel
              pendingProjectId={pendingProjectId}
              onPendingChange={setPendingProjectId}
            />
            <IssueStatusPicker
              issueId={issueId}
              issue={issue}
              hideLabel
              pendingProjectId={pendingProjectId}
              onPendingResolved={() => setPendingProjectId(null)}
            />
            <IssueAssigneePicker issueId={issueId} issue={issue} hideLabel />
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
              <Markdown content={issue.description} />
            ) : (
              <p className={styles.descriptionEmpty}>No description.</p>
            )}
          </div>

          {issue.progress && (
            <div className={styles.section}>
              <span className={styles.sectionLabel}>Progress</span>
              <div className={styles.sectionBody}>
                <Markdown content={issue.progress} />
              </div>
            </div>
          )}

          {issue.feedback && (
            <div className={styles.section}>
              <span className={styles.sectionLabel}>Feedback</span>
              <div className={styles.sectionBody}>
                <Markdown content={issue.feedback} />
              </div>
            </div>
          )}

          <div className={styles.section}>
            <CommentsPanel issueId={issueId} />
          </div>

          {issue.form && (
            <div className={styles.section}>
              <span className={styles.sectionLabel}>Form</span>
              <FormPanel issueId={issueId} form={issue.form} formResponse={issue.form_response} />
            </div>
          )}

          <SessionList issueId={issueId} />
        </div>
      </div>

      {/* ── Right rail ── */}
      <IssueRightPanel
        record={record}
        onOpenStatusModal={() => setUpdateModalOpen(true)}
        activeTabKey={rightPanelTab}
        onTabChange={handleRightPanelChange}
        data-mobile-active={overviewActive ? "false" : "true"}
      />

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
