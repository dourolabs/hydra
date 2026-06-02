import type { ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { PreviewCard, type PreviewCardTone } from "@hydra/ui";
import styles from "./previewCards.module.css";

/** Tag inline content as the mono-id treatment so the CSS in
 * `PreviewCard.module.css` (`[data-pc-mono]`) picks up tabular-nums + keep-all. */
export function MonoId({ id }: { id: string }) {
  return <span data-pc-mono="true">{id}</span>;
}

interface NavigatingPreviewCardProps {
  to: string;
  tone: PreviewCardTone;
  topRow: ReactNode;
  title: ReactNode;
  bodyExcerpt?: ReactNode;
  footer?: ReactNode;
  ariaLabel: string;
}

/** Wrap PreviewCard with a react-router navigate handler. */
export function NavigatingPreviewCard(props: NavigatingPreviewCardProps) {
  const navigate = useNavigate();
  const { to, ...rest } = props;
  return <PreviewCard {...rest} onClick={() => navigate(to)} />;
}

interface FallbackPreviewCardProps {
  id: string;
  kindLabel: string;
  to: string;
}

/** Minimal card shown on hook error or 404. Mirrors HydraLink's fallback principle. */
export function FallbackPreviewCard({ id, kindLabel, to }: FallbackPreviewCardProps) {
  return (
    <NavigatingPreviewCard
      to={to}
      tone="neutral"
      topRow={
        <>
          <MonoId id={id} />
          <span className={styles.kindChip}>{kindLabel}</span>
        </>
      }
      title={<span data-pc-mono="true">{id}</span>}
      ariaLabel={`${kindLabel} ${id}`}
    />
  );
}

interface SkeletonPreviewCardProps {
  id: string;
  kindLabel: string;
}

/** Loading skeleton that keeps the chrome stable so layout doesn't shift. */
export function SkeletonPreviewCard({ id, kindLabel }: SkeletonPreviewCardProps) {
  return (
    <PreviewCard
      tone="neutral"
      topRow={
        <>
          <MonoId id={id} />
          <span className={styles.kindChip}>{kindLabel}</span>
        </>
      }
      title={<span className={`${styles.skeletonLine} ${styles.skeletonLineLong}`} aria-hidden="true" />}
      bodyExcerpt={
        <span className={styles.skeletonLine} aria-hidden="true" />
      }
      ariaLabel={`Loading ${kindLabel} ${id}`}
    />
  );
}
