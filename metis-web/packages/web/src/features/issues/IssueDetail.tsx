import { useState, useRef, useEffect, useCallback } from "react";
import { Avatar, Badge, Input, MarkdownViewer, Panel, Select, Tabs } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { Issue, IssueVersionRecord, IssueStatus, IssueType } from "@metis/api";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { formatTimestamp } from "../../utils/time";
import { useUpdateIssue } from "./useIssue";
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

const STATUS_OPTIONS: SelectOption[] = [
  { value: "open", label: "open" },
  { value: "in-progress", label: "in-progress" },
  { value: "closed", label: "closed" },
  { value: "failed", label: "failed" },
  { value: "dropped", label: "dropped" },
];

const TYPE_OPTIONS: SelectOption[] = [
  { value: "task", label: "task" },
  { value: "bug", label: "bug" },
  { value: "feature", label: "feature" },
  { value: "chore", label: "chore" },
];

function EditableStatus({
  issue,
  issueId,
}: {
  issue: Issue;
  issueId: string;
}) {
  const [editing, setEditing] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const mutation = useUpdateIssue(issueId);

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const newStatus = e.target.value as IssueStatus;
    if (newStatus !== issue.status) {
      mutation.mutate({ ...issue, status: newStatus });
    }
    setEditing(false);
  };

  useEffect(() => {
    if (!editing) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        wrapperRef.current &&
        !wrapperRef.current.contains(e.target as Node)
      ) {
        setEditing(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [editing]);

  if (editing) {
    return (
      <div ref={wrapperRef} className={styles.editableDropdown}>
        <Select
          options={STATUS_OPTIONS}
          value={issue.status}
          onChange={handleChange}
          autoFocus
        />
      </div>
    );
  }

  return (
    <button
      type="button"
      className={styles.editableField}
      onClick={() => setEditing(true)}
      title="Click to edit status"
    >
      <Badge status={issueToBadgeStatus(issue.status)} />
      <span className={styles.editIcon}>&#9998;</span>
    </button>
  );
}

function EditableType({
  issue,
  issueId,
}: {
  issue: Issue;
  issueId: string;
}) {
  const [editing, setEditing] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const mutation = useUpdateIssue(issueId);

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const newType = e.target.value as IssueType;
    if (newType !== issue.type) {
      mutation.mutate({ ...issue, type: newType });
    }
    setEditing(false);
  };

  useEffect(() => {
    if (!editing) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        wrapperRef.current &&
        !wrapperRef.current.contains(e.target as Node)
      ) {
        setEditing(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [editing]);

  if (editing) {
    return (
      <div ref={wrapperRef} className={styles.editableDropdown}>
        <Select
          options={TYPE_OPTIONS}
          value={issue.type}
          onChange={handleChange}
          autoFocus
        />
      </div>
    );
  }

  return (
    <button
      type="button"
      className={styles.editableField}
      onClick={() => setEditing(true)}
      title="Click to edit type"
    >
      <span className={styles.type}>{issue.type}</span>
      <span className={styles.editIcon}>&#9998;</span>
    </button>
  );
}

function EditableAssignee({
  issue,
  issueId,
}: {
  issue: Issue;
  issueId: string;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(issue.assignee ?? "");
  const wrapperRef = useRef<HTMLDivElement>(null);
  const mutation = useUpdateIssue(issueId);

  useEffect(() => {
    setValue(issue.assignee ?? "");
  }, [issue.assignee]);

  const save = useCallback(() => {
    const trimmed = value.trim();
    const newAssignee = trimmed || null;
    if (newAssignee !== (issue.assignee ?? null)) {
      mutation.mutate({ ...issue, assignee: newAssignee });
    }
    setEditing(false);
  }, [value, issue, mutation]);

  const cancel = useCallback(() => {
    setValue(issue.assignee ?? "");
    setEditing(false);
  }, [issue.assignee]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      save();
    } else if (e.key === "Escape") {
      cancel();
    }
  };

  useEffect(() => {
    if (!editing) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        wrapperRef.current &&
        !wrapperRef.current.contains(e.target as Node)
      ) {
        cancel();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [editing, cancel]);

  if (editing) {
    return (
      <div ref={wrapperRef} className={styles.editableInput}>
        <Input
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Enter assignee..."
          autoFocus
        />
        <div className={styles.editableActions}>
          <button
            type="button"
            className={styles.saveBtn}
            onClick={save}
          >
            Save
          </button>
          <button
            type="button"
            className={styles.cancelBtn}
            onClick={cancel}
          >
            Cancel
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.metaItem}>
      <span className={styles.metaLabel}>Assignee</span>
      <button
        type="button"
        className={styles.editableField}
        onClick={() => setEditing(true)}
        title="Click to edit assignee"
      >
        <span className={styles.metaValue}>
          {issue.assignee ? (
            <>
              <Avatar name={issue.assignee} size="sm" />
              {issue.assignee}
            </>
          ) : (
            <span className={styles.unset}>Unassigned</span>
          )}
        </span>
        <span className={styles.editIcon}>&#9998;</span>
      </button>
    </div>
  );
}

export function IssueDetail({ record }: IssueDetailProps) {
  const [activeTab, setActiveTab] = useState("children");
  const { issue } = record;

  return (
    <div className={styles.detail}>
      {/* Header: ID + Status + Type */}
      <div className={styles.header}>
        <span className={styles.issueId}>{record.issue_id}</span>
        <EditableStatus issue={issue} issueId={record.issue_id} />
        <EditableType issue={issue} issueId={record.issue_id} />
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
        <EditableAssignee issue={issue} issueId={record.issue_id} />
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>
      </div>

      {/* Description */}
      <Panel header={<span className={styles.sectionTitle}>Description</span>}>
        <div className={styles.sectionBody}>
          {issue.description ? (
            <MarkdownViewer content={issue.description} />
          ) : (
            <p className={styles.empty}>No description.</p>
          )}
        </div>
      </Panel>

      {/* Progress */}
      {issue.progress && (
        <Panel header={<span className={styles.sectionTitle}>Progress</span>}>
          <div className={styles.sectionBody}>
            <MarkdownViewer content={issue.progress} />
          </div>
        </Panel>
      )}

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
