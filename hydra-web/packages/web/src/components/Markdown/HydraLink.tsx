import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { hydraIdKind, type HydraIdKind } from "@hydra/api";
import type { HydraLinkProps } from "@hydra/ui";
import { useDocument } from "../../features/documents/useDocument";
import { useIssue } from "../../features/issues/useIssue";
import { usePatch } from "../../features/patches/usePatch";
import { useConversation } from "../../features/chat/useConversations";
import { useLabel } from "../../features/labels/useLabel";
import styles from "./HydraLink.module.css";

const KIND_LABEL: Record<HydraIdKind, string> = {
  issue: "Issue",
  patch: "Patch",
  document: "Document",
  conversation: "Conversation",
  session: "Session",
  label: "Label",
};

function Fallback({ raw }: { raw: string }) {
  return <span>{raw}</span>;
}

function RoutedLink({
  to,
  title,
  children,
}: {
  to: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <Link to={to} title={title} className={styles.link}>
      {children}
    </Link>
  );
}

function IssueLink({ id, raw }: HydraLinkProps) {
  const { data, isError } = useIssue(id);
  if (isError || !data) return <Fallback raw={raw} />;
  const title = data.issue.title || id;
  return (
    <RoutedLink to={`/issues/${id}`} title={`${KIND_LABEL.issue}: ${title}`}>
      {title}
    </RoutedLink>
  );
}

function PatchLink({ id, raw }: HydraLinkProps) {
  const { data, isError } = usePatch(id);
  if (isError || !data) return <Fallback raw={raw} />;
  const title = data.patch.title || id;
  return (
    <RoutedLink to={`/patches/${id}`} title={`${KIND_LABEL.patch}: ${title}`}>
      {title}
    </RoutedLink>
  );
}

function DocumentLink({ id, raw }: HydraLinkProps) {
  const { data, isError } = useDocument(id);
  if (isError || !data) return <Fallback raw={raw} />;
  const title = data.document.title || data.document.path || id;
  return (
    <RoutedLink to={`/documents/${id}`} title={`${KIND_LABEL.document}: ${title}`}>
      {title}
    </RoutedLink>
  );
}

function ConversationLink({ id, raw }: HydraLinkProps) {
  const { data, isError } = useConversation(id);
  if (isError || !data) return <Fallback raw={raw} />;
  const title = data.title || id;
  return (
    <RoutedLink to={`/chat/${id}`} title={`${KIND_LABEL.conversation}: ${title}`}>
      {title}
    </RoutedLink>
  );
}

function SessionLink({ id }: HydraLinkProps) {
  // Sessions have no title field. Route to the session log page (the only
  // standalone session view today) with the id as link text.
  return (
    <RoutedLink to={`/sessions/${id}`} title={`${KIND_LABEL.session}: ${id}`}>
      {id}
    </RoutedLink>
  );
}

function LabelLink({ id, raw }: HydraLinkProps) {
  // Labels have no detail page; render the name as a tooltipped span.
  const { data, isError } = useLabel(id);
  if (isError || !data) return <Fallback raw={raw} />;
  return (
    <span className={styles.label} title={`${KIND_LABEL.label}: ${data.name}`}>
      {data.name}
    </span>
  );
}

/**
 * Render a `[[<hydra-id>]]` token as a titled link to the referenced
 * entity's detail page. Falls back to the literal `[[id]]` while the title
 * query is loading or on error/404. Designed to plug into MarkdownViewer's
 * `hydraLinkComponent` prop.
 */
export function HydraLink({ id, raw }: HydraLinkProps) {
  const kind = hydraIdKind(id);
  switch (kind) {
    case "issue":
      return <IssueLink id={id} raw={raw} />;
    case "patch":
      return <PatchLink id={id} raw={raw} />;
    case "document":
      return <DocumentLink id={id} raw={raw} />;
    case "conversation":
      return <ConversationLink id={id} raw={raw} />;
    case "session":
      return <SessionLink id={id} raw={raw} />;
    case "label":
      return <LabelLink id={id} raw={raw} />;
    case null:
      return <Fallback raw={raw} />;
  }
}
