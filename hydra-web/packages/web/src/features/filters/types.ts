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
}

export type FilterDefinitions<TItem> = Record<string, FilterDefinition<TItem>>;
