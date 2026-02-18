import { Hono } from "hono";
import { getCookie, setCookie, deleteCookie } from "hono/cookie";
import { config } from "./config.js";

export const auth = new Hono();

/**
 * POST /auth/login
 * Accepts { token: string }, validates against metis-server /v1/whoami,
 * and sets an HttpOnly cookie on success.
 */
auth.post("/login", async (c) => {
  const body = await c.req.json<{ token?: string }>();
  const token = body?.token;

  if (!token) {
    return c.json({ error: "token is required" }, 400);
  }

  // Validate the token against metis-server
  const resp = await fetch(`${config.metisServerUrl}/v1/whoami`, {
    headers: { Authorization: `Bearer ${token}` },
  });

  if (!resp.ok) {
    return c.json({ error: "invalid token" }, 401);
  }

  const user = await resp.json();

  setCookie(c, config.cookieName, token, {
    httpOnly: true,
    secure: true,
    sameSite: "Strict",
    path: "/",
  });

  return c.json(user);
});

/**
 * POST /auth/logout
 * Clears the auth cookie.
 */
auth.post("/logout", (c) => {
  deleteCookie(c, config.cookieName, { path: "/" });
  return c.json({ ok: true });
});

/**
 * GET /auth/me
 * Proxies to metis-server /v1/whoami using the token from the cookie.
 */
auth.get("/me", async (c) => {
  const token = getCookie(c, config.cookieName);

  if (!token) {
    return c.json({ error: "not authenticated" }, 401);
  }

  const resp = await fetch(`${config.metisServerUrl}/v1/whoami`, {
    headers: { Authorization: `Bearer ${token}` },
  });

  if (!resp.ok) {
    return c.json({ error: "token expired or invalid" }, 401);
  }

  const user = await resp.json();
  return c.json(user);
});
