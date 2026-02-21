import { type ReactNode } from "react";
import styles from "./Panel.module.css";

export interface PanelProps {
  header?: ReactNode;
  children: ReactNode;
  scrollable?: boolean;
  interactive?: boolean;
  className?: string;
}

export function Panel({ header, children, scrollable = false, interactive = false, className }: PanelProps) {
  const cls = [styles.panel, interactive && styles.interactive, className].filter(Boolean).join(" ");

  return (
    <div className={cls}>
      {header && <div className={styles.header}>{header}</div>}
      <div className={[styles.body, scrollable && styles.scrollable].filter(Boolean).join(" ")}>
        {children}
      </div>
    </div>
  );
}
