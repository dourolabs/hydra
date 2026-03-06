import styles from "./Badge.module.css";

export type BadgeStatus =
  | "open"
  | "in-progress"
  | "closed"
  | "failed"
  | "dropped"
  | "blocked"
  | "rejected"
  | "merged"
  | "changes-requested"
  | "approved"
  | "created"
  | "pending"
  | "running"
  | "complete"
  | "success"
  | "unknown";

export interface BadgeProps {
  status: BadgeStatus;
  className?: string;
}

const statusLabels: Record<BadgeStatus, string> = {
  open: "Open",
  "in-progress": "In Progress",
  closed: "Closed",
  failed: "Failed",
  dropped: "Dropped",
  blocked: "Blocked",
  rejected: "Rejected",
  merged: "Merged",
  "changes-requested": "Changes Requested",
  approved: "Approved",
  created: "Created",
  pending: "Pending",
  running: "Running",
  complete: "Complete",
  success: "Success",
  unknown: "Unknown",
};

export function Badge({ status, className }: BadgeProps) {
  const cls = [styles.badge, styles[status.replace("-", "_")], className].filter(Boolean).join(" ");

  return <span className={cls}>{statusLabels[status]}</span>;
}
