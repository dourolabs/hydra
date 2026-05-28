import { describe, it, expect } from "vitest";
import { HYDRA_ID_PREFIXES } from "@hydra/api";
import { HYDRA_ID_REGEX } from "@hydra/ui";

// `@hydra/ui` intentionally has no `@hydra/api` dependency, so the
// `[ipdcsl]-` character class inside `HYDRA_ID_REGEX` is a hand-maintained
// mirror of the prefixes declared in `HYDRA_ID_PREFIXES`. This test fails
// loudly if a new kind is added to `@hydra/api` without the corresponding
// update to the markdown regex, which is the lightest possible coupling
// between the two packages.
describe("HYDRA_ID_REGEX vs HYDRA_ID_PREFIXES contract", () => {
  for (const [kind, prefix] of Object.entries(HYDRA_ID_PREFIXES)) {
    it(`recognizes the ${kind} prefix ("${prefix}")`, () => {
      const sample = `[[${prefix}abcd]]`;
      HYDRA_ID_REGEX.lastIndex = 0;
      const match = HYDRA_ID_REGEX.exec(sample);
      expect(match, `expected HYDRA_ID_REGEX to match ${sample}`).not.toBeNull();
      expect(match![1]).toBe(`${prefix}abcd`);
    });
  }
});
