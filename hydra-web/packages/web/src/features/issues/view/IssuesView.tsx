import { useState } from "react";
import type {
  IssueStatus,
  IssueType,
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { Avatar, Badge, Icons, Kbd, Picker, PickerRow, TypeChip } from "@hydra/ui";
import type { ChildStatus } from "../../dashboard/computeIssueProgress";
import { normalizeIssueStatus } from "../../../utils/statusMapping";
import type { IssueFilters } from "../usePaginatedIssues";
import { IssuesTable } from "./IssuesTable";
import { IssuesBoard } from "./IssuesBoard";
import styles from "./IssuesView.module.css";

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
  { value: "merge-request", label: "Merge" },
  { value: "review-request", label: "Review" },
];

function typeLabel(value: IssueType | null): string {
  if (!value) return "Any";
  return TYPE_OPTIONS.find((o) => o.value === value)?.label ?? value;
}

interface IssuesViewProps {
  layout: IssuesLayout;
  onLayoutChange: (layout: IssuesLayout) => void;
  // Table-only data (board owns its own fetches via baseFilters/username).
  issues: IssueSummaryRecord[];
  childStatusMap: Map<string, ChildStatus[]>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onLoadMore: () => void;
  // Shared
  baseFilters: IssueFilters;
  username: string;
  filterRootId: string | null;
  searchValue: string;
  onSearchChange: (value: string) => void;
  selectedStatus: IssueStatus | null;
  onStatusChange: (status: IssueStatus | null) => void;
  selectedType: IssueType | null;
  onTypeChange: (type: IssueType | null) => void;
  selectedCreator: string;
  onCreatorChange: (creator: string) => void;
  // Phase 4b: `selectedAssignee` and the `onAssigneeChange` argument are
  // Principal path strings (`users/<name>` / `agents/<name>` / `external/<sys>/<name>`)
  // — the canonical wire form. The picker maps them to display names via
  // `userOptions[].assigneePath`.
  selectedAssignee: string;
  onAssigneeChange: (assigneePath: string) => void;
  // Selectable rows for the Creator and Assignee dropdowns. Each row has a
  // display `name` (used for Creator, which stays bare) and an `assigneePath`
  // (used for Assignee). The page is responsible for ensuring the current
  // user appears in `userOptions`.
  //
  // Creator filter renders users only. Assignee filter renders both sections:
  // Agents first, then Users, each sorted by name.
  agentOptions: { name: string; assigneePath: string }[];
  userOptions: { name: string; assigneePath: string }[];
  eyebrow: string;
  title: string;
}

export function IssuesView({
  layout,
  onLayoutChange,
  issues,
  childStatusMap,
  sessionsByIssue,
  isLoading,
  hasNextPage,
  isFetchingNextPage,
  onLoadMore,
  baseFilters,
  username,
  filterRootId,
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
  agentOptions,
  userOptions,
  eyebrow,
  title,
}: IssuesViewProps) {
  const [openPicker, setOpenPicker] = useState<FilterPickerKey>(null);

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
              onClick={() => onLayoutChange("table")}
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
              onClick={() => onLayoutChange("board")}
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
                  <Avatar name={selectedCreator} kind="human" size="md" />
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
            {userOptions.map((opt) => (
              <PickerRow
                key={opt.name}
                active={selectedCreator === opt.name}
                onClick={() => {
                  onCreatorChange(opt.name);
                  setOpenPicker(null);
                }}
              >
                <Avatar name={opt.name} kind="human" size="md" />
                <span>{opt.name}</span>
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
                (() => {
                  const agentMatch = agentOptions.find(
                    (o) => o.assigneePath === selectedAssignee,
                  );
                  const userMatch = !agentMatch
                    ? userOptions.find((o) => o.assigneePath === selectedAssignee)
                    : undefined;
                  const kind: "agent" | "human" = agentMatch ? "agent" : "human";
                  const label =
                    agentMatch?.name ?? userMatch?.name ?? selectedAssignee;
                  return (
                    <span className={styles.pillContent}>
                      <Avatar name={label} kind={kind} size="md" />
                      <span>{label}</span>
                    </span>
                  );
                })()
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
            {agentOptions.length > 0 && (
              <>
                <div className={styles.popSection}>Agents</div>
                {agentOptions.map((opt) => (
                  <PickerRow
                    key={opt.assigneePath}
                    active={selectedAssignee === opt.assigneePath}
                    onClick={() => {
                      onAssigneeChange(opt.assigneePath);
                      setOpenPicker(null);
                    }}
                  >
                    <Avatar name={opt.name} kind="agent" size="md" />
                    <span>{opt.name}</span>
                  </PickerRow>
                ))}
              </>
            )}
            {userOptions.length > 0 && (
              <>
                <div className={styles.popSection}>Users</div>
                {userOptions.map((opt) => (
                  <PickerRow
                    key={opt.assigneePath}
                    active={selectedAssignee === opt.assigneePath}
                    onClick={() => {
                      onAssigneeChange(opt.assigneePath);
                      setOpenPicker(null);
                    }}
                  >
                    <Avatar name={opt.name} kind="human" size="md" />
                    <span>{opt.name}</span>
                  </PickerRow>
                ))}
              </>
            )}
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
        {layout === "table" && (
          <>
            {isLoading && issues.length === 0 && (
              <div className={styles.empty}>Loading issues…</div>
            )}

            {!isLoading && issues.length === 0 && (
              <div className={styles.empty}>No issues match the current filters.</div>
            )}

            {issues.length > 0 && (
              <IssuesTable
                issues={issues}
                childStatusMap={childStatusMap}
                sessionsByIssue={sessionsByIssue}
                filterRootId={filterRootId}
              />
            )}

            {hasNextPage && (
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
          </>
        )}

        {layout === "board" && (
          <IssuesBoard
            baseFilters={baseFilters}
            username={username}
            filterRootId={filterRootId}
          />
        )}
      </div>
    </div>
  );
}
