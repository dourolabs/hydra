import type { ReactNode } from "react";
import styles from "./SplitLayout.module.css";

interface SplitLayoutProps {
  left: ReactNode;
  right: ReactNode;
  /** Left pane width as percentage (default 40). */
  leftWidth?: number;
}

export function SplitLayout({ left, right, leftWidth = 40 }: SplitLayoutProps) {
  return (
    <div className={styles.container}>
      <div className={styles.left} style={{ flex: `0 0 ${leftWidth}%` }}>
        {left}
      </div>
      <div className={styles.divider} />
      <div className={styles.right}>
        {right}
      </div>
    </div>
  );
}
