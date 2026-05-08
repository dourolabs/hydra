import { useEffect, useRef, useMemo, useCallback } from "react";
import AnsiToHtml from "ansi-to-html";
import styles from "./LogViewer.module.css";

export interface LogViewerProps {
  lines: string[];
  autoScroll?: boolean;
  className?: string;
  onAutoScrollChange?: (isAutoScrolling: boolean) => void;
}

const ansiConverter = new AnsiToHtml({
  fg: "#e0e0e0",
  bg: "transparent",
  newline: false,
  escapeXML: true,
});

export function LogViewer({ lines, autoScroll = true, className, onAutoScrollChange }: LogViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);
  const programmaticScrollRef = useRef(false);

  const renderedLines = useMemo(() => {
    return lines.map((line) => ansiConverter.toHtml(line));
  }, [lines]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const handleScroll = () => {
      if (programmaticScrollRef.current) {
        programmaticScrollRef.current = false;
        return;
      }
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      const wasScrolled = userScrolledRef.current;
      userScrolledRef.current = !atBottom;
      if (wasScrolled !== !atBottom) {
        onAutoScrollChange?.(atBottom);
      }
    };

    el.addEventListener("scroll", handleScroll);
    return () => el.removeEventListener("scroll", handleScroll);
  }, [onAutoScrollChange]);

  useEffect(() => {
    if (autoScroll && !userScrolledRef.current && containerRef.current) {
      programmaticScrollRef.current = true;
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  const scrollToBottom = useCallback(() => {
    if (containerRef.current) {
      programmaticScrollRef.current = true;
      userScrolledRef.current = false;
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, []);

  // When autoScroll is turned on externally, scroll to bottom and reset state
  const prevAutoScroll = useRef(autoScroll);
  useEffect(() => {
    if (autoScroll && !prevAutoScroll.current) {
      scrollToBottom();
    }
    prevAutoScroll.current = autoScroll;
  }, [autoScroll, scrollToBottom]);

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
