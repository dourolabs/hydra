import type { ComponentType, ReactNode } from "react";

export type FilterOp = "in" | "not_in";

export type FilterGroup = "properties" | "people" | "context" | "relations";

export type FilterKind = "enum" | "user" | "relation";

export interface Filter {
  _uid: string;
  id: string;
  op: FilterOp;
  values: string[];
}

export interface FilterOption {
  value: string;
  label: string;
  chip: ReactNode;
  render: ReactNode;
  sub?: string;
}

export interface IconProps {
  size?: number;
}

export type FilterIcon = ComponentType<IconProps>;

export interface FilterDefinition<TItem> {
  label: string;
  icon: FilterIcon;
  group: FilterGroup;
  kind: FilterKind;
  entityLabel?: string;
  options: FilterOption[];
  apply: (item: TItem, filter: Filter) => boolean;
  /**
   * When true, the value picker behaves as a radio (picking a value replaces
   * any previously selected value; picking the same value again deselects).
   * Used by filter definitions that map to a server param accepting only one
   * value — e.g., the Issues page's `status` / `type` / `creator` / `assignee`
   * server filters.
   */
  singleSelect?: boolean;
  /**
   * When true, the value picker exposes the `is` / `is not` segmented toggle
   * and the chip renders the op prefix. Defaults to `false` for filter
   * definitions whose backing server param can only express positive
   * membership; future definitions that the server can negate should set
   * `notInSupported: true`.
   */
  notInSupported?: boolean;
}

export type FilterDefinitions<TItem> = Record<string, FilterDefinition<TItem>>;
