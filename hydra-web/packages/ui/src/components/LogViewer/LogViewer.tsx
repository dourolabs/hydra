import { useEffect, useRef, useCallback, type ReactElement } from "react";
import { List, useListRef, useDynamicRowHeight, type RowComponentProps } from "react-window";
import AnsiToHtml from "ansi-to-html";
import { useViewerWrap } from "../../hooks/useViewerWrap";
import { IconWrap, IconNoWrap } from "../Icon/Icon";
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
  wrap: boolean;
}

function LogRow({
  index,
  style,
  getHtml,
  lineNumberWidth,
  wrap,
}: RowComponentProps<RowExtraProps>): ReactElement {
  const html = getHtml(index);
  return (
    <div
      className={`${styles.row}${wrap ? ` ${styles.rowWrap}` : ""}`}
      style={style}
      data-testid="log-row"
    >
      <span
        className={styles.lineNumber}
        style={{ minWidth: lineNumberWidth }}
      >
        {index + 1}
      </span>
      <span
        className={`${styles.lineContent}${wrap ? ` ${styles.lineContentWrap}` : ""}`}
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

  const [wrap, setWrap] = useViewerWrap("log");

  // Dynamic row heights for wrap mode — measures rendered rows. Always
  // instantiated so the hook order is stable, but only passed to <List/>
  // when wrap is on (fixed-height is more efficient otherwise).
  const dynamicRowHeight = useDynamicRowHeight({
    defaultRowHeight: ROW_HEIGHT,
    key: wrap ? "wrap" : "nowrap",
  });

  // Incremental ANSI conversion cache
  const htmlCacheRef = useRef<string[]>([]);

  const getHtml = useCallback((index: number): string => {
    let cache = htmlCacheRef.current;
    // Trim cache synchronously if lines were trimmed (e.g., line cap)
    if (cache.length > lines.length) {
      cache = cache.slice(cache.length - lines.length);
      htmlCacheRef.current = cache;
    }
    if (index < cache.length && cache[index] !== undefined) {
      return cache[index];
    }
    // Strip embedded \r — white-space: pre renders a bare CR as a line break,
    // which would stack visual lines inside a single virtualized row.
    while (cache.length <= index) {
      const raw = lines[cache.length];
      cache.push(ansiConverter.toHtml(raw.replace(/\r/g, "")));
    }
    return cache[index];
  }, [lines]);

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

  const cls = [styles.logViewer, wrap ? styles.wrap : null, className]
    .filter(Boolean)
    .join(" ");

  const toolbar = (
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
  );

  if (lines.length === 0) {
    return (
      <div className={cls}>
        {toolbar}
        <div className={styles.empty}>No log output</div>
      </div>
    );
  }

  return (
    <div className={cls}>
      {toolbar}
      <List<RowExtraProps>
        listRef={listRefObj}
        rowComponent={LogRow}
        rowCount={lines.length}
        rowHeight={wrap ? dynamicRowHeight : ROW_HEIGHT}
        rowProps={{ getHtml, lineNumberWidth, wrap }}
        overscanCount={50}
        className={styles.list}
      />
    </div>
  );
}
