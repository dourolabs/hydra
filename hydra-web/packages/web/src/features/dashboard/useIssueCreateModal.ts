import { useContext } from "react";
import {
  IssueCreateModalContext,
  type IssueCreateModalContextValue,
} from "./IssueCreateModalProvider";

export function useIssueCreateModal(): IssueCreateModalContextValue {
  const ctx = useContext(IssueCreateModalContext);
  if (!ctx) {
    throw new Error(
      "useIssueCreateModal must be used within an IssueCreateModalProvider",
    );
  }
  return ctx;
}
