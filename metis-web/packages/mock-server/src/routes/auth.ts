import { Hono } from "hono";
import { getCookie, setCookie, deleteCookie } from "hono/cookie";
import { DEV_USERNAME, DEV_GITHUB_USER_ID, DEV_TOKEN } from "../auth.js";
import type {
  WhoAmIResponse,
  UserSummary,
  GithubTokenResponse,
  LoginResponse,
} from "@hydra/api";

const COOKIE_NAME = "metis_token";

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

  // POST /auth/logout — clear cookie, return { ok: true }
  app.post("/logout", (c) => {
    deleteCookie(c, COOKIE_NAME, { path: "/" });
    return c.json({ ok: true });
  });

  return app;
}
