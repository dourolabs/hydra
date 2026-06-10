import {
  createContext,
  useCallback,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { StatusKey } from "@hydra/api";

export interface IssueCreateModalInitial {
  projectId?: string;
  status?: StatusKey;
}

export interface IssueCreateModalContextValue {
  isOpen: boolean;
  initial: IssueCreateModalInitial | null;
  open: (initial?: IssueCreateModalInitial) => void;
  close: () => void;
}

export const IssueCreateModalContext =
  createContext<IssueCreateModalContextValue | null>(null);

export function IssueCreateModalProvider({ children }: { children: ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);
  const [initial, setInitial] = useState<IssueCreateModalInitial | null>(null);
  const open = useCallback((next?: IssueCreateModalInitial) => {
    setInitial(next ?? null);
    setIsOpen(true);
  }, []);
  const close = useCallback(() => setIsOpen(false), []);
  const value = useMemo(
    () => ({ isOpen, initial, open, close }),
    [isOpen, initial, open, close],
  );
  return (
    <IssueCreateModalContext.Provider value={value}>
      {children}
    </IssueCreateModalContext.Provider>
  );
}
