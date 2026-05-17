import { useState } from "react";
import type { IssueVersionRecord } from "@hydra/api";
import { IssueActivity } from "./IssueActivity";
import { IssueDetailsTab } from "./IssueDetailsTab";
import { IssueRelatedTab } from "./IssueRelatedTab";
import styles from "./IssueRightPanel.module.css";

export type IssueRightPanelTabKey = "related" | "activity" | "details";

const TABS: { key: IssueRightPanelTabKey; label: string }[] = [
  { key: "related", label: "Related" },
  { key: "activity", label: "Activity" },
  { key: "details", label: "Details" },
];

interface IssueRightPanelProps {
  record: IssueVersionRecord;
  onOpenStatusModal: () => void;
  activeTabKey?: IssueRightPanelTabKey;
  onTabChange?: (key: IssueRightPanelTabKey) => void;
  "data-mobile-active"?: "true" | "false";
}

export function IssueRightPanel({
  record,
  onOpenStatusModal,
  activeTabKey,
  onTabChange,
  "data-mobile-active": dataMobileActive,
}: IssueRightPanelProps) {
  const [internalTab, setInternalTab] = useState<IssueRightPanelTabKey>("related");
  const isControlled = activeTabKey !== undefined;
  const activeTab = isControlled ? activeTabKey : internalTab;
  const issueId = record.issue_id;

  const handleTabClick = (key: IssueRightPanelTabKey) => {
    if (!isControlled) setInternalTab(key);
    onTabChange?.(key);
  };

  return (
    <aside className={styles.wrapper} data-mobile-active={dataMobileActive}>
      <div className={styles.tabs} role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            className={`${styles.tab}${activeTab === t.key ? ` ${styles.tabActive}` : ""}`}
            aria-selected={activeTab === t.key}
            onClick={() => handleTabClick(t.key)}
            data-testid={`issue-rail-tab-${t.key}`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className={styles.body}>
        {activeTab === "related" && <IssueRelatedTab issueId={issueId} />}
        {activeTab === "activity" && <IssueActivity issueId={issueId} />}
        {activeTab === "details" && (
          <IssueDetailsTab record={record} onOpenStatusModal={onOpenStatusModal} />
        )}
      </div>
    </aside>
  );
}
