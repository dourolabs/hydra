import { apiFetch } from "./client";

/**
 * Actor identity as returned by the metis-server /v1/whoami endpoint.
 * Tagged union on the "type" field.
 */
export type ActorIdentity =
  | { type: "user"; username: string }
  | { type: "task"; task_id: string; creator: string };

export interface WhoAmIResponse {
  actor: ActorIdentity;
}

/** Extract a display name from any actor identity. */
export function actorDisplayName(actor: ActorIdentity): string {
  return actor.type === "user" ? actor.username : actor.task_id;
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
