import type { StatusDefinition } from "@hydra/api";

export interface Change {
  field: string;
  before?: string;
  after?: string;
  value?: string;
  beforeStatus?: StatusDefinition;
  afterStatus?: StatusDefinition;
}
