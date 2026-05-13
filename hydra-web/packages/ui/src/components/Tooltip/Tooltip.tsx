import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import styles from "./Tooltip.module.css";

export type TooltipTrigger = "hover" | "click" | "hover-or-click";

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  position?: "top" | "bottom" | "left" | "right";
  trigger?: TooltipTrigger;
  className?: string;
}

const AUTO_DISMISS_MS = 2000;

export function Tooltip({
  content,
  children,
  position = "top",
  trigger = "hover",
  className,
}: TooltipProps) {
  const [visible, setVisible] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hoverEnabled = trigger === "hover" || trigger === "hover-or-click";
  const clickEnabled = trigger === "click" || trigger === "hover-or-click";

  useEffect(() => {
    if (visible) {
      timerRef.current = setTimeout(() => setVisible(false), AUTO_DISMISS_MS);
    }
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [visible]);

  const handleClick = (e: MouseEvent<HTMLSpanElement>) => {
    if (!clickEnabled) return;
    e.stopPropagation();
    setVisible((v) => !v);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLSpanElement>) => {
    if (!clickEnabled) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      setVisible((v) => !v);
    }
  };

  return (
    <span
      className={[styles.wrapper, className].filter(Boolean).join(" ")}
      onMouseEnter={hoverEnabled ? () => setVisible(true) : undefined}
      onMouseLeave={hoverEnabled ? () => setVisible(false) : undefined}
      onClick={clickEnabled ? handleClick : undefined}
      onKeyDown={clickEnabled ? handleKeyDown : undefined}
      role={clickEnabled ? "button" : undefined}
      tabIndex={clickEnabled ? 0 : undefined}
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
