import { useEffect, useRef, useMemo } from "react";
import AnsiToHtml from "ansi-to-html";
import styles from "./LogViewer.module.css";

export interface LogViewerProps {
  lines: string[];
  autoScroll?: boolean;
  className?: string;
}

const ansiConverter = new AnsiToHtml({
  fg: "#e0e0e0",
  bg: "transparent",
  newline: false,
  escapeXML: true,
});

export function LogViewer({ lines, autoScroll = true, className }: LogViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);

  const renderedLines = useMemo(() => {
    return lines.map((line) => ansiConverter.toHtml(line));
  }, [lines]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const handleScroll = () => {
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      userScrolledRef.current = !atBottom;
    };

    el.addEventListener("scroll", handleScroll);
    return () => el.removeEventListener("scroll", handleScroll);
  }, []);

  useEffect(() => {
    if (autoScroll && !userScrolledRef.current && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  const cls = [styles.logViewer, className].filter(Boolean).join(" ");

  return (
    <div className={cls} ref={containerRef}>
      <table className={styles.table}>
        <tbody>
          {renderedLines.map((html, i) => (
            <tr key={i} className={styles.row}>
              <td className={styles.lineNumber}>{i + 1}</td>
              <td className={styles.lineContent} dangerouslySetInnerHTML={{ __html: html }} />
            </tr>
          ))}
        </tbody>
      </table>
      {lines.length === 0 && <div className={styles.empty}>No log output</div>}
    </div>
  );
}
