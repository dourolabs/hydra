import type { CSSProperties } from "react";
import type { StatusDefinition } from "@hydra/api";
import { StatusIcon } from "./StatusIcon";
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

function pulseStyle(color: string): CSSProperties {
  return {
    boxShadow: `0 0 0 3px color-mix(in srgb, ${color} 18%, transparent)`,
    borderRadius: "50%",
  };
}

export function StatusChip({
  status,
  className,
  "data-testid": testId,
}: StatusChipProps) {
  const cls = [styles.chip, className].filter(Boolean).join(" ");
  const inProgress = isInProgressKey(status.key);
  const iconWrapCls = [styles.iconWrap, inProgress && styles.iconWrapInProgress]
    .filter(Boolean)
    .join(" ");
  return (
    <span className={cls} data-testid={testId}>
      <span
        className={iconWrapCls}
        style={inProgress ? pulseStyle(status.color) : undefined}
      >
        <StatusIcon statusKey={status.key} color={status.color} size={12} />
      </span>
      <span className={styles.label}>{status.label}</span>
    </span>
  );
}
