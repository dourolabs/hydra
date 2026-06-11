import type { SVGProps } from "react";

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "stroke"> {
  size?: number;
  stroke?: number;
}

function IconBase({
  size = 16,
  stroke = 1.5,
  children,
  ...rest
}: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const IconIssue = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="8" cy="8" r="5.5" />
    <circle cx="8" cy="8" r="1.5" fill="currentColor" stroke="none" />
  </IconBase>
);

export const IconChat = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M2.5 4.5a1.5 1.5 0 0 1 1.5-1.5h8a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5H6.5L4 13.5V11H4a1.5 1.5 0 0 1-1.5-1.5z" />
  </IconBase>
);

export const IconPatch = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="4" cy="3.5" r="1.5" />
    <circle cx="4" cy="12.5" r="1.5" />
    <circle cx="12" cy="8" r="1.5" />
    <path d="M4 5v6" />
    <path d="M5.5 3.5h2.5a3 3 0 0 1 3 3v0" />
  </IconBase>
);

export const IconDoc = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M4 2h5l3 3v9H4z" />
    <path d="M9 2v3h3" />
  </IconBase>
);

export const IconFolder = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1 1.5h5A1.5 1.5 0 0 1 14 6v6.5A1.5 1.5 0 0 1 12.5 14h-9A1.5 1.5 0 0 1 2 12.5z" />
  </IconBase>
);

export const IconAgent = (p: IconProps) => (
  <IconBase {...p}>
    <rect x="3" y="3.5" width="10" height="9" rx="1.5" />
    <circle cx="6" cy="8" r="0.7" fill="currentColor" />
    <circle cx="10" cy="8" r="0.7" fill="currentColor" />
    <path d="M8 2v1.5" />
  </IconBase>
);

export const IconRepo = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M3 3.5v9A1.5 1.5 0 0 0 4.5 14H13V2.5H4.5A1.5 1.5 0 0 0 3 4z" />
    <path d="M5 11h7" />
  </IconBase>
);

export const IconKey = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="5" cy="11" r="2.5" />
    <path d="M6.8 9.2L13 3" />
    <path d="M11 5l1.5 1.5" />
  </IconBase>
);

export const IconSearch = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="7" cy="7" r="4" />
    <path d="M10 10l3 3" />
  </IconBase>
);

export const IconPlus = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M8 3v10M3 8h10" />
  </IconBase>
);

export const IconFilter = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M2.5 3.5h11l-4.2 5v4l-2.6 1v-5z" />
  </IconBase>
);

export const IconSort = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M4 3v10M4 13l-2-2M4 13l2-2" />
    <path d="M12 13V3M12 3l-2 2M12 3l2 2" />
  </IconBase>
);

export const IconSettings = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="8" cy="8" r="1.8" />
    <path d="M8 2v1.5M8 12.5V14M2 8h1.5M12.5 8H14M3.8 3.8l1.1 1.1M11.1 11.1l1.1 1.1M3.8 12.2l1.1-1.1M11.1 4.9l1.1-1.1" />
  </IconBase>
);

export const IconBell = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M4 11V7.5a4 4 0 1 1 8 0V11l1 1.5H3z" />
    <path d="M6.5 13a1.5 1.5 0 0 0 3 0" />
  </IconBase>
);

export const IconChevronRight = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M5 3l5 5-5 5" />
  </IconBase>
);

export const IconChevronDown = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M3 5l5 5 5-5" />
  </IconBase>
);

export const IconBranch = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="4" cy="3" r="1.5" />
    <circle cx="4" cy="13" r="1.5" />
    <circle cx="12" cy="6" r="1.5" />
    <path d="M4 4.5v7" />
    <path d="M4 8a4 4 0 0 0 4-4h2.5" />
  </IconBase>
);

export const IconCheck = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M3 8.5l3 3 7-7" />
  </IconBase>
);

export const IconX = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />
  </IconBase>
);

export const IconPlay = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M4 3l9 5-9 5z" fill="currentColor" />
  </IconBase>
);

export const IconPause = (p: IconProps) => (
  <IconBase {...p}>
    <rect x="4" y="3" width="3" height="10" fill="currentColor" stroke="none" />
    <rect x="9" y="3" width="3" height="10" fill="currentColor" stroke="none" />
  </IconBase>
);

export const IconMenu = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M2.5 4h11M2.5 8h11M2.5 12h11" />
  </IconBase>
);

export const IconMore = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="3.5" cy="8" r="1" fill="currentColor" stroke="none" />
    <circle cx="8" cy="8" r="1" fill="currentColor" stroke="none" />
    <circle cx="12.5" cy="8" r="1" fill="currentColor" stroke="none" />
  </IconBase>
);

export const IconDot = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="8" cy="8" r="2" fill="currentColor" stroke="none" />
  </IconBase>
);

export const IconSpark = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M8 2v4M8 10v4M2 8h4M10 8h4M4 4l2.5 2.5M11.5 11.5L9.5 9.5M4 12l2.5-2.5M11.5 4.5L9.5 6.5" />
  </IconBase>
);

export const IconTime = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="8" cy="8" r="5.5" />
    <path d="M8 5v3l2 1.5" />
  </IconBase>
);

export const IconLink = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M9 7l-2 2" />
    <path d="M7 4.5L8.5 3a2.5 2.5 0 0 1 3.5 3.5L10.5 8" />
    <path d="M9 11.5L7.5 13a2.5 2.5 0 0 1-3.5-3.5L5.5 8" />
  </IconBase>
);

export const IconExternalLink = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M9 3h4v4" />
    <path d="M13 3l-6 6" />
    <path d="M11 9.5v3a1 1 0 0 1-1 1H3.5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1H6.5" />
  </IconBase>
);

export const IconArchive = (p: IconProps) => (
  <IconBase {...p}>
    <rect x="2" y="3" width="12" height="3" rx="0.5" />
    <path d="M3 6v6.5a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V6" />
    <path d="M6.5 9h3" />
  </IconBase>
);

export const IconChart = (p: IconProps) => (
  <IconBase {...p}>
    <path d="M2.5 13.5h11" />
    <path d="M4.5 13.5V9" />
    <path d="M7.5 13.5V5.5" />
    <path d="M10.5 13.5V8" />
    <path d="M13 13.5V3.5" />
  </IconBase>
);

export const IconCoin = (p: IconProps) => (
  <IconBase {...p}>
    <circle cx="8" cy="8" r="5.5" />
    <path d="M8 4.5v7" />
    <path d="M9.75 6.25H7a1.25 1.25 0 0 0 0 2.5h2a1.25 1.25 0 0 1 0 2.5H6" />
  </IconBase>
);
