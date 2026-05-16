import type { ReactNode } from "react";
import styles from "./Chip.module.css";

export type ChipTone = "default" | "acc" | "agent" | "muted";

export interface ChipProps {
  tone?: ChipTone;
  mono?: boolean;
  className?: string;
  children: ReactNode;
}

export function Chip({ tone = "default", mono = false, className, children }: ChipProps) {
  const cls = [styles.chip, mono && styles.mono, className].filter(Boolean).join(" ");
  return (
    <span className={cls} data-tone={tone}>
      {children}
    </span>
  );
}
