import type { ActorIdentity, WhoAmIResponse, DeviceStartResponse, DevicePollResponse } from "@hydra/api";
import { apiFetch } from "./client";

/** Extract a display name from any actor identity. */
export function actorDisplayName(actor: ActorIdentity): string {
  if (actor.type === "user") return actor.username;
  if (actor.type === "adhoc") return actor.session_id;
  return actor.name;
}

/**
 * Render an `ActorIdentity` as a Principal path (`users/<name>` /
 * `agents/<name>`), which is the canonical form Phase 4b uses on the wire
 * (`?assignee=…`, API body fields). Returns `null` for actors that don't
 * map cleanly onto a Principal (adhoc).
 */
export function actorPrincipalPath(actor: ActorIdentity): string | null {
  if (actor.type === "user") return `users/${actor.username}`;
  if (actor.type === "agent") return `agents/${actor.name}`;
  return null;
}

export function logout(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>("/auth/logout", {
    method: "POST",
  });
}

export function fetchMe(): Promise<WhoAmIResponse> {
  return apiFetch<WhoAmIResponse>("/auth/me");
}

export function deviceStart(): Promise<DeviceStartResponse> {
  return apiFetch<DeviceStartResponse>("/auth/login/device/start", {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export function devicePoll(deviceSessionId: string): Promise<DevicePollResponse> {
  return apiFetch<DevicePollResponse>("/auth/login/device/poll", {
    method: "POST",
    body: JSON.stringify({ device_session_id: deviceSessionId }),
  });
}

/** Check if GitHub auth is available by checking for the client-id endpoint. */
export async function isGithubAuthAvailable(): Promise<boolean> {
  try {
    const resp = await fetch("/api/v1/github/app/client-id", { credentials: "include" });
    return resp.ok;
  } catch {
    return false;
  }
}
