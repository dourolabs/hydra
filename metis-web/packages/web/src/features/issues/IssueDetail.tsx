import { useState, useCallback } from "react";
import { Avatar, Badge, Button, MarkdownViewer, Panel, Tabs, Textarea } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useUpdateIssue } from "./useIssue";
import { useToast } from "../toast/useToast";
import { IssueTodoList } from "./IssueTodoList";
import { IssueChildren } from "./IssueChildren";
import { IssueActivity } from "./IssueActivity";
import { JobList } from "../jobs/JobList";
import { PatchList } from "../patches/PatchList";
import styles from "./IssueDetail.module.css";

interface IssueDetailProps {
  record: IssueVersionRecord;
}

const TABS = [
  { id: "children", label: "Children" },
  { id: "jobs", label: "Jobs" },
  { id: "patches", label: "Patches" },
  { id: "todo", label: "Todo" },
  { id: "activity", label: "Activity" },
];

interface EditableSectionProps {
  title: string;
  content: string;
  emptyText: string;
  onSave: (value: string) => void;
  isSaving: boolean;
}

function EditableSection({
  title,
  content,
  emptyText,
  onSave,
  isSaving,
}: EditableSectionProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(content);

  const handleEdit = useCallback(() => {
    setDraft(content);
    setEditing(true);
  }, [content]);

  const handleCancel = useCallback(() => {
    setDraft(content);
    setEditing(false);
  }, [content]);

  const handleSave = useCallback(() => {
    onSave(draft);
    setEditing(false);
  }, [draft, onSave]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSave();
      }
    },
    [handleSave],
  );

  return (
    <Panel
      header={
        <div className={styles.panelHeader}>
          <span className={styles.sectionTitle}>{title}</span>
          {!editing && (
            <Button
              variant="ghost"
              size="sm"
              onClick={handleEdit}
              className={styles.editButton}
            >
              Edit
            </Button>
          )}
        </div>
      }
    >
      <div className={styles.sectionBody}>
        {editing ? (
          <div className={styles.editMode}>
            <Textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={handleKeyDown}
              rows={8}
              className={styles.editTextarea}
            />
            <div className={styles.editActions}>
              <Button
                variant="primary"
                size="sm"
                onClick={handleSave}
                disabled={isSaving}
              >
                {isSaving ? "Saving..." : "Save"}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={handleCancel}
                disabled={isSaving}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : content ? (
          <MarkdownViewer content={content} />
        ) : (
          <p className={styles.empty}>{emptyText}</p>
        )}
      </div>
    </Panel>
  );
}

export function IssueDetail({ record }: IssueDetailProps) {
  const [activeTab, setActiveTab] = useState("children");
  const { issue } = record;
  const { addToast } = useToast();
  const updateMutation = useUpdateIssue(record.issue_id);

  const handleSaveDescription = useCallback(
    (value: string) => {
      updateMutation.mutate(
        { ...issue, description: value },
        {
          onSuccess: () => addToast("Description updated", "success"),
          onError: (err) =>
            addToast(
              err instanceof Error
                ? err.message
                : "Failed to update description",
              "error",
            ),
        },
      );
    },
    [issue, updateMutation, addToast],
  );

  const handleSaveProgress = useCallback(
    (value: string) => {
      updateMutation.mutate(
        { ...issue, progress: value },
        {
          onSuccess: () => addToast("Progress updated", "success"),
          onError: (err) =>
            addToast(
              err instanceof Error
                ? err.message
                : "Failed to update progress",
              "error",
            ),
        },
      );
    },
    [issue, updateMutation, addToast],
  );

  return (
    <div className={styles.detail}>
      {/* Header: ID + Status */}
      <div className={styles.header}>
        <span className={styles.issueId}>{record.issue_id}</span>
        <Badge status={issueToBadgeStatus(issue.status)} />
        <span className={styles.type}>{issue.type}</span>
      </div>

      {/* Metadata */}
      <div className={styles.meta}>
        {issue.creator && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Creator</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.creator} size="sm" />
              {issue.creator}
            </span>
          </div>
        )}
        {issue.assignee && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Assignee</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.assignee} size="sm" />
              {issue.assignee}
            </span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>
      </div>

      {/* Description */}
      <EditableSection
        title="Description"
        content={issue.description}
        emptyText="No description."
        onSave={handleSaveDescription}
        isSaving={updateMutation.isPending}
      />

      {/* Progress */}
      <EditableSection
        title="Progress"
        content={issue.progress}
        emptyText="No progress notes."
        onSave={handleSaveProgress}
        isSaving={updateMutation.isPending}
      />

      {/* Tabbed sections: Children, Tasks, Patches, Todo */}
      <Panel
        header={
          <Tabs
            tabs={TABS}
            activeTab={activeTab}
            onTabChange={setActiveTab}
          />
        }
      >
        <div className={styles.sectionBody}>
          {activeTab === "children" && (
            <IssueChildren issueId={record.issue_id} />
          )}
          {activeTab === "jobs" && <JobList issueId={record.issue_id} />}
          {activeTab === "patches" && (
            <PatchList
              patchIds={issue.patches ?? []}
              issueId={record.issue_id}
            />
          )}
          {activeTab === "todo" && (
            <IssueTodoList items={issue.todo_list ?? []} />
          )}
          {activeTab === "activity" && (
            <IssueActivity issueId={record.issue_id} />
          )}
        </div>
      </Panel>
    </div>
  );
}
