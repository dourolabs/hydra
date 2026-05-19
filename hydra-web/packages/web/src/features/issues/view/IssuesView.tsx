import { useEffect, useState } from "react";
import type {
  IssueStatus,
  IssueType,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { Avatar, Badge, Icons, Kbd, Picker, PickerRow, TypeChip } from "@hydra/ui";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import { IssuesTable } from "./IssuesTable";
import { IssuesBoard } from "./IssuesBoard";
import styles from "./IssuesView.module.css";

const LAYOUT_STORAGE_KEY = "hydra:issues:layout";

export type IssuesLayout = "table" | "board";

type FilterPickerKey = "status" | "type" | "creator" | "assignee" | null;

// Issue statuses surfaced as filter options. The empty option ("any") renders
// as the Picker's default "Any" pill — we only iterate the real statuses
// below to render colored Badge chips.
const STATUS_FILTER_VALUES: IssueStatus[] = [
  "open",
  "in-progress",
  "failed",
  "closed",
  "dropped",
];

interface TypeOption {
  value: IssueType | "";
  label: string;
}

const TYPE_OPTIONS: TypeOption[] = [
  { value: "", label: "All types" },
  { value: "task", label: "Task" },
  { value: "bug", label: "Bug" },
  { value: "feature", label: "Feature" },
  { value: "chore", label: "Chore" },
  { value: "merge-request", label: "Merge request" },
  { value: "review-request", label: "Review request" },
];

function typeLabel(value: IssueType | null): string {
  if (!value) return "Any";
  return TYPE_OPTIONS.find((o) => o.value === value)?.label ?? value;
}

interface IssuesViewProps {
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  filterRootId: string | null;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  searchValue: string;
  onSearchChange: (value: string) => void;
  selectedStatus: IssueStatus | null;
  onStatusChange: (status: IssueStatus | null) => void;
  selectedType: IssueType | null;
  onTypeChange: (type: IssueType | null) => void;
  selectedCreator: string;
  onCreatorChange: (creator: string) => void;
  selectedAssignee: string;
  onAssigneeChange: (assignee: string) => void;
  // List of selectable names for the Creator and Assignee dropdowns. The page
  // is responsible for ensuring the current user appears here.
  userOptions: string[];
  eyebrow: string;
  title: string;
}

function readLayout(): IssuesLayout {
  if (typeof window === "undefined") return "table";
  try {
    const v = window.localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (v === "board" || v === "table") return v;
  } catch {
    /* ignore */
  }
  return "table";
}

function writeLayout(layout: IssuesLayout): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, layout);
  } catch {
    /* ignore */
  }
}

