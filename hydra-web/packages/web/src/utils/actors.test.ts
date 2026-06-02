import { describe, it, expect } from "vitest";
import type { ActorId, ActorRef } from "@hydra/api";
import { actorIdDisplayName, actorAvatarKind } from "./actors";

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

describe("actorAvatarKind", () => {
  it("classifies authenticated User as human", () => {
    const actor: ActorRef = {
      Authenticated: { actor_id: { User: { name: "alice" } } },
    };
    expect(actorAvatarKind(actor)).toBe("human");
  });

  it("classifies authenticated Agent as agent", () => {
    const actor: ActorRef = {
      Authenticated: { actor_id: { Agent: { name: "swe" } } },
    };
    expect(actorAvatarKind(actor)).toBe("agent");
  });

  it("classifies authenticated Adhoc as agent", () => {
    const actor: ActorRef = {
      Authenticated: { actor_id: { Adhoc: { session_id: "s-abc" } } },
    };
    expect(actorAvatarKind(actor)).toBe("agent");
  });

  it("classifies System workers as agent", () => {
    const actor: ActorRef = {
      System: { worker_name: "reaper", on_behalf_of: null },
    };
    expect(actorAvatarKind(actor)).toBe("agent");
  });

  it("classifies Automation as agent", () => {
    const actor: ActorRef = {
      Automation: { automation_name: "auto-merge", triggered_by: null },
    };
    expect(actorAvatarKind(actor)).toBe("agent");
  });
});
