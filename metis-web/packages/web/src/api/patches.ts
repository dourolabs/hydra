import { apiFetch } from "./client";

/** Nested patch data inside a PatchVersionRecord. */
export interface PatchData {
  title: string;
  description: string;
  status: string;
  github?: {
    url?: string;
  };
}

/** Server response shape: versioned record wrapping a Patch. */
export interface PatchVersionRecord {
  patch_id: string;
  version: number;
  timestamp: string;
  patch: PatchData;
}

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
  return apiFetch<PatchVersionRecord>(
    `/api/v1/patches/${encodeURIComponent(patchId)}`,
  );
}
