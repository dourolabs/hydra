import { createContext, useContext } from "react";

export interface ChatCreateModalContextValue {
  isOpen: boolean;
  open: () => void;
  close: () => void;
}

export const ChatCreateModalContext =
  createContext<ChatCreateModalContextValue | null>(null);

export function useChatCreateModal(): ChatCreateModalContextValue {
  const ctx = useContext(ChatCreateModalContext);
  if (!ctx) {
    throw new Error(
      "useChatCreateModal must be used within a ChatCreateModalProvider",
    );
  }
  return ctx;
}
