import type { ActorIdentity, WhoAmIResponse, DeviceStartResponse, DevicePollResponse } from "@hydra/api";
import { apiFetch } from "./client";

/** Extract a display name from any actor identity. */
export function actorDisplayName(actor: ActorIdentity): string {
  if (actor.type === "user") return actor.username;
  if (actor.type === "session") return actor.session_id;
  if (actor.type === "service") return actor.service_name;
  return actor.issue_id;
}

export function login(token: string): Promise<WhoAmIResponse> {
  return apiFetch<WhoAmIResponse>("/auth/login", {
    method: "POST",
    body: JSON.stringify({ token }),
  });
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
