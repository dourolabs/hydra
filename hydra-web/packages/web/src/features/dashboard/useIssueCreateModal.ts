import { createContext, useContext } from "react";
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

export function useIssueCreateModal(): IssueCreateModalContextValue {
  const ctx = useContext(IssueCreateModalContext);
  if (!ctx) {
    throw new Error(
      "useIssueCreateModal must be used within an IssueCreateModalProvider",
    );
  }
  return ctx;
}
