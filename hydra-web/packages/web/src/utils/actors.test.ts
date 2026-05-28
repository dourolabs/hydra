import { describe, it, expect } from "vitest";
import type { ActorId } from "@hydra/api";
import { actorIdDisplayName } from "./actors";

// These fixtures are the *exact* JSON shapes ts-rs promises in
// `packages/api/src/generated/ActorId.ts`. They double as a contract
// pin between the hand-rolled serde impl in
// `hydra-common/src/actor_ref.rs` and the TS binding — if the Rust
// wire form drifts from the binding, this test catches it.
describe("actorIdDisplayName — externally-tagged variants", () => {
  it("renders User as the bare username", () => {
    const id: ActorId = { User: { name: "alice" } };
    expect(actorIdDisplayName(id)).toBe("alice");
  });

  it("renders Agent as the bare agent name", () => {
    const id: ActorId = { Agent: { name: "swe" } };
    expect(actorIdDisplayName(id)).toBe("swe");
  });

  it("renders Adhoc as the session id", () => {
    const id: ActorId = { Adhoc: { session_id: "s-abcdef" } };
    expect(actorIdDisplayName(id)).toBe("s-abcdef");
  });

  it("renders External as external/<system>/<username>", () => {
    const id: ActorId = {
      External: { system: "github", username: "jayantk" },
    };
    expect(actorIdDisplayName(id)).toBe("external/github/jayantk");
  });
});

describe("ActorId wire-form contract", () => {
  // JSON.stringify of each variant matches what Rust's hand-rolled
  // Serialize impl emits (see `actor_id_*_serde_round_trip` tests in
  // `hydra-common/src/actor_ref.rs`).
  it("User serializes to {User: {name}}", () => {
    const id: ActorId = { User: { name: "alice" } };
    expect(JSON.parse(JSON.stringify(id))).toEqual({
      User: { name: "alice" },
    });
  });

  it("External serializes to {External: {system, username}}", () => {
    const id: ActorId = {
      External: { system: "github", username: "jayantk" },
    };
    expect(JSON.parse(JSON.stringify(id))).toEqual({
      External: { system: "github", username: "jayantk" },
    });
  });
});
