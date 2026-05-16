import styles from "./HydraMark.module.css";

export const HYDRA_VARIANTS = [
  "triskele",
  "rings",
  "hex",
  "borromean",
  "coil",
  "trinity",
] as const;

export type HydraVariant = (typeof HYDRA_VARIANTS)[number];

export interface HydraMarkProps {
  variant?: HydraVariant;
  size?: number;
  className?: string;
}

export function HydraMark({ variant = "triskele", size = 18, className }: HydraMarkProps) {
  const v = (HYDRA_VARIANTS as readonly string[]).includes(variant) ? variant : "triskele";
  const cls = [styles.mark, className].filter(Boolean).join(" ");
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 32 32",
    xmlns: "http://www.w3.org/2000/svg",
    "aria-hidden": "true" as const,
    className: cls,
  };

  if (v === "triskele") {
    return (
      <svg {...common} fill="none">
        <g transform="translate(16 16)" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          {[0, 120, 240].map((deg) => (
            <g key={deg} transform={`rotate(${deg})`}>
              <path d="M 0,-12 A 12,12 0 0,1 10.39,6" />
              <circle cx="0" cy="-12" r="2.2" fill="currentColor" stroke="none" />
            </g>
          ))}
          <circle cx="0" cy="0" r="1.8" fill="currentColor" stroke="none" />
        </g>
      </svg>
    );
  }

  if (v === "rings") {
    return (
      <svg {...common} fill="none" stroke="currentColor">
        <circle cx="16" cy="16" r="13" strokeWidth="1.6" strokeDasharray="22 6" />
        <circle
          cx="16"
          cy="16"
          r="9"
          strokeWidth="1.6"
          strokeDasharray="14 5"
          transform="rotate(60 16 16)"
        />
        <circle
          cx="16"
          cy="16"
          r="5"
          strokeWidth="1.6"
          strokeDasharray="8 3"
          transform="rotate(-30 16 16)"
        />
        <circle cx="16" cy="3" r="1.6" fill="currentColor" stroke="none" />
      </svg>
    );
  }

  if (v === "hex") {
    return (
      <svg
        {...common}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      >
        <polygon points="16,3 27.3,9.5 27.3,22.5 16,29 4.7,22.5 4.7,9.5" />
        <path d="M 16,3   Q 22,16 27.3,22.5" />
        <path d="M 16,3   Q 10,16 4.7,22.5" />
        <path d="M 4.7,9.5 Q 16,16 27.3,9.5" />
        <circle cx="16" cy="16" r="2" fill="currentColor" stroke="none" />
      </svg>
    );
  }

  if (v === "borromean") {
    return (
      <svg {...common} fill="none" stroke="currentColor" strokeWidth="1.6">
        <circle cx="16" cy="11.5" r="7" />
        <circle cx="11" cy="19.5" r="7" />
        <circle cx="21" cy="19.5" r="7" />
      </svg>
    );
  }

  if (v === "coil") {
    return (
      <svg {...common} fill="none" stroke="currentColor" strokeWidth="1.4">
        <ellipse cx="16" cy="16" rx="12.5" ry="5.5" />
        <ellipse cx="16" cy="16" rx="12.5" ry="5.5" transform="rotate(60 16 16)" />
        <ellipse cx="16" cy="16" rx="12.5" ry="5.5" transform="rotate(120 16 16)" />
        <circle cx="16" cy="16" r="1.5" fill="currentColor" stroke="none" />
      </svg>
    );
  }

  // trinity
  return (
    <svg {...common} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M 7,29 Q 3,21 7,15 Q 11,9 7,5" />
      <path d="M 16,29 Q 12,21 16,15 Q 20,9 16,5" />
      <path d="M 25,29 Q 21,21 25,15 Q 29,9 25,5" />
      <polygon points="7,5 5,3 9,3" fill="currentColor" stroke="none" />
      <polygon points="16,5 14,3 18,3" fill="currentColor" stroke="none" />
      <polygon points="25,5 23,3 27,3" fill="currentColor" stroke="none" />
    </svg>
  );
}
