import { Icons } from "@hydra/ui";
import { shortRelativeTime } from "../../utils/time";
import styles from "./Runtime.module.css";

export type RunTimeStatus = "in_progress" | "failed" | "idle";
export type TimeVisualStyle = "instrument" | "stopwatch" | "bracketed" | "typographic";
export type TimeSize = "sm";

interface RunTimeProps {
  /** Pre-formatted duration text (e.g. "2m 14s", "1h 32m"). */
  value: string;
  /** Drives live/failed/static styling. */
  status?: RunTimeStatus;
  /** Visual treatment — instrument is the pill/dot default; siblings exist for paired
   *  experiments with AgoTime. The style prop must match across a RunTime / AgoTime pair. */
  style?: TimeVisualStyle;
  size?: TimeSize;
}

const RUNTIME_CLASS: Record<TimeVisualStyle, string> = {
  instrument: styles.rtInstrument,
  stopwatch: styles.rtStopwatch,
  bracketed: styles.rtBracketed,
  typographic: styles.rtTypographic,
};

const AGO_CLASS: Record<TimeVisualStyle, string> = {
  instrument: styles.atBare,
  stopwatch: styles.atLoose,
  bracketed: styles.atItalic,
  typographic: styles.atTypographic,
};

function classes(...parts: Array<string | false | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/**
 * RunTime renders a *measurement* — a span of elapsed time, often live.
 * Defaults to the "instrument" pill so it reads as something being measured,
 * not as a static annotation. Pair with AgoTime using the same `style`.
 */
export function RunTime({
  value,
  status = "idle",
  style = "instrument",
  size = "sm",
}: RunTimeProps) {
  const live = status === "in_progress";
  const failed = status === "failed";
  const title = live ? `Running for ${value}` : `Ran for ${value}`;
  const stateClass = classes(live && styles.isLive, failed && styles.isFailed);

  if (style === "stopwatch") {
    return (
      <span
        className={classes(styles.rt, RUNTIME_CLASS.stopwatch, stateClass)}
        data-size={size}
        data-testid={live ? "runtime-active" : "runtime-idle"}
        title={title}
      >
        <span className={styles.rtIcon}>
          <Icons.IconTime size={14} />
        </span>
        <span className={classes(styles.rtNum, styles.mono)}>{value}</span>
      </span>
    );
  }

  if (style === "bracketed") {
    return (
      <span
        className={classes(styles.rt, RUNTIME_CLASS.bracketed, stateClass)}
        data-size={size}
        data-testid={live ? "runtime-active" : "runtime-idle"}
        title={title}
      >
        <span className={styles.rtBracket}>[</span>
        <span className={classes(styles.rtNum, styles.mono)}>{value}</span>
        <span className={styles.rtBracket}>]</span>
      </span>
    );
  }

  if (style === "typographic") {
    return (
      <span
        className={classes(styles.rt, RUNTIME_CLASS.typographic, stateClass)}
        data-size={size}
        data-testid={live ? "runtime-active" : "runtime-idle"}
        title={title}
      >
        <span className={classes(styles.rtNum, styles.mono)}>{value}</span>
      </span>
    );
  }

  return (
    <span
      className={classes(styles.rt, RUNTIME_CLASS.instrument, stateClass)}
      data-size={size}
      data-testid={live ? "runtime-active" : "runtime-idle"}
      title={title}
    >
      <span className={styles.rtDot} aria-hidden="true" />
      <span className={classes(styles.rtNum, styles.mono)}>{value}</span>
    </span>
  );
}

type AgoTimeProps = {
  style?: TimeVisualStyle;
  size?: TimeSize;
  /** Suffix appended after the value. Defaults to "ago"; the component
   *  omits it automatically when the value already reads as a moment
   *  ("now", "just now"). */
  suffix?: string;
} & ({ iso: string | null | undefined; value?: never } | { value: string; iso?: never });

function effectiveSuffix(value: string, suffix: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "now" || normalized === "just now") return "";
  return suffix;
}

/**
 * AgoTime renders a *timestamp* — a single point in time, relative to now.
 * Intentionally quieter than RunTime so the two can sit side-by-side without
 * competing. Pair with RunTime using the same `style`.
 */
export function AgoTime(props: AgoTimeProps) {
  const { style = "instrument", size = "sm", suffix = "ago" } = props;
  const value = "iso" in props && props.iso !== undefined ? shortRelativeTime(props.iso) : props.value!;
  const title = `Last updated ${value} ${suffix}`.trim();
  const tail = effectiveSuffix(value, suffix);

  if (style === "stopwatch") {
    return (
      <span className={classes(styles.at, AGO_CLASS.stopwatch)} data-size={size} title={title}>
        <span className={styles.atTilde}>~</span>
        <span className={styles.atNum}>{value}</span>
      </span>
    );
  }
  if (style === "bracketed") {
    return (
      <span className={classes(styles.at, AGO_CLASS.bracketed)} data-size={size} title={title}>
        {tail ? `${value} ${tail}` : value}
      </span>
    );
  }
  if (style === "typographic") {
    return (
      <span className={classes(styles.at, AGO_CLASS.typographic)} data-size={size} title={title}>
        {tail ? `${value} ${tail}` : value}
      </span>
    );
  }
  return (
    <span className={classes(styles.at, AGO_CLASS.instrument)} data-size={size} title={title}>
      {value}
      {tail && (
        <>
          {" "}
          <span className={styles.atSuffix}>{tail}</span>
        </>
      )}
    </span>
  );
}
