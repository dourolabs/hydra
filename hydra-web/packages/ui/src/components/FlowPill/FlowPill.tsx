import { useId } from "react";
import styles from "./FlowPill.module.css";

export type FlowPillPhase = "blocked" | "progress" | "done";

export interface FlowPillProps {
  phase: FlowPillPhase;
  num: number;
  den: number;
  title?: string;
  size?: number;
  "data-testid"?: string;
}

const PHASE_COLOR: Record<FlowPillPhase, string> = {
  blocked: "var(--s-blocked)",
  progress: "var(--acc)",
  done: "var(--s-closed)",
};

function FlowCoin({ phase, level, size }: { phase: FlowPillPhase; level: number; size: number }) {
  const color = PHASE_COLOR[phase];
  const cx = 12;
  const cy = 12;
  const r = 8.2;
  const lv = Math.max(0, Math.min(1, level));
  const fillH = 2 * r * lv;
  const fillY = cy + r - fillH;
  const track = "color-mix(in oklch, var(--fg-2) 24%, transparent)";
  const rawId = useId();
  const id = `fp-${rawId.replace(/:/g, "")}`;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <defs>
        <clipPath id={id}>
          <circle cx={cx} cy={cy} r={r} />
        </clipPath>
      </defs>
      <circle cx={cx} cy={cy} r={r} stroke={track} strokeWidth={1.6} />
      {lv > 0 && (
        <g clipPath={`url(#${id})`}>
          <rect x={cx - r} y={fillY} width={2 * r} height={fillH} fill={color} opacity="0.9" />
          {lv < 1 && (
            <rect
              x={cx - r}
              y={fillY}
              width={2 * r}
              height={1.3}
              fill={`color-mix(in oklch, white 32%, ${color})`}
            />
          )}
        </g>
      )}
    </svg>
  );
}

export function FlowPill({
  phase,
  num,
  den,
  title,
  size = 16,
  "data-testid": testId,
}: FlowPillProps) {
  const level = den > 0 ? num / den : 0;
  return (
    <span
      className={styles.flowpill}
      data-phase={phase}
      title={title}
      data-testid={testId}
    >
      <FlowCoin phase={phase} level={level} size={size} />
      <span className={styles.count}>
        <span className={phase === "blocked" ? styles.num : styles.done}>{num}</span>
        <span className={styles.slash}>/</span>
        <span className={styles.total}>{den}</span>
      </span>
    </span>
  );
}
