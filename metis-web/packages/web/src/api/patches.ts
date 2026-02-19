import { apiFetch } from "./client";

export interface Patch {
  patch_id: string;
  title: string;
  status: string;
  description: string;
  github?: {
    html_url?: string;
  };
}

export function fetchPatch(patchId: string): Promise<Patch> {
  return apiFetch<Patch>(`/api/v1/patches/${encodeURIComponent(patchId)}`);
}
