import { useState, useEffect } from "react";
import styles from "./ColorPicker.module.css";

/**
 * Default color palette matching the backend's DEFAULT_COLORS in
 * `hydra-server/src/app/colors.rs`. Re-exported through here so both
 * label and status pickers share the same set per `/designs/per-project-issue-statuses.md`
 * §4 "Frontend display".
 */
export const LABEL_COLOR_PALETTE = [
  "#e74c3c", // red
  "#e67e22", // orange
  "#f1c40f", // yellow
  "#2ecc71", // green
  "#1abc9c", // teal
  "#3498db", // blue
  "#9b59b6", // purple
  "#e91e63", // pink
  "#795548", // brown
  "#607d8b", // blue grey
];

interface ColorPickerProps {
  value: string;
  onChange: (color: string) => void;
  /** Palette of swatch colors. Defaults to `LABEL_COLOR_PALETTE`. */
  palette?: readonly string[];
  /** Show a free-text hex input next to the swatches. Defaults to false. */
  allowCustom?: boolean;
  className?: string;
  "aria-label"?: string;
}

/**
 * A small color palette + optional custom-hex input. Used by the label
 * creator and by the project status editor.
 */
export function ColorPicker({
  value,
  onChange,
  palette = LABEL_COLOR_PALETTE,
  allowCustom = false,
  className,
  "aria-label": ariaLabel,
}: ColorPickerProps) {
  const [custom, setCustom] = useState(value);

  useEffect(() => {
    setCustom(value);
  }, [value]);

  const containerClass = [styles.container, className].filter(Boolean).join(" ");

  return (
    <div className={containerClass} aria-label={ariaLabel}>
      <div className={styles.palette}>
        {palette.map((color) => (
          <button
            key={color}
            type="button"
            className={`${styles.swatch} ${color === value ? styles.swatchSelected : ""}`}
            style={{ backgroundColor: color }}
            onClick={(e) => {
              e.stopPropagation();
              onChange(color);
            }}
            aria-label={`Select color ${color}`}
          />
        ))}
      </div>
      {allowCustom && (
        <span className={styles.customWrapper}>
          <input
            type="text"
            className={styles.customInput}
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            onBlur={() => {
              if (isValidHex(custom)) onChange(custom);
              else setCustom(value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                if (isValidHex(custom)) onChange(custom);
                else setCustom(value);
              }
            }}
            placeholder="#rrggbb"
            aria-label="Custom hex color"
          />
        </span>
      )}
    </div>
  );
}

function isValidHex(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}
