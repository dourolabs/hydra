export const KIND_LABEL = {
  issue: "Issue",
  patch: "Patch",
  document: "Document",
  session: "Session",
  conversation: "Conversation",
} as const;

/** First non-empty line of a multi-line string, trimmed. */
export function firstNonEmptyLine(text: string | null | undefined): string | null {
  if (!text) return null;
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (trimmed) return trimmed;
  }
  return null;
}
