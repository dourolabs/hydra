const ORDERED_LIST_MARKER_RE = /^( {0,3})(\d{1,9})([.)])([ \t]*)$/;
const FENCE_RE = /^ {0,3}([`~]{3,})/;

// CommonMark treats a line containing only `N.` or `N)` (optionally indented
// up to 3 spaces, with trailing whitespace) as an empty ordered-list item,
// which renders as a blank <li>. Escape the marker punctuation so the line
// falls back to plain text. Fenced code blocks are left untouched.
export function escapeBareOrderedListMarkers(content: string): string {
  const lines = content.split("\n");
  let fence: string | null = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fm = FENCE_RE.exec(line);
    if (fm) {
      const ch = fm[1][0];
      if (fence === null) fence = ch;
      else if (fence === ch) fence = null;
      continue;
    }
    if (fence !== null) continue;
    lines[i] = line.replace(
      ORDERED_LIST_MARKER_RE,
      (_, indent: string, num: string, punct: string, trail: string) =>
        `${indent}${num}\\${punct}${trail}`,
    );
  }
  return lines.join("\n");
}
