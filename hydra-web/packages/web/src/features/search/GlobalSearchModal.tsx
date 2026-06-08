import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { useNavigate } from "react-router-dom";
import { Icons, Kbd } from "@hydra/ui";
import { useGlobalSearch } from "./useGlobalSearch";
import {
  conversationToItem,
  documentToItem,
  issueToItem,
  patchToItem,
  sessionToItem,
  type SearchItem,
  type SearchItemKind,
} from "./searchItems";
import { ProjectChip } from "../projects/ProjectChip";
import { useProjects } from "../projects/useProjects";
import styles from "./GlobalSearchModal.module.css";

interface GlobalSearchModalProps {
  open: boolean;
  onClose: () => void;
}

interface SearchGroup {
  kind: SearchItemKind;
  heading: string;
  items: SearchItem[];
}

const GROUP_HEADINGS: Record<SearchItemKind, string> = {
  issue: "Issues",
  patch: "Patches",
  document: "Documents",
  conversation: "Chats",
  session: "Sessions",
};

const ROW_ICONS: Record<SearchItemKind, React.ComponentType<{ size?: number }>> = {
  issue: Icons.IconIssue,
  patch: Icons.IconPatch,
  document: Icons.IconDoc,
  conversation: Icons.IconChat,
  session: Icons.IconSpark,
};

const DEBOUNCE_MS = 200;

