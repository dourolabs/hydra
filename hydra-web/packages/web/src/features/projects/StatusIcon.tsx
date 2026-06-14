import type { CSSProperties, FC } from "react";
import { Icons } from "@hydra/ui";

export type StatusPhase =
  | "planning"
  | "active"
  | "review"
  | "exception"
  | "terminal";

type IconComponent = FC<Icons.IconProps>;

export const STATUS_PHASE_MAP: Record<
  string,
  { phase: StatusPhase; Icon: IconComponent }
> = {
  backlog: { phase: "planning", Icon: Icons.IconArchive },
  specification: { phase: "planning", Icon: Icons.IconDoc },
  "in-progress": { phase: "active", Icon: Icons.IconHalfCircle },
  execution: { phase: "active", Icon: Icons.IconPlay },
  preview: { phase: "review", Icon: Icons.IconEye },
  review: { phase: "review", Icon: Icons.IconSearch },
  rework: { phase: "review", Icon: Icons.IconRefresh },
  escalation: { phase: "exception", Icon: Icons.IconAlert },
  complete: { phase: "terminal", Icon: Icons.IconCheck },
  cancelled: { phase: "terminal", Icon: Icons.IconX },
};

export function getStatusPhase(key: string): StatusPhase | undefined {
  return STATUS_PHASE_MAP[key]?.phase;
}

interface StatusIconProps extends Omit<Icons.IconProps, "size" | "color"> {
  statusKey: string;
  color: string;
  size?: number;
}

const DOT_SIZE = 7;

export function StatusIcon({
  statusKey,
  color,
  size = 12,
  style,
  ...rest
}: StatusIconProps) {
  const entry = STATUS_PHASE_MAP[statusKey];
  if (!entry) {
    const dotStyle: CSSProperties = {
      display: "inline-block",
      width: DOT_SIZE,
      height: DOT_SIZE,
      borderRadius: "50%",
      background: color,
      flexShrink: 0,
      ...style,
    };
    return <span style={dotStyle} aria-hidden />;
  }
  const { Icon } = entry;
  return <Icon size={size} style={{ color, ...style }} {...rest} />;
}
