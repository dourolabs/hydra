import { Input, Select } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { IssueFilterValues as IssueFilterState, SortOption } from "./useIssueFilters";
import styles from "./IssueFilters.module.css";

interface IssueFiltersProps {
  filters: IssueFilterState;
  assignees: string[];
  onFilterChange: (updates: Partial<IssueFilterState>) => void;
}

const STATUS_OPTIONS: SelectOption[] = [
  { value: "", label: "All statuses" },
  { value: "open", label: "Open" },
  { value: "in-progress", label: "In-Progress" },
  { value: "closed", label: "Closed" },
  { value: "failed", label: "Failed" },
  { value: "dropped", label: "Dropped" },
];

const TYPE_OPTIONS: SelectOption[] = [
  { value: "", label: "All types" },
  { value: "bug", label: "Bug" },
  { value: "feature", label: "Feature" },
  { value: "task", label: "Task" },
  { value: "chore", label: "Chore" },
];

const SORT_OPTIONS: SelectOption[] = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "updated", label: "Recently updated" },
  { value: "status", label: "By status" },
];

function buildAssigneeOptions(assignees: string[]): SelectOption[] {
  return [
    { value: "", label: "All assignees" },
    ...assignees.map((a) => ({ value: a, label: a })),
  ];
}

export function IssueFilters({ filters, assignees, onFilterChange }: IssueFiltersProps) {
  return (
    <div className={styles.filters}>
      <div className={styles.searchWrapper}>
        <Input
          placeholder="Search issues…"
          value={filters.q}
          onChange={(e) => onFilterChange({ q: e.target.value })}
          aria-label="Search issues"
        />
        {filters.q && (
          <button
            type="button"
            className={styles.clearButton}
            onClick={() => onFilterChange({ q: "" })}
            aria-label="Clear search"
          >
            ✕
          </button>
        )}
      </div>
      <Select
        label="Status"
        options={STATUS_OPTIONS}
        value={filters.statuses.length === 1 ? filters.statuses[0] : ""}
        onChange={(e) => {
          const val = e.target.value;
          onFilterChange({ statuses: val ? [val] : [] });
        }}
      />
      <Select
        label="Assignee"
        options={buildAssigneeOptions(assignees)}
        value={filters.assignee}
        onChange={(e) => onFilterChange({ assignee: e.target.value })}
      />
      <Select
        label="Type"
        options={TYPE_OPTIONS}
        value={filters.type}
        onChange={(e) => onFilterChange({ type: e.target.value })}
      />
      <Select
        label="Sort"
        options={SORT_OPTIONS}
        value={filters.sort}
        onChange={(e) => onFilterChange({ sort: e.target.value as SortOption })}
      />
    </div>
  );
}
