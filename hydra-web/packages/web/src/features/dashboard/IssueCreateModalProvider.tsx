import {
  useCallback,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  IssueCreateModalContext,
  type IssueCreateModalInitial,
} from "./useIssueCreateModal";

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
