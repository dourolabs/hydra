import {
  createContext,
  useCallback,
  useContext,
  useState,
  type ReactNode,
} from "react";
import { Toast } from "@metis/ui";
import type { ToastVariant } from "@metis/ui";
import styles from "./ToastContainer.module.css";

interface ToastItem {
  id: number;
  message: string;
  variant: ToastVariant;
  duration: number;
}

interface ToastContextValue {
  addToast: (message: string, variant?: ToastVariant, duration?: number) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

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

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within ToastProvider");
  }
  return ctx;
}
