import { useMemo } from "react";
import { useViewerWrap } from "../../hooks/useViewerWrap";
import { IconWrap, IconNoWrap } from "../Icon/Icon";
import styles from "./DiffViewer.module.css";

export interface DiffViewerProps {
  diff: string;
  maxLines?: number;
  className?: string;
}

interface DiffLine {
  type: "addition" | "deletion" | "context" | "header";
  content: string;
}

function parseDiff(diff: string, maxLines?: number): { lines: DiffLine[]; truncated: boolean } {
  const rawLines = diff.split("\n");
  const lines: DiffLine[] = [];
  let count = 0;

  for (const raw of rawLines) {
    if (maxLines !== undefined && count >= maxLines) {
      return { lines, truncated: true };
    }

    if (raw.startsWith("diff --git") || raw.startsWith("index ") || raw.startsWith("---") || raw.startsWith("+++")) {
      lines.push({ type: "header", content: raw });
    } else if (raw.startsWith("@@")) {
      lines.push({ type: "header", content: raw });
    } else if (raw.startsWith("+")) {
      lines.push({ type: "addition", content: raw });
    } else if (raw.startsWith("-")) {
      lines.push({ type: "deletion", content: raw });
    } else {
      lines.push({ type: "context", content: raw });
    }
    count++;
  }

  return { lines, truncated: false };
}

export function DiffViewer({ diff, maxLines, className }: DiffViewerProps) {
  const { lines, truncated } = useMemo(() => parseDiff(diff, maxLines), [diff, maxLines]);
  const [wrap, setWrap] = useViewerWrap("diff");

  if (lines.length === 0) {
    return <p className={styles.empty}>No diff available.</p>;
  }

  return (
    <div className={`${styles.container}${className ? ` ${className}` : ""}`}>
      <div className={styles.toolbar}>
        <button
          type="button"
          className={styles.toolbarButton}
          onClick={() => setWrap(!wrap)}
          aria-pressed={wrap}
          aria-label={wrap ? "Disable line wrap" : "Enable line wrap"}
          title={wrap ? "Disable line wrap" : "Enable line wrap"}
        >
          {wrap ? <IconWrap size={14} /> : <IconNoWrap size={14} />}
        </button>
      </div>
      <pre className={`${styles.diff} ${wrap ? styles.wrap : ""}`}>
        {lines.map((line, i) => (
          <span key={i} className={styles[line.type]}>
            {line.content}
            {"\n"}
          </span>
        ))}
        {truncated && (
          <span className={styles.truncated}>... diff truncated ({maxLines} lines shown)</span>
        )}
      </pre>
    </div>
  );
}
