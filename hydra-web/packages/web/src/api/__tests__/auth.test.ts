import { describe, it, expect, vi, afterEach } from "vitest";
import { ApiError } from "@hydra/api";
import { isGithubAuthAvailable } from "../auth";
import { apiClient } from "../client";

describe("isGithubAuthAvailable", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns true when the client-id endpoint responds with a payload", async () => {
    vi.spyOn(apiClient, "getGithubAppClientId").mockResolvedValue({
      client_id: "abc123",
    });
    await expect(isGithubAuthAvailable()).resolves.toBe(true);
  });

  it("returns false when the endpoint returns 404", async () => {
    vi.spyOn(apiClient, "getGithubAppClientId").mockRejectedValue(
      new ApiError(404, "not found"),
    );
    await expect(isGithubAuthAvailable()).resolves.toBe(false);
  });

  it("returns false on a network error", async () => {
    vi.spyOn(apiClient, "getGithubAppClientId").mockRejectedValue(
      new TypeError("network down"),
    );
    await expect(isGithubAuthAvailable()).resolves.toBe(false);
  });
});
