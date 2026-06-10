import type { ReactNode } from "react";
import { Panel, Spinner } from "@hydra/ui";
import styles from "./ChartCard.module.css";

export interface ChartCardProps {
  title: string;
  testId?: string;
  /** Falls back to `title` when omitted. */
  ariaLabel?: string;
  isLoading?: boolean;
  error?: unknown;
  disabled?: boolean;
  disabledHint?: string;
  children?: ReactNode;
}

/**
 * Wraps a single analytics chart with title + loading / error / empty
 * states. Children render the chart body once the data hook resolves.
 * Marked as a labeled landmark region so the page is keyboard-navigable
 * card-by-card.
 */
export function ChartCard({
  title,
  testId,
  ariaLabel,
  isLoading,
  error,
  disabled,
  disabledHint,
  children,
}: ChartCardProps) {
  return (
    <section
      className={styles.card}
      data-testid={testId}
      role="region"
      aria-label={ariaLabel ?? title}
    >
      <Panel header={title}>
        {disabled && (
          <div
            className={`${styles.placeholder} ${styles.disabled}`}
            data-testid="chart-card-disabled"
          >
            {disabledHint ?? "Filter required"}
          </div>
        )}
        {!disabled && isLoading && (
          <div className={styles.placeholder} data-testid="chart-card-loading">
            <Spinner />
          </div>
        )}
        {!disabled && !isLoading && Boolean(error) && (
          <div className={`${styles.placeholder} ${styles.error}`} data-testid="chart-card-error">
            {extractErrorMessage(error)}
          </div>
        )}
        {!disabled && !isLoading && !error && children}
      </Panel>
    </section>
  );
}

function extractErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Failed to load chart data";
}
