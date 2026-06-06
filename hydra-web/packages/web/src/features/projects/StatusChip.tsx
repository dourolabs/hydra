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

export function StatusChip({
  definition,
  fallbackKey,
  className,
  "data-testid": testId,
}: StatusChipProps) {
  if (definition) {
    const color = definition.color;
    const cls = [styles.chip, className].filter(Boolean).join(" ");
    return (
      <span className={cls} style={{ borderColor: color }} data-testid={testId}>
        <span className={styles.dot} style={{ backgroundColor: color }} />
        {definition.icon && (
          <span className={styles.icon} aria-hidden="true">
            {iconChar(definition.icon)}
          </span>
        )}
        <span className={styles.label}>{definition.label}</span>
        {definition.interactive && (
          <span
            className={styles.interactiveTag}
            data-testid="status-interactive-tag"
            title="Ready issues in this status spawn an interactive conversation"
          >
            interactive
          </span>
        )}
      </span>
    );
  }

  if (!fallbackKey) {
    return null;
  }

  const cls = [styles.chip, className].filter(Boolean).join(" ");
  const neutral = "var(--color-border-secondary, #6b7280)";
  return (
    <span className={cls} style={{ borderColor: neutral }} data-testid={testId}>
      <span className={styles.dot} style={{ backgroundColor: neutral }} />
      <span className={styles.label}>{fallbackKey}</span>
    </span>
  );
}

function iconChar(icon: string): string {
  return icon.charAt(0).toUpperCase();
}