export function IssuesView({
  issues,
  childStatusMap,
  sessionsByIssue,
  isLoading,
  filterRootId,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  searchValue,
  onSearchChange,
  selectedStatus,
  onStatusChange,
  selectedType,
  onTypeChange,
  selectedCreator,
  onCreatorChange,
  selectedAssignee,
  onAssigneeChange,
  userOptions,
  eyebrow,
  title,
}: IssuesViewProps) {
  const [layout, setLayout] = useState<IssuesLayout>(readLayout);
  const [openPicker, setOpenPicker] = useState<FilterPickerKey>(null);

  useEffect(() => {
    writeLayout(layout);
  }, [layout]);

  const toggle = (key: Exclude<FilterPickerKey, null>) =>
    setOpenPicker((prev) => (prev === key ? null : key));

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>{eyebrow}</span>
          <h1 className={styles.pageTitle}>{title}</h1>
        </div>
        <span className={styles.headSpacer} />
        <div className={styles.headRight}>
          <div className={styles.segmented} role="tablist" aria-label="Layout">
            <button
              type="button"
              role="tab"
              aria-selected={layout === "table"}
              className={layout === "table" ? styles.segmentedActive : undefined}
              onClick={() => setLayout("table")}
              data-testid="issues-layout-table"
            >
              <Icons.IconMenu size={14} />
              Table
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={layout === "board"}
              className={layout === "board" ? styles.segmentedActive : undefined}
              onClick={() => setLayout("board")}
              data-testid="issues-layout-board"
            >
              <Icons.IconDot size={14} />
              Board
            </button>
          </div>
        </div>
      </div>

      <div className={styles.toolbar}>
        <div data-testid="issues-filter-status">
          <Picker
            label="Status"
            open={openPicker === "status"}
            onToggle={() => toggle("status")}
            value={
              selectedStatus ? (
                <Badge status={normalizeIssueStatus(selectedStatus)} />
              ) : (
                <span className={styles.pillValue}>Any</span>
              )
            }
          >
            <PickerRow
              active={!selectedStatus}
              onClick={() => {
                onStatusChange(null);
                setOpenPicker(null);
              }}
            >
              <span>Any status</span>
            </PickerRow>
            {STATUS_FILTER_VALUES.map((value) => (
              <PickerRow
                key={value}
                active={selectedStatus === value}
                onClick={() => {
                  onStatusChange(value);
                  setOpenPicker(null);
                }}
              >
                <Badge status={normalizeIssueStatus(value)} />
              </PickerRow>
            ))}
          </Picker>
        </div>

        <div data-testid="issues-filter-type">
          <Picker
            label="Type"
            open={openPicker === "type"}
            onToggle={() => toggle("type")}
            value={
              selectedType ? (
                <TypeChip type={selectedType} />
              ) : (
                <span className={styles.pillValue}>{typeLabel(null)}</span>
              )
            }
          >
            {TYPE_OPTIONS.map((opt) => (
              <PickerRow
                key={opt.value || "any"}
                active={(selectedType ?? "") === opt.value}
                onClick={() => {
                  onTypeChange(opt.value === "" ? null : opt.value);
                  setOpenPicker(null);
                }}
              >
                {opt.value ? (
                  <TypeChip type={opt.value} />
                ) : (
                  <span>{opt.label}</span>
                )}
              </PickerRow>
            ))}
          </Picker>
        </div>

        <div data-testid="issues-filter-creator">
          <Picker
            label="Creator"
            wide
            open={openPicker === "creator"}
            onToggle={() => toggle("creator")}
            value={
              selectedCreator ? (
                <span className={styles.pillContent}>
                  <Avatar name={selectedCreator} kind="agent" size="md" />
                  <span>{selectedCreator}</span>
                </span>
              ) : (
                <span className={styles.pillValue}>Any</span>
              )
            }
          >
            <PickerRow
              active={!selectedCreator}
              onClick={() => {
                onCreatorChange("");
                setOpenPicker(null);
              }}
            >
              <span>Any creator</span>
            </PickerRow>
            {userOptions.map((name) => (
              <PickerRow
                key={name}
                active={selectedCreator === name}
                onClick={() => {
                  onCreatorChange(name);
                  setOpenPicker(null);
                }}
              >
                <Avatar name={name} kind="agent" size="md" />
                <span>{name}</span>
              </PickerRow>
            ))}
          </Picker>
        </div>

        <div data-testid="issues-filter-assignee">
          <Picker
            label="Assignee"
            wide
            open={openPicker === "assignee"}
            onToggle={() => toggle("assignee")}
            value={
              selectedAssignee ? (
                <span className={styles.pillContent}>
                  <Avatar name={selectedAssignee} kind="agent" size="md" />
                  <span>{selectedAssignee}</span>
                </span>
              ) : (
                <span className={styles.pillValue}>Any</span>
              )
            }
          >
            <PickerRow
              active={!selectedAssignee}
              onClick={() => {
                onAssigneeChange("");
                setOpenPicker(null);
              }}
            >
              <span>Any assignee</span>
            </PickerRow>
            {userOptions.map((name) => (
              <PickerRow
                key={name}
                active={selectedAssignee === name}
                onClick={() => {
                  onAssigneeChange(name);
                  setOpenPicker(null);
                }}
              >
                <Avatar name={name} kind="agent" size="md" />
                <span>{name}</span>
              </PickerRow>
            ))}
          </Picker>
        </div>

        <span className={styles.toolbarSpacer} />
        <div className={styles.searchBox}>
          <Icons.IconSearch className={styles.searchIcon} size={14} />
          <input
            type="text"
            placeholder="Search issues…"
            value={searchValue}
            onChange={(e) => onSearchChange(e.target.value)}
            aria-label="Search issues"
            data-testid="issues-search"
          />
          <Kbd>/</Kbd>
        </div>
      </div>

      <div className={styles.body}>
        {isLoading && issues.length === 0 && (
          <div className={styles.empty}>Loading issues…</div>
        )}

        {!isLoading && issues.length === 0 && (
          <div className={styles.empty}>No issues match the current filters.</div>
        )}

        {issues.length > 0 && layout === "table" && (
          <IssuesTable
            issues={issues}
            childStatusMap={childStatusMap}
            sessionsByIssue={sessionsByIssue}
            filterRootId={filterRootId}
          />
        )}

        {issues.length > 0 && layout === "board" && (
          <IssuesBoard
            issues={issues}
            childStatusMap={childStatusMap}
            filterRootId={filterRootId}
          />
        )}

        {hasNextPage && layout === "table" && (
          <div className={styles.loadMore}>
            <button
              type="button"
              className={styles.loadMoreButton}
              onClick={onLoadMore}
              disabled={isFetchingNextPage}
            >
              {isFetchingNextPage ? "Loading…" : "Load more"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
