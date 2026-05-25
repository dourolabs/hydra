import { describe, it, expect } from "vitest";
import type { ActorId } from "@hydra/api";
import { actorIdDisplayName } from "./actors";

// These fixtures are the *exact* JSON shapes ts-rs promises in
// `packages/api/src/generated/ActorId.ts`. They double as a contract
// pin between the hand-rolled serde impl in
// `hydra-common/src/actor_ref.rs` and the TS binding — if the Rust
// wire form drifts from the binding, this test catches it.
describe("actorIdDisplayName — Phase-1 internally-tagged variants", () => {
  it("renders User as the bare username", () => {
    const id: ActorId = { kind: "user", name: "alice" };
    expect(actorIdDisplayName(id)).toBe("alice");
  });

  it("renders Agent as the bare agent name", () => {
    const id: ActorId = { kind: "agent", name: "swe" };
    expect(actorIdDisplayName(id)).toBe("swe");
  });

  it("renders Adhoc as the session id", () => {
    const id: ActorId = { kind: "adhoc", session_id: "s-abcdef" };
    expect(actorIdDisplayName(id)).toBe("s-abcdef");
  });

  it("renders External as external/<system>/<username>", () => {
    const id: ActorId = {
      kind: "external",
      system: "github",
      username: "jayantk",
    };
    expect(actorIdDisplayName(id)).toBe("external/github/jayantk");
  });
});

describe("actorIdDisplayName — legacy externally-tagged variants", () => {
  it("renders Username", () => {
    const id: ActorId = { Username: "alice" };
    expect(actorIdDisplayName(id)).toBe("alice");
  });

  it("renders Session", () => {
    const id: ActorId = { Session: "s-abcdef" };
    expect(actorIdDisplayName(id)).toBe("s-abcdef");
  });

  it("renders Issue", () => {
    const id: ActorId = { Issue: "i-abcdef" };
    expect(actorIdDisplayName(id)).toBe("i-abcdef");
  });

  it("renders Service", () => {
    const id: ActorId = { Service: "bff" };
    expect(actorIdDisplayName(id)).toBe("bff");
  });
});

describe("actorIdDisplayName — Legacy bare-string fallback", () => {
  it("renders a raw string without throwing on `in`", () => {
    // Rust serializes ActorId::Legacy(raw) as a bare JSON string.
    // The TS binding therefore admits `string` in the union, and we
    // must not run `"X" in id` against a primitive — that would throw
    // TypeError at runtime.
    const id: ActorId = "free-form-legacy-blob";
    expect(actorIdDisplayName(id)).toBe("free-form-legacy-blob");
  });
});

describe("ActorId wire-form contract", () => {
  // JSON.stringify of each variant matches what Rust's hand-rolled
  // Serialize impl emits (see `actor_id_*_serde_round_trip` tests in
  // `hydra-common/src/actor_ref.rs`).
  it("User serializes to {kind, name}", () => {
    const id: ActorId = { kind: "user", name: "alice" };
    expect(JSON.parse(JSON.stringify(id))).toEqual({
      kind: "user",
      name: "alice",
    });
  });

  it("External serializes to {kind, system, username}", () => {
    const id: ActorId = {
      kind: "external",
      system: "github",
      username: "jayantk",
    };
    expect(JSON.parse(JSON.stringify(id))).toEqual({
      kind: "external",
      system: "github",
      username: "jayantk",
    });
  });

  it("Username (legacy) serializes to {Username}", () => {
    const id: ActorId = { Username: "alice" };
    expect(JSON.parse(JSON.stringify(id))).toEqual({ Username: "alice" });
  });
});
