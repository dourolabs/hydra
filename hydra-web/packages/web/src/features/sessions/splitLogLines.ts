/** Split a log chunk into lines, normalizing CRLF and stripping bare CRs. */
export function splitLogLines(text: string): string[] {
  return text.split(/\r?\n/).map((line) => line.replace(/\r/g, ""));
}
