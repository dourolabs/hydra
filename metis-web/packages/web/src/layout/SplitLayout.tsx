import type { ReactNode } from "react";
import styles from "./SplitLayout.module.css";

interface SplitLayoutProps {
  left: ReactNode;
  right: ReactNode;
  /** Left pane width as percentage (default 40). */
  leftWidth?: number;
  /** When true on mobile, show the right (detail) pane instead of the left (list) pane. */
  mobileDetailVisible?: boolean;
  /** Callback to return to the list view on mobile. */
  onMobileBack?: () => void;
}

export function SplitLayout({
  left,
  right,
  leftWidth = 40,
  mobileDetailVisible = false,
  onMobileBack,
}: SplitLayoutProps) {
  const containerClass = `${styles.container}${mobileDetailVisible ? ` ${styles.mobileDetailVisible}` : ""}`;

  return (
    <div className={containerClass}>
      <div className={styles.left} style={{ flex: `0 0 ${leftWidth}%` }}>
        {left}
      </div>
      <div className={styles.divider} />
      <div className={styles.right}>
        {onMobileBack && (
          <button
            type="button"
            className={styles.backButton}
            onClick={onMobileBack}
          >
            <svg
              className={styles.backIcon}
              viewBox="0 0 20 20"
              fill="currentColor"
            >
              <path
                fillRule="evenodd"
                d="M9.707 16.707a1 1 0 01-1.414 0l-6-6a1 1 0 010-1.414l6-6a1 1 0 011.414 1.414L5.414 9H17a1 1 0 110 2H5.414l4.293 4.293a1 1 0 010 1.414z"
                clipRule="evenodd"
              />
            </svg>
            Back
          </button>
        )}
        {right}
      </div>
    </div>
  );
}
