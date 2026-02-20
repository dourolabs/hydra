import { useEffect, useRef, useState } from "react";
import styles from "./Toast.module.css";

export type ToastVariant = "success" | "error" | "info";

export interface ToastProps {
  message: string;
  variant?: ToastVariant;
  duration?: number;
  onClose?: () => void;
  className?: string;
}

export function Toast({
  message,
  variant = "info",
  duration = 4000,
  onClose,
  className,
}: ToastProps) {
  const [exiting, setExiting] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(null);

  useEffect(() => {
    if (duration > 0) {
      timerRef.current = setTimeout(() => {
        setExiting(true);
      }, duration);
    }
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [duration]);

  const handleAnimationEnd = () => {
    if (exiting && onClose) {
      onClose();
    }
  };

  const cls = [
    styles.toast,
    styles[variant],
    exiting ? styles.exit : styles.enter,
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cls} role="status" onAnimationEnd={handleAnimationEnd}>
      <span className={styles.message}>{message}</span>
      {onClose && (
        <button
          type="button"
          className={styles.close}
          onClick={() => setExiting(true)}
          aria-label="Dismiss"
        >
          &times;
        </button>
      )}
    </div>
  );
}
