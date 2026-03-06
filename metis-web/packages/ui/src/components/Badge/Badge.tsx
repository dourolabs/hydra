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
  open: "open",
  "in-progress": "in-progress",
  closed: "closed",
  failed: "failed",
  dropped: "dropped",
  blocked: "blocked",
  rejected: "rejected",
  merged: "merged",
  "changes-requested": "changes-requested",
  approved: "approved",
  created: "created",
  pending: "pending",
  running: "running",
  complete: "complete",
  success: "success",
  unknown: "unknown",
};

export function Badge({ status, className }: BadgeProps) {
  const cls = [styles.badge, styles[status.replace("-", "_")], className].filter(Boolean).join(" ");

  return <span className={cls}>{statusLabels[status]}</span>;
}
