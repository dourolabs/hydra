import type { CSSProperties } from "react";
import type { StatusDefinition } from "@hydra/api";
import styles from "./StatusChip.module.css";

interface StatusChipProps {
  /** The server-resolved status definition; consumed verbatim. */
  status: StatusDefinition;
  className?: string;
  "data-testid"?: string;
}

function isInProgressKey(key: string): boolean {
  return key === "in-progress";
}

function dotStyle(color: string, inProgress: boolean): CSSProperties {
  if (!inProgress) return { background: color };
  return {
    background: color,
    boxShadow: `0 0 0 3px color-mix(in srgb, ${color} 18%, transparent)`,
  };
}

export function StatusChip({
  status,
  className,
  "data-testid": testId,
}: StatusChipProps) {
  const cls = [styles.chip, className].filter(Boolean).join(" ");
  const inProgress = isInProgressKey(status.key);
  const dotCls = [styles.dot, inProgress && styles.dotInProgress].filter(Boolean).join(" ");
  return (
    <span className={cls} data-testid={testId}>
      <span className={dotCls} style={dotStyle(status.color, inProgress)} />
      <span className={styles.label}>{status.label}</span>
    </span>
  );
}
