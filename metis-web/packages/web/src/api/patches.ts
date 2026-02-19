import type { PatchVersionRecord } from "@metis/api";
import { apiClient } from "./client";

export type { PatchVersionRecord };

/** Flattened patch type used throughout the UI. */
export interface Patch {
  patch_id: string;
  title: string;
  status: string;
  description: string;
  github_url?: string;
}

/** Convert a PatchVersionRecord to the flat Patch type used in the UI. */
export function toPatch(record: PatchVersionRecord): Patch {
  return {
    patch_id: record.patch_id,
    title: record.patch.title,
    status: record.patch.status,
    description: record.patch.description,
    github_url: record.patch.github?.url ?? undefined,
  };
}

export function fetchPatch(patchId: string): Promise<PatchVersionRecord> {
  return apiClient.getPatch(patchId);
}
