import { Badge, type BadgeStatus } from "@hydra/ui";
import type { FilterOption } from "../types";

/**
 * Generic enum filter option factory: `tones` maps an enum value to a
 * `BadgeStatus` so the chip and row both render as a colored `<Badge>`.
 *
 * The caller decides the iteration order of `tones` (object key order is
 * preserved as the rendered order).
 */
export function statusOptions<S extends string>(
  tones: Record<S, BadgeStatus>,
  labels?: Partial<Record<S, string>>,
): FilterOption[] {
  return (Object.keys(tones) as S[]).map((value) => {
    const tone = tones[value];
    return {
      value,
      label: labels?.[value] ?? value,
      chip: <Badge status={tone} />,
      render: <Badge status={tone} />,
    };
  });
}
