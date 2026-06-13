import type { ReactNode } from "react";
import { useIsMobile } from "../hooks/useIsMobile";
import styles from "./FloatingActionButton.module.css";

interface FloatingActionButtonProps {
  icon: ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
}

export function FloatingActionButton({
  icon,
  label,
  onClick,
  testId,
}: FloatingActionButtonProps) {
  const isMobile = useIsMobile();
  if (!isMobile) return null;
  return (
    <button
      type="button"
      className={styles.fab}
      onClick={onClick}
      aria-label={label}
      data-testid={testId ?? "floating-action-button"}
    >
      <span className={styles.icon} aria-hidden="true">
        {icon}
      </span>
    </button>
  );
}
