import { useState, type ReactNode } from "react";
import styles from "./Tooltip.module.css";

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  position?: "top" | "bottom" | "left" | "right";
  className?: string;
}

export function Tooltip({ content, children, position = "top", className }: TooltipProps) {
  const [visible, setVisible] = useState(false);

  return (
    <span
      className={[styles.wrapper, className].filter(Boolean).join(" ")}
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
    >
      {children}
      {visible && (
        <span className={[styles.tooltip, styles[position]].join(" ")} role="tooltip">
          {content}
        </span>
      )}
    </span>
  );
}
