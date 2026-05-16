import type { ReactNode } from "react";
import styles from "./Kbd.module.css";

export interface KbdProps {
  className?: string;
  children: ReactNode;
}

export function Kbd({ className, children }: KbdProps) {
  const cls = [styles.kbd, className].filter(Boolean).join(" ");
  return <kbd className={cls}>{children}</kbd>;
}
