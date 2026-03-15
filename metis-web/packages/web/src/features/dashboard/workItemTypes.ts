import type {
  IssueSummaryRecord,
  PatchSummaryRecord,
  DocumentSummaryRecord,
} from "@metis/api";

export type WorkItem =
  | {
      kind: "issue";
      id: string;
      data: IssueSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
    }
  | {
      kind: "patch";
      id: string;
      data: PatchSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
      sourceIssueId: string | undefined;
    }
  | {
      kind: "document";
      id: string;
      data: DocumentSummaryRecord;
      lastUpdated: string;
      isTerminal: boolean;
      sourceIssueId: string | undefined;
    };
