/** First line of the description, truncated. */
export function descriptionSnippet(desc: string, max = 80): string {
  const line = desc.split("\n")[0].trim();
  if (line.length <= max) return line;
  return line.slice(0, max) + "\u2026";
}
