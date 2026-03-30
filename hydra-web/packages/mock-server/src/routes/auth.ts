import { Hono } from "hono";
import { getCookie, setCookie, deleteCookie } from "hono/cookie";
import { DEV_USERNAME, DEV_GITHUB_USER_ID, DEV_TOKEN } from "../auth.js";
import type {
  WhoAmIResponse,
  UserSummary,
  GithubTokenResponse,
  LoginResponse,
  DeviceStartResponse,
  DevicePollResponse,
} from "@hydra/api";

const COOKIE_NAME = "hydra_token";

/**
 * API-level auth routes under /v1 (token-based, no cookies).
 */
export function createAuthRoutes(): Hono {
  const app = new Hono();

  // POST /v1/login — accepts any body, returns dev token
  app.post("/v1/login", async (c) => {
    const resp: LoginResponse = {
      login_token: DEV_TOKEN,
      user: { username: DEV_USERNAME, github_user_id: DEV_GITHUB_USER_ID },
    };
    return c.json(resp);
  });

  // GET /v1/whoami
  app.get("/v1/whoami", (c) => {
    const resp: WhoAmIResponse = {
      actor: { type: "user", username: DEV_USERNAME },
    };
    return c.json(resp);
  });

  // GET /v1/users/:username
  app.get("/v1/users/:username", (c) => {
    const username = c.req.param("username");
    const resp: UserSummary = {
      username,
      github_user_id: DEV_GITHUB_USER_ID,
    };
    return c.json(resp);
  });

  // GET /v1/github/app/client-id — returns mock client ID (no auth required)
  app.get("/v1/github/app/client-id", (c) => {
    return c.json({ client_id: "mock-github-client-id" });
  });

  // GET /v1/github/token
  app.get("/v1/github/token", (c) => {
    const resp: GithubTokenResponse = {
      github_token: "ghp_mock_token_for_dev",
    };
    return c.json(resp);
  });

  return app;
}

/**
 * BFF-style auth routes under /auth (cookie-based).
 * These mimic the Rust BFF's cookie handling for e2e tests.
 */
export function createBffAuthRoutes(): Hono {
  const app = new Hono();

  // POST /auth/login — validate token, set cookie, return whoami response
  app.post("/login", async (c) => {
    const body = await c.req.json<{ token?: string }>();
    const token = body.token;

    if (!token) {
      return c.json({ error: "token is required" }, 400);
    }

    if (token !== DEV_TOKEN) {
      return c.json({ error: "invalid token" }, 401);
    }

    setCookie(c, COOKIE_NAME, token, {
      httpOnly: true,
      sameSite: "Strict",
      path: "/",
    });

    const resp: WhoAmIResponse = {
      actor: { type: "user", username: DEV_USERNAME },
    };
    return c.json(resp);
  });

  // GET /auth/me — check cookie, return whoami response or 401
  app.get("/me", (c) => {
    const token = getCookie(c, COOKIE_NAME);

    if (!token) {
      return c.json({ error: "not authenticated" }, 401);
    }

    const resp: WhoAmIResponse = {
      actor: { type: "user", username: DEV_USERNAME },
    };
    return c.json(resp);
  });

  // POST /auth/login/device/start — mock device flow start (instant)
  app.post("/login/device/start", (c) => {
    const resp: DeviceStartResponse = {
      device_session_id: "mock-device-session-001",
      user_code: "MOCK-1234",
      verification_uri: "https://github.com/login/device",
      expires_in: 900,
      interval: 1,
    };
    return c.json(resp);
  });

  // POST /auth/login/device/poll — mock device flow poll (instant completion)
  app.post("/login/device/poll", async (c) => {
    const body = await c.req.json<{ device_session_id?: string }>();
    if (!body.device_session_id) {
      return c.json({ error: "device_session_id is required" }, 400);
    }

    // Set auth cookie so the user is logged in after device flow completes
    setCookie(c, COOKIE_NAME, DEV_TOKEN, {
      httpOnly: true,
      sameSite: "Strict",
      path: "/",
    });

    const resp: DevicePollResponse = {
      status: "complete",
      login_token: DEV_TOKEN,
      user: { username: DEV_USERNAME, github_user_id: DEV_GITHUB_USER_ID },
      error: null,
    };
    return c.json(resp);
  });

  // POST /auth/logout — clear cookie, return { ok: true }
  app.post("/logout", (c) => {
    deleteCookie(c, COOKIE_NAME, { path: "/" });
    return c.json({ ok: true });
  });

  return app;
}
