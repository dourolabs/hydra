import { MarkdownViewer, type MarkdownViewerProps } from "@hydra/ui";
import { HydraLink } from "./HydraLink";

export type MarkdownProps = Omit<MarkdownViewerProps, "hydraLinkComponent">;

/**
 * App-level wrapper around `MarkdownViewer` that wires the `[[<hydra-id>]]`
 * renderer to the React Router / API-aware `HydraLink` component. Prefer
 * this over `MarkdownViewer` everywhere user-authored markdown is rendered.
 */
export function Markdown(props: MarkdownProps) {
  return <MarkdownViewer {...props} hydraLinkComponent={HydraLink} />;
}
