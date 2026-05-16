import styles from "./TypeChip.module.css";

export type IssueType =
  | "task"
  | "bug"
  | "feature"
  | "chore"
  | "merge-request"
  | "review-request";

export interface TypeChipProps {
  type: IssueType | string;
  className?: string;
}

export function TypeChip({ type, className }: TypeChipProps) {
  const cls = [styles.typeChip, className].filter(Boolean).join(" ");
  return (
    <span className={cls} data-type={type}>
      {type}
    </span>
  );
}
