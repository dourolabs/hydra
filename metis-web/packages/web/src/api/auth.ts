import { apiFetch } from "./client";

export interface User {
  user_id: string;
  display_name?: string;
}

export function login(token: string): Promise<User> {
  return apiFetch<User>("/auth/login", {
    method: "POST",
    body: JSON.stringify({ token }),
  });
}

export function logout(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>("/auth/logout", {
    method: "POST",
  });
}

export function fetchMe(): Promise<User> {
  return apiFetch<User>("/auth/me");
}
