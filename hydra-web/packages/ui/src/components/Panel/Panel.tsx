import { type ReactNode } from "react";
import styles from "./Panel.module.css";

export interface PanelProps {
  header?: ReactNode;
  children: ReactNode;
  scrollable?: boolean;
  fillHeight?: boolean;
  interactive?: boolean;
  className?: string;
}

export function Panel({
  header,
  children,
  scrollable = false,
  fillHeight = false,
  interactive = false,
  className,
}: PanelProps) {
  const cls = [
    styles.panel,
    interactive && styles.interactive,
    fillHeight && styles.fillHeight,
    className,
  ]
    .filter(Boolean)
    .join(" ");

  const bodyCls = [
    styles.body,
    scrollable && styles.scrollable,
    fillHeight && styles.bodyFill,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cls}>
      {header && <div className={styles.header}>{header}</div>}
      <div className={bodyCls}>{children}</div>
    </div>
  );
}
