import { describe, it, expect } from "vitest";
import type { SessionSettings } from "@hydra/api";
import { collapseSessionSettings } from "../SessionSettingsFields";

describe("collapseSessionSettings", () => {
  it("collapses an empty payload to undefined so the wire body stays slim", () => {
    expect(collapseSessionSettings({})).toBeUndefined();
  });

  it("collapses a payload of all-null surfaced fields to undefined", () => {
    const empty: SessionSettings = {
      image: null,
      model: null,
      cpu_limit: null,
      memory_limit: null,
      max_retries: null,
      idle_timeout: null,
    };
    expect(collapseSessionSettings(empty)).toBeUndefined();
  });

  it("returns the payload verbatim when a surfaced field is set", () => {
    const value: SessionSettings = { cpu_limit: "500m" };
    expect(collapseSessionSettings(value)).toBe(value);
  });

  it("preserves un-surfaced repo_name when every surfaced field is empty", () => {
    const value: SessionSettings = { repo_name: "dourolabs/hydra" };
    expect(collapseSessionSettings(value)).toBe(value);
  });

  it("preserves un-surfaced remote_url", () => {
    const value: SessionSettings = { remote_url: "https://example.test/repo.git" };
    expect(collapseSessionSettings(value)).toBe(value);
  });

  it("preserves un-surfaced branch", () => {
    const value: SessionSettings = { branch: "main" };
    expect(collapseSessionSettings(value)).toBe(value);
  });

  it("preserves un-surfaced non-empty secrets list", () => {
    const value: SessionSettings = { secrets: ["GITHUB_TOKEN"] };
    expect(collapseSessionSettings(value)).toBe(value);
  });

  it("treats an empty secrets array as empty (collapses to undefined)", () => {
    expect(collapseSessionSettings({ secrets: [] })).toBeUndefined();
  });

  it("preserves the full bundle on a CLI-managed payload that also has a surfaced field", () => {
    const value: SessionSettings = {
      cpu_limit: "500m",
      repo_name: "dourolabs/hydra",
      branch: "main",
      secrets: ["GITHUB_TOKEN"],
    };
    expect(collapseSessionSettings(value)).toBe(value);
  });
});
