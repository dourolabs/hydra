import { describe, it, expect } from "vitest";
import { Store, StoreError } from "./store.js";

describe("Store", () => {
  describe("create", () => {
    it("creates an entity with version 1", () => {
      const store = new Store();
      const entry = store.create("items", "id-1", { name: "test" }, "issue");
      expect(entry.version).toBe(1);
      expect(entry.data).toEqual({ name: "test" });
      expect(entry.timestamp).toBeTruthy();
    });

    it("throws 409 if entity already exists", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      expect(() => store.create("items", "id-1", { name: "test2" }, "issue")).toThrow(StoreError);
      try {
        store.create("items", "id-1", { name: "test2" }, "issue");
      } catch (e) {
        expect((e as StoreError).status).toBe(409);
      }
    });
  });

  describe("get", () => {
    it("returns the latest version", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      store.update("items", "id-1", { name: "v2" }, "issue");
      const entry = store.get("items", "id-1");
      expect(entry?.version).toBe(2);
      expect(entry?.data).toEqual({ name: "v2" });
    });

    it("returns null for non-existent entity", () => {
      const store = new Store();
      expect(store.get("items", "nonexistent")).toBeNull();
    });

    it("returns null for deleted entity by default", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      store.delete("items", "id-1", "issue");
      expect(store.get("items", "id-1")).toBeNull();
    });

    it("returns deleted entity when includeDeleted is true", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      store.delete("items", "id-1", "issue");
      const entry = store.get("items", "id-1", true);
      expect(entry).not.toBeNull();
      expect(entry?.deleted).toBe(true);
    });
  });

  describe("update", () => {
    it("bumps version on update", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      const entry = store.update("items", "id-1", { name: "v2" }, "issue");
      expect(entry.version).toBe(2);
    });

    it("throws 404 for non-existent entity", () => {
      const store = new Store();
      expect(() => store.update("items", "nonexistent", { name: "v1" }, "issue")).toThrow(
        StoreError,
      );
    });

    it("throws 404 for deleted entity", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      store.delete("items", "id-1", "issue");
      expect(() => store.update("items", "id-1", { name: "v2" }, "issue")).toThrow(StoreError);
    });
  });

  describe("delete", () => {
    it("marks entity as deleted", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      const entry = store.delete("items", "id-1", "issue");
      expect(entry.deleted).toBe(true);
      expect(entry.version).toBe(2);
    });

    it("throws 404 for non-existent entity", () => {
      const store = new Store();
      expect(() => store.delete("items", "nonexistent", "issue")).toThrow(StoreError);
    });

    it("throws 404 for already deleted entity", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "test" }, "issue");
      store.delete("items", "id-1", "issue");
      expect(() => store.delete("items", "id-1", "issue")).toThrow(StoreError);
    });
  });

  describe("getVersion", () => {
    it("returns specific version", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      store.update("items", "id-1", { name: "v2" }, "issue");
      const v1 = store.getVersion("items", "id-1", 1);
      const v2 = store.getVersion("items", "id-1", 2);
      expect(v1?.data).toEqual({ name: "v1" });
      expect(v2?.data).toEqual({ name: "v2" });
    });

    it("returns null for non-existent version", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      expect(store.getVersion("items", "id-1", 99)).toBeNull();
    });
  });

  describe("list", () => {
    it("returns all non-deleted entities", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "a" }, "issue");
      store.create("items", "id-2", { name: "b" }, "issue");
      store.create("items", "id-3", { name: "c" }, "issue");
      store.delete("items", "id-2", "issue");
      const items = store.list("items");
      expect(items).toHaveLength(2);
      expect(items.map((i) => i.id)).toEqual(["id-1", "id-3"]);
    });

    it("includes deleted entities when includeDeleted is true", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "a" }, "issue");
      store.create("items", "id-2", { name: "b" }, "issue");
      store.delete("items", "id-2", "issue");
      const items = store.list("items", true);
      expect(items).toHaveLength(2);
    });
  });

  describe("listVersions", () => {
    it("returns all versions for an entity", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      store.update("items", "id-1", { name: "v2" }, "issue");
      store.update("items", "id-1", { name: "v3" }, "issue");
      const versions = store.listVersions("items", "id-1");
      expect(versions).toHaveLength(3);
      expect(versions.map((v) => v.version)).toEqual([1, 2, 3]);
    });

    it("returns empty array for non-existent entity", () => {
      const store = new Store();
      expect(store.listVersions("items", "nonexistent")).toEqual([]);
    });
  });

  describe("events", () => {
    it("emits events on mutations", () => {
      const store = new Store();
      const events: unknown[] = [];
      store.subscribe((e) => events.push(e));

      store.create("items", "id-1", { name: "test" }, "issue");
      store.update("items", "id-1", { name: "updated" }, "issue");
      store.delete("items", "id-1", "issue");

      expect(events).toHaveLength(3);
    });

    it("supports unsubscribe", () => {
      const store = new Store();
      const events: unknown[] = [];
      const unsubscribe = store.subscribe((e) => events.push(e));

      store.create("items", "id-1", { name: "test" }, "issue");
      unsubscribe();
      store.create("items", "id-2", { name: "test2" }, "issue");

      expect(events).toHaveLength(1);
    });

    it("returns events since a given ID", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      store.create("items", "id-2", { name: "v2" }, "issue");
      store.update("items", "id-1", { name: "v1-updated" }, "issue");

      const events = store.getEventsSince(1);
      expect(events).toHaveLength(2);
    });

    it("skips event emission when ssePrefix is null", () => {
      const store = new Store();
      const events: unknown[] = [];
      store.subscribe((e) => events.push(e));

      store.create("items", "id-1", { name: "test" }, null);
      store.update("items", "id-1", { name: "updated" }, null);
      store.delete("items", "id-1", null);

      expect(events).toHaveLength(0);
    });
  });

  describe("getCreationTime", () => {
    it("returns the timestamp of the first version", () => {
      const store = new Store();
      store.create("items", "id-1", { name: "v1" }, "issue");
      store.update("items", "id-1", { name: "v2" }, "issue");
      const creationTime = store.getCreationTime("items", "id-1");
      expect(creationTime).toBeTruthy();
      // Creation time should be the timestamp of version 1
      const v1 = store.getVersion("items", "id-1", 1);
      expect(creationTime).toBe(v1?.timestamp);
    });

    it("returns undefined for non-existent entity", () => {
      const store = new Store();
      expect(store.getCreationTime("items", "nonexistent")).toBeUndefined();
    });
  });
});
