import { useEffect, useRef, useCallback, type ReactElement } from "react";
import { List, useListRef, type RowComponentProps } from "react-window";
import AnsiToHtml from "ansi-to-html";
import styles from "./LogViewer.module.css";

export interface LogViewerProps {
  lines: string[];
  autoScroll?: boolean;
  className?: string;
  onAutoScrollChange?: (isAutoScrolling: boolean) => void;
}

const ROW_HEIGHT = 20;

const ansiConverter = new AnsiToHtml({
  fg: "#e0e0e0",
  bg: "transparent",
  newline: false,
  escapeXML: true,
});

interface RowExtraProps {
  getHtml: (index: number) => string;
  lineNumberWidth: string;
}

function LogRow({
  index,
  style,
  getHtml,
  lineNumberWidth,
}: RowComponentProps<RowExtraProps>): ReactElement {
  const html = getHtml(index);
  return (
    <div className={styles.row} style={style}>
      <span
        className={styles.lineNumber}
        style={{ minWidth: lineNumberWidth }}
      >
        {index + 1}
      </span>
      <span
        className={styles.lineContent}
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </div>
  );
}

export function LogViewer({
  lines,
  autoScroll = true,
  className,
  onAutoScrollChange,
}: LogViewerProps) {
  const listRefObj = useListRef(null);
  const userScrolledRef = useRef(false);
  const programmaticScrollRef = useRef(false);

  // Incremental ANSI conversion cache
  const htmlCacheRef = useRef<string[]>([]);

  const getHtml = useCallback((index: number): string => {
    const cache = htmlCacheRef.current;
    if (index < cache.length && cache[index] !== undefined) {
      return cache[index];
    }
    // Extend cache up to this index
    while (cache.length <= index) {
      cache.push(ansiConverter.toHtml(lines[cache.length]));
    }
    return cache[index];
  }, [lines]);

  // When lines shrink (e.g., line cap trimming), trim the cache from the front
  useEffect(() => {
    if (htmlCacheRef.current.length > lines.length) {
      htmlCacheRef.current = htmlCacheRef.current.slice(
        htmlCacheRef.current.length - lines.length
      );
    }
  }, [lines.length]);

  // Line number width based on total lines
  const lineNumberWidth = lines.length > 0
    ? `${String(lines.length).length + 1}ch`
    : "3ch";

  // Handle scroll events to detect user scroll
  useEffect(() => {
    const el = listRefObj.current?.element;
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
  }, [onAutoScrollChange, lines.length, listRefObj]);

  // Auto-scroll to bottom when new lines arrive
  useEffect(() => {
    if (autoScroll && !userScrolledRef.current && listRefObj.current && lines.length > 0) {
      programmaticScrollRef.current = true;
      listRefObj.current.scrollToRow({ index: lines.length - 1, align: "end" });
    }
  }, [lines.length, autoScroll, listRefObj]);

  // When autoScroll is turned on externally, scroll to bottom and reset state
  const prevAutoScroll = useRef(autoScroll);
  useEffect(() => {
    if (autoScroll && !prevAutoScroll.current) {
      userScrolledRef.current = false;
      if (listRefObj.current && lines.length > 0) {
        programmaticScrollRef.current = true;
        listRefObj.current.scrollToRow({ index: lines.length - 1, align: "end" });
      }
    }
    prevAutoScroll.current = autoScroll;
  }, [autoScroll, lines.length, listRefObj]);

  const cls = [styles.logViewer, className].filter(Boolean).join(" ");

  if (lines.length === 0) {
    return (
      <div className={cls}>
        <div className={styles.empty}>No log output</div>
      </div>
    );
  }

  return (
    <div className={cls}>
      <List<RowExtraProps>
        listRef={listRefObj}
        rowComponent={LogRow}
        rowCount={lines.length}
        rowHeight={ROW_HEIGHT}
        rowProps={{ getHtml, lineNumberWidth }}
        overscanCount={50}
        style={{ height: "100%" }}
      />
    </div>
  );
}
