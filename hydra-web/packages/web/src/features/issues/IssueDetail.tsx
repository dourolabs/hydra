import { useCallback, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Badge, Button, TypeChip } from "@hydra/ui";
import { Markdown } from "../../components/Markdown";
import type { IssueVersionRecord } from "@hydra/api";
import { normalizeIssueStatus } from "../../utils/statusMapping";
import { useIssue } from "./useIssue";
import { IssueRightPanel, type IssueRightPanelTabKey } from "./IssueRightPanel";
import { IssueUpdateModal } from "./IssueUpdateModal";
import { FeedbackModal } from "./FeedbackModal";
import { FormPanel } from "./FormPanel";
import { SessionList } from "../sessions/SessionList";
import { useSessionsByIssue } from "../sessions/useSessionsByIssue";
import { useSessionDuration } from "../dashboard/useSessionDuration";
import { MobileTabBar, type MobileTabBarItem } from "../../components/MobileTabBar";
import { AgoTime } from "../../components/Runtime/Runtime";
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
      {record && <Badge status={normalizeIssueStatus(record.issue.status)} />}
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

  const [mobileTab, setMobileTab] = useState<MobileTabKey>("overview");
  const [rightPanelTab, setRightPanelTab] = useState<IssueRightPanelTabKey>("related");
  const [updateModalOpen, setUpdateModalOpen] = useState(false);
  const [feedbackModalOpen, setFeedbackModalOpen] = useState(false);

  const { data: sessions } = useSessionsByIssue(issueId);
  const { durationText, isRunning } = useSessionDuration(sessions);

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

  const status = normalizeIssueStatus(issue.status);
  const settings = issue.session_settings;
  const overviewActive = mobileTab === "overview";

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
            <AgoTime iso={record.creation_time} />
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
