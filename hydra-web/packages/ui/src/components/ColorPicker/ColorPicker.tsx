import { useState, useEffect } from "react";
import styles from "./ColorPicker.module.css";

export interface ColorPickerProps {
  value: string;
  onChange: (color: string) => void;
  /** Palette of swatch colors. */
  palette: readonly string[];
  /** Show a free-text hex input next to the swatches. Defaults to false. */
  allowCustom?: boolean;
  className?: string;
  "aria-label"?: string;
}

export function ColorPicker({
  value,
  onChange,
  palette,
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
