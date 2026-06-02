import type { ReactNode } from "react";
import styles from "./PreviewCard.module.css";

export type PreviewCardTone =
  | "open"
  | "in-progress"
  | "closed"
  | "failed"
  | "dropped"
  | "blocked"
  | "rejected"
  | "neutral";

export interface PreviewCardProps {
  /** Status-keyed tone driving the colored left edge. */
  tone: PreviewCardTone;
  /** Slot for badge + mono id + type chip + timestamp etc. */
  topRow: ReactNode;
  /** Card title (entity name / synthesized label). */
  title: ReactNode;
  /** Optional 2-line excerpt below the title. */
  bodyExcerpt?: ReactNode;
  /** Optional footer slot for assignee, repo, etc. */
  footer?: ReactNode;
  /** Click handler. The whole card is a `<button>`. */
  onClick?: () => void;
  /** Accessible label describing the card target. */
  ariaLabel: string;
  className?: string;
}

/**
 * Visual chrome for a reference preview card stacked at the end of a chat
 * message. Carries no knowledge of issues/patches/etc. — the per-kind
 * callers wire up data hooks and slot the formatted pieces in.
 */
export function PreviewCard({
  tone,
  topRow,
  title,
  bodyExcerpt,
  footer,
  onClick,
  ariaLabel,
  className,
}: PreviewCardProps) {
  const cls = [styles.card, className].filter(Boolean).join(" ");
  return (
    <button
      type="button"
      className={cls}
      data-tone={tone}
      onClick={onClick}
      aria-label={ariaLabel}
    >
      <span className={styles.edge} aria-hidden="true" />
      <span className={styles.content}>
        <span className={styles.topRow}>{topRow}</span>
        <span className={styles.title}>{title}</span>
        {bodyExcerpt && <span className={styles.bodyExcerpt}>{bodyExcerpt}</span>}
        {footer && <span className={styles.footer}>{footer}</span>}
      </span>
    </button>
  );
}
