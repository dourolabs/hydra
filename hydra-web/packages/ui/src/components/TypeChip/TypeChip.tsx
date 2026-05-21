import styles from "./TypeChip.module.css";

export type IssueType =
  | "task"
  | "bug"
  | "feature"
  | "chore"
  | "merge-request"
  | "review-request";

const ISSUE_TYPE_DISPLAY: Partial<Record<string, string>> = {
  "review-request": "review",
  "merge-request": "merge",
};

export function issueTypeDisplayLabel(type: IssueType | string): string {
  return ISSUE_TYPE_DISPLAY[type] ?? type;
}

export interface TypeChipProps {
  type: IssueType | string;
  className?: string;
}

export function TypeChip({ type, className }: TypeChipProps) {
  const cls = [styles.typeChip, className].filter(Boolean).join(" ");
  return (
    <span className={cls} data-type={type}>
      {issueTypeDisplayLabel(type)}
    </span>
  );
}
