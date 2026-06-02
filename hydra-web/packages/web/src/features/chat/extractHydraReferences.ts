/**
 * Extract `[[<hydra-id>]]` references from a chat message body, dropping
 * occurrences inside fenced or inline code spans.
 *
 * The regex shape mirrors `HYDRA_ID_REGEX` in
 * `@hydra/ui/components/MarkdownViewer/remarkHydraLinks.ts`, with one
 * deliberate divergence: the character class excludes `l-`, because labels
 * keep their inline-only tooltip rendering and don't get preview cards.
 *
 * Returns capture-group ids (e.g. `"i-abcd"`) in source order, deduplicated
 * (first occurrence wins).
 */

// Note: character class intentionally drops `l` — see file-level comment.
const REFERENCE_REGEX = /\[\[([ipdcs]-[a-z]{4,12})\]\]/g;

// Fenced code blocks (``` ... ```), including any language hint.
const FENCED_CODE_REGEX = /```[\s\S]*?```/g;

// Double-backtick inline code spans first (so an embedded single backtick
// inside doesn't terminate the match), then single-backtick spans. Both are
// constrained to a single line — multi-line spans aren't valid inline code in
// commonmark anyway.
const INLINE_CODE_DOUBLE_REGEX = /``[^`\n]*``/g;
const INLINE_CODE_SINGLE_REGEX = /`[^`\n]*`/g;

function stripCodeSpans(text: string): string {
  return text
    .replace(FENCED_CODE_REGEX, "")
    .replace(INLINE_CODE_DOUBLE_REGEX, "")
    .replace(INLINE_CODE_SINGLE_REGEX, "");
}

export function extractHydraReferences(text: string): string[] {
  if (!text) return [];
  const stripped = stripCodeSpans(text);
  const seen = new Set<string>();
  const out: string[] = [];
  for (const match of stripped.matchAll(REFERENCE_REGEX)) {
    const id = match[1];
    if (!id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}
