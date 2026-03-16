// ---------------------------------------------------------------------------
// DOC_PATH_RE — detect document paths in issue text
// ---------------------------------------------------------------------------

export const DOC_PATH_RE = /(?:^|\s)(\/\S+\.md)/gm;

/**
 * Extract all unique document paths from a text string.
 */
export function extractDocumentPaths(text: string): string[] {
  const paths = new Set<string>();
  let match: RegExpExecArray | null;
  // Reset lastIndex since we use the global flag
  DOC_PATH_RE.lastIndex = 0;
  while ((match = DOC_PATH_RE.exec(text)) !== null) {
    paths.add(match[1]);
  }
  return Array.from(paths);
}
