import type { StatusDefinition } from "@hydra/api";
import styles from "./StatusChip.module.css";

interface StatusChipProps {
  /** The server-resolved status; consumed verbatim — no client-side
   *  resolution. When omitted (e.g. an issue summary the server didn't
   *  populate), the chip falls back to a neutral key-only rendering via
   *  `fallbackKey`. */
  definition?: StatusDefinition | null;
  /** Status key to display when `definition` is missing. */
  fallbackKey?: string | null;
  className?: string;
  "data-testid"?: string;
}

/**
 * Dumb chip renderer for a `StatusDefinition`. Shares the visual pattern
 * of `LabelChip` (colored dot + label, pill-shaped border). The icon, if
 * declared on the status, is rendered next to the dot as a single
 * character placeholder — the theme's icon set is not yet wired up so
 * an `IconKey` is treated as a plain string per PR 5 scope.
 */
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
      </span>
    );
  }

  const key = fallbackKey ?? "unknown";
  const cls = [styles.chip, className].filter(Boolean).join(" ");
  const neutral = "var(--color-border-secondary, #6b7280)";
  return (
    <span className={cls} style={{ borderColor: neutral }} data-testid={testId}>
      <span className={styles.dot} style={{ backgroundColor: neutral }} />
      <span className={styles.label}>{key}</span>
    </span>
  );
}

function iconChar(icon: string): string {
  return icon.charAt(0).toUpperCase();
}
