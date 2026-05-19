import styles from "./Badge.module.css";

export type BadgeStatus =
  | "open"
  | "in-progress"
  | "closed"
  | "issue-closed"
  | "failed"
  | "dropped"
  | "blocked"
  | "merged"
  | "changes-requested"
  | "rejected"
  | "approved"
  | "created"
  | "pending"
  | "running"
  | "complete"
  | "success"
  | "conv-active"
  | "conv-idle"
  | "conv-closed"
  | "unknown";

export interface BadgeProps {
  status: BadgeStatus;
  className?: string;
}

const statusLabels: Record<BadgeStatus, string> = {
  open: "Open",
  "in-progress": "In progress",
  closed: "Closed",
  "issue-closed": "Closed",
  failed: "Failed",
  dropped: "Dropped",
  blocked: "Blocked",
  merged: "Merged",
  "changes-requested": "Changes requested",
  rejected: "Rejected",
  approved: "Approved",
  created: "Created",
  pending: "Pending",
  running: "Running",
  complete: "Complete",
  success: "Success",
  "conv-active": "Active",
  "conv-idle": "Idle",
  "conv-closed": "Closed",
  unknown: "Unknown",
};

// Maps a Badge status to the underlying tone (a small fixed palette).
const statusTones: Record<BadgeStatus, string> = {
  open: "open",
  "in-progress": "in_progress",
  closed: "failed", // legacy: "closed" badge for patches/sessions = red/failure
  "issue-closed": "closed",
  failed: "failed",
  dropped: "dropped",
  blocked: "blocked",
  merged: "closed",
  "changes-requested": "rejected",
  rejected: "rejected",
  approved: "closed",
  created: "open",
  pending: "open",
  running: "in_progress",
  complete: "closed",
  success: "closed",
  "conv-active": "conv_active",
  "conv-idle": "conv_idle",
  "conv-closed": "conv_closed",
  unknown: "neutral",
};

export function Badge({ status, className }: BadgeProps) {
  const tone = statusTones[status];
  const cls = [styles.badge, className].filter(Boolean).join(" ");

  return (
    <span className={cls} data-tone={tone}>
      <span className={styles.dot} />
      <span className={styles.label}>{statusLabels[status]}</span>
    </span>
  );
}
