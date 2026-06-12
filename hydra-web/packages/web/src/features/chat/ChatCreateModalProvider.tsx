import { useCallback, useMemo, useState, type ReactNode } from "react";
import { ChatCreateModalContext } from "./useChatCreateModal";

export function ChatCreateModalProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);
  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  const value = useMemo(
    () => ({ isOpen, open, close }),
    [isOpen, open, close],
  );
  return (
    <ChatCreateModalContext.Provider value={value}>
      {children}
    </ChatCreateModalContext.Provider>
  );
}
