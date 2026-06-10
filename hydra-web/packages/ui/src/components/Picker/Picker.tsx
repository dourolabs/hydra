import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import * as Icons from "../Icon";
import styles from "./Picker.module.css";

export interface PickerProps {
  /** Caption rendered above the trigger pill (e.g. "Assignee"). */
  label: string;
  /** Content rendered inside the trigger pill — typically the current value. */
  value: ReactNode;
  open: boolean;
  onToggle: () => void;
  /** Widens the popover's min-width — useful when rows have rich content. */
  wide?: boolean;
  /**
   * Suppress the visible caption above the trigger pill. The `label` is still
   * forwarded to the trigger's `aria-label` so the dropdown stays accessible.
   */
  hideLabel?: boolean;
  /** Items rendered inside the popover. Use PickerRow for selectable rows. */
  children: ReactNode;
  /** Forwarded to the picker's root element for targeting from tests. */
  "data-testid"?: string;
}

const POP_MAX_HEIGHT = 280;
const POP_GAP = 4;

export function Picker({
  label,
  value,
  open,
  onToggle,
  wide,
  hideLabel,
  children,
  "data-testid": testId,
}: PickerProps) {
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [popStyle, setPopStyle] = useState<CSSProperties | null>(null);

  // Outside-click handler must ignore both the trigger wrap and the portaled popover.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (wrapRef.current?.contains(target)) return;
      if (popRef.current?.contains(target)) return;
      onToggle();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open, onToggle]);

  useLayoutEffect(() => {
    if (!open) {
      setPopStyle(null);
      return;
    }
    const reposition = () => {
      const wrap = wrapRef.current;
      if (!wrap) return;
      const rect = wrap.getBoundingClientRect();
      const viewportH = window.innerHeight;
      const spaceBelow = viewportH - rect.bottom;
      const spaceAbove = rect.top;
      // Flip upward if there's not enough room below AND there's more room above.
      const openUp =
        spaceBelow < POP_MAX_HEIGHT + POP_GAP && spaceAbove > spaceBelow;
      const next: CSSProperties = { left: rect.left };
      if (openUp) {
        next.bottom = viewportH - rect.top + POP_GAP;
      } else {
        next.top = rect.bottom + POP_GAP;
      }
      setPopStyle(next);
    };
    reposition();
    window.addEventListener("resize", reposition);
    // Capture-phase scroll catches ancestor scrolling (e.g. a modal body).
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open]);

  return (
    <div className={styles.picker} ref={wrapRef} data-testid={testId}>
      {!hideLabel && <div className={styles.pickerLabel}>{label}</div>}
      <button
        type="button"
        className={`${styles.pill}${open ? ` ${styles.pillActive}` : ""}`}
        onClick={onToggle}
        aria-expanded={open}
        aria-label={label}
      >
        {value}
        <span className={styles.pillChevron}>
          <Icons.IconChevronDown size={12} />
        </span>
      </button>
      {open &&
        popStyle &&
        createPortal(
          <div
            ref={popRef}
            className={`${styles.pop}${wide ? ` ${styles.popWide}` : ""}`}
            style={popStyle}
          >
            {children}
          </div>,
          document.body,
        )}
    </div>
  );
}

export interface PickerRowProps {
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
  /** Forwarded to the row's `<button>` for targeting from tests. */
  "data-testid"?: string;
}

export function PickerRow({
  active,
  onClick,
  children,
  "data-testid": testId,
}: PickerRowProps) {
  return (
    <button
      type="button"
      className={`${styles.popRow}${active ? ` ${styles.popRowActive}` : ""}`}
      onClick={onClick}
      data-testid={testId}
    >
      {children}
      <span className={styles.popCheck}>
        <Icons.IconCheck size={12} />
      </span>
    </button>
  );
}
