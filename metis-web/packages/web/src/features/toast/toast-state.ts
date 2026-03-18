import { createContext } from "react";
import type { ToastVariant } from "@hydra/ui";

export interface ToastContextValue {
  addToast: (message: string, variant?: ToastVariant, duration?: number) => void;
}

export const ToastContext = createContext<ToastContextValue | null>(null);
