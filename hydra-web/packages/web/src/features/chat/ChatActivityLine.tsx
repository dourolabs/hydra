import { useCallback, useEffect, useState } from "react";
import type { ActivityCategory, ActivityRun, ActivityStep } from "./deriveActivitySteps";
import styles from "./ChatActivityLine.module.css";

interface ChatActivityLineProps {
  run: ActivityRun;
  /**
   * Inject a clock so tests can pin the timer. Defaults to wall-clock time
   * read on each interval tick / mount.
   */
  now?: () => number;
}

const TIMER_TICK_MS = 250;

/** Format total run elapsed as `M:SS`. */
function formatTimer(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format a per-step duration as `X.Xs`. Sub-100ms shows `0.1s` (floor). */
function formatStepDur(ms: number): string {
  const v = Math.max(0, ms) / 1000;
  return `${v.toFixed(1)}s`;
}

function categoryVar(cat: ActivityCategory): string {
  return `var(--c-${cat})`;
}

function CategoryIcon({ category }: { category: ActivityCategory }) {
  // 13px single-stroke lucide-style glyphs. `currentColor` lets the orb's
  // `color: var(--cat)` rule drive the stroke.
  const common = {
    width: 13,
    height: 13,
    viewBox: "0 0 16 16",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.5,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };
  switch (category) {
    case "read":
      return (
        <svg {...common}>
          <circle cx="7" cy="7" r="4" />
          <path d="M13.5 13.5 10 10" />
        </svg>
      );
    case "edit":
      return (
        <svg {...common}>
          <path d="M11 2.5 13.5 5 6 12.5l-3 1 1-3z" />
        </svg>
      );
    case "run":
      return (
        <svg {...common}>
          <path d="M3 4l3 4-3 4" />
          <path d="M8 12h5" />
        </svg>
      );
    case "submit":
      return (
        <svg {...common}>
          <path d="M13 3 7 9" />
          <path d="M13 3v4M13 3h-4" />
          <path d="M13 9v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3" />
        </svg>
      );
    case "done":
      return (
        <svg {...common}>
          <path d="M3 8.5 6.5 12 13 4.5" />
        </svg>
      );
    case "error":
      return (
        <svg {...common}>
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      );
    case "think":
    default:
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="2" />
          <circle cx="8" cy="8" r="5" />
        </svg>
      );
  }
}

function Chevron() {
  return (
    <svg
      width={12}
      height={12}
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M4 2.5 8 6 4 9.5" />
    </svg>
  );
}

/**
 * Returns the current wall-clock value, refreshed every `TIMER_TICK_MS`
 * while `live` is true. Only this component re-renders on each tick — the
 * transcript above stays stable.
 */
function useTickingNow(live: boolean, now: () => number): number {
  const [value, setValue] = useState(now);
  useEffect(() => {
    if (!live) return;
    setValue(now());
    const id = window.setInterval(() => setValue(now()), TIMER_TICK_MS);
    return () => window.clearInterval(id);
  }, [live, now]);
  return value;
}

/** Inline activity indicator rendered as the trailing transcript item. */
export function ChatActivityLine({ run, now = Date.now }: ChatActivityLineProps) {
  const { steps, current, state, startedAt } = run;
  const live = state === "live";

  const tickingNow = useTickingNow(live, now);

  const [open, setOpen] = useState(false);
  const toggle = useCallback(() => setOpen((v) => !v), []);

  // Per-spec visibility: visible whenever there's an active step OR the run
  // closed out with at least one historical step (so users can review it).
  if (current === null && steps.length === 0) return null;

  // Drive the colored side strip / orb / verb / timer / sweep.
  // Terminal: the row settles to the `done`/`error` accent regardless of
  // whichever category the last step belonged to.
  const displayCategory: ActivityCategory = current
    ? current.category
    : state === "error"
      ? "error"
      : "done";

  // Total elapsed since the user kicked off this run.
  const lastStep = steps[steps.length - 1];
  const runEndTs = !live ? (lastStep?.endTs ?? lastStep?.startTs ?? startedAt) : tickingNow;
  const elapsedMs = Math.max(0, runEndTs - startedAt);

  // Collapsed-row text: live shows the current step, terminal shows a summary.
  const verb = current ? current.verb : `${steps.length} steps`;
  const detail = current
    ? current.detail
    : steps.length > 0
      ? `${formatStepDur(elapsedMs)} total`
      : null;
  const toolName = current?.toolName ?? null;

  return (
    <div
      className={styles.ai}
      data-testid="chat-activity-line"
      data-live={live ? "true" : "false"}
      data-open={open ? "true" : "false"}
      data-state={state}
      data-category={displayCategory}
      style={{ "--cat": categoryVar(displayCategory) } as React.CSSProperties}
    >
      <button
        type="button"
        className={styles.row}
        onClick={toggle}
        aria-expanded={open}
        aria-label={
          open
            ? "Collapse activity feed"
            : `Expand activity feed (${steps.length} step${steps.length === 1 ? "" : "s"})`
        }
        data-testid="chat-activity-line-toggle"
      >
        <span className={styles.orb} aria-hidden>
          <CategoryIcon category={displayCategory} />
        </span>
        <span className={styles.text} role="status" aria-live="polite">
          <span className={styles.verb} data-testid="chat-activity-line-verb">
            {verb}
          </span>
          {detail !== null ? (
            <span className={styles.detail} data-testid="chat-activity-line-detail">
              {detail}
            </span>
          ) : null}
          {toolName !== null ? (
            <code className={styles.code} data-testid="chat-activity-line-tool">
              {toolName}
            </code>
          ) : null}
        </span>
        <span className={styles.timer} data-testid="chat-activity-line-timer">
          <span className={styles.dot} aria-hidden />
          {formatTimer(elapsedMs)}
        </span>
        <span className={styles.chev} aria-hidden>
          <Chevron />
        </span>
        <span className={styles.sweep} aria-hidden />
      </button>
      {open ? (
        <div className={styles.feed} data-testid="chat-activity-line-feed" role="list">
          {steps.map((step, i) => (
            <FeedRow key={i} step={step} live={live} tickingNow={tickingNow} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

interface FeedRowProps {
  step: ActivityStep;
  live: boolean;
  tickingNow: number;
}

function FeedRow({ step, live, tickingNow }: FeedRowProps) {
  const active = step.endTs === null;
  const stepState = active ? (live ? "active" : "done") : "done";
  const endTs = step.endTs ?? (live ? tickingNow : step.startTs);
  const dur = formatStepDur(Math.max(0, endTs - step.startTs));
  return (
    <div
      className={styles.step}
      role="listitem"
      data-state={stepState}
      data-category={step.category}
      data-testid="chat-activity-line-step"
      style={{ "--sc": categoryVar(step.category) } as React.CSSProperties}
    >
      <span className={styles.node} aria-hidden />
      <span className={styles.lbl}>
        <span className={styles.v}>{step.verb}</span>
        {step.detail !== null ? <span className={styles.d}>{step.detail}</span> : null}
        {step.toolName !== null ? <code className={styles.codeSm}>{step.toolName}</code> : null}
      </span>
      <span className={styles.dur}>{dur}</span>
    </div>
  );
}
