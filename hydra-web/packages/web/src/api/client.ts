import { HydraApiClient, ApiError } from "@hydra/api";

export { ApiError };

/**
 * Shared HydraApiClient instance configured for the BFF proxy.
 * All API calls to /api/v1/* go through this client.
 */
export const apiClient = new HydraApiClient({ baseUrl: "/api" });

/**
 * Low-level fetch wrapper for BFF-specific routes (e.g. /auth/*) that
 * are not part of the hydra-server API and therefore not covered by
 * HydraApiClient.
 */
export async function apiFetch<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  const resp = await fetch(path, {
    ...init,
    credentials: "include",
    headers: {
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });

  if (!resp.ok) {
    const body = await resp.json().catch(() => ({ error: resp.statusText }));
    throw new ApiError(resp.status, body.error ?? resp.statusText);
  }

  return resp.json() as Promise<T>;
}