export function GlobalSearchModal({ open, onClose }: GlobalSearchModalProps) {
  const navigate = useNavigate();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const [rawQuery, setRawQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  useEffect(() => {
    if (!open) {
      setRawQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
      return;
    }
    previouslyFocused.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const t = window.setTimeout(() => {
      inputRef.current?.focus();
    }, 0);
    return () => {
      window.clearTimeout(t);
      const prev = previouslyFocused.current;
      previouslyFocused.current = null;
      if (prev && document.contains(prev)) {
        prev.focus();
      }
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const t = window.setTimeout(() => {
      setDebouncedQuery(rawQuery.trim());
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [rawQuery, open]);

  useEffect(() => {
    if (!open) return;
    const handler = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose]);

  const results = useGlobalSearch(debouncedQuery);
  const { data: projects } = useProjects();

  const projectsById = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of projects ?? []) {
      map.set(p.project_id, p.project.key);
    }
    return map;
  }, [projects]);

  const groups = useMemo<SearchGroup[]>(() => {
    const all: SearchGroup[] = [
      {
        kind: "issue",
        heading: GROUP_HEADINGS.issue,
        items: results.issues.map((r) => issueToItem(r, projectsById)),
      },
      {
        kind: "patch",
        heading: GROUP_HEADINGS.patch,
        items: results.patches.map(patchToItem),
      },
      {
        kind: "document",
        heading: GROUP_HEADINGS.document,
        items: results.documents.map(documentToItem),
      },
      {
        kind: "conversation",
        heading: GROUP_HEADINGS.conversation,
        items: results.conversations.map(conversationToItem),
      },
      {
        kind: "session",
        heading: GROUP_HEADINGS.session,
        items: results.sessions.map(sessionToItem),
      },
    ];
    return all.filter((g) => g.items.length > 0);
  }, [results, projectsById]);

  const flatItems = useMemo<SearchItem[]>(
    () => groups.flatMap((g) => g.items),
    [groups],
  );

  useEffect(() => {
    setSelectedIndex(0);
  }, [debouncedQuery, flatItems.length]);

  const goTo = useCallback(
    (item: SearchItem) => {
      onClose();
      navigate(item.href);
    },
    [navigate, onClose],
  );

  const handleInputKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        if (flatItems.length === 0) return;
        setSelectedIndex((prev) => (prev + 1) % flatItems.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        if (flatItems.length === 0) return;
        setSelectedIndex((prev) =>
          prev <= 0 ? flatItems.length - 1 : prev - 1,
        );
      } else if (event.key === "Enter") {
        event.preventDefault();
        const item = flatItems[selectedIndex];
        if (item) goTo(item);
      }
    },
    [flatItems, selectedIndex, goTo],
  );

  const handleBackdropClick = useCallback(
    (event: MouseEvent<HTMLDivElement>) => {
      if (event.target === event.currentTarget) {
        onClose();
      }
    },
    [onClose],
  );

  const handleInputChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      setRawQuery(event.target.value);
    },
    [],
  );

  if (!open) return null;

  const hasQuery = debouncedQuery.length > 0;
  const noResults = hasQuery && !results.isLoading && flatItems.length === 0;

  let flatCursor = 0;

  return (
    <div
      className={styles.backdrop}
      onClick={handleBackdropClick}
      data-testid="global-search-backdrop"
    >
      <div
        className={styles.modal}
        role="dialog"
        aria-modal="true"
        aria-label="Search"
        data-testid="global-search-modal"
      >
        <div className={styles.inputRow}>
          <span className={styles.inputIcon}>
            <Icons.IconSearch size={16} />
          </span>
          <input
            ref={inputRef}
            type="text"
            className={styles.input}
            placeholder="Search issues, patches, docs…"
            value={rawQuery}
            onChange={handleInputChange}
            onKeyDown={handleInputKeyDown}
            data-testid="global-search-input"
            aria-label="Search query"
            aria-autocomplete="list"
            aria-controls="global-search-results"
            autoComplete="off"
            spellCheck={false}
          />
        </div>

        <div
          id="global-search-results"
          className={styles.results}
          data-testid="global-search-results"
          role="listbox"
        >
          {!hasQuery && (
            <div className={styles.hint} data-testid="global-search-empty-hint">
              Type to search…
            </div>
          )}

          {noResults && (
            <div className={styles.hint} data-testid="global-search-no-results">
              No results
            </div>
          )}

          {hasQuery &&
            groups.map((group) => {
              const Icon = ROW_ICONS[group.kind];
              return (
                <div
                  key={group.kind}
                  className={styles.group}
                  data-testid={`global-search-group-${group.kind}`}
                >
                  <div className={styles.groupHeading}>
                    <span>{group.heading}</span>
                    <span className={styles.groupCount}>{group.items.length}</span>
                  </div>
                  <ul className={styles.list}>
                    {group.items.map((item) => {
                      const index = flatCursor++;
                      const selected = index === selectedIndex;
                      return (
                        <li key={`${item.kind}-${item.id}`}>
                          <button
                            type="button"
                            role="option"
                            aria-selected={selected}
                            className={`${styles.row}${selected ? ` ${styles.rowSelected}` : ""}`}
                            onClick={() => goTo(item)}
                            onMouseEnter={() => setSelectedIndex(index)}
                            data-testid={`global-search-row-${item.kind}-${item.id}`}
                            title={item.label}
                          >
                            <span className={styles.rowIcon}>
                              <Icon size={14} />
                            </span>
                            {item.kind === "issue" && item.projectKey && (
                              <ProjectChip
                                projectKey={item.projectKey}
                                className={styles.rowProjectChip}
                                data-testid={`rail-row-project-chip-${item.id}`}
                              />
                            )}
                            <span className={styles.rowLabel}>{item.label}</span>
                            {item.meta && (
                              <span className={styles.rowMeta}>{item.meta}</span>
                            )}
                            <span className={styles.rowGo} aria-hidden="true">
                              <Icons.IconChevronRight size={12} />
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              );
            })}
        </div>

        <div className={styles.footer}>
          <span className={styles.footerItem}>
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> navigate
          </span>
          <span className={styles.footerItem}>
            <Kbd>↵</Kbd> open
          </span>
          <span className={styles.footerItem}>
            <Kbd>esc</Kbd> close
          </span>
        </div>
      </div>
    </div>
  );
}
