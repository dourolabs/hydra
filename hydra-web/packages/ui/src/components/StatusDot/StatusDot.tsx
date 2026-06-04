import type { BadgeStatus } from "../Badge/Badge";
import styles from "./StatusDot.module.css";

const STATUS_DOT_TONE: Partial<Record<BadgeStatus, string>> = {
  open: styles.toneOpen,
  "in-progress": styles.toneInProgress,
  closed: styles.toneClosed,
  "issue-closed": styles.toneClosed,
  approved: styles.toneClosed,
  failed: styles.toneFailed,
  dropped: styles.toneDropped,
  blocked: styles.toneBlocked,
  pending: styles.toneInProgress,
  running: styles.toneInProgress,
  complete: styles.toneClosed,
  "changes-requested": styles.toneRejected,
  merged: styles.toneClosed,
  "conv-active": styles.toneInProgress,
  "conv-idle": styles.toneOpen,
  "conv-closed": styles.toneClosed,
};

export interface StatusDotProps {
  status: BadgeStatus;
  className?: string;
}

export function StatusDot({ status, className }: StatusDotProps) {
  const tone = STATUS_DOT_TONE[status] ?? styles.toneNeutral;
  const cls = [styles.dot, tone, className].filter(Boolean).join(" ");
  return <span className={cls} aria-hidden="true" />;
}
