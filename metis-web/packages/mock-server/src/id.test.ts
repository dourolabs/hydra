import { describe, it, expect } from "vitest";
import { generateId } from "./id.js";

describe("generateId", () => {
  it("generates issue IDs with i- prefix", () => {
    const id = generateId("issue");
    expect(id).toMatch(/^i-[a-f0-9]{9}$/);
  });

  it("generates job IDs with t- prefix", () => {
    const id = generateId("job");
    expect(id).toMatch(/^t-[a-f0-9]{9}$/);
  });

  it("generates patch IDs with p- prefix", () => {
    const id = generateId("patch");
    expect(id).toMatch(/^p-[a-f0-9]{9}$/);
  });

  it("generates document IDs with d- prefix", () => {
    const id = generateId("document");
    expect(id).toMatch(/^d-[a-f0-9]{9}$/);
  });

  it("generates unique IDs", () => {
    const ids = new Set<string>();
    for (let i = 0; i < 100; i++) {
      ids.add(generateId("issue"));
    }
    expect(ids.size).toBe(100);
  });
});
