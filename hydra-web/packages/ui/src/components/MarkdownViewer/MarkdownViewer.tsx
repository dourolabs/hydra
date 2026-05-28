import type { ComponentType } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { escapeBareOrderedListMarkers } from "./escapeBareOrderedListMarkers";
import { HYDRA_LINK_URL_PREFIX, remarkHydraLinks } from "./remarkHydraLinks";
import styles from "./MarkdownViewer.module.css";

export interface HydraLinkProps {
  /** The Hydra id, e.g. `i-abc123`. */
  id: string;
  /** The original `[[id]]` text, for plain-text fallbacks. */
  raw: string;
}

export type HydraLinkComponent = ComponentType<HydraLinkProps>;

export interface MarkdownViewerProps {
  content: string;
  className?: string;
  /**
   * Renderer for `[[<hydra-id>]]` tokens. When omitted, matches render as the
   * original `[[id]]` plain text. Inject from `@hydra/web` to wire React
   * Router links and API title lookups without making `@hydra/ui` depend on
   * those packages.
   */
  hydraLinkComponent?: HydraLinkComponent;
}

function urlTransform(url: string): string {
  // Pass through our synthetic `hydra-id:` scheme; sanitize everything else
  // with react-markdown's built-in transform.
  if (url.startsWith(HYDRA_LINK_URL_PREFIX)) return url;
  return defaultUrlTransform(url);
}

export function MarkdownViewer({
  content,
  className,
  hydraLinkComponent: HydraLink,
}: MarkdownViewerProps) {
  const cls = [styles.markdown, className].filter(Boolean).join(" ");

  return (
    <div className={cls}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkHydraLinks]}
        rehypePlugins={[rehypeHighlight]}
        urlTransform={urlTransform}
        components={{
          a: ({ children, href, ...props }) => {
            if (typeof href === "string" && href.startsWith(HYDRA_LINK_URL_PREFIX)) {
              const id = href.slice(HYDRA_LINK_URL_PREFIX.length);
              const raw = `[[${id}]]`;
              if (HydraLink) {
                return <HydraLink id={id} raw={raw} />;
              }
              return <>{raw}</>;
            }
            return (
              <a {...props} href={href} target="_blank" rel="noopener noreferrer">
                {children}
              </a>
            );
          },
        }}
      >
        {escapeBareOrderedListMarkers(content)}
      </ReactMarkdown>
    </div>
  );
}
