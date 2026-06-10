import type { DocumentSummaryRecord, PathChildEntry } from "@hydra/api";

export const INDENT_STEP_PX = 12;

export interface NodeProps {
  entry: PathChildEntry;
  depth: number;
  pathToDoc?: Map<string, DocumentSummaryRecord>;
  pathToDocLoading?: boolean;
}

export function indentStyle(depth: number) {
  return { paddingLeft: `${depth * INDENT_STEP_PX + 8}px` } as const;
}
