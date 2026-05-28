// Matches `[[<prefix><suffix>]]` where prefix is one of the six registered
// Hydra id prefixes and suffix is 4-12 lowercase ASCII letters (mirrors
// HydraId::validate_str in hydra-common/src/ids.rs).
//
// Anything that does not match this exact shape (including agent-memory
// kebab-case slugs like `[[round-2-acceptance-check]]`) is left as plain
// text by the markdown renderer.
export const HYDRA_ID_REGEX = /\[\[([ipdcsl]-[a-z]{4,12})\]\]/g;

// URL scheme used to mark the synthetic `link` mdast node we emit per match.
// The MarkdownViewer's `a` component override detects this prefix and routes
// rendering through the caller-supplied `hydraLinkComponent`.
export const HYDRA_LINK_URL_PREFIX = "hydra-id:";

interface MdastTextNode {
  type: "text";
  value: string;
}

interface MdastLinkNode {
  type: "link";
  url: string;
  title: null;
  children: MdastTextNode[];
}

interface MdastParent {
  type: string;
  children?: Array<MdastTextNode | MdastLinkNode | MdastParent>;
  value?: string;
}

function splitText(value: string): Array<MdastTextNode | MdastLinkNode> | null {
  HYDRA_ID_REGEX.lastIndex = 0;
  const matches = [...value.matchAll(HYDRA_ID_REGEX)];
  if (matches.length === 0) return null;

  const out: Array<MdastTextNode | MdastLinkNode> = [];
  let cursor = 0;
  for (const match of matches) {
    const start = match.index ?? 0;
    const end = start + match[0].length;
    const id = match[1];
    if (start > cursor) {
      out.push({ type: "text", value: value.slice(cursor, start) });
    }
    out.push({
      type: "link",
      url: `${HYDRA_LINK_URL_PREFIX}${id}`,
      title: null,
      children: [{ type: "text", value: match[0] }],
    });
    cursor = end;
  }
  if (cursor < value.length) {
    out.push({ type: "text", value: value.slice(cursor) });
  }
  return out;
}

function walk(node: MdastParent): void {
  if (!node.children || node.children.length === 0) return;
  const next: Array<MdastTextNode | MdastLinkNode | MdastParent> = [];
  for (const child of node.children) {
    if (child.type === "text" && typeof (child as MdastTextNode).value === "string") {
      const replaced = splitText((child as MdastTextNode).value);
      if (replaced) {
        next.push(...replaced);
        continue;
      }
    } else {
      // mdast `code` and `inlineCode` nodes store their content in `value`
      // rather than `children`, so the recursion never sees them — code
      // blocks remain literal automatically.
      walk(child as MdastParent);
    }
    next.push(child);
  }
  node.children = next;
}

/**
 * Remark plugin that rewrites every `[[<hydra-id>]]` occurrence inside text
 * nodes into a synthetic `link` node. The MarkdownViewer's `a` component
 * override detects the synthetic URL prefix and renders the caller-supplied
 * `hydraLinkComponent` in place of a real anchor.
 */
export function remarkHydraLinks() {
  return (tree: MdastParent) => {
    walk(tree);
  };
}
