import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { Icons } from "@hydra/ui";
import styles from "./OverflowMenu.module.css";

export interface OverflowMenuItem {
  key: string;
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  testId?: string;
  disabled?: boolean;
}

export interface OverflowMenuProps {
  items: OverflowMenuItem[];
  triggerLabel: string;
  triggerTestId?: string;
  menuTestId?: string;
  className?: string;
}

export function OverflowMenu({
  items,
  triggerLabel,
  triggerTestId,
  menuTestId,
  className,
}: OverflowMenuProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const menuId = useId();

  const close = useCallback((opts?: { restoreFocus?: boolean }) => {
    setOpen(false);
    if (opts?.restoreFocus !== false) {
      triggerRef.current?.focus();
    }
  }, []);

  const openMenu = useCallback((focusIndex = 0) => {
    setOpen(true);
    setActiveIndex(focusIndex);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onDocMouseDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (wrapRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const el = itemRefs.current[activeIndex];
    el?.focus();
  }, [open, activeIndex]);

  const onTriggerKeyDown = (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openMenu(0);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      openMenu(items.length - 1);
    }
  };

  const onMenuKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => (i + 1) % items.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => (i - 1 + items.length) % items.length);
    } else if (e.key === "Home") {
      e.preventDefault();
      setActiveIndex(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setActiveIndex(items.length - 1);
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  const onItemClick = (item: OverflowMenuItem) => {
    if (item.disabled) return;
    item.onSelect();
    close();
  };

  return (
    <div
      className={className ? `${styles.wrap} ${className}` : styles.wrap}
      ref={wrapRef}
    >
      <button
        ref={triggerRef}
        type="button"
        className={styles.trigger}
        aria-label={triggerLabel}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        data-testid={triggerTestId}
        onClick={() => (open ? close({ restoreFocus: false }) : openMenu(0))}
        onKeyDown={onTriggerKeyDown}
      >
        <Icons.IconMore />
      </button>
      {open && (
        <div
          id={menuId}
          role="menu"
          className={styles.menu}
          data-testid={menuTestId}
          onKeyDown={onMenuKeyDown}
          aria-label={triggerLabel}
        >
          {items.map((item, idx) => (
            <button
              key={item.key}
              ref={(el) => {
                itemRefs.current[idx] = el;
              }}
              type="button"
              role="menuitem"
              tabIndex={-1}
              className={styles.menuItem}
              data-active={idx === activeIndex ? "true" : undefined}
              data-testid={item.testId}
              disabled={item.disabled}
              onClick={() => onItemClick(item)}
              onMouseEnter={() => setActiveIndex(idx)}
            >
              {item.icon !== undefined && (
                <span className={styles.menuItemIcon} aria-hidden="true">
                  {item.icon}
                </span>
              )}
              <span>{item.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
