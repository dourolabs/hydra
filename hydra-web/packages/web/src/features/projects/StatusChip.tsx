import type { CSSProperties } from "react";
import type { StatusDefinition } from "@hydra/api";
import styles from "./StatusChip.module.css";

interface StatusChipProps {
  /** The server-resolved status definition; consumed verbatim. */
  definition?: StatusDefinition | null;
  /** Status key to display when `definition` is missing (e.g. activity
   *  log entries that only carry the bare status key, or kanban column
   *  heads keyed by a hardcoded status string). */
  fallbackKey?: string | null;
  className?: string;
  "data-testid"?: string;
}

const NEUTRAL_DOT = "var(--s-neutral)";

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
  definition,
  fallbackKey,
  className,
  "data-testid": testId,
}: StatusChipProps) {
  const cls = [styles.chip, className].filter(Boolean).join(" ");

  if (definition) {
    const inProgress = isInProgressKey(definition.key);
    const dotCls = [styles.dot, inProgress && styles.dotInProgress].filter(Boolean).join(" ");
    return (
      <span className={cls} data-testid={testId}>
        <span className={dotCls} style={dotStyle(definition.color, inProgress)} />
        <span className={styles.label}>{definition.label}</span>
      </span>
    );
  }

  if (!fallbackKey) {
    return null;
  }

  const inProgress = isInProgressKey(fallbackKey);
  const dotCls = [styles.dot, inProgress && styles.dotInProgress].filter(Boolean).join(" ");
  return (
    <span className={cls} data-testid={testId}>
      <span className={dotCls} style={dotStyle(NEUTRAL_DOT, inProgress)} />
      <span className={styles.label}>{fallbackKey}</span>
    </span>
  );
}
