import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";

interface IssueCreateModalContextValue {
  isOpen: boolean;
  open: () => void;
  close: () => void;
}

const IssueCreateModalContext = createContext<IssueCreateModalContextValue | null>(
  null,
);

export function IssueCreateModalProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);
  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  const value = useMemo(
    () => ({ isOpen, open, close }),
    [isOpen, open, close],
  );
  return (
    <IssueCreateModalContext.Provider value={value}>
      {children}
    </IssueCreateModalContext.Provider>
  );
}

export function useIssueCreateModal(): IssueCreateModalContextValue {
  const ctx = useContext(IssueCreateModalContext);
  if (!ctx) {
    throw new Error(
      "useIssueCreateModal must be used within an IssueCreateModalProvider",
    );
  }
  return ctx;
}
