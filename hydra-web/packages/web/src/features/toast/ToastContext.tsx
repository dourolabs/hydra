import { useCallback, useState, type ReactNode } from "react";
import { Toast } from "@hydra/ui";
import type { ToastVariant } from "@hydra/ui";
import { ToastContext } from "./toast-state";
import styles from "./ToastContainer.module.css";

interface ToastItem {
  id: number;
  message: string;
  variant: ToastVariant;
  duration: number;
}

let nextId = 0;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const addToast = useCallback(
    (message: string, variant: ToastVariant = "info", duration = 4000) => {
      const id = nextId++;
      setToasts((prev) => [...prev, { id, message, variant, duration }]);
    },
    [],
  );

  const removeToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ addToast }}>
      {children}
      <div className={styles.container}>
        {toasts.map((t) => (
          <Toast
            key={t.id}
            message={t.message}
            variant={t.variant}
            duration={t.duration}
            onClose={() => removeToast(t.id)}
          />
        ))}
      </div>
    </ToastContext.Provider>
  );
}
