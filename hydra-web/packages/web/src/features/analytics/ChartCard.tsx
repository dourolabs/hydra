import type { ReactNode } from "react";
import { Spinner } from "@hydra/ui";
import styles from "./ChartCard.module.css";

export interface ChartCardProps {
  title: string;
  testId?: string;
  isLoading?: boolean;
  error?: unknown;
  disabled?: boolean;
  disabledHint?: string;
  children?: ReactNode;
}

/**
 * Wraps a single analytics chart with title + loading / error / empty
 * states. The chart implementations (PR 4 / PR 5) render `children`
 * once their data hook resolves; this PR ships only the placeholder
 * shell, so children render the "coming soon" copy directly.
 */
export function ChartCard({
  title,
  testId,
  isLoading,
  error,
  disabled,
  disabledHint,
  children,
}: ChartCardProps) {
  return (
    <section className={styles.card} data-testid={testId}>
      <header className={styles.head}>{title}</header>
      <div className={styles.body}>
        {disabled && (
          <div className={styles.disabled} data-testid="chart-card-disabled">
            {disabledHint ?? "Filter required"}
          </div>
        )}
        {!disabled && isLoading && (
          <div className={styles.loading} data-testid="chart-card-loading">
            <Spinner />
          </div>
        )}
        {!disabled && !isLoading && Boolean(error) && (
          <div className={styles.error} data-testid="chart-card-error">
            {extractErrorMessage(error)}
          </div>
        )}
        {!disabled && !isLoading && !error && children}
      </div>
    </section>
  );
}

function extractErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Failed to load chart data";
}
